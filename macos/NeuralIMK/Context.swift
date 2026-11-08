import Cocoa
import CoreGraphics
import InputMethodKit

/// Reads the app context (docs/03-context-io.md) without Accessibility or
/// Screen Recording permissions.
enum ContextProbe {
    /// Window bounds (Quartz, top-left origin) of one PID, cached for 1 s.
    private static var windowCache: (pid: pid_t, at: TimeInterval, bounds: [CGRect])?

    /// Caret line rect in Cocoa screen coordinates; `.zero` if unknown.
    static func caretRect(_ client: IMKTextInput) -> NSRect {
        var rect = NSRect.zero
        _ = client.attributes(forCharacterIndex: 0, lineHeightRectangle: &rect)
        return rect
    }

    /// Opaque id of the text input client (object identity).
    static func fieldId(_ client: IMKTextInput) -> UInt64 {
        UInt64(bitPattern: Int64(ObjectIdentifier(client as AnyObject).hashValue))
    }

    /// Caret position relative to the app window, `(-1, -1)` if unknown.
    static func caretRel(caret: NSRect, bundleId: String) -> (Float, Float) {
        let unknown: (Float, Float) = (-1, -1)
        guard caret.height > 0 || caret.width > 0 || caret.origin != .zero,
              let mainHeight = NSScreen.screens.first?.frame.height,
              let pid = pid(for: bundleId)
        else { return unknown }

        // Cocoa (bottom-left of main screen) -> Quartz (top-left of main screen).
        let q = CGRect(x: caret.minX, y: mainHeight - caret.minY - caret.height,
                       width: caret.width, height: caret.height)
        let center = CGPoint(x: q.midX, y: q.midY)
        guard let win = windowBounds(pid: pid).first(where: { $0.contains(center) }),
              win.width > 0, win.height > 0
        else { return unknown }

        let clamp = { (v: CGFloat) -> Float in Float(min(max(v, 0), 1)) }
        return (clamp((q.minX - win.minX) / win.width), clamp((q.minY - win.minY) / win.height))
    }

    /// PID of the client app: the frontmost app if it matches, else the
    /// first running app with that bundle id.
    private static func pid(for bundleId: String) -> pid_t? {
        if let front = NSWorkspace.shared.frontmostApplication,
           bundleId.isEmpty || front.bundleIdentifier == bundleId {
            return front.processIdentifier
        }
        return NSRunningApplication.runningApplications(withBundleIdentifier: bundleId).first?.processIdentifier
    }

    /// On-screen, layer-0 window bounds of `pid` (front to back).
    private static func windowBounds(pid: pid_t) -> [CGRect] {
        let now = ProcessInfo.processInfo.systemUptime
        if let cache = windowCache, cache.pid == pid, now - cache.at < 1.0 {
            return cache.bounds
        }
        let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
        let bounds = list.compactMap { info -> CGRect? in
            guard let owner = info[kCGWindowOwnerPID as String] as? Int, owner == Int(pid),
                  let layer = info[kCGWindowLayer as String] as? Int, layer == 0,
                  let dict = info[kCGWindowBounds as String] as? NSDictionary
            else { return nil }
            return CGRect(dictionaryRepresentation: dict as CFDictionary)
        }
        windowCache = (pid, now, bounds)
        return bounds
    }
}
