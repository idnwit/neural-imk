import SwiftUI

@main
struct NeuralIMKApp: App {
    @StateObject private var model = AppModel()
    var body: some Scene {
        WindowGroup("NeuralIMK") { RootView().environmentObject(model).frame(minWidth: 760, minHeight: 520) }
            .commands {
                CommandGroup(replacing: .appSettings) {
                    Button(Strings.settings) { model.section = .settings }.keyboardShortcut(",", modifiers: .command)
                }
                CommandMenu(Strings.view) {
                    ForEach(AppSection.allCases.filter { $0 != .log || model.developerMode }, id: \.self) { s in
                        Button(s.title) { model.section = s }.keyboardShortcut(KeyEquivalent(Character("\(s.rawValue)")), modifiers: .command)
                    }
                    Button(Strings.refresh) { model.refresh() }.keyboardShortcut("r", modifiers: .command)
                }
            }
    }
}
