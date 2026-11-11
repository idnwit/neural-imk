import Foundation
import os

/// Swift copy of `NimkOut`: strings are copied out of Rust-owned buffers
/// immediately, so they stay valid after the next FFI call.
struct CoreOutput {
    var consumed = false
    var commit: String?
    var marked: String?
    var mode = UInt8(NIMK_MODE_UNCHANGED)
    var overlay = UInt8(NIMK_OVERLAY_NONE)

    init() {}

    init(_ raw: NimkOut) {
        consumed = raw.consumed
        commit = raw.commit.map { String(cString: $0) }
        marked = raw.marked.map { String(cString: $0) }
        mode = raw.mode
        overlay = raw.overlay
    }
}

/// Owner of the Rust `NimkCore`. All calls must happen on the main thread
/// (IMK delivers events there; the engine is not thread-safe).
final class Core {
    static let shared = Core()

    /// Fallback when the bundle has no identifier (never in the shipped app).
    private static let defaultBundleId = "com.giangndm.inputmethod.NeuralIMK"

    let bundleId: String
    /// User data (checkpoints, event log): `<Application Support>/<bundle id>`,
    /// both resolved by `FileManager`, never spelled out.
    let dataDir: URL
    /// Diagnostics file: `<Library>/Logs/<bundle id>.log`.
    let logURL: URL
    /// Optional user override: `<Application Support>/<bundle id>/config.toml`.
    let configURL: URL
    /// Read-only pretrained models shipped in `Contents/Resources/models`.
    let bundledModelsDir: URL?
    private let osLog: Logger
    private var handle: OpaquePointer?

    /// Resolves a system directory through the SDK.
    private static func systemDir(_ dir: FileManager.SearchPathDirectory) -> URL {
        let fm = FileManager.default
        if let url = try? fm.url(for: dir, in: .userDomainMask, appropriateFor: nil, create: true) {
            return url
        }
        return fm.temporaryDirectory
    }

