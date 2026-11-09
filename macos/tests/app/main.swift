import Foundation

func check<T: Equatable>(_ got: T, _ want: T, _ line: Int = #line) {
    if got != want { print("FAIL line \(line): got \(got), want \(want)"); exit(1) }
}

// ConfigKeys: insert at top before tables, replace in place, preserve the rest.
let user = "# mine\nswitch_margin = 0.05\n\n[[app_category]]\nname = \"terminal\"\nctrl_space = true\n"
check(ConfigKeys.set(user, values: [("double_shift", true), ("ctrl_space", false)]),
      "double_shift = true\nctrl_space = false\n# mine\nswitch_margin = 0.05\n\n[[app_category]]\nname = \"terminal\"\nctrl_space = true\n")
check(ConfigKeys.set("ctrl_space = true # old\nx = 1\n", values: [("ctrl_space", false)]),
      "ctrl_space = false\nx = 1\n")
check(ConfigKeys.set("", values: [("double_shift", false)]), "double_shift = false\n")
check(ConfigKeys.set("  double_shift=true\n[t]\n", values: [("double_shift", false)]), "double_shift = false\n[t]\n")

// Presets
check(HotkeyPreset.doubleShift.values.map { "\($0.0)=\($0.1)" }, ["double_shift=true", "ctrl_space=false"])
check(HotkeyPreset.from(doubleShift: true, ctrlSpace: true), .both)
check(HotkeyPreset.from(doubleShift: false, ctrlSpace: false), .off)

// Versions
check(VersionPolicy.action(bundled: 5, installed: nil), .install)
check(VersionPolicy.action(bundled: 5, installed: 4), .update)
check(VersionPolicy.action(bundled: 5, installed: 5), .none)
check(VersionPolicy.action(bundled: 5, installed: 6), .none)
check(VersionPolicy.action(bundled: nil, installed: 3), .none)

// Usage
let json = #"{"version":1,"date":"2026-11-09","apps":{"a":{"vi":3,"en":1,"corrected":0},"b":{"vi":0,"en":9,"corrected":2}},"total":{"vi":3,"en":10,"corrected":2}}"#
let day = UsageData.parse(Data(json.utf8))
check(day?.total, UsageCounts(vi: 3, en: 10, corrected: 2))
check(UsageData.topApps([day!], limit: 1).map { $0.app }, ["b"])
check(UsageData.parse(Data("nope".utf8)), nil)

// Correction hint
check(CorrectionHint.shouldShow(count: 0), true)
check(CorrectionHint.shouldShow(count: 3), false)
check(CorrectionHint.next(count: 2), 3)


// Model accuracy from the StatsTracker blob.
let stats = #"{"version":2,"words_total":10,"corrected_total":4,"served_words_total":10,"corrected_by_category":[0,0,0,0,0],"keys_total":0,"keys_match_total":0,"flips_total":0,"recent":[[true,3,1,0],[false,2,2,0],[false,4,4,1],[false,1,1,0]]}"#
check(ModelAccuracy.parse(Data(stats.utf8)),
      ModelAccuracy(wordsRecent: 4, correctedRecent: 1, wordsTotal: 10, correctedTotal: 4))
