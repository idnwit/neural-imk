import Foundation

/// Menu and overlay text for fine-tuning on the user's own log. Pure (no FFI
/// types) so it can be tested with plain `swiftc`.
enum TuneText {
    /// Fewest new words a run needs (same as the engine's default); below it
    /// the button says there is not enough data instead of running.
    static let minNewWords: UInt64 = 200

    /// Title of the button. `newWords == nil` while the count is still being
    /// taken in the background.
    static func buttonTitle(newWords: UInt64?, corrections: UInt64, running: Bool) -> String {
        if running { return "Đang tinh chỉnh model…" }
        guard let newWords else { return "Tinh chỉnh với dữ liệu mới… (đang đếm)" }
        return "Tinh chỉnh với dữ liệu mới… (\(StatsText.grouped(newWords)) từ mới, "
            + "\(StatsText.grouped(corrections)) lần sửa)"
    }

    /// "Lần tinh chỉnh gần nhất: chưa có", "…: 3 giờ trước (2 lần)".
    static func lastRunLine(runs: UInt32, lastUnixMs: UInt64, nowUnixMs: UInt64) -> String {
        guard runs > 0, lastUnixMs > 0 else { return "Lần tinh chỉnh gần nhất: chưa có" }
        let mins = nowUnixMs > lastUnixMs ? (nowUnixMs - lastUnixMs) / 60_000 : 0
        let ago: String
        switch mins {
        case 0..<60: ago = "\(mins) phút trước"
        case 60..<2880: ago = "\(mins / 60) giờ trước"
        default: ago = "\(mins / 1440) ngày trước"
        }
        return "Lần tinh chỉnh gần nhất: \(ago) (\(runs) lần)"
    }

    /// Whether a click can start a run: enough new words and none running.
    static func canRun(newWords: UInt64?, running: Bool) -> Bool {
        !running && (newWords ?? 0) >= minNewWords
    }

    /// Overlay text after a run. `code` is a `NIMK_TUNE_*` value (0 ok,
    /// 1 too little data, 2 failed, anything else a bad call).
    static func resultText(code: UInt8, newWords: UInt64, replayWords: UInt64, installed: Bool) -> String {
        switch code {
        case 0:
            let n = StatsText.grouped(newWords)
            return installed ? "Đã tinh chỉnh với \(n) từ mới" : "Tinh chỉnh xong nhưng chưa nạp được model"
        case 1:
            return "Chưa đủ dữ liệu mới (cần ít nhất \(minNewWords) từ)"
        default:
            return "Tinh chỉnh thất bại — xem log chẩn đoán"
        }
    }
}
