import Foundation

/// Removes Gatekeeper's `com.apple.quarantine` attribute from a copied
/// bundle so the installed input method does not inherit it from the
/// downloaded app.
enum Quarantine {
    static let name = "com.apple.quarantine"

    /// Strips the attribute from `root` and everything below it (symlinks
    /// themselves, never their targets). "No such attribute" is not an
    /// error; returns one message per other failure.
    @discardableResult
    static func strip(_ root: URL) -> [String] {
        var failures: [String] = []
        var paths = [root.path]
        if let e = FileManager.default.enumerator(atPath: root.path) {
            while let rel = e.nextObject() as? String {
                paths.append(root.appendingPathComponent(rel).path)
            }
        }
        for p in paths where removexattr(p, name, XATTR_NOFOLLOW) != 0 {
            let err = errno
            if err != ENOATTR {
                failures.append("\(p): \(String(cString: strerror(err)))")
            }
        }
        return failures
    }

    /// Whether `path` carries the attribute (tests and diagnostics).
    static func has(_ path: String) -> Bool {
        getxattr(path, name, nil, 0, 0, XATTR_NOFOLLOW) >= 0
    }
}
