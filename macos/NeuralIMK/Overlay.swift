import Cocoa

/// "VI"/"EN" badge shown just below the caret when the mode changes.
/// Never takes focus and never intercepts the mouse (docs/06-gui.md).
final class Overlay {
    static let shared = Overlay()

    private let size = NSSize(width: 34, height: 22)
    private let label = NSTextField(labelWithString: "")
    private let background = NSView()
    /// Bumped on every show so a stale fade-out does not hide a newer badge.
    private var generation = 0

    private lazy var panel: NSPanel = {
        let panel = NSPanel(contentRect: NSRect(origin: .zero, size: size),
                            styleMask: [.borderless, .nonactivatingPanel],
                            backing: .buffered, defer: true)
        panel.level = .statusBar
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.ignoresMouseEvents = true
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.becomesKeyOnlyIfNeeded = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]

        background.frame = NSRect(origin: .zero, size: size)
        background.wantsLayer = true
        background.layer?.cornerRadius = 6
        label.frame = NSRect(x: 0, y: 3, width: size.width, height: 16)
        label.alignment = .center
        label.font = .boldSystemFont(ofSize: 12)
        label.textColor = .white
        background.addSubview(label)
        panel.contentView = background
        return panel
    }()

    /// Shows the badge for `mode` at `caret` (Cocoa screen coordinates).
    /// Auto switches are blue, manual (hotkey) switches orange.
    func show(mode: UInt8, manual: Bool, caret: NSRect) {
        let name = mode == UInt8(NIMK_MODE_VI) ? "VI" : "EN"
        label.attributedStringValue = NSAttributedString(string: name)
        label.stringValue = name
        label.font = .boldSystemFont(ofSize: 12)
        present(width: size.width, tint: manual ? .systemOrange : .systemBlue, caret: caret)
    }

    /// Shows a short text badge (e.g. the model name) at `caret`, blue.
    func showText(_ text: String, caret: NSRect) {
        label.attributedStringValue = NSAttributedString(string: text)
        label.stringValue = text
        label.font = .boldSystemFont(ofSize: 12)
        let width = max(size.width, ceil(label.intrinsicContentSize.width) + 16)
        present(width: width, tint: .systemBlue, caret: caret)
    }

    private func present(width: CGFloat, tint: NSColor, caret: NSRect) {
        let panel = self.panel
        label.textColor = .white
        panel.setContentSize(NSSize(width: width, height: size.height))
        background.frame = NSRect(origin: .zero, size: NSSize(width: width, height: size.height))
        label.frame = NSRect(x: 0, y: 3, width: width, height: 16)
        background.layer?.backgroundColor = tint.withAlphaComponent(0.92).cgColor

        let anchor = caret == .zero
            ? NSRect(origin: NSEvent.mouseLocation, size: .zero)
            : caret
        panel.setFrameOrigin(NSPoint(x: anchor.minX, y: anchor.minY - size.height - 4))
        panel.alphaValue = 1
        panel.orderFrontRegardless()

        generation += 1
        let current = generation
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { [weak self] in
            guard let self, self.generation == current else { return }
            NSAnimationContext.runAnimationGroup({ ctx in
                ctx.duration = 0.6
                panel.animator().alphaValue = 0
            }, completionHandler: { [weak self] in
                guard let self, self.generation == current else { return }
                panel.orderOut(nil)
            })
        }
    }
}

/// Menu-bar "VI"/"EN" indicator, created in the IME process itself
/// (docs/06-gui.md option A). Unverified: a `LSBackgroundOnly` input method
/// may not be allowed to show status items; if it never appears, move it to
/// a separate `LSUIElement` agent (option B).
final class ModeStatusItem {
    static let shared = ModeStatusItem()

    private var item: NSStatusItem?

    /// Creates the item (idempotent); call once at startup on the main thread.
    func install() {
        guard item == nil else { return }
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        item.button?.font = .monospacedSystemFont(ofSize: 12, weight: .semibold)
        self.item = item
        show(mode: Core.shared.mode, auto: false)
    }

    /// Updates the title; a trailing dot marks a mode chosen by the model.
    func show(mode: UInt8, auto: Bool) {
        let text = mode == UInt8(NIMK_MODE_VI) ? "VI" : "EN"
        item?.button?.title = auto ? "\(text)·" : text
        item?.button?.toolTip = "Model: \(Core.modelName(Core.shared.model))"
    }
}
