import Foundation

/// Rewrites top-level boolean keys of a TOML file, preserving everything
/// else byte for byte. Only lines before the first table header count as
/// top level; missing keys are inserted at the very top.
enum ConfigKeys {
    static func set(_ text: String, values: [(String, Bool)]) -> String {
        var lines = text.isEmpty ? [] : text.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        let firstTable = lines.firstIndex { $0.trimmingCharacters(in: .whitespaces).hasPrefix("[") } ?? lines.count
        var missing: [String] = []
        for (key, value) in values {
            let line = "\(key) = \(value)"
            let idx = lines[..<firstTable].firstIndex { raw in
                let t = raw.trimmingCharacters(in: .whitespaces)
                guard t.hasPrefix(key) else { return false }
                return t.dropFirst(key.count).trimmingCharacters(in: .whitespaces).hasPrefix("=")
            }
            if let idx { lines[idx] = line } else { missing.append(line) }
        }
        return (missing + lines).map { $0 + "\n" }.joined()
    }
}
