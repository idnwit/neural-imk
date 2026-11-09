import Foundation

func check(_ got: String, _ want: String, _ line: Int = #line) {
    if got != want {
        print("FAIL line \(line): got \(got), want \(want)")
        exit(1)
    }
}

check(StatsText.grouped(3450), "3 450")
check(StatsText.grouped(999), "999")
check(StatsText.grouped(1_234_567), "1 234 567")
check(StatsText.percent(6, of: 500), "1,2%")
check(StatsText.percent(0, of: 0), "0,0%")
check(StatsText.percent(69, of: 3450), "2,0%")
check(StatsText.quality(wordsRecent: 500, correctedRecent: 6, wordsTotal: 3450, correctedTotal: 69),
      "sửa 1,2% (500 từ gần nhất) · 2,0% tổng (3 450 từ)")
check(StatsText.quality(wordsRecent: 0, correctedRecent: 0, wordsTotal: 0, correctedTotal: 0),
      "chưa có dữ liệu")
check(StatsText.modelLine(name: "GRU", active: true, learning: true, quality: "x"), "Model: GRU ✓ — x")
check(StatsText.modelLine(name: "Linear", active: false, learning: true, quality: "x"),
      "Model: Linear (chạy ngầm) — x")
check(StatsText.modelLine(name: "Linear", active: false, learning: false, quality: "x"),
      "Model: Linear (đóng băng) — x")
print("statstext ok")
