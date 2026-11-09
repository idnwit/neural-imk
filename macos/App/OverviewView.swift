import SwiftUI

struct OverviewView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        Form {
            SwiftUI.Section(Strings.installState) {
                installRow
                if case .installed = model.installState {
                    if model.imeSelected {
                        Label(Strings.imeSelected, systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                    } else {
                        HStack {
                            Label(Strings.typingInABC, systemImage: "exclamationmark.triangle.fill")
                                .foregroundStyle(.orange)
                            Spacer()
                            Button(Strings.selectNeuralIMK) {
                                _ = InputSource.enable(bundleId: model.imeBundleId)
                                InputSource.select(bundleId: model.imeBundleId)
                                model.refresh()
                            }
                        }
                    }
                }
            }
            SwiftUI.Section(Strings.activeModel) {
                LabeledContent(Strings.activeModel, value: model.activeModel)
                if let acc = model.activeAccuracy, let recent = acc.recent {
                    LabeledContent(Strings.recentAccuracy, value: "\(percent(recent)) · \(Strings.wordsCount(acc.wordsRecent))")
                    if let life = acc.lifetime {
                        LabeledContent(Strings.lifetimeAccuracy, value: "\(percent(life)) · \(Strings.wordsCount(acc.wordsTotal))")
                    }
                } else {
                    Text(Strings.noWordsYet).foregroundStyle(.secondary)
                }
            }
            SwiftUI.Section(Strings.cheatSheet) {
                CheatSheetView()
                Button(Strings.reviewGuide) { model.showOnboarding = true }
            }
        }
        .formStyle(.grouped)
        .navigationTitle(Strings.overview)
        .onAppear { model.setPolling(true) }
        .onDisappear { model.setPolling(false) }
    }

    @ViewBuilder private var installRow: some View {
        switch model.installState {
        case .installed(let v):
            LabeledContent(Strings.installed, value: "\(Strings.version) \(v)")
        case .notInstalled:
            HStack {
                Text(Strings.notInstalled)
                Spacer()
                Button(Strings.retry) { model.install() }
            }
        case .failed(let msg):
            HStack {
                VStack(alignment: .leading) {
                    Text(Strings.installFailed).foregroundStyle(.red)
                    Text(msg).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                }
                Spacer()
                Button(Strings.retry) { model.install() }
            }
        }
    }

    private func percent(_ v: Double) -> String { "\(Int((v * 100).rounded()))%" }
}

/// The hotkey cheat sheet (Overview and the last onboarding step).
struct CheatSheetView: View {
    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
            row(Strings.doubleShift, Strings.doubleShiftAction)
            row(Strings.doubleShiftAgain, Strings.doubleShiftAgainAction)
            row(Strings.commandPrefix, Strings.commandPrefixAction)
        }
        .accessibilityElement(children: .combine)
    }

    private func row(_ key: String, _ action: String) -> some View {
        GridRow {
            Text(key).font(.body.monospaced()).bold()
            Text(action)
        }
    }
}
