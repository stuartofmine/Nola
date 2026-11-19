// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Routes first launch and restores unfinished model setup.

import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var container: ServiceContainer
    @AppStorage("hasCompletedOnboarding") private var completed = false
    @AppStorage("onboarding.guide.version") private var guideVersion = 0
    @AppStorage("onboarding.modelSetupRequested") private var modelSetupRequested = false

    var onWorkspaceAppear: () -> Void = {}

    var body: some View {
        if completed {
            MainTabView().onAppear(perform: onWorkspaceAppear)
        } else {
            OnboardingView(chooseModel: { modelSetupRequested = true }, skip: finish)
                .sheet(isPresented: $modelSetupRequested) {
                    ModelsView { model in
                        container.chatManager.selectModel(model, persistAsDefault: false)
                        finish()
                    }
                }
        }
    }

    private func finish() {
        modelSetupRequested = false
        guideVersion = 3
        completed = true
    }
}
