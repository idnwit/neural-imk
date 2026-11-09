import SwiftUI

/// Four-step first-run sheet.
struct OnboardingView: View {
    @EnvironmentObject var model: AppModel
    @State private var step = 0
    @State private var practiceText = ""
    @State private var corrected = false
    @State private var observer: NSObjectProtocol?
    @State private var prompt = SettingsLogic.pick(Strings.practicePrompts, Int.random(in: 0..<1000))

    private static let correctedName = Notification.Name("com.giangndm.inputmethod.NeuralIMK.corrected")
    private let titles = [Strings.stepInstall, Strings.stepSelect, Strings.stepPractice, Strings.stepDone]

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(Strings.onboardingTitle).font(.title2)
                Spacer()
                Text("\(Strings.stepOf(step + 1, titles.count)) · \(titles[step])")
                    .foregroundStyle(.secondary)
                    .accessibilityLabel(Strings.stepOf(step + 1, titles.count))
            }
            Group {
                switch step {
                case 0: installStep
                case 1: selectStep
                case 2: practiceStep.onAppear { corrected = false }
                default: CheatSheetView()
                }
            }
            .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
            HStack {
                // Esc stays with the user (an IME commit key; the practice step
                // has a text field): Back has no keyboard shortcut.
                if step > 0 { Button(Strings.back) { step -= 1 } }
                Spacer()
                if step < titles.count - 1 {
                    Button(Strings.next) { step += 1 }.keyboardShortcut(.defaultAction)
                } else {
                    Button(Strings.start) {
                        UserDefaults.standard.set(true, forKey: "onboardingDone")
                        model.showOnboarding = false
                    }.keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(24)
        .frame(width: 520)
        .onAppear {
            model.setPolling(true)
            observer = DistributedNotificationCenter.default().addObserver(
                forName: Self.correctedName, object: nil, queue: .main) { _ in corrected = true }
        }
        .onDisappear {
            model.setPolling(false)
            if let observer { DistributedNotificationCenter.default().removeObserver(observer) }
            observer = nil
        }
    }

    @ViewBuilder private var installStep: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch model.installState {
            case .installed(let v):
                Label("\(Strings.installed) · \(Strings.version) \(v)", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            case .notInstalled:
                HStack { Text(Strings.notInstalled); Button(Strings.retry) { model.reinstall() } }
            case .failed(let msg):
                Text(Strings.installFailed).foregroundStyle(.red)
                Text(msg).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                Button(Strings.retry) { model.reinstall() }
            }
            Text(Strings.statsStayLocal).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var selectStep: some View {
        VStack(alignment: .leading, spacing: 10) {
            if model.imeSelected {
                Label(Strings.imeSelected, systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            } else {
                Label(Strings.typingInABC, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange)
            }
            HStack {
                Button(Strings.enableNeuralIMK) {
                    _ = InputSource.enable(bundleId: model.imeBundleId)
                    InputSource.select(bundleId: model.imeBundleId)
                }
                Button(Strings.openKeyboardSettings) {
                    if let u = URL(string: "x-apple.systempreferences:com.apple.Keyboard-Settings.extension") {
                        NSWorkspace.shared.open(u)
                    }
                }
            }
            Text(Strings.globeTip).font(.caption).foregroundStyle(.secondary)
            Text(Strings.capsLockTip).font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var practiceStep: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(Strings.practiceInstruction)
            Text(prompt).font(.body.monospaced()).textSelection(.enabled)
            TextField(Strings.practicePlaceholder, text: $practiceText)
            if corrected {
                Label(Strings.practiceDone, systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            }
        }
    }
}
