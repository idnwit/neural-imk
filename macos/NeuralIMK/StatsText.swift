import Foundation

/// Menu text for the per-model quality counters. Pure (no FFI types) so it
/// can be tested with plain `swiftc`.
enum StatsText {
    /// `1234567` -> "1 234 567" (thin grouping with plain spaces).
    static func grouped(_ n: UInt64) -> String {
        let digits = Array(String(n))
        var out = ""
        for (i, ch) in digits.enumerated() {
            if i > 0 && (digits.count - i) % 3 == 0 { out.append(" ") }
            out.append(ch)
        }
        return out
    }

    /// Percentage with one decimal and a decimal comma: "1,2%".
    static func percent(_ part: UInt64, of whole: UInt64) -> String {
        guard whole > 0 else { return "0,0%" }
        // Tenths of a percent, rounded half up.
        let tenths = (part * 1000 + whole / 2) / whole
        return "\(tenths / 10),\(tenths % 10)%"
    }

    /// "sửa 1,2% (500 từ gần nhất) · 2,0% tổng (3 450 từ)", or a note when
    /// nothing has been counted yet.
    static func quality(wordsRecent: UInt64, correctedRecent: UInt64,
                        wordsTotal: UInt64, correctedTotal: UInt64) -> String {
        guard wordsTotal > 0 else { return "chưa có dữ liệu" }
        return "sửa \(percent(correctedRecent, of: wordsRecent)) (\(grouped(wordsRecent)) từ gần nhất)"
            + " · \(percent(correctedTotal, of: wordsTotal)) tổng (\(grouped(wordsTotal)) từ)"
    }

    /// Whole menu line of one model.
    static func modelLine(name: String, active: Bool, learning: Bool, quality: String) -> String {
        let role = active ? " ✓" : (learning ? " (chạy ngầm)" : " (đóng băng)")
        return "Model: \(name)\(role) — \(quality)"
    }
}