    private init() {
        let fm = FileManager.default
        let id = Bundle.main.bundleIdentifier ?? Core.defaultBundleId
        bundleId = id
        osLog = Logger(subsystem: id, category: "core")
        dataDir = Core.systemDir(.applicationSupportDirectory).appendingPathComponent(id, isDirectory: true)
        logURL = Core.systemDir(.libraryDirectory)
            .appendingPathComponent("Logs", isDirectory: true)
            .appendingPathComponent("\(id).log")
        configURL = dataDir.appendingPathComponent("config.toml")
        try? fm.createDirectory(at: dataDir, withIntermediateDirectories: true)
        try? fm.createDirectory(at: logURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        if let res = Bundle.main.resourceURL {
            bundledModelsDir = res.appendingPathComponent("models", isDirectory: true)
        } else {
            bundledModelsDir = nil
        }

        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0"
        handle = dataDir.path.withCString { dir in
            version.withCString { ver in
                if let models = bundledModelsDir?.path {
                    return models.withCString { nimk_new(dir, $0, ver) }
                }
                return nimk_new(dir, nil, ver)
            }
        }
        log(handle == nil
            ? "nimk_new failed: engine unavailable, passing every key through"
            : "started \(Core.buildInfo), data dir \(dataDir.path)")
        drainDiagnostics()
        _ = reloadConfig()
    }

    var isLive: Bool { handle != nil }

    /// Re-reads the user override (a missing file is fine) and applies it
    /// over the engine's embedded defaults. Returns the number of rejected
    /// values (each is explained in the diagnostics log).
    @discardableResult
    func reloadConfig() -> Int {
        let text = try? String(contentsOf: configURL, encoding: .utf8)
        let rejected: UInt32
        if let text {
            rejected = text.withCString { nimk_set_config(handle, $0) }
        } else {
            rejected = nimk_set_config(handle, nil)
        }
        log(text == nil ? "config: no \(configURL.lastPathComponent), defaults in force"
                        : "config: loaded \(configURL.path), \(rejected) value(s) rejected")
        drainDiagnostics()
        return Int(rejected)
    }

    /// Seconds of silence after which marked text is committed (0 = off);
    /// follows the focused app's category.
    var idleCommitSecs: Double { nimk_idle_commit_secs(handle) }

    /// The caret moved without a key (mouse click): the engine leaves raw mode.
    func noteNavigation() { nimk_note_navigation(handle) }

    /// The core re-reads the wall clock and UTC offset (after sleep, on
    /// activation) so per-day usage stays on the right date.
    func noteWallClock() { nimk_note_wall_clock(handle) }

    /// Caret-flip strategies of the focused app (`NIMK_RETRO_*` bits).
    var retroPolicy: UInt8 { nimk_retro_policy(handle) }

    /// Caret flip of `word` right after a hotkey press that changed no
    /// marked text (ADR 0015): the replacement and its mode code, or nil.
    func retroFlip(_ word: String, timestamp: Double) -> (text: String, mode: UInt8)? {
        var mode = UInt8(NIMK_MODE_UNCHANGED)
        let text = word.withCString { nimk_retro_flip(handle, $0, timestamp, &mode) }
            .map { String(cString: $0) }
        drainDiagnostics()
        return text.map { ($0, mode) }
    }

    /// Current mode code (`NIMK_MODE_EN` / `NIMK_MODE_VI`).
    var mode: UInt8 { nimk_mode(handle) }

    /// Active model code (`NIMK_MODEL_LINEAR` / `NIMK_MODEL_GRU`).
    var model: UInt8 { nimk_model(handle) }

    /// Display name of a model code.
    /// "0.1.0 (34) · a1b2c3d · 2026-11-09T17:05": version, build number,
    /// commit and local build time stamped by
    /// scripts/build-app.sh, shown in the input menu.
    static var buildInfo: String {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        let commit = info?["NIMKGitCommit"] as? String ?? "?"
        let time = info?["NIMKBuildTime"] as? String ?? "?"
        return "\(version) (\(build)) · \(commit) · \(time)"
    }

    static func modelName(_ kind: UInt8) -> String {
        kind == UInt8(NIMK_MODEL_GRU) ? "GRU" : "Linear (nhanh)"
    }

    /// Where a model would load from: `NIMK_CHECKPOINT_NONE` / `_BUNDLED` / `_USER`.
    func checkpointSource(_ kind: UInt8) -> UInt8 { nimk_checkpoint_source(handle, kind) }

    /// Whether online training of a model is on.
    func onlineTraining(_ kind: UInt8) -> Bool { nimk_online_training(handle, kind) }

    /// Turns online training of a model on or off (persisted by the engine).
    func setOnlineTraining(_ kind: UInt8, _ on: Bool) {
        let ok = nimk_set_online_training(handle, kind, on)
        drainDiagnostics()
        log("online training \(Core.modelName(kind)) = \(on): \(ok ? "ok" : "rejected")")
    }

    /// One-line quality summary of a model for the menu.
    func qualityText(_ kind: UInt8) -> String {
        var s = NimkModelStats()
        guard nimk_model_stats(handle, kind, &s) else { return "chưa có dữ liệu" }
        return StatsText.quality(wordsRecent: UInt64(s.words_recent), correctedRecent: UInt64(s.corrected_recent),
                                 wordsTotal: s.words_total, correctedTotal: s.corrected_total)
    }

    /// Drops what the active model learned and reloads the bundled one; the
    /// output commits any word being composed.
    func resetModel(timestamp: Double) -> CoreOutput {
        var out = NimkOut()
        let ok = nimk_reset_model(handle, timestamp, &out)
        drainDiagnostics()
        log(ok ? "model reset to the bundled default" : "model reset rejected")
        return handle == nil ? CoreOutput() : CoreOutput(out)
    }

    /// Switches the model; the output commits any word being composed.
    func setModel(_ kind: UInt8, timestamp: Double) -> CoreOutput {
        var out = NimkOut()
        let ok = nimk_set_model(handle, kind, timestamp, &out)
        drainDiagnostics()
        log(ok ? "model switched to \(Core.modelName(kind))" : "model switch to \(kind) rejected")
        return handle == nil ? CoreOutput() : CoreOutput(out)
    }

    // MARK: Fine-tuning on the user's own log

    /// New data since the last fine-tuning run. Reads the whole log and
    /// touches no core state, so call it off the main thread.
    func tunePending() -> NimkTunePending? {
        var p = NimkTunePending()
        let ok = dataDir.path.withCString { dir -> Bool in
            if let models = bundledModelsDir?.path {
                return models.withCString { nimk_tune_pending(dir, $0, &p) }
            }
            return nimk_tune_pending(dir, nil, &p)
        }
        return ok ? p : nil
    }

    /// Saves what the models learned so the run reads current weights; the
    /// output commits any word being composed. Main thread.
    func tunePrepare(timestamp: Double) -> CoreOutput {
        var out = NimkOut()
        let ok = nimk_tune_prepare(handle, timestamp, &out)
        drainDiagnostics()
        if !ok { log("tune: prepare rejected") }
        return handle == nil ? CoreOutput() : CoreOutput(out)
    }

    /// Fine-tunes model `kind` on the new data. Blocks for seconds to
    /// minutes and touches no core state: call it off the main thread, after
    /// `tunePrepare`, never twice at once. Returns a `NIMK_TUNE_*` code.
    func tuneRun(kind: UInt8) -> (code: UInt8, report: NimkTuneReport) {
        var report = NimkTuneReport()
        let nowMs = UInt64(Date().timeIntervalSince1970 * 1000)
        let code = dataDir.path.withCString { dir -> UInt8 in
            if let models = bundledModelsDir?.path {
                return models.withCString {
                    nimk_tune_run(dir, $0, kind, TuneText.minNewWords, nowMs, &report)
                }
            }
            return nimk_tune_run(dir, nil, kind, TuneText.minNewWords, nowMs, &report)
        }
        return (code, report)
    }

    /// Loads the result of `tuneRun` into the active model; the output
    /// commits any word being composed. Main thread.
    func tuneInstall(timestamp: Double) -> (installed: Bool, output: CoreOutput) {
        var out = NimkOut()
        let ok = nimk_tune_install(handle, timestamp, &out)
        drainDiagnostics()
        log(ok ? "tune: tuned model installed" : "tune: nothing to install")
        return (ok, handle == nil ? CoreOutput() : CoreOutput(out))
    }

    var policyId: String? {
        nimk_policy_id(handle).map { String(cString: $0) }
    }

    func key(kind: UInt8, keycode: UInt16, modifiers: UInt32, chars: String?, timestamp: Double) -> CoreOutput {
        var out = NimkOut()
        func call(_ text: UnsafePointer<CChar>?) {
            var key = NimkKey(kind: kind, keycode: keycode, modifiers: modifiers, chars: text, timestamp: timestamp)
            nimk_key(handle, &key, &out)
        }
        if let chars { chars.withCString(call) } else { call(nil) }
        drainDiagnostics()
        return handle == nil ? CoreOutput() : CoreOutput(out)
    }

    func context(bundleId: String, fieldId: UInt64, caretRel: (Float, Float), timestamp: Double) -> CoreOutput {
        var out = NimkOut()
        bundleId.withCString { bid in
            var ctx = NimkCtx(bundle_id: bid, field_id: fieldId, caret_rel_x: caretRel.0,
                              caret_rel_y: caretRel.1, timestamp: timestamp)
            nimk_context(handle, &ctx, &out)
        }
        drainDiagnostics()
        return handle == nil ? CoreOutput() : CoreOutput(out)
    }

    func commitPending(timestamp: Double) -> CoreOutput {
        var out = NimkOut()
        nimk_commit_pending(handle, timestamp, &out)
        drainDiagnostics()
        return handle == nil ? CoreOutput() : CoreOutput(out)
    }

    /// Flushes the engine and frees the core; later calls pass keys through.
    func shutdown() {
        guard let live = handle else { return }
        drainDiagnostics()
        handle = nil
        log(nimk_shutdown(live) ? "shutdown ok" : "shutdown failed (log flush or snapshot)")
    }

    /// Appends engine/FFI warnings to the debug log. Cheap when there are none.
    func drainDiagnostics() {
        guard let raw = nimk_take_diagnostics(handle) else { return }
        log(String(cString: raw))
    }

    func log(_ message: String) {
        osLog.notice("\(message, privacy: .public)")
        let stamp = ISO8601DateFormatter().string(from: Date())
        let text = message.split(separator: "\n", omittingEmptySubsequences: false)
            .map { "\(stamp) \($0)\n" }.joined()
        guard let data = text.data(using: .utf8) else { return }
        if let fh = try? FileHandle(forWritingTo: logURL) {
            defer { try? fh.close() }
            _ = try? fh.seekToEnd()
            try? fh.write(contentsOf: data)
        } else {
            try? data.write(to: logURL)
        }
    }
}
