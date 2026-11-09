import Cocoa
import InputMethodKit

/// IMK controller: forwards events to the Rust core and applies its output.
/// No decisions are made here (docs/02-keyboard-io.md).
@objc(InputController)
class InputController: IMKInputController {
    /// Whether the engine currently shows marked text (a word being composed
    /// or a pending word, ADR 0011). When false, the next letter starts a
    /// word, so the context is refreshed.
    private var composing = false

    /// Commits marked text after a silence (`idle_commit_secs`, ADR 0012).
    private let idleCommit = IdleCommit()

    /// Settles the context of rapid activations; shared because Electron
    /// activates two different clients per click.
    private static let contextDebounce = Debouncer()
    private static let activationDelay = 0.12
    private static let caretRetryDelay = 0.08

    private static let notFound = NSRange(location: NSNotFound, length: 0)

    override func recognizedEvents(_ sender: Any!) -> Int {
        Int(NSEvent.EventTypeMask([.keyDown, .flagsChanged]).rawValue)
    }

    override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
        guard let event, let client = sender as? IMKTextInput else { return false }
        let kind: UInt8
        var chars: String?
        switch event.type {
        case .keyDown:
            kind = UInt8(NIMK_KIND_KEY_DOWN)
            // `characters` raises for flagsChanged; only read it for keyDown.
            chars = event.characters
        case .flagsChanged:
            kind = UInt8(NIMK_KIND_FLAGS_CHANGED)
        default:
            return false
        }

        // A key before the debounced context fired: send it now, so the
        // engine knows the field before the first key.
        let flushed = Self.contextDebounce.flush { sendContext(client) }
        if !flushed, !composing, startsWord(event, chars: chars) {
            sendContext(client)
        }

