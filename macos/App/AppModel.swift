import AppKit
import Combine
import Foundation

enum InstallState: Equatable {
    case notInstalled
    case installed(String)
    case failed(String)
}

/// Paths, install state and live input-source state shared by every screen.
final class AppModel: ObservableObject {
    static let defaultIMEBundleId = "com.giangndm.inputmethod.NeuralIMK"
    static let defaultModel = "gru_action"

    let imeBundleId: String
    let dataDir: URL
    let logURL: URL
    let configURL: URL
    let nestedIME: URL
    let installedIME: URL
    /// The companion app's own log (install errors).
    let appLogURL: URL

    @Published var installState: InstallState = .notInstalled
    @Published var imeSelected = false
    @Published var developerMode: Bool { didSet { defaults.set(developerMode, forKey: "developerMode") } }
    @Published var showPerAppUsage: Bool { didSet { defaults.set(showPerAppUsage, forKey: "showPerAppUsage") } }
    @Published var section: AppSection = .overview
    @Published var showOnboarding = false
    /// Bumped whenever the IME writes a `hooks/` blob or on refresh.
    @Published var dataRevision = 0
    @Published var activeModel = AppModel.defaultModel
    @Published var activeAccuracy: ModelAccuracy?
    /// Set when the user uninstalls from Settings: launching the app again
    /// must not silently reinstall; Overview offers to reinstall instead.
    @Published var uninstalledByUser: Bool {
        didSet { defaults.set(uninstalledByUser, forKey: "uninstalledByUser") }
    }

    private let defaults = UserDefaults.standard
    private var launched = false
    private var pollers = 0
    private var timer: Timer?
    private var watchers: [String: DispatchSourceFileSystemObject] = [:]

