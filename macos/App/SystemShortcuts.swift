import Foundation

/// Reads whether macOS's own "Select previous/next input source" shortcut
/// (symbolic hotkeys 60 and 61) is enabled; macOS then swallows Ctrl+Space.
enum SystemShortcuts {
    static func inputSourceShortcutEnabled() -> Bool {
        var dict = UserDefaults(suiteName: "com.apple.symbolichotkeys")?
            .dictionary(forKey: "AppleSymbolicHotKeys")
        if dict == nil {
            let plist = FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/Preferences/com.apple.symbolichotkeys.plist")
            dict = NSDictionary(contentsOf: plist)?["AppleSymbolicHotKeys"] as? [String: Any]
        }
        guard let dict else { return false }
        return ["60", "61"].contains { id in
            ((dict[id] as? [String: Any])?["enabled"] as? Bool) == true
        }
    }
}
