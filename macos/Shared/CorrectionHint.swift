/// The "double-Shift again to undo" hint shows for the first corrections only.
struct CorrectionHint {
    static let maxShown = 3
    static func shouldShow(count: Int) -> Bool { count < maxShown }
    static func next(count: Int) -> Int { count + 1 }
}
