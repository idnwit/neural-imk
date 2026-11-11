import Foundation

/// Fine-tuning on demand (the menu button): counts the new data in the
/// background and runs one fine-tuning pass when the user clicks.
///
/// Sequence of a run: `prepare` on the main thread (save what the models
/// learned), the heavy `run` on a utility queue (typing goes on), then
/// `install` on the main thread. What the model learns online while the run
/// is going is replaced by the install; the run takes seconds to a few
/// minutes. Not automatic on purpose: nobody knows yet how often it pays off.
final class Tuner {
    static let shared = Tuner()

    /// Last count of new data; `nil` until the first background count ends.
    /// Main thread only.
    private(set) var pending: NimkTunePending?
    /// A run is in flight. Main thread only.
    private(set) var running = false
    private var counting = false

    /// Counts the new data in the background and stores the result (the next
    /// menu opening shows it).
    func refresh() {
        guard !counting else { return }
        counting = true
        DispatchQueue.global(qos: .utility).async {
            let p = Core.shared.tunePending()
            DispatchQueue.main.async {
                self.counting = false
                if let p { self.pending = p }
            }
        }
    }

    /// Starts a run when there is enough new data and none is running.
    /// `prepared` gets, at once, the output of saving the models (it commits
    /// a word being composed: apply it to the client). `done` gets the text
    /// to show and the output of installing the result (same), later, on the
    /// main thread. Returns `false` when it did not start.
    @discardableResult
    func start(prepared: (CoreOutput) -> Void, done: @escaping (String, CoreOutput) -> Void) -> Bool {
        guard TuneText.canRun(newWords: pending?.new_words, running: running) else { return false }
        running = true
        let kind = Core.shared.model
        let name = Core.modelName(kind)
        Core.shared.log("tune: start \(name), \(pending?.new_words ?? 0) new words")
        prepared(Core.shared.tunePrepare(timestamp: ProcessInfo.processInfo.systemUptime))
        DispatchQueue.global(qos: .utility).async {
            let started = Date()
            let (code, report) = Core.shared.tuneRun(kind: kind)
            DispatchQueue.main.async {
                var installed = false
                var output = CoreOutput()
                if code == UInt8(NIMK_TUNE_OK) {
                    let result = Core.shared.tuneInstall(timestamp: ProcessInfo.processInfo.systemUptime)
                    installed = result.installed
                    output = result.output
                }
                self.running = false
                let secs = Int(Date().timeIntervalSince(started))
                Core.shared.log("tune: \(name) code \(code), \(report.new_words) new + "
                    + "\(report.replay_words) replayed words, \(report.keys) keys, \(secs)s, "
                    + "installed \(installed)")
                self.refresh()
                done(TuneText.resultText(code: code, newWords: report.new_words,
                                         replayWords: report.replay_words, installed: installed), output)
            }
        }
        return true
    }
}
