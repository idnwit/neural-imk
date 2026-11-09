import SwiftUI

struct SettingsView: View {
    @EnvironmentObject var model: AppModel
    @State private var preset: HotkeyPreset = .doubleShift
    @State private var loaded = false
    @State private var shortcutConflict = false
    @State private var tryText = ""
    @State private var error: String?
    @State private var confirmDeleteUsage = false
    @State private var showUninstall = false

    private static let reloadName = Notification.Name("com.giangndm.inputmethod.NeuralIMK.reloadConfig")

    var body: some View {
        Form {
            Section(Strings.hotkeySection) {
                Picker(Strings.hotkeyPicker, selection: $preset) {
                    ForEach(HotkeyPreset.allCases, id: \.self) { Text(label($0)).tag($0) }
                }
                .pickerStyle(.segmented)
                .onChange(of: preset) { new in if loaded { apply(new) } }
                if preset == .off {
                    Text(Strings.noHotkeyUseMenu).font(.caption).foregroundStyle(.secondary)
                }
                if SettingsLogic.usesCtrlSpace(preset) && shortcutConflict {
                    HStack {
                        Label(Strings.systemShortcutWarning, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                        Spacer()
                        Button(Strings.openKeyboardShortcuts) {
                            open("x-apple.systempreferences:com.apple.preference.keyboard?Shortcuts")
                        }
                    }
                }
                TextField(Strings.tryNow, text: $tryText, prompt: Text(Strings.tryNowPlaceholder))
                if let error { Text(error).font(.caption).foregroundStyle(.red) }
            }
            Section {
                Toggle(Strings.showPerAppUsage, isOn: $model.showPerAppUsage)
                Toggle(Strings.developerMode, isOn: $model.developerMode)
            }
            Section(Strings.data) {
                Button(Strings.deleteUsage, role: .destructive) { confirmDeleteUsage = true }
                Button(Strings.uninstall, role: .destructive) { showUninstall = true }
            }
        }
        .formStyle(.grouped)
        .navigationTitle(Strings.settings)
        .onAppear {
            let text = (try? String(contentsOf: model.configURL)) ?? ""
            preset = SettingsLogic.preset(fromConfig: text)
            loaded = true
            shortcutConflict = SystemShortcuts.inputSourceShortcutEnabled()
        }
        .confirmationDialog(Strings.deleteUsageConfirm, isPresented: $confirmDeleteUsage, titleVisibility: .visible) {
            Button(Strings.delete, role: .destructive) { deleteUsage() }
            Button(Strings.cancel, role: .cancel) {}
        } message: { Text(Strings.deleteUsageMessage) }
        .sheet(isPresented: $showUninstall) { UninstallSheet().environmentObject(model) }
    }

    private func label(_ p: HotkeyPreset) -> String {
        switch p {
        case .doubleShift: return Strings.presetDoubleShift
        case .ctrlSpace: return Strings.presetCtrlSpace
        case .both: return Strings.presetBoth
        case .off: return Strings.presetOff
        }
    }

    private func apply(_ new: HotkeyPreset) {
        do {
            let text = (try? String(contentsOf: model.configURL)) ?? ""
            let updated = ConfigKeys.set(text, values: new.values)
            try FileManager.default.createDirectory(at: model.dataDir, withIntermediateDirectories: true)
            try Data(updated.utf8).write(to: model.configURL, options: .atomic)
            error = nil
            DistributedNotificationCenter.default().postNotificationName(
                Self.reloadName, object: nil, userInfo: nil, deliverImmediately: true)
        } catch {
            self.error = "\(Strings.configWriteFailed): \(error.localizedDescription)"
        }
    }

    private func deleteUsage() {
        let fm = FileManager.default
        let dir = model.blobURL("hooks/usage")
        if fm.fileExists(atPath: dir.path) {
            do { try fm.removeItem(at: dir) } catch { self.error = error.localizedDescription }
        }
        model.refresh()
    }

    private func open(_ url: String) {
        if let u = URL(string: url) { NSWorkspace.shared.open(u) }
    }
}

struct UninstallSheet: View {
    @EnvironmentObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var deleteData = false
    @State private var done = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if done {
                Text(Strings.removeFromInputSources)
                HStack {
                    Spacer()
                    Button(Strings.openKeyboardSettings) {
                        if let u = URL(string: "x-apple.systempreferences:com.apple.Keyboard-Settings.extension") {
                            NSWorkspace.shared.open(u)
                        }
                    }
                    Button(Strings.close) { dismiss() }.keyboardShortcut(.defaultAction)
                }
            } else {
                Text(Strings.uninstallConfirm).font(.headline)
                Text(Strings.uninstallMessage)
                Toggle(Strings.deleteLearnedData, isOn: $deleteData)
                if let error { Text("\(Strings.uninstallFailed): \(error)").foregroundStyle(.red) }
                HStack {
                    Spacer()
                    Button(Strings.cancel, role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                    Button(Strings.uninstallAction, role: .destructive) { run() }
                }
            }
        }
        .padding(20)
        .frame(width: 440)
    }

    private func run() {
        do {
            try Installer.uninstall(model, deleteData: deleteData)
            model.refresh()
            done = true
        } catch {
            self.error = error.localizedDescription
        }
    }
}
