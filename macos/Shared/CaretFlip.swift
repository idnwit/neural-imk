import Foundation

/// Pure parts of the caret flip (ADR 0015): finding the word at the caret,
/// the terminal line tracker and the terminal keystroke plan. No AppKit or
/// IMK here, so it is unit-tested in macos/tests/app and shared by the IME
/// and the companion app's simulator.
enum CaretFlip {
    /// Non-letters the caret may sit after and still target the word
    /// before them (`tét, |`).
    static let maxGap = 3

    static func isLetter(_ scalar: Unicode.Scalar) -> Bool {
        CharacterSet.letters.contains(scalar)
    }

    static func isLetter(_ c: Character) -> Bool {
        !c.unicodeScalars.isEmpty && c.unicodeScalars.allSatisfy(isLetter)
    }

    /// The letter run containing `caret` (UTF-16 offset into `text`), or
    /// the one ending at most `maxGap` non-letters before it.
    static func wordRange(in text: NSString, caret: Int) -> NSRange? {
        func letter(_ i: Int) -> Bool {
            guard i >= 0, i < text.length, let u = Unicode.Scalar(text.character(at: i)) else { return false }
            return isLetter(u)
        }
        let c = min(max(caret, 0), text.length)
        var lo: Int
        if letter(c) {
            lo = c
        } else if letter(c - 1) {
            lo = c - 1
        } else {
            var i = c - 2
            while i >= c - 1 - maxGap, i >= 0, !letter(i) { i -= 1 }
            guard i >= c - 1 - maxGap, letter(i) else { return nil }
            lo = i
        }
        var hi = lo
        while letter(lo - 1) { lo -= 1 }
        while letter(hi + 1) { hi += 1 }
        return NSRange(location: lo, length: hi - lo + 1)
    }

    /// Same rule on Characters (the line tracker's unit). `nil` when there
    /// is no word or, with unknown edges, the word may continue past them.
    static func wordRange(in chars: [Character], caret: Int, edgesKnown: Bool) -> Range<Int>? {
        func letter(_ i: Int) -> Bool { i >= 0 && i < chars.count && isLetter(chars[i]) }
        let c = min(max(caret, 0), chars.count)
        var lo: Int
        if letter(c) {
            lo = c
        } else if letter(c - 1) {
            lo = c - 1
        } else {
            var i = c - 2
            while i >= c - 1 - maxGap, i >= 0, !letter(i) { i -= 1 }
            guard i >= c - 1 - maxGap, letter(i) else { return nil }
            lo = i
        }
        var hi = lo
        while letter(lo - 1) { lo -= 1 }
        while letter(hi + 1) { hi += 1 }
        if !edgesKnown && (lo == 0 || hi == chars.count - 1) { return nil }
        return lo..<(hi + 1)
    }
}

/// Mirror of a terminal's input line, rebuilt from what the IME sees: the
/// text it inserts and the keys it passes through. Terminals expose no
/// text, so this is the only way to know the word at the caret.
struct TerminalLine: Equatable {
    /// Known text around the caret.
    private(set) var chars: [Character] = []
    /// Caret index in `chars`.
    private(set) var caret = 0
    /// The line is still in step with the terminal.
    private(set) var valid = true
    /// `chars` is the whole line (tracked since a Return), not a window.
    private(set) var edgesKnown = false

    // Virtual keycodes (Carbon `kVK_*`).
    static let kReturn: UInt16 = 36, kEnter: UInt16 = 76, kTab: UInt16 = 48, kEscape: UInt16 = 53
    static let kBackspace: UInt16 = 51, kForwardDelete: UInt16 = 117
    static let kLeft: UInt16 = 123, kRight: UInt16 = 124, kDown: UInt16 = 125, kUp: UInt16 = 126
    static let kHome: UInt16 = 115, kEnd: UInt16 = 119, kPageUp: UInt16 = 116, kPageDown: UInt16 = 121

    // `NSEvent.ModifierFlags` raw bits.
    static let shift: UInt = 1 << 17, control: UInt = 1 << 18, option: UInt = 1 << 19, command: UInt = 1 << 20

    /// Focus came to a new client: the text around the caret is unknown.
    mutating func reset() {
        self = TerminalLine()
    }

