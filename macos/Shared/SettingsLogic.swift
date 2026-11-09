import Foundation

/// Pure helpers behind Settings and Onboarding.
enum SettingsLogic {
    /// Reads a top-level boolean key (before the first table header).
    static func readBool(_ text: String, key: String) -> Bool? {
        for raw in text.components(separatedBy: "\n") {
            let t = raw.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("[") { return nil }
            guard t.hasPrefix(key) else { continue }
            let rest = t.dropFirst(key.count).trimmingCharacters(in: .whitespaces)
            guard rest.hasPrefix("=") else { continue }
            let value = rest.dropFirst().split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false)
                .first.map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
            if value == "true" { return true }
            if value == "false" { return false }
            return nil
        }
        return nil
    }

    /// The preset the config currently expresses; missing keys mean the
    /// engine defaults (double_shift on, ctrl_space off).
    static func preset(fromConfig text: String) -> HotkeyPreset {
        HotkeyPreset.from(doubleShift: readBool(text, key: "double_shift") ?? true,
                          ctrlSpace: readBool(text, key: "ctrl_space") ?? false)
    }

    /// Picks a prompt; any integer works as the random source.
    static func pick(_ prompts: [String], _ n: Int) -> String {
        guard !prompts.isEmpty else { return "" }
        return prompts[((n % prompts.count) + prompts.count) % prompts.count]
    }

    /// Whether a preset uses Ctrl+Space (so the system shortcut conflicts).
    static func usesCtrlSpace(_ p: HotkeyPreset) -> Bool { p == .ctrlSpace || p == .both }
}
