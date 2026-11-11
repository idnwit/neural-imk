import Foundation

/// The input line of a minimal terminal, for the companion app's caret-flip
/// simulator: it interprets text the way a shell or a coding agent's prompt
/// does (DEL erases, `ESC [ C` / `ESC [ D` move, Return submits), so the
/// IME's terminal strategy can be tried without a real terminal.
struct FakeTerminal: Equatable {
    private(set) var line: [Character] = []
    private(set) var caret = 0
    /// Submitted lines, oldest first.
    private(set) var history: [String] = []
    /// An escape sequence still being received.
    private var pending = ""

    /// Interprets `text` as received from the keyboard; returns a readable
    /// trace of what it did (`⌫`, `←`, `→`, `⏎`, or the typed text).
    mutating func receive(_ text: String) -> String {
        var trace = ""
        var typed = ""
        func flushTyped() {
            if !typed.isEmpty {
                trace += typed
                typed = ""
            }
        }
        for ch in text {
            if !pending.isEmpty || ch == "\u{1b}" {
                pending.append(ch)
                switch pending {
                case "\u{1b}", "\u{1b}[":
                    continue
                case "\u{1b}[C":
                    flushTyped()
                    caret = min(caret + 1, line.count)
                    trace += "→"
                case "\u{1b}[D":
                    flushTyped()
                    caret = max(caret - 1, 0)
                    trace += "←"
                default:
                    flushTyped()
                    trace += "�"  // unsupported sequence
                }
                pending = ""
                continue
            }
            switch ch {
            case "\u{7f}", "\u{08}":
                flushTyped()
                backspace()
                trace += "⌫"
            case "\r", "\n":
                flushTyped()
                submit()
                trace += "⏎"
            default:
                line.insert(ch, at: caret)
                caret += 1
                typed.append(ch)
            }
        }
        flushTyped()
        return trace
    }

    mutating func backspace() {
        guard caret > 0 else { return }
        line.remove(at: caret - 1)
        caret -= 1
    }

    mutating func moveLeft() { caret = max(caret - 1, 0) }
    mutating func moveRight() { caret = min(caret + 1, line.count) }
    mutating func home() { caret = 0 }
    mutating func end() { caret = line.count }

    mutating func submit() {
        history.append(String(line))
        line = []
        caret = 0
    }

    mutating func clear() {
        self = FakeTerminal()
    }
}
