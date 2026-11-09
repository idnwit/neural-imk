import Foundation

/// Pure selection and summary logic of the Statistics and Log screens.
enum StatsLogic {
    static let dayLimit = 30
    static let logLines = 500
    static let logTailBytes = 256 * 1024

    /// File names that are `YYYY-MM-DD`, oldest first, keeping the last `limit`.
    static func recentDateNames(_ names: [String], limit: Int = dayLimit) -> [String] {
        let dated = names.filter { isDate($0) }.sorted()
        return Array(dated.suffix(limit))
    }

    private static func isDate(_ s: String) -> Bool {
        let u = Array(s.utf8)
        guard u.count == 10 else { return false }
        for (i, c) in u.enumerated() {
            if i == 4 || i == 7 { if c != 45 { return false } } else if c < 48 || c > 57 { return false }
        }
        return true
    }

    /// Days sorted by date, last `limit` kept.
    static func lastDays(_ days: [UsageDay], limit: Int = dayLimit) -> [UsageDay] {
        Array(days.sorted { $0.date < $1.date }.suffix(limit))
    }

    /// Accessibility label for the words-per-day chart, nil with no words.
    /// Example: "30 ngày, trung bình 840 từ mỗi ngày, 62% tiếng Việt".
    static func wordsSummary(_ days: [UsageDay]) -> String? {
        guard !days.isEmpty else { return nil }
        let vi = days.reduce(0) { $0 + $1.total.vi }
        let en = days.reduce(0) { $0 + $1.total.en }
        let total = vi + en
        guard total > 0 else { return nil }
        let avg = Int((Double(total) / Double(days.count)).rounded())
        let pct = Int((Double(vi) * 100 / Double(total)).rounded())
        return "\(days.count) ngày, trung bình \(avg) từ mỗi ngày, \(pct)% tiếng Việt"
    }

    /// Accessibility label for the corrections-per-day chart.
    static func correctionsSummary(_ days: [UsageDay]) -> String? {
        guard !days.isEmpty else { return nil }
        let corrected = days.reduce(0) { $0 + $1.total.corrected }
        let peak = days.map { $0.total.corrected }.max() ?? 0
        return "\(days.count) ngày, \(corrected) lần sửa, nhiều nhất \(peak) lần một ngày"
    }

    static func percent(_ v: Double?) -> String {
        guard let v else { return "—" }
        return "\(Int((v * 100).rounded()))%"
    }

    /// Last `max` non-empty lines of log bytes read from `offset`; the first
    /// line is dropped when `offset > 0` (it may be cut in the middle).
    static func tailLines(_ data: Data, offset: UInt64, max: Int = logLines) -> [String] {
        var lines = String(decoding: data, as: UTF8.self).split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        if offset > 0, !lines.isEmpty { lines.removeFirst() }
        lines = lines.filter { !$0.isEmpty }
        return Array(lines.suffix(max))
    }

    /// Reads the tail of a file; empty when it is missing.
    static func readLogTail(_ url: URL) -> [String] {
        guard let h = try? FileHandle(forReadingFrom: url) else { return [] }
        defer { try? h.close() }
        let size = (try? h.seekToEnd()) ?? 0
        let offset = size > UInt64(logTailBytes) ? size - UInt64(logTailBytes) : 0
        try? h.seek(toOffset: offset)
        let data = (try? h.readToEnd()) ?? Data()
        return tailLines(data, offset: offset)
    }

    /// Plain case-insensitive substring filter; empty query keeps everything.
    static func filter(_ lines: [String], query: String) -> [String] {
        query.isEmpty ? lines : lines.filter { $0.localizedCaseInsensitiveContains(query) }
    }
}