    init() {
        let fm = FileManager.default
        nestedIME = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/InputMethods/NeuralIMK.app")
        let info = NSDictionary(contentsOf: nestedIME.appendingPathComponent("Contents/Info.plist"))
        imeBundleId = (info?["CFBundleIdentifier"] as? String) ?? AppModel.defaultIMEBundleId
        let library = (try? fm.url(for: .libraryDirectory, in: .userDomainMask, appropriateFor: nil, create: false))
            ?? fm.homeDirectoryForCurrentUser.appendingPathComponent("Library")
        let support = (try? fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: false))
            ?? library.appendingPathComponent("Application Support")
        installedIME = library.appendingPathComponent("Input Methods/NeuralIMK.app")
        dataDir = support.appendingPathComponent(imeBundleId)
        logURL = library.appendingPathComponent("Logs/\(imeBundleId).log")
        configURL = dataDir.appendingPathComponent("config.toml")
        appLogURL = library.appendingPathComponent("Logs/\(Bundle.main.bundleIdentifier ?? "com.giangndm.NeuralIMK").log")
        developerMode = defaults.bool(forKey: "developerMode")
        showPerAppUsage = defaults.object(forKey: "showPerAppUsage") as? Bool ?? true
        uninstalledByUser = defaults.bool(forKey: "uninstalledByUser")
        refresh()
    }

    /// Runs once when the window first appears: install or update the IME,
    /// then show onboarding if it was never completed.
    func launch() {
        guard !launched else { return }
        launched = true
        if !uninstalledByUser { install() }
        if !defaults.bool(forKey: "onboardingDone") { showOnboarding = true }
    }

    /// Installs or updates the IME if the nested one is newer.
    func install() {
        do {
            let action = try Installer.installIfNeeded(self)
            if action != .none { log("IME \(action) -> \(versionString(installedIME) ?? "?")") }
            refresh()
        } catch {
            let msg = error.localizedDescription
            log("install failed: \(msg)")
            refresh()
            installState = .failed(msg)
        }
    }

    /// An explicit install from the UI: clears `uninstalledByUser`.
    func reinstall() {
        uninstalledByUser = false
        install()
    }

    func refresh() {
        if let v = versionString(installedIME) {
            installState = .installed(v)
        } else {
            installState = .notInstalled
        }
        imeSelected = InputSource.isSelected(bundleId: imeBundleId)
        loadModel()
        armWatchers()
        dataRevision += 1
    }

    // MARK: Input-source polling (onboarding and Overview)

    /// Visible screens that need `imeSelected` live call this on appear (true)
    /// and disappear (false); the 1 s timer runs while any of them is shown.
    func setPolling(_ on: Bool) {
        pollers = max(0, pollers + (on ? 1 : -1))
        if pollers > 0, timer == nil {
            let t = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
                guard let self else { return }
                let sel = InputSource.isSelected(bundleId: self.imeBundleId)
                if sel != self.imeSelected { self.imeSelected = sel }
            }
            RunLoop.main.add(t, forMode: .common)
            timer = t
        } else if pollers == 0 {
            timer?.invalidate()
            timer = nil
        }
    }

    // MARK: Data

    func blobURL(_ key: String) -> URL { dataDir.appendingPathComponent("blobs/\(key)") }

    private func loadModel() {
        var model = AppModel.defaultModel
        if let data = try? Data(contentsOf: blobURL("settings/model")),
           let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let name = obj["model"] as? String, !name.isEmpty {
            model = name
        }
        activeModel = model
        activeAccuracy = (try? Data(contentsOf: blobURL("hooks/model_stats/\(model)"))).flatMap(ModelAccuracy.parse)
    }

    /// Watches `blobs/hooks` and its subdirectories (the IME writes blobs by
    /// tmp + rename inside them) and `blobs/settings`. A directory that does
    /// not exist yet is represented by its nearest existing ancestor, up to
    /// the parent of `dataDir` (Application Support) for a brand-new user,
    /// so the event for its creation re-arms the watchers. Watchers of
    /// directories no longer wanted (an ancestor once the real directory
    /// exists) are cancelled.
    private func armWatchers() {
        let fm = FileManager.default
        let hooks = blobURL("hooks")
        let top = dataDir.deletingLastPathComponent()
        var wanted = [hooks, blobURL("settings")]
        if let subs = try? fm.contentsOfDirectory(at: hooks, includingPropertiesForKeys: [.isDirectoryKey]) {
            wanted += subs.filter { (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true }
        }
        var dirs: [URL] = []
        for w in wanted {
            var d = w
            while d.path != top.path, d.path.count > top.path.count, !fm.fileExists(atPath: d.path) {
                d = d.deletingLastPathComponent()
            }
            if !fm.fileExists(atPath: d.path) { continue }
            if !dirs.contains(d) { dirs.append(d) }
        }
        let keep = Set(dirs.map(\.path))
        for (path, src) in watchers where !keep.contains(path) {
            src.cancel()
            watchers[path] = nil
        }
        for dir in dirs where watchers[dir.path] == nil {
            let fd = open(dir.path, O_EVTONLY)
            guard fd >= 0 else { continue }
            let src = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .delete, .rename], queue: .main)
            let path = dir.path
            src.setEventHandler { [weak self] in
                guard let self else { return }
                if src.data.contains(.delete) || src.data.contains(.rename) {
                    src.cancel()
                    self.watchers[path] = nil
                }
                self.dataChanged()
            }
            src.setCancelHandler { close(fd) }
            watchers[path] = src
            src.resume()
        }
    }

    private func dataChanged() {
        loadModel()
        armWatchers()
        dataRevision += 1
    }

    // MARK: Helpers

    func versionString(_ bundle: URL) -> String? {
        guard let info = NSDictionary(contentsOf: bundle.appendingPathComponent("Contents/Info.plist")) else { return nil }
        let short = info["CFBundleShortVersionString"] as? String ?? "?"
        let build = info["CFBundleVersion"] as? String ?? "?"
        return "\(short) (\(build))"
    }

    func log(_ message: String) {
        let line = "\(ISO8601DateFormatter().string(from: Date())) \(message)\n"
        let fm = FileManager.default
        try? fm.createDirectory(at: appLogURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        if let h = try? FileHandle(forWritingTo: appLogURL) {
            h.seekToEndOfFile()
            h.write(Data(line.utf8))
            try? h.close()
        } else {
            try? Data(line.utf8).write(to: appLogURL)
        }
    }
}
