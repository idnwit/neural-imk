import SwiftUI

/// Sidebar sections; the raw value is the Cmd+<n> shortcut.
enum AppSection: Int, CaseIterable {
    case overview = 1, statistics, settings, log

    var title: String {
        switch self {
        case .overview: return Strings.overview
        case .statistics: return Strings.statistics
        case .settings: return Strings.settings
        case .log: return Strings.log
        }
    }

    var systemImage: String {
        switch self {
        case .overview: return "house"
        case .statistics: return "chart.bar"
        case .settings: return "gearshape"
        case .log: return "doc.plaintext"
        }
    }
}

struct RootView: View {
    @EnvironmentObject var model: AppModel

    private var sections: [AppSection] { AppSection.allCases.filter { $0 != .log || model.developerMode } }

    var body: some View {
        NavigationSplitView {
            List(sections, id: \.self, selection: sectionBinding) { s in
                Label(s.title, systemImage: s.systemImage).tag(s)
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180)
        } detail: {
            detail
        }
        .onAppear { model.launch() }
        .onChange(of: model.developerMode) { on in
            if !on && model.section == .log { model.section = .overview }
        }
        .sheet(isPresented: $model.showOnboarding) { OnboardingView().environmentObject(model) }
    }

    private var sectionBinding: Binding<AppSection?> {
        Binding(get: { model.section }, set: { if let s = $0 { model.section = s } })
    }

    @ViewBuilder private var detail: some View {
        switch model.section {
        case .overview: OverviewView()
        case .statistics: StatisticsView()
        case .settings: SettingsView()
        case .log: LogView()
        }
    }
}

/// Stand-in for a screen not built yet.
struct PlaceholderView: View {
    let title: String
    var body: some View {
        VStack(spacing: 8) {
            Text(title).font(.title2)
            Text(Strings.comingSoon).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
