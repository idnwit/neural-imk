import Foundation

/// Recent word accuracy read from the IME's `hooks/model_stats/<kind>` blob
/// (the `StatsTracker` JSON of crates/nimk-hook/src/model_stats/tracker.rs:
/// lifetime counters plus `recent`, the rolling window of words, each
/// `[corrected, keys, keys_matched, flips]`).
struct ModelAccuracy: Equatable {
    let wordsRecent: Int
    let correctedRecent: Int
    let wordsTotal: Int
    let correctedTotal: Int

    /// `1 - corrected / words` over the window; nil with no words.
    var recent: Double? { wordsRecent > 0 ? 1 - Double(correctedRecent) / Double(wordsRecent) : nil }
    var lifetime: Double? { wordsTotal > 0 ? 1 - Double(correctedTotal) / Double(wordsTotal) : nil }

    /// Decodes the blob; nil when unreadable or of another version.
    static func parse(_ data: Data) -> ModelAccuracy? {
        guard let s = try? JSONDecoder().decode(SavedLoose.self, from: data), s.version == 2 else { return nil }
        let window = s.recent.suffix(500)  // tracker.rs WINDOW
        let corrected = window.filter { $0 }.count
        return ModelAccuracy(wordsRecent: window.count, correctedRecent: corrected,
                             wordsTotal: s.words_total, correctedTotal: s.corrected_total)
    }

    /// `recent` entries mix a bool with integers; only the bool is needed.
    private struct SavedLoose: Decodable {
        let version: Int
        let words_total: Int
        let corrected_total: Int
        let recent: [Bool]

        private enum Keys: String, CodingKey { case version, words_total, corrected_total, recent }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            version = try c.decode(Int.self, forKey: .version)
            words_total = try c.decode(Int.self, forKey: .words_total)
            corrected_total = try c.decode(Int.self, forKey: .corrected_total)
            var list = try c.nestedUnkeyedContainer(forKey: .recent)
            var out: [Bool] = []
            while !list.isAtEnd {
                var entry = try list.nestedUnkeyedContainer()
                out.append(try entry.decode(Bool.self))
            }
            recent = out
        }
    }
}
