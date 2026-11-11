import AppKit
import SwiftUI

/// Caret-flip simulator (ADR 0015): an ordinary text editor (the IME's
/// text-client strategy) and a terminal-like line that exposes no text (the
/// terminal strategy), with a trace of what the terminal received. Both are
/// real text clients of the installed IME; nothing here calls the engine.
struct CaretLabView: View {
    @StateObject private var lab = CaretLab()

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(Strings.labIntro).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                GroupBox {
                    VStack(alignment: .leading, spacing: 8) {
                        steps(Strings.labEditorSteps)
                        LabEditor(text: $lab.editorText)
                            .frame(minHeight: 110)
                            .overlay(RoundedRectangle(cornerRadius: 6).stroke(.quaternary))
                        HStack {
                            Spacer()
                            Button(Strings.labClear) { lab.editorText = "" }
                        }
                    }
                    .padding(4)
                } label: {
                    Label(Strings.labEditorTitle, systemImage: "doc.text")
                }

                GroupBox {
                    VStack(alignment: .leading, spacing: 8) {
                        steps(Strings.labTerminalSteps)
                        LabTerminal(lab: lab)
                            .frame(height: 150)
                            .clipShape(RoundedRectangle(cornerRadius: 6))
                        Text(Strings.labTrace).font(.caption).foregroundStyle(.secondary)
                        ScrollView {
                            VStack(alignment: .leading, spacing: 2) {
                                ForEach(lab.trace.indices.reversed(), id: \.self) { i in
                                    Text(lab.trace[i]).font(.system(.caption, design: .monospaced))
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                }
                            }
                        }
                        .frame(height: 90)
                        .padding(6)
                        .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary.opacity(0.4)))
                        HStack {
                            Text(Strings.labTerminalNote).font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            Button(Strings.labClear) { lab.clearTerminal() }
                        }
                    }
                    .padding(4)
                } label: {
                    Label(Strings.labTerminalTitle, systemImage: "terminal")
                }
            }
            .padding(20)
        }
        .navigationTitle(Strings.caretLab)
    }

    private func steps(_ items: [String]) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(items.indices, id: \.self) { i in
                Text("\(i + 1). \(items[i])").font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// State shared by the simulator's views.
final class CaretLab: ObservableObject {
    @Published var editorText = ""
    @Published private(set) var terminal = FakeTerminal()
    /// What the terminal received, oldest first.
    @Published private(set) var trace: [String] = []
    private var lastTrace = Date.distantPast

    func receive(_ text: String) {
        let t = terminal.receive(text)
        // Units of one caret flip arrive a few ms apart: one trace line.
        let now = Date()
        if now.timeIntervalSince(lastTrace) < 0.08, let last = trace.last {
            trace[trace.count - 1] = last + t
        } else {
            trace.append(t)
            if trace.count > 50 { trace.removeFirst() }
        }
        lastTrace = now
    }

    func command(_ f: (inout FakeTerminal) -> Void, trace label: String) {
        f(&terminal)
        trace.append(label)
        lastTrace = .distantPast
    }

    func clearTerminal() {
        terminal.clear()
        trace = []
    }
}

/// Plain NSTextView: a text client that exposes its text to the IME.
struct LabEditor: NSViewRepresentable {
    @Binding var text: String

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        if let view = scroll.documentView as? NSTextView {
            view.font = .systemFont(ofSize: 15)
            view.isRichText = false
            view.allowsUndo = true
            view.isAutomaticQuoteSubstitutionEnabled = false
            view.isAutomaticSpellingCorrectionEnabled = false
            view.isAutomaticTextReplacementEnabled = false
            view.textContainerInset = NSSize(width: 6, height: 6)
            view.delegate = context.coordinator
        }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? NSTextView, view.string != text else { return }
        view.string = text
    }

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var text: Binding<String>
        init(text: Binding<String>) { self.text = text }
        func textDidChange(_ note: Notification) {
            if let view = note.object as? NSTextView { text.wrappedValue = view.string }
        }
    }
}

struct LabTerminal: NSViewRepresentable {
    @ObservedObject var lab: CaretLab

    func makeNSView(context: Context) -> TerminalView {
        let view = TerminalView()
        view.lab = lab
        return view
    }

    func updateNSView(_ view: TerminalView, context: Context) {
        view.needsDisplay = true
    }
}

