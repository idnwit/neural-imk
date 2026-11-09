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

print("app ok")
