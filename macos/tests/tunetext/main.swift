import Foundation

func check<T: Equatable>(_ got: T, _ want: T, _ line: Int = #line) {
    if got != want {
        print("FAIL line \(line): got \(got), want \(want)")
        exit(1)
    }
}

check(TuneText.buttonTitle(newWords: nil, corrections: 0, running: false),
      "Tinh chỉnh với dữ liệu mới… (đang đếm)")
check(TuneText.buttonTitle(newWords: 3450, corrections: 12, running: false),
      "Tinh chỉnh với dữ liệu mới… (3 450 từ mới, 12 lần sửa)")
check(TuneText.buttonTitle(newWords: 3450, corrections: 12, running: true), "Đang tinh chỉnh model…")

check(TuneText.lastRunLine(runs: 0, lastUnixMs: 0, nowUnixMs: 1_000_000), "Lần tinh chỉnh gần nhất: chưa có")
check(TuneText.lastRunLine(runs: 2, lastUnixMs: 1_000_000, nowUnixMs: 1_000_000 + 5 * 60_000),
      "Lần tinh chỉnh gần nhất: 5 phút trước (2 lần)")
check(TuneText.lastRunLine(runs: 1, lastUnixMs: 1_000_000, nowUnixMs: 1_000_000 + 3 * 3_600_000),
      "Lần tinh chỉnh gần nhất: 3 giờ trước (1 lần)")
check(TuneText.lastRunLine(runs: 1, lastUnixMs: 1_000_000, nowUnixMs: 1_000_000 + 4 * 86_400_000),
      "Lần tinh chỉnh gần nhất: 4 ngày trước (1 lần)")
// A clock that went backwards must not underflow.
check(TuneText.lastRunLine(runs: 1, lastUnixMs: 2_000_000, nowUnixMs: 1_000_000),
      "Lần tinh chỉnh gần nhất: 0 phút trước (1 lần)")

check(TuneText.canRun(newWords: nil, running: false), false)
check(TuneText.canRun(newWords: 199, running: false), false)
check(TuneText.canRun(newWords: 200, running: false), true)
check(TuneText.canRun(newWords: 5000, running: true), false)

check(TuneText.resultText(code: 0, newWords: 1234, replayWords: 3000, installed: true),
      "Đã tinh chỉnh với 1 234 từ mới")
check(TuneText.resultText(code: 0, newWords: 1234, replayWords: 0, installed: false),
      "Tinh chỉnh xong nhưng chưa nạp được model")
check(TuneText.resultText(code: 1, newWords: 0, replayWords: 0, installed: false),
      "Chưa đủ dữ liệu mới (cần ít nhất 200 từ)")
check(TuneText.resultText(code: 2, newWords: 0, replayWords: 0, installed: false),
      "Tinh chỉnh thất bại — xem log chẩn đoán")
check(TuneText.resultText(code: 3, newWords: 0, replayWords: 0, installed: false),
      "Tinh chỉnh thất bại — xem log chẩn đoán")
print("tunetext ok")
