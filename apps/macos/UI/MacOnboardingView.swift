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
    private enum SetupPath: String, CaseIterable, Identifiable {
        case local
        case api
        case chatgpt

        var id: String { rawValue }

        var symbol: String {
            switch self {
            case .local: "cpu"
            case .api: "key.horizontal"
            case .chatgpt: "person.badge.key"
            }
        }

        var title: String {
            switch self {
            case .local: String(localized: "本地离线模型")
            case .api: String(localized: "连接 API 密钥")
            case .chatgpt: String(localized: "ChatGPT 授权")
            }
        }

        var subtitle: String {
            switch self {
            case .local: String(localized: "在 Mac 本地私密运行开源模型，无需联网")
            case .api: String(localized: "一键预设 OpenAI、Anthropic、DeepSeek、OpenRouter 或 Ollama")
            case .chatgpt: String(localized: "使用现有 ChatGPT 账号安全授权并选择可用模型")
            }
        }
    }

    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var selectedPath: SetupPath = .api
    @State private var hoveredPath: SetupPath?
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
                VStack(alignment: .leading, spacing: 20) {
                    VStack(alignment: .leading, spacing: 12) {
                        ZStack {
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .fill(MacDesignColor.brand)
                            Text(verbatim: "n")
                                .font(.system(size: 24, weight: .bold, design: .rounded))
                                .foregroundStyle(.white)
                                .offset(y: -1)
                        }
                        .frame(width: 44, height: 44)
                        .accessibilityHidden(true)

                        Text("mac.welcome.title")
                            .font(.largeTitle.weight(.semibold))
                            .accessibilityAddTraits(.isHeader)
                        Text("mac.welcome.description")
                            .font(.body)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }

                    VStack(spacing: 10) {
                        ForEach(SetupPath.allCases) { path in
                            setupPathCard(path)
                        }
                    }

                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 12) { actions }
                        VStack(alignment: .leading, spacing: 10) { actions }
                    }
                    .padding(.top, 4)
                }
                .frame(maxWidth: 500, alignment: .leading)
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

    private func setupPathCard(_ path: SetupPath) -> some View {
        let isSelected = selectedPath == path
        let isHovered = hoveredPath == path
        return Button {
            withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.responsive) {
                selectedPath = path
            }
        } label: {
            HStack(alignment: .center, spacing: 14) {
                Image(systemName: path.symbol)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(isSelected ? MacDesignColor.brand : .secondary)
                    .frame(width: 36, height: 36)
                    .background(
                        (isSelected ? MacDesignColor.brand : Color.primary).opacity(isSelected ? 0.12 : 0.05),
                        in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                    )
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 3) {
                    Text(path.title)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(.primary)
                    Text(path.subtitle)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(isSelected ? MacDesignColor.brand : Color.secondary.opacity(0.45))
                    .accessibilityHidden(true)
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                isSelected ? MacDesignColor.brand.opacity(0.06)
                    : (isHovered ? MacDesignColor.hover : MacDesignColor.panel),
                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous)
                    .strokeBorder(
                        isSelected ? MacDesignColor.brand : MacDesignColor.borderSubtle,
                        lineWidth: isSelected ? 1.5 : 1
                    )
            }
            .contentShape(RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            hoveredPath = hovering ? path : (hoveredPath == path ? nil : hoveredPath)
        }
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityIdentifier("onboarding.path." + path.rawValue)
    }

    private func openSelectedPath() {
        switch selectedPath {
        case .local:
            MacModelCenterNavigationState.shared.showLocalModels()
        case .api:
            MacModelCenterNavigationState.shared.showRemoteModels(action: .api)
        case .chatgpt:
            MacModelCenterNavigationState.shared.showRemoteModels(action: .accounts)
        }
        openWindow(id: "model-center")
        onComplete()
    }

    @ViewBuilder private var actions: some View {
        Button(action: openSelectedPath) {
            Text(selectedPath.title)
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
