import AppKit

enum InstallError: LocalizedError {
    case copyFailed(String)
    var errorDescription: String? { if case .copyFailed(let m) = self { return m }; return nil }
}

/// Copies the nested IME into ~/Library/Input Methods and registers it.
enum Installer {
    static func bundleVersion(_ url: URL) -> Int? {
        guard let info = NSDictionary(contentsOf: url.appendingPathComponent("Contents/Info.plist")),
              let v = info["CFBundleVersion"] as? String else { return nil }
        return Int(v)
    }

    static func installIfNeeded(_ m: AppModel) throws -> InstallAction {
        let action = VersionPolicy.action(bundled: bundleVersion(m.nestedIME), installed: bundleVersion(m.installedIME))
        guard action != .none else { return .none }
        let fm = FileManager.default
        let dir = m.installedIME.deletingLastPathComponent()
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        // Same volume as the destination, so the final step is a rename.
        let tmp = dir.appendingPathComponent(".NeuralIMK.installing.app")
        try? fm.removeItem(at: tmp)
        do { try fm.copyItem(at: m.nestedIME, to: tmp) } catch {
            try? fm.removeItem(at: tmp)
            throw InstallError.copyFailed(error.localizedDescription)
        }
        // A downloaded app passes its quarantine to the copy; the input
        // method must not carry it (docs/07 checklist step 1).
        for failure in Quarantine.strip(tmp) {
            m.log("cannot remove quarantine: \(failure)")
        }
        terminateIME(bundleId: m.imeBundleId)
        do {
            if fm.fileExists(atPath: m.installedIME.path) {
                _ = try fm.replaceItemAt(m.installedIME, withItemAt: tmp)
            } else {
                try fm.moveItem(at: tmp, to: m.installedIME)
            }
        } catch {
            try? fm.removeItem(at: tmp)
            throw error
        }
        InputSource.register(m.installedIME)
        _ = InputSource.enable(bundleId: m.imeBundleId)
        return action
    }

    /// Asks the IME to quit, then SIGTERM (the IME flushes its engine on
    /// SIGTERM, see macos/NeuralIMK/main.swift); waits up to 2 s in total.
    static func terminateIME(bundleId: String) {
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleId)
        guard !running.isEmpty else { return }
        running.forEach { $0.terminate() }
        wait(for: running, until: Date().addingTimeInterval(0.5))
        // LSBackgroundOnly apps may ignore the quit Apple event.
        for app in running where !app.isTerminated && app.processIdentifier > 0 {
            kill(app.processIdentifier, SIGTERM)
        }
        wait(for: running, until: Date().addingTimeInterval(1.5))
    }

    private static func wait(for running: [NSRunningApplication], until deadline: Date) {
        while running.contains(where: { !$0.isTerminated }) && Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
    }

    static func uninstall(_ m: AppModel, deleteData: Bool) throws {
        terminateIME(bundleId: m.imeBundleId)
        let fm = FileManager.default
        if fm.fileExists(atPath: m.installedIME.path) { try fm.removeItem(at: m.installedIME) }
        if deleteData, fm.fileExists(atPath: m.dataDir.path) { try fm.removeItem(at: m.dataDir) }
    }
}