        let out = Core.shared.key(
            kind: kind,
            keycode: event.keyCode,
            modifiers: UInt32(truncatingIfNeeded: event.modifierFlags.rawValue),
            chars: chars,
            timestamp: event.timestamp)
        apply(out, client: client)
        if event.type == .flagsChanged, let marked = out.marked {
            Self.refreshMarked(marked, client: client)
        }
        armIdleCommit(client)
        return out.consumed
    }

    /// Many clients (Chromium, winit-based terminals) only repaint marked
    /// text while handling a keyDown, so a change made on flagsChanged (the
    /// double-Shift flip) stays invisible until the next key. Re-send it on
    /// the next run-loop turn, after the app has finished the event, and
    /// once more a little later for slow clients.
    private static func refreshMarked(_ marked: String, client: IMKTextInput) {
        let send = {
            client.setMarkedText(marked,
                                 selectionRange: NSRange(location: marked.utf16.count, length: 0),
                                 replacementRange: notFound)
        }
        DispatchQueue.main.async(execute: send)
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(40), execute: send)
        Core.shared.log("flip: deferred marked-text refresh")
    }

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        composing = false
        if let client = sender as? IMKTextInput {
            // A newer activation (or a deactivation) within the window
            // replaces this one.
            Self.contextDebounce.schedule(after: Self.activationDelay) { [weak self] in
                self?.sendContextProbingCaret(client)
            }
        }
        ModeStatusItem.shared.show(mode: Core.shared.mode, auto: false)
    }

    override func deactivateServer(_ sender: Any!) {
        Self.contextDebounce.cancel()
        idleCommit.cancel()
        commitPending(sender)
        super.deactivateServer(sender)
    }

    override func commitComposition(_ sender: Any!) {
        commitPending(sender)
        // IMK calls this on a mouse click: the caret moved without a key.
        Core.shared.noteNavigation()
    }

    override func menu() -> NSMenu! {
        let menu = NSMenu(title: "NeuralIMK")
        let modeName = Core.shared.mode == UInt8(NIMK_MODE_VI) ? "Tiếng Việt (VI)" : "English (EN)"
        let modeItem = NSMenuItem(title: "Mode: \(modeName)", action: nil, keyEquivalent: "")
        modeItem.isEnabled = false
        menu.addItem(modeItem)
        let policy = Core.shared.isLive ? (Core.shared.policyId ?? "?") : "engine unavailable"
        let policyItem = NSMenuItem(title: "Policy: \(policy)", action: nil, keyEquivalent: "")
        policyItem.isEnabled = false
        menu.addItem(policyItem)
        // Model items sit at the top level: IMK does not reliably deliver
        // actions of submenu items (unverified), so none are nested.
        menu.addItem(.separator())
        let active = Core.shared.model
        for kind in [UInt8(NIMK_MODEL_LINEAR), UInt8(NIMK_MODEL_GRU)] {
            var title = "Model: \(Core.modelName(kind))"
            switch Core.shared.checkpointSource(kind) {
            case UInt8(NIMK_CHECKPOINT_USER): title += " (đã học thêm)"
            case UInt8(NIMK_CHECKPOINT_BUNDLED): title += " (gốc)"
            default: title += " (chưa pre-train)"
            }
            if kind == active { title += " ✓" }
            let item = NSMenuItem(title: title, action: #selector(selectModel(_:)), keyEquivalent: "")
            item.target = self
            item.tag = Int(kind)
            item.representedObject = NSNumber(value: kind)
            item.state = kind == active ? .on : .off
            menu.addItem(item)
        }
        let reset = NSMenuItem(title: "Khôi phục model gốc", action: #selector(resetModel(_:)), keyEquivalent: "")
        reset.target = self
        menu.addItem(reset)
        menu.addItem(.separator())
        let reload = NSMenuItem(title: "Reload config", action: #selector(reloadConfig(_:)), keyEquivalent: "")
        reload.target = self
        menu.addItem(reload)
        let open = NSMenuItem(title: "Open data folder", action: #selector(openDataFolder(_:)), keyEquivalent: "")
        open.target = self
        menu.addItem(open)
        let log = NSMenuItem(title: "Show diagnostics log", action: #selector(showDiagnosticsLog(_:)), keyEquivalent: "")
        log.target = self
        menu.addItem(log)
        return menu
    }

    /// The menu item behind an action's `sender`. IMK passes a dictionary
    /// holding the item under `kIMKCommandMenuItemName`, not the item itself.
    private func menuItem(from sender: Any?) -> NSMenuItem? {
        if let item = sender as? NSMenuItem { return item }
        if let dict = sender as? [AnyHashable: Any] {
            return dict[kIMKCommandMenuItemName] as? NSMenuItem
        }
        return nil
    }

    @objc func selectModel(_ sender: Any?) {
        guard let item = menuItem(from: sender) else {
            Core.shared.log("menu: select model: no menu item in sender \(String(describing: sender))")
            return
        }
        let kind = (item.representedObject as? NSNumber).map { UInt8(truncatingIfNeeded: $0.intValue) }
            ?? UInt8(truncatingIfNeeded: item.tag)
        Core.shared.log("menu: select model \(Core.modelName(kind))")
        guard kind != Core.shared.model else { return }
        let out = Core.shared.setModel(kind, timestamp: ProcessInfo.processInfo.systemUptime)
        let client = self.client()
        if let client { apply(out, client: client) }
        ModeStatusItem.shared.show(mode: Core.shared.mode, auto: false)
        let caret = client.map { ContextProbe.caretRect($0) } ?? .zero
        Overlay.shared.showText(Core.modelName(Core.shared.model), caret: caret)
    }

    @objc func resetModel(_ sender: Any?) {
        Core.shared.log("menu: reset model \(Core.modelName(Core.shared.model))")
        let out = Core.shared.resetModel(timestamp: ProcessInfo.processInfo.systemUptime)
        if let client = self.client() { apply(out, client: client) }
        ModeStatusItem.shared.show(mode: Core.shared.mode, auto: false)
        let caret = client().map { ContextProbe.caretRect($0) } ?? .zero
        Overlay.shared.showText("Đã khôi phục \(Core.modelName(Core.shared.model))", caret: caret)
    }

    @objc func reloadConfig(_ sender: Any?) {
        Core.shared.log("menu: reload config")
        let rejected = Core.shared.reloadConfig()
        let caret = client().map { ContextProbe.caretRect($0) } ?? .zero
        Overlay.shared.showText(rejected == 0 ? "Đã nạp config" : "Config: \(rejected) giá trị bị bỏ", caret: caret)
    }

    @objc func openDataFolder(_ sender: Any?) {
        Core.shared.log("menu: open data folder")
        NSWorkspace.shared.open(Core.shared.dataDir)
    }

    @objc func showDiagnosticsLog(_ sender: Any?) {
        Core.shared.log("menu: show diagnostics log")
        Core.shared.drainDiagnostics()
        let url = Core.shared.logURL
        if !FileManager.default.fileExists(atPath: url.path) {
            Core.shared.log("log created")
        }
        NSWorkspace.shared.open(url)
    }

    // MARK: - Helpers

    /// A plain letter (no Cmd/Ctrl/Opt) begins a new word.
    private func startsWord(_ event: NSEvent, chars: String?) -> Bool {
        guard event.type == .keyDown,
              event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
              let first = chars?.unicodeScalars.first
        else { return false }
        return CharacterSet.letters.contains(first)
    }

    /// Activation path: when the caret rect is still zero (the client is not
    /// laid out yet), probe once more shortly after; if it stays zero the
    /// context goes out without a caret.
    private func sendContextProbingCaret(_ client: IMKTextInput) {
        if ContextProbe.caretRect(client) == .zero {
            Self.contextDebounce.schedule(after: Self.caretRetryDelay) { [weak self] in
                self?.sendContext(client)
            }
        } else {
            sendContext(client)
        }
    }

    private func sendContext(_ client: IMKTextInput) {
        let bundleId = client.bundleIdentifier() ?? ""
        let caret = ContextProbe.caretRect(client)
        let out = Core.shared.context(
            bundleId: bundleId,
            fieldId: ContextProbe.fieldId(client),
            caretRel: ContextProbe.caretRel(caret: caret, bundleId: bundleId),
            timestamp: ProcessInfo.processInfo.systemUptime)
        apply(out, client: client, caret: caret)
    }

    /// (Re)starts the idle-commit timer after a key: it fires only while text
    /// is marked and the active app's `idle_commit_secs` is above zero.
    private func armIdleCommit(_ client: IMKTextInput) {
        idleCommit.update(secs: Core.shared.idleCommitSecs, marked: composing) { [weak self] in
            guard let self else { return }
            let out = Core.shared.commitPending(timestamp: ProcessInfo.processInfo.systemUptime)
            self.apply(out, client: client)
            self.composing = false
        }
    }

    private func commitPending(_ sender: Any!) {
        idleCommit.cancel()
        let out = Core.shared.commitPending(timestamp: ProcessInfo.processInfo.systemUptime)
        if let client = sender as? IMKTextInput {
            apply(out, client: client)
        }
        composing = false
    }

    /// Applies an engine answer: commit, then marked text, then UI.
    private func apply(_ out: CoreOutput, client: IMKTextInput, caret: NSRect? = nil) {
        if let commit = out.commit {
            client.insertText(commit, replacementRange: Self.notFound)
            composing = false
        }
        if let marked = out.marked {
            client.setMarkedText(marked,
                                 selectionRange: NSRange(location: marked.utf16.count, length: 0),
                                 replacementRange: Self.notFound)
            composing = !marked.isEmpty
        }
        let isAuto = out.overlay == UInt8(NIMK_OVERLAY_AUTO)
        let mode = out.mode == UInt8(NIMK_MODE_UNCHANGED) ? nil : out.mode
        if let mode {
            ModeStatusItem.shared.show(mode: mode, auto: isAuto)
        }
        if out.overlay != UInt8(NIMK_OVERLAY_NONE) {
            showOverlay(mode: mode ?? Core.shared.mode,
                        manual: out.overlay == UInt8(NIMK_OVERLAY_MANUAL),
                        client: client, caret: caret)
        }
    }

    /// Shows the mode badge at a freshly probed caret. A zero rect is
    /// probed once more after a short delay; only if it is still zero does
    /// the overlay fall back to the mouse location.
    private func showOverlay(mode: UInt8, manual: Bool, client: IMKTextInput, caret: NSRect?) {
        let first = caret ?? ContextProbe.caretRect(client)
        if first != .zero {
            Overlay.shared.show(mode: mode, manual: manual, caret: first)
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.caretRetryDelay) {
            Overlay.shared.show(mode: mode, manual: manual, caret: ContextProbe.caretRect(client))
        }
    }
}
