import Cocoa
import InputMethodKit

/// Caret flip (ADR 0015): finds the word at the caret in the client, asks
/// the engine for its other rendering and edits the client in place. Every
/// step is logged with the prefix "caret-flip".
enum CaretFlipper {
    /// Gap between the keystroke-sized units sent to a terminal.
    private static let unitDelay = 0.012

    private static let notFound = NSRange(location: NSNotFound, length: 0)

    private static func log(_ s: String) { Core.shared.log("caret-flip \(s)") }

    /// A word found in a text client: its range and the text from its start
    /// to the caret (the word plus any gap), in the client's UTF-16 units.
    struct TextTarget {
        let word: String
        let range: NSRange
        let caret: Int
        let gap: String
    }

    /// Tries a caret flip after a hotkey press that found no word. Returns
    /// whether the client was edited; `false` leaves the press's plain mode
    /// switch in force.
    static func run(client: IMKTextInput, line: inout TerminalLine, timestamp: Double) -> Bool {
        let app = client.bundleIdentifier() ?? "?"
        let policy = Core.shared.retroPolicy
        let sel = client.selectedRange()
        if sel.location != NSNotFound {
            guard policy & UInt8(NIMK_RETRO_TEXT) != 0 else { return false }
            guard let target = textTarget(client, selection: sel) else {
                log("[\(app)] text: no word at caret sel=\(sel)")
                return false
            }
            guard let (text, mode) = Core.shared.retroFlip(target.word, timestamp: timestamp) else {
                log("[\(app)] text: nothing to flip for \"\(target.word)\"")
                return false
            }
            if target.caret < NSMaxRange(target.range) {
                // Caret inside the word: replace the word, caret ends after it.
                client.insertText(text, replacementRange: target.range)
            } else {
                let span = NSRange(location: target.range.location, length: target.caret - target.range.location)
                client.insertText(text + target.gap, replacementRange: span)
            }
            log("[\(app)] text: \"\(target.word)\" -> \"\(text)\" at \(target.range)")
            feedback(client: client, text: text, mode: mode)
            return true
        }
        guard policy & UInt8(NIMK_RETRO_TERMINAL) != 0 else { return false }
        guard let (range, word) = line.word() else {
            log("[\(app)] terminal: line unknown (valid=\(line.valid)); press Return to resync")
            return false
        }
        guard let (text, mode) = Core.shared.retroFlip(word, timestamp: timestamp) else {
            log("[\(app)] terminal: nothing to flip for \"\(word)\"")
            return false
        }
        let units = line.plan(replacing: range, with: text)
        line.replace(range, with: text)
        send(units, to: client)
        log("[\(app)] terminal: \"\(word)\" -> \"\(text)\" (\(units.count) units)")
        feedback(client: client, text: text, mode: mode)
        return true
    }

    /// The selected word, or the word at (or just before) the caret, read
    /// through windows that end at the caret: Chromium returns nothing for
    /// ranges past the end of the text.
    static func textTarget(_ client: IMKTextInput, selection sel: NSRange) -> TextTarget? {
        if sel.length > 0, let s = read(client, sel), !s.isEmpty,
           s.unicodeScalars.allSatisfy(CaretFlip.isLetter) {
            return TextTarget(word: s, range: sel, caret: NSMaxRange(sel), gap: "")
        }
        let caret = sel.location
        var before = ""
        for r in [40, 20, 10, 5] {
            let start = max(0, caret - r)
            guard caret > start, let s = read(client, NSRange(location: start, length: caret - start)) else { continue }
            before = s
            break
        }
        var after = ""
        for r in [20, 10, 5, 2, 1] {
            if let s = read(client, NSRange(location: caret, length: r)) {
                after = s
                break
            }
        }
        let text = (before + after) as NSString
        let base = caret - (before as NSString).length
        guard let word = CaretFlip.wordRange(in: text, caret: caret - base) else { return nil }
        let range = NSRange(location: base + word.location, length: word.length)
        let gapStart = NSMaxRange(word)
        let gap = caret - base > gapStart
            ? text.substring(with: NSRange(location: gapStart, length: caret - base - gapStart))
            : ""
        return TextTarget(word: text.substring(with: word), range: range, caret: caret, gap: gap)
    }

    private static func read(_ client: IMKTextInput, _ range: NSRange) -> String? {
        var actual = notFound
        if let s = client.string(from: range, actualRange: &actual), !s.isEmpty { return s }
        if let s = client.attributedSubstring(from: range)?.string, !s.isEmpty { return s }
        return nil
    }

    /// Sends the units one per run-loop step, `unitDelay` apart.
    private static func send(_ units: [String], to client: IMKTextInput) {
        for (i, unit) in units.enumerated() {
            DispatchQueue.main.asyncAfter(deadline: .now() + unitDelay * Double(i)) {
                client.insertText(unit, replacementRange: notFound)
            }
        }
    }

    private static func feedback(client: IMKTextInput, text: String, mode: UInt8) {
        ModeStatusItem.shared.show(mode: Core.shared.mode, auto: false)
        DistributedNotificationCenter.default().postNotificationName(
            Notification.Name("com.giangndm.inputmethod.NeuralIMK.corrected"),
            object: nil, userInfo: nil, deliverImmediately: true)
        let lang = mode == UInt8(NIMK_MODE_VI) ? "VI" : "EN"
        let caret = ContextProbe.caretRect(client)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            Overlay.shared.showText("→ \(lang) \(text)", caret: caret, manual: true)
        }
    }
}
