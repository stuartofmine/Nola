// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides the native macOS first-run guide and gates only the primary chat window.

import SwiftUI

struct MacOnboardingRootView<Workspace: View>: View {
    var bypass = MacOnboardingPolicy.shouldBypass
    @ViewBuilder let workspace: () -> Workspace
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage("nola.mac.onboarding.completed.v2") private var completed = false

    var body: some View {
        if completed || bypass {
            workspace()
        } else {
            MacOnboardingView {
                withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) {
                    completed = true
                }
            }
            .transition(.opacity)
        }
    }
}

private enum MacOnboardingPolicy {
    static var shouldBypass: Bool {
        let environment = ProcessInfo.processInfo.environment
        return environment["XCTestConfigurationFilePath"] != nil
            || environment["NOLA_SKIP_ONBOARDING"] == "1"
            || CommandLine.arguments.contains("--nola-background-scheduler-run")
            || CommandLine.arguments.contains("--nola-background-refresh-data")
            || CommandLine.arguments.contains("--nola-background-refresh-web-source")
    }
}

struct MacOnboardingView: View {
    @Environment(\.openWindow) private var openWindow
    @State private var isConnectHovered = false
    let isHelpWindow: Bool
    let onComplete: () -> Void

    init(isHelpWindow: Bool = false, onComplete: @escaping () -> Void) {
        self.isHelpWindow = isHelpWindow
        self.onComplete = onComplete
    }

    var body: some View {
        ZStack(alignment: .top) {
            MacPageBackground()
            MacWindowDragRegion()
                .frame(height: 40)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("mac.welcome.title")
                        .font(.largeTitle.weight(.semibold))
                        .accessibilityAddTraits(.isHeader)
                    Text("mac.welcome.description")
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)

                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 12) { actions }
                        VStack(alignment: .leading, spacing: 10) { actions }
                    }
                    .padding(.top, 8)
                }
                .frame(maxWidth: 460, alignment: .leading)
                .padding(32)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .scrollBounceBehavior(.basedOnSize)
            .defaultScrollAnchor(.center, for: .alignment)
            .padding(.top, 40)
        }
        .background {
            MacChatWindowChromeConfigurator(title: String(localized: "mac.welcome.title"))
                .frame(width: 0, height: 0)
        }
    }

    @ViewBuilder private var actions: some View {
        Button {
            MacModelCenterNavigationState.shared.showRemoteModels()
            openWindow(id: "model-center")
            onComplete()
        } label: {
            Text("mac.welcome.connect")
        }
        .buttonStyle(.nolaProminent)
        .keyboardShortcut(.defaultAction)
        .accessibilityIdentifier("onboarding.connect")

        Button(action: onComplete) {
            if isHelpWindow { Text("common.close") }
            else { Text("onboarding.guide.action.later") }
        }
        .buttonStyle(MacActionButtonStyle())
        .keyboardShortcut(.cancelAction)
        .accessibilityIdentifier("onboarding.dismiss")
    }
}

struct MacWelcomeHelpWindow: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        MacOnboardingView(isHelpWindow: true) { dismiss() }
    }
}
