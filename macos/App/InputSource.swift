import Carbon
import Foundation

/// Text Input Sources wrappers for the nested IME.
enum InputSource {
    static func register(_ url: URL) { TISRegisterInputSource(url as CFURL) }

    private static func sources(bundleId: String) -> [TISInputSource] {
        let filter = [kTISPropertyBundleID as String: bundleId] as CFDictionary
        guard let list = TISCreateInputSourceList(filter, true)?.takeRetainedValue() as? [TISInputSource] else { return [] }
        return list
    }

    /// Enables every input source of the IME; true if at least one is enabled.
    static func enable(bundleId: String) -> Bool {
        sources(bundleId: bundleId).reduce(false) { ok, src in TISEnableInputSource(src) == noErr || ok }
    }

    static func isSelected(bundleId: String) -> Bool {
        guard let cur = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
              let ptr = TISGetInputSourceProperty(cur, kTISPropertyBundleID) else { return false }
        return (Unmanaged<CFString>.fromOpaque(ptr).takeUnretainedValue() as String) == bundleId
    }

    static func select(bundleId: String) {
        if let src = sources(bundleId: bundleId).first { TISSelectInputSource(src) }
    }
}
