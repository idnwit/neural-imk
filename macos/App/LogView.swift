import AppKit
import SwiftUI

/// Developer-mode view of the IME log: last lines, filterable.
struct LogView: View {
    @EnvironmentObject var model: AppModel
    @State private var lines: [String] = []
    @State private var filter = ""
    @FocusState private var filterFocused: Bool

    private var shown: [String] { StatsLogic.filter(lines, query: filter) }

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                TextField(Strings.filter, text: $filter)
                    .textFieldStyle(.roundedBorder)
                    .focused($filterFocused)
                    .onExitCommand { filter = "" }
                Button(Strings.openDataFolder) { NSWorkspace.shared.open(model.dataDir) }
                Button("") { filterFocused = true }
                    .keyboardShortcut("f")
                    .opacity(0)
                    .frame(width: 0, height: 0)
                    .accessibilityHidden(true)
            }
            if lines.isEmpty {
                Text(Strings.noLog).foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(Array(shown.enumerated()), id: \.offset) { _, line in
                    Text(line).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                }
            }
        }
        .padding(12)
        .navigationTitle(Strings.log)
        .onAppear(perform: reload)
        .onChange(of: model.dataRevision) { _ in reload() }
    }

    private func reload() { lines = StatsLogic.readLogTail(model.logURL) }
}
