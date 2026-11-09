import Foundation

/// Idle-commit timer (ADR 0012): while text is marked (a word composing or
/// pending), commits it after `secs` without a key. The engine has no clock,
/// so the host owns this. Every key re-arms it; commit, deactivation and
/// `secs == 0` cancel it. Main-thread only.
final class IdleCommit {
    private let debouncer: Debouncer

    init(debouncer: Debouncer = Debouncer()) {
        self.debouncer = debouncer
    }

    var isArmed: Bool { debouncer.isPending }

    /// Call after each key (and after any engine output). Arms the timer
    /// when `secs > 0` and something is marked; cancels it otherwise.
    func update(secs: TimeInterval, marked: Bool, fire: @escaping () -> Void) {
        guard secs > 0, marked else {
            debouncer.cancel()
            return
        }
        debouncer.schedule(after: secs, fire)
    }

    func cancel() {
        debouncer.cancel()
    }
}
