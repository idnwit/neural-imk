/// What the installer does given the bundled and installed CFBundleVersion.
enum InstallAction: Equatable { case install, update, none }

enum VersionPolicy {
    static func action(bundled: Int?, installed: Int?) -> InstallAction {
        guard let bundled else { return .none }
        guard let installed else { return .install }
        return bundled > installed ? .update : .none
    }
}
