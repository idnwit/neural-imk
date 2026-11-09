import Foundation

struct UsageCounts: Decodable, Equatable { let vi: Int; let en: Int; let corrected: Int }
struct UsageDay: Decodable, Equatable { let date: String; let apps: [String: UsageCounts]; let total: UsageCounts }

/// Parsing and aggregation of the IME's `hooks/usage/<date>` blobs.
enum UsageData {
    static func parse(_ data: Data) -> UsageDay? { try? JSONDecoder().decode(UsageDay.self, from: data) }

    static func topApps(_ days: [UsageDay], limit: Int) -> [(app: String, words: Int)] {
        var words: [String: Int] = [:]
        for day in days { for (app, c) in day.apps { words[app, default: 0] += c.vi + c.en } }
        return words.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }
            .prefix(limit).map { (app: $0.key, words: $0.value) }
    }
}
