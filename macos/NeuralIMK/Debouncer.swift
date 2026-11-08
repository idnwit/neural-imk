import Foundation

/// Runs one delayed action; scheduling again, or `cancel()`, drops the
/// previous one that has not run yet. Used to settle the context of rapid
/// client activations (Electron activates two clients per click).
/// Main-thread only.
final class Debouncer {
    private var generation = 0
    private(set) var isPending = false

    /// Schedules `work` after `delay` seconds on `queue`, replacing any
    /// pending action.
    func schedule(after delay: TimeInterval, queue: DispatchQueue = .main, _ work: @escaping () -> Void) {
        generation += 1
        isPending = true
        let current = generation
        queue.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, self.generation == current else { return }
            self.isPending = false
            work()
        }
    }

    /// Drops the pending action, if any.
    func cancel() {
        generation += 1
        isPending = false
    }

    /// Runs `work` now if an action was pending (dropping that action) and
    /// returns whether it did.
    @discardableResult
    func flush(_ work: () -> Void) -> Bool {
        guard isPending else { return false }
        cancel()
        work()
        return true
    }
}
