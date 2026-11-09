import SwiftUI

/// Placeholder; the 4-step onboarding replaces it in Task 12.
struct OnboardingView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(spacing: 16) {
            Text(Strings.onboardingTitle).font(.title2)
            CheatSheetView()
            Button(Strings.close) { model.showOnboarding = false }
                .keyboardShortcut(.defaultAction)
        }
        .padding(24)
        .frame(minWidth: 420)
    }
}
