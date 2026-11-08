import Cocoa
import InputMethodKit

/// IMK controller: forwards events to the Rust core and applies its output.
/// No decisions are made here (docs/02-keyboard-io.md).
@objc(InputController)
class InputController: IMKInputController {
    /// Whether the engine currently shows marked text in this client. When
    /// false, the next letter starts a word, so the context is refreshed.
    private var composing = false

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

        if !composing, startsWord(event, chars: chars) {
            sendContext(client)
        }

        let out = Core.shared.key(
            kind: kind,
            keycode: event.keyCode,
            modifiers: UInt32(truncatingIfNeeded: event.modifierFlags.rawValue),
            chars: chars,
            timestamp: event.timestamp)
        apply(out, client: client)
        return out.consumed
    }

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        composing = false
        if let client = sender as? IMKTextInput {
            sendContext(client)
        }
        ModeStatusItem.shared.show(mode: Core.shared.mode, auto: false)
    }

    override func deactivateServer(_ sender: Any!) {
        commitPending(sender)
        super.deactivateServer(sender)
    }

    override func commitComposition(_ sender: Any!) {
        commitPending(sender)
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
        menu.addItem(.separator())
        let open = NSMenuItem(title: "Open data folder", action: #selector(openDataFolder(_:)), keyEquivalent: "")
        open.target = self
        menu.addItem(open)
        let log = NSMenuItem(title: "Show diagnostics log", action: #selector(showDiagnosticsLog(_:)), keyEquivalent: "")
        log.target = self
        menu.addItem(log)
        return menu
    }

    @objc func openDataFolder(_ sender: Any?) {
        NSWorkspace.shared.open(Core.shared.dataDir)
    }

    @objc func showDiagnosticsLog(_ sender: Any?) {
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

    private func commitPending(_ sender: Any!) {
        let out = Core.shared.commitPending(timestamp: ProcessInfo.processInfo.systemUptime)
        if let client = sender as? IMKTextInput {
            apply(out, client: client)
        }
        composing = false
    }

    /// Applies an engine answer: commit first, then marked text, then UI.
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
            Overlay.shared.show(mode: mode ?? Core.shared.mode,
                                manual: out.overlay == UInt8(NIMK_OVERLAY_MANUAL),
                                caret: caret ?? ContextProbe.caretRect(client))
        }
    }
}
