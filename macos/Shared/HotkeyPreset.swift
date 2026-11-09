/// The four hotkey choices; each maps to the engine's two config bools.
enum HotkeyPreset: String, CaseIterable {
    case doubleShift, ctrlSpace, both, off

    var values: [(String, Bool)] {
        switch self {
        case .doubleShift: return [("double_shift", true), ("ctrl_space", false)]
        case .ctrlSpace: return [("double_shift", false), ("ctrl_space", true)]
        case .both: return [("double_shift", true), ("ctrl_space", true)]
        case .off: return [("double_shift", false), ("ctrl_space", false)]
        }
    }

    static func from(doubleShift: Bool, ctrlSpace: Bool) -> HotkeyPreset {
        switch (doubleShift, ctrlSpace) {
        case (true, false): return .doubleShift
        case (false, true): return .ctrlSpace
        case (true, true): return .both
        case (false, false): return .off
        }
    }
}