    /// The line can no longer be followed until the next Return.
    mutating func invalidate() {
        valid = false
        chars = []
        caret = 0
    }

    /// Text inserted at the caret (IME commits, typed characters).
    mutating func insert(_ text: String) {
        guard valid else { return }
        for ch in text {
            if ch == "\r" || ch == "\n" {
                newLine()
            } else if ch.unicodeScalars.contains(where: { $0.value < 0x20 || $0.value == 0x7f }) {
                invalidate()  // control bytes we did not plan
                return
            } else {
                chars.insert(ch, at: caret)
                caret += 1
            }
        }
    }

    private mutating func newLine() {
        chars = []
        caret = 0
        valid = true
        edgesKnown = true
    }

    private mutating func move(by delta: Int) {
        let to = caret + delta
        if to < 0 || to > chars.count {
            // Past the known text: fine only at a real line edge.
            if !edgesKnown { invalidate() }
            return
        }
        caret = to
    }

    /// A key the IME passed through to the terminal.
    mutating func key(keycode: UInt16, modifiers: UInt, chars typed: String?) {
        let chord = modifiers & (Self.control | Self.command | Self.option)
        switch keycode {
        case Self.kReturn, Self.kEnter:
            newLine()
            return
        case Self.kTab, Self.kEscape, Self.kUp, Self.kDown, Self.kPageUp, Self.kPageDown:
            invalidate()
            return
        default:
            break
        }
        guard valid else { return }
        switch (keycode, chord) {
        case (Self.kBackspace, 0):
            if caret > 0 {
                chars.remove(at: caret - 1)
                caret -= 1
            } else if !edgesKnown {
                invalidate()
            }
        case (Self.kForwardDelete, 0):
            if caret < chars.count {
                chars.remove(at: caret)
            } else if !edgesKnown {
                invalidate()
            }
        case (Self.kLeft, 0):
            move(by: -1)
        case (Self.kRight, 0):
            move(by: 1)
        case (Self.kHome, 0), (Self.kLeft, Self.command):
            edgesKnown ? (caret = 0) : invalidate()
        case (Self.kEnd, 0), (Self.kRight, Self.command):
            edgesKnown ? (caret = chars.count) : invalidate()
        case (_, Self.control) where typed == "\u{01}":  // Ctrl+A
            edgesKnown ? (caret = 0) : invalidate()
        case (_, Self.control) where typed == "\u{05}":  // Ctrl+E
            edgesKnown ? (caret = chars.count) : invalidate()
        case (_, 0):
            if let typed, !typed.isEmpty { insert(typed) }
        default:
            // Option/Ctrl/Cmd chords: word moves, kills, paste... unknown.
            invalidate()
        }
    }

    /// The word at the caret, if the line is known well enough.
    func word() -> (range: Range<Int>, text: String)? {
        guard valid, let r = CaretFlip.wordRange(in: chars, caret: caret, edgesKnown: edgesKnown) else { return nil }
        return (r, String(chars[r]))
    }

    /// The bytes that replace the word at `range` with `new`: move to the
    /// word's end, DEL its characters, type the new word, then move back
    /// over the gap if the caret was after the word (a caret inside the
    /// word ends after it, as in text clients). One element per
    /// keystroke-sized unit, so programs that tell typing from pasting by
    /// chunk size see keystrokes.
    func plan(replacing range: Range<Int>, with new: String) -> [String] {
        let right = "\u{1b}[C", left = "\u{1b}[D", del = "\u{7f}"
        let gap = max(caret - range.upperBound, 0)
        var units: [String] = []
        if caret < range.upperBound {
            units += Array(repeating: right, count: range.upperBound - caret)
        }
        units += Array(repeating: left, count: gap)
        units += Array(repeating: del, count: range.count)
        units.append(new)
        units += Array(repeating: right, count: gap)
        return units
    }

    /// Applies a replacement made with `plan` to the mirror.
    mutating func replace(_ range: Range<Int>, with new: String) {
        let newChars = Array(new)
        let gap = max(caret - range.upperBound, 0)
        chars.replaceSubrange(range, with: newChars)
        caret = range.lowerBound + newChars.count + gap
    }
}