/// A terminal-like text client: like a real terminal it reports no
/// selection and no text to the IME, and it interprets what the IME inserts
/// (DEL, arrow escape sequences) instead of showing it.
final class TerminalView: NSView, NSTextInputClient {
    weak var lab: CaretLab?
    private var marked = ""
    private let font = NSFont.monospacedSystemFont(ofSize: 14, weight: .regular)
    private let prompt = "> "

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
    }

    override func becomeFirstResponder() -> Bool {
        needsDisplay = true
        return true
    }

    override func resignFirstResponder() -> Bool {
        needsDisplay = true
        return true
    }

    override func keyDown(with event: NSEvent) {
        interpretKeyEvents([event])
        needsDisplay = true
    }

    override func flagsChanged(with event: NSEvent) {
        // The double-Shift hotkey reaches the IME only through here.
        _ = inputContext?.handleEvent(event)
    }

    // MARK: Commands the IME passes through

    override func doCommand(by selector: Selector) {
        guard let lab else { return }
        switch selector {
        case #selector(deleteBackward(_:)): lab.command({ $0.backspace() }, trace: "⌫ (phím)")
        case #selector(moveLeft(_:)): lab.command({ $0.moveLeft() }, trace: "← (phím)")
        case #selector(moveRight(_:)): lab.command({ $0.moveRight() }, trace: "→ (phím)")
        case #selector(moveToBeginningOfLine(_:)), #selector(moveToBeginningOfParagraph(_:)),
             #selector(moveToLeftEndOfLine(_:)):
            lab.command({ $0.home() }, trace: "⇤ (phím)")
        case #selector(moveToEndOfLine(_:)), #selector(moveToEndOfParagraph(_:)),
             #selector(moveToRightEndOfLine(_:)):
            lab.command({ $0.end() }, trace: "⇥ (phím)")
        case #selector(insertNewline(_:)): lab.command({ $0.submit() }, trace: "⏎ (phím)")
        default: break
        }
        needsDisplay = true
    }

    // MARK: NSTextInputClient

    func insertText(_ string: Any, replacementRange: NSRange) {
        let text = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        marked = ""
        lab?.receive(text)
        needsDisplay = true
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        marked = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        needsDisplay = true
    }

    func unmarkText() {
        if !marked.isEmpty { lab?.receive(marked) }
        marked = ""
        needsDisplay = true
    }

    func selectedRange() -> NSRange { NSRange(location: NSNotFound, length: 0) }

    func markedRange() -> NSRange {
        marked.isEmpty ? NSRange(location: NSNotFound, length: 0) : NSRange(location: 0, length: marked.utf16.count)
    }

    func hasMarkedText() -> Bool { !marked.isEmpty }

    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? { nil }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    func characterIndex(for point: NSPoint) -> Int { NSNotFound }

    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        let local = caretRect()
        guard let window else { return .zero }
        return window.convertToScreen(convert(local, to: nil))
    }

    // MARK: Drawing

    private var lineHeight: CGFloat { ceil(font.ascender - font.descender + font.leading) + 2 }
    private var charWidth: CGFloat { ("M" as NSString).size(withAttributes: [.font: font]).width }

    private func caretRect() -> NSRect {
        guard let lab else { return .zero }
        let rows = min(lab.terminal.history.count, maxHistoryRows)
        let column = prompt.count + lab.terminal.caret + marked.count
        return NSRect(x: 8 + CGFloat(column) * charWidth, y: 6 + CGFloat(rows) * lineHeight,
                      width: 1, height: lineHeight)
    }

    private var maxHistoryRows: Int { max(Int((bounds.height - 12) / lineHeight) - 1, 0) }

    override func draw(_ dirty: NSRect) {
        NSColor(white: 0.1, alpha: 1).setFill()
        bounds.fill()
        guard let lab else { return }
        let text: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor(white: 0.9, alpha: 1)]
        let dim: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor(white: 0.55, alpha: 1)]
        var y: CGFloat = 6
        for past in lab.terminal.history.suffix(maxHistoryRows) {
            (prompt + past as NSString).draw(at: NSPoint(x: 8, y: y), withAttributes: dim)
            y += lineHeight
        }
        let line = lab.terminal.line
        let before = String(line[..<lab.terminal.caret])
        let after = String(line[lab.terminal.caret...])
        let shown = NSMutableAttributedString(string: prompt + before, attributes: text)
        shown.append(NSAttributedString(string: marked, attributes: text.merging([
            .underlineStyle: NSUnderlineStyle.single.rawValue,
        ]) { $1 }))
        shown.append(NSAttributedString(string: after, attributes: text))
        shown.draw(at: NSPoint(x: 8, y: y))
        if window?.firstResponder === self {
            NSColor.systemGreen.setFill()
            var caret = caretRect()
            caret.size.width = 2
            caret.fill()
        } else if line.isEmpty && marked.isEmpty {
            (Strings.labTerminalPlaceholder as NSString).draw(
                at: NSPoint(x: 8 + CGFloat(prompt.count) * charWidth, y: y), withAttributes: dim)
        }
    }
}
