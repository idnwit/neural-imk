import Foundation

/// Per-app verdict on whether `insertText(_:replacementRange:)` really replaces
/// text. Apps that cannot are never touched: the engine still flips the mode and
/// learns, but the text stays as typed (no CGEvent fallback, by decision).
final class ReplaceCompat {
    static let shared = ReplaceCompat()

    enum Verdict: String, Codable { case unknown, ok, unsupported }

    private static let denyList: Set<String> = [
        "com.mitchellh.ghostty", "com.apple.Terminal", "com.googlecode.iterm2",
        "com.github.wez.wezterm", "net.kovidgoyal.kitty", "org.alacritty",
        "dev.warp.Warp-Stable", "dev.warp.Warp", "org.mozilla.firefox",
    ]
    private static let denyPrefixes = ["com.jetbrains."]
    private static let allowList: Set<String> = [
        "com.apple.TextEdit", "com.apple.Notes", "com.apple.iWork.Pages",
        "com.apple.MobileSMS", "com.apple.dt.Xcode",
    ]

    /// Runtime verdicts (decided by post-check) and user overrides (file wins).
    private var learned: [String: Verdict] = [:]
    private var overrides: [String: Verdict] = [:]
    private let fileURL: URL

    private init() {
        fileURL = Core.shared.dataDir.appendingPathComponent("replace-compat.json")
        load()
    }

    func verdict(_ bundle: String) -> Verdict {
        if let o = overrides[bundle] { return o }
        if Self.denyList.contains(bundle) || Self.denyPrefixes.contains(where: bundle.hasPrefix) {
            return .unsupported
        }
        if Self.allowList.contains(bundle) { return .ok }
        return learned[bundle] ?? .unknown
    }

    /// Whether a replace may be attempted at all.
    func allows(_ bundle: String) -> Bool { verdict(bundle) != .unsupported }

    /// Records a runtime result; an explicit user override is never changed.
    func record(_ bundle: String, _ v: Verdict) {
        guard overrides[bundle] == nil, learned[bundle] != v else { return }
        learned[bundle] = v
        save()
    }

    /// Menu toggle: forces the app on or off.
    func toggle(_ bundle: String) {
        overrides[bundle] = allows(bundle) ? .unsupported : .ok
        save()
    }

    private func load() {
        guard let data = try? Data(contentsOf: fileURL),
              let file = try? JSONDecoder().decode(File.self, from: data)
        else { return }
        learned = file.learned
        overrides = file.overrides
    }

    private func save() {
        let file = File(learned: learned, overrides: overrides)
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? enc.encode(file) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }

    private struct File: Codable {
        var learned: [String: Verdict]
        var overrides: [String: Verdict]
    }
}