check(ModelAccuracy.parse(Data(stats.utf8))?.recent, 0.75)
check(ModelAccuracy.parse(Data(#"{"version":1,"words_total":0,"corrected_total":0,"recent":[]}"#.utf8)), nil)
check(ModelAccuracy.parse(Data("{}".utf8)), nil)
check(ModelAccuracy(wordsRecent: 0, correctedRecent: 0, wordsTotal: 0, correctedTotal: 0).recent, nil)

// Statistics logic
check(StatsLogic.recentDateNames(["2026-01-02", ".DS_Store", "2026-01-01", "x-y", "2026-01-03"], limit: 2), ["2026-01-02", "2026-01-03"])
func mk(_ d: String, _ vi: Int, _ en: Int, _ c: Int) -> UsageDay { UsageDay(date: d, apps: [:], total: UsageCounts(vi: vi, en: en, corrected: c)) }
check(StatsLogic.lastDays([mk("b", 0, 0, 0), mk("a", 0, 0, 0), mk("c", 0, 0, 0)], limit: 2).map { $0.date }, ["b", "c"])
check(StatsLogic.wordsSummary([mk("a", 600, 240, 1), mk("b", 600, 240, 3)]), "2 ngày, trung bình 840 từ mỗi ngày, 71% tiếng Việt")
check(StatsLogic.wordsSummary([mk("a", 0, 0, 1)]), nil)
check(StatsLogic.wordsSummary([]), nil)
check(StatsLogic.correctionsSummary([mk("a", 1, 1, 1), mk("b", 1, 1, 3)]), "2 ngày, 4 lần sửa, nhiều nhất 3 lần một ngày")
check(StatsLogic.percent(0.756), "76%")
check(StatsLogic.percent(nil), "—")
check(StatsLogic.tailLines(Data("a\nb\nc\n".utf8), offset: 0, max: 2), ["b", "c"])
check(StatsLogic.tailLines(Data("rtial\nb\nc".utf8), offset: 5, max: 10), ["b", "c"])
check(StatsLogic.filter(["Error x", "ok", "an ERROR"], query: "error"), ["Error x", "an ERROR"])
check(StatsLogic.filter(["a"], query: ""), ["a"])
let tmpLog = FileManager.default.temporaryDirectory.appendingPathComponent("statslogic-\(getpid()).log")
let big = (0..<20000).map { "line \($0) padding padding" }.joined(separator: "\n") + "\n"
try? Data(big.utf8).write(to: tmpLog)
let tail = StatsLogic.readLogTail(tmpLog)
check(tail.count, 500)
check(tail.last, "line 19999 padding padding")
try? FileManager.default.removeItem(at: tmpLog)
check(StatsLogic.readLogTail(tmpLog), [])

// SettingsLogic
check(SettingsLogic.preset(fromConfig: ""), .doubleShift)
check(SettingsLogic.preset(fromConfig: "ctrl_space = true # x\n"), .both)
check(SettingsLogic.preset(fromConfig: "double_shift=false\nctrl_space = true\n"), .ctrlSpace)
check(SettingsLogic.preset(fromConfig: "double_shift = false\n[t]\nctrl_space = true\n"), .off)
check(SettingsLogic.readBool("a = 1\n", key: "a"), nil)
check(SettingsLogic.pick(["a", "b", "c"], 4), "b")
check(SettingsLogic.pick(["a", "b", "c"], -1), "c")
check(SettingsLogic.pick([], 1), "")
check(SettingsLogic.usesCtrlSpace(.both), true)
check(SettingsLogic.usesCtrlSpace(.doubleShift), false)

// Quarantine: stripped recursively from a copied bundle, missing is fine.
let qRoot = FileManager.default.temporaryDirectory.appendingPathComponent("quarantine-\(getpid()).app")
try? FileManager.default.removeItem(at: qRoot)
let qInner = qRoot.appendingPathComponent("Contents/MacOS")
try? FileManager.default.createDirectory(at: qInner, withIntermediateDirectories: true)
let qFile = qInner.appendingPathComponent("bin")
try? Data("x".utf8).write(to: qFile)
let qPlain = qRoot.appendingPathComponent("Contents/Info.plist")
try? Data("y".utf8).write(to: qPlain)
let qValue = "0081;00000000;Safari;"
for p in [qRoot.path, qFile.path] {
    check(setxattr(p, Quarantine.name, qValue, qValue.utf8.count, 0, 0), 0)
}
check(Quarantine.has(qFile.path), true)
check(Quarantine.has(qPlain.path), false)
check(Quarantine.strip(qRoot), [])
check(Quarantine.has(qRoot.path), false)
check(Quarantine.has(qFile.path), false)
check(Quarantine.strip(qRoot), [])
try? FileManager.default.removeItem(at: qRoot)

print("app ok")
