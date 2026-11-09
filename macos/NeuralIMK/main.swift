import Cocoa
import InputMethodKit

// Entry point: register the IMK server under the connection name from
// Info.plist, build the Rust core, then run the AppKit loop.

let bundle = Bundle.main
let connectionName = bundle.infoDictionary?["InputMethodConnectionName"] as? String
    ?? "com.giangndm.inputmethod.NeuralIMK_Connection"

let app = NSApplication.shared

guard let server = IMKServer(name: connectionName, bundleIdentifier: bundle.bundleIdentifier) else {
    NSLog("NeuralIMK: cannot create IMKServer for connection %@", connectionName)
    exit(1)
}

// Build the engine eagerly so start-up diagnostics reach the log.
_ = Core.shared
ModeStatusItem.shared.install()

// `killall NeuralIMK` (install.sh) sends SIGTERM: flush the engine first.
signal(SIGTERM, SIG_IGN)
let termSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
termSource.setEventHandler {
    Core.shared.shutdown()
    exit(0)
}
termSource.resume()

// The companion app rewrote config.toml (docs/06-gui.md).
let reloadObserver = DistributedNotificationCenter.default().addObserver(
    forName: Notification.Name("com.giangndm.inputmethod.NeuralIMK.reloadConfig"),
    object: nil, queue: .main) { _ in
    _ = Core.shared.reloadConfig()
}

withExtendedLifetime((server, termSource, reloadObserver)) {
    app.run()
}
