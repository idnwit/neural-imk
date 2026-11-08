import Foundation

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

    let dataDir: URL
    let logURL: URL
    private var handle: OpaquePointer?

    private init() {
        let home = URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true)
        let fm = FileManager.default
        dataDir = home.appendingPathComponent("Library/Application Support/NeuralIMK", isDirectory: true)
        logURL = home.appendingPathComponent("Library/Logs/NeuralIMK.log")
        try? fm.createDirectory(at: dataDir, withIntermediateDirectories: true)
        try? fm.createDirectory(at: logURL.deletingLastPathComponent(), withIntermediateDirectories: true)

        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0"
        handle = dataDir.path.withCString { dir in
            version.withCString { ver in nimk_new(dir, ver) }
        }
        log(handle == nil
            ? "nimk_new failed: engine unavailable, passing every key through"
            : "started \(version), data dir \(dataDir.path)")
        drainDiagnostics()
    }

    var isLive: Bool { handle != nil }

    /// Current mode code (`NIMK_MODE_EN` / `NIMK_MODE_VI`).
    var mode: UInt8 { nimk_mode(handle) }

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
