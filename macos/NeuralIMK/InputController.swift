import Cocoa
import InputMethodKit

/// IMK controller: forwards events to the Rust core and applies its output.
/// No decisions are made here (docs/02-keyboard-io.md).
@objc(InputController)
class InputController: IMKInputController {
    /// Whether the engine currently shows marked text in this client. When
    /// false, the next letter starts a word, so the context is refreshed.
    private var composing = false

    /// Text known to precede the caret (see `track`); never trusted blindly.
    private var recent = ""

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
        if !out.consumed { trackPassedKey(event, chars: chars) }
        return out.consumed
    }

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        composing = false
        recent = ""
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
        if let diag = Core.shared.lastReplaceDiagnostic {
            let item = NSMenuItem(title: "Last replace issue: \(diag)", action: nil, keyEquivalent: "")
            item.isEnabled = false
            menu.addItem(item)
        }
        if let bundle = client()?.bundleIdentifier() {
            let on = ReplaceCompat.shared.allows(bundle)
            let toggle = NSMenuItem(title: "Retro-correction in this app: \(on ? "on" : "off")",
                                    action: #selector(toggleRetro(_:)), keyEquivalent: "")
            toggle.target = self
            toggle.representedObject = bundle
            menu.addItem(toggle)
        }
        menu.addItem(.separator())
        let open = NSMenuItem(title: "Open data folder", action: #selector(openDataFolder(_:)), keyEquivalent: "")
        open.target = self
        menu.addItem(open)
        let log = NSMenuItem(title: "Show diagnostics log", action: #selector(showDiagnosticsLog(_:)), keyEquivalent: "")
        log.target = self
        menu.addItem(log)
        return menu
    }

    @objc func toggleRetro(_ sender: NSMenuItem) {
        guard let bundle = sender.representedObject as? String else { return }
        ReplaceCompat.shared.toggle(bundle)
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

    /// Retroactively replaces `delete` UTF-16 units before the caret (ADR 0008).
    ///
    /// Safe-or-skip: the text is touched only if the app is not known to ignore
    /// `replacementRange` and the `delete` units before the caret read back
    /// exactly as the text we ourselves inserted (`recent`). Otherwise nothing is
    /// changed (the engine has already flipped the mode and learned). There is
    /// deliberately no CGEvent-backspace fallback. Returns true when replaced.
    private func applyReplace(_ text: String, delete: Int, client: IMKTextInput) -> Bool {
        let bundle = client.bundleIdentifier() ?? "?"
        guard ReplaceCompat.shared.allows(bundle) else {
            reportReplaceFailure("không sửa được ở app này: \(bundle) is deny-listed or switched off")
            return false
        }
        // Pre-check: where we expect the old text to be.
        let sel = client.selectedRange()
        let units = Array(recent.utf16)
        guard sel.location != NSNotFound, delete > 0, sel.location >= delete, units.count >= delete else {
            reportReplaceFailure("replace skipped (\(bundle)): caret=\(sel.location), delete=\(delete), known=\(units.count)")
            return false
        }
        let expected = String(decoding: units.suffix(delete), as: UTF16.self)
        let range = NSRange(location: sel.location - delete, length: delete)
        var actual = NSRange(location: NSNotFound, length: 0)
        guard let before = client.string(from: range, actualRange: &actual),
              NSEqualRanges(actual, range), before == expected
        else {
            // Stale caret (e.g. Chromium) or user moved: skip, verdict unchanged.
            reportReplaceFailure("replace skipped (\(bundle)): text before caret is not \"\(expected)\"")
            return false
        }
        client.insertText(text, replacementRange: range)
        // Post-check: the range must now hold the new text.
        let after = NSRange(location: range.location, length: text.utf16.count)
        var actual2 = NSRange(location: NSNotFound, length: 0)
        if let got = client.string(from: after, actualRange: &actual2), got == text {
            ReplaceCompat.shared.record(bundle, .ok)
            recent = String(decoding: units.dropLast(delete), as: UTF16.self) + text
            return true
        }
        // The app ignored the range (e.g. appended): never try again here.
        ReplaceCompat.shared.record(bundle, .unsupported)
        reportReplaceFailure("replace post-check failed (\(bundle)): marked unsupported")
        return false
    }

    private func reportReplaceFailure(_ message: String) {
        Core.shared.lastReplaceDiagnostic = message
        Core.shared.log(message)
    }

    /// Tracks the text just before the caret that we know about: our commits
    /// plus the plain characters we let through. Used as the expected text.
    private func track(commit: String) { remember(commit) }

    private func trackPassedKey(_ event: NSEvent, chars: String?) {
        guard event.type == .keyDown else { return }
        if event.keyCode == 51 { // backspace
            if !recent.isEmpty { recent.removeLast() }
        } else if event.modifierFlags.intersection([.command, .control]).isEmpty,
                  let chars, !chars.isEmpty,
                  chars.unicodeScalars.allSatisfy({ $0.value >= 0x20 && !(0xF700...0xF8FF).contains($0.value) }) {
            remember(chars)
        } else {
            recent = "" // navigation, shortcuts, etc.: unknown context
        }
    }

    private func remember(_ s: String) {
        recent += s
        if recent.count > 256 { recent = String(recent.suffix(256)) }
    }

    /// Applies an engine answer: replace first, then commit, marked text, UI.
    private func apply(_ out: CoreOutput, client: IMKTextInput, caret: NSRect? = nil) {
        var replaceSkipped = false
        if let text = out.replaceText {
            replaceSkipped = !applyReplace(text, delete: out.replaceDelete, client: client)
            composing = false
        }
        if let commit = out.commit {
            client.insertText(commit, replacementRange: Self.notFound)
            track(commit: commit)
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
                                skipped: replaceSkipped,
                                caret: caret ?? ContextProbe.caretRect(client))
        }
    }
}
