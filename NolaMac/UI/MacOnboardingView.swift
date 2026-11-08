// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides the native macOS first-run guide and gates only the primary chat window.

import SwiftUI

struct MacOnboardingRootView: View {
    let canonicalRepository: MacCanonicalConversationRepository
    let localAgentRuntime: any MacLocalAgentRuntime
    let remoteHost: any MacRemoteAgentRunning
    let inspectorDependencies: MacInspectorDependencies
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage("nola.mac.onboarding.completed.v2") private var completed = false

    var body: some View {
        Group {
            if completed || MacOnboardingPolicy.shouldBypass {
                MacChatWorkspaceView(
                    canonicalRepository: canonicalRepository,
                    localAgentRuntime: localAgentRuntime,
                    remoteHost: remoteHost,
                    inspectorDependencies: inspectorDependencies
                )
            } else {
                MacOnboardingView {
                    withAnimation(reduceMotion ? nil : .easeOut(duration: 0.22)) {
                        completed = true
                    }
                }
                .transition(.opacity)
            }
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

private struct MacOnboardingView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @AppStorage("nola.mac.runtime.preference") private var runtimePreference = "automatic"
    @AppStorage("nola.mac.planned.sources") private var plannedSources = ""

    @State private var step = 0
    @State private var navigationDirection = 1
    @State private var selectedSources: Set<String> = ["workspace"]

    let onComplete: () -> Void

    private let totalSteps = 4

    var body: some View {
        ZStack {
            LinearGradient(
                colors: [
                    Color(nsColor: .windowBackgroundColor),
                    Color.accentColor.opacity(0.045),
                    Color(nsColor: .windowBackgroundColor)
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .ignoresSafeArea()

            Circle()
                .fill(Color.teal.opacity(0.07))
                .frame(width: 480, height: 480)
                .blur(radius: 100)
                .offset(x: 390, y: -290)

            VStack(spacing: 0) {
                header

                ScrollView {
                    page
                        .id(step)
                        .frame(maxWidth: 760)
                        .padding(.horizontal, 42)
                        .padding(.vertical, 34)
                        .transition(stepTransition)
                }
                .scrollIndicators(.never)

                footer
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.24), value: step)
        .onAppear {
            let restored = Set(
                plannedSources
                    .split(separator: ",")
                    .map(String.init)
                    .filter { !$0.isEmpty }
            )
            if !restored.isEmpty {
                selectedSources = restored
            }
        }
        .onChange(of: selectedSources) { _, value in
            plannedSources = value.sorted().joined(separator: ",")
        }
    }

    private var stepTransition: AnyTransition {
        if reduceMotion {
            return .opacity
        }
        return .asymmetric(
            insertion: .opacity.combined(with: .offset(x: navigationDirection > 0 ? 18 : -18)),
            removal: .opacity.combined(with: .offset(x: navigationDirection > 0 ? -12 : 12))
        )
    }

    private var header: some View {
        VStack(spacing: 16) {
            HStack(spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(Color.accentColor.opacity(0.13))
                    Image(systemName: "sparkles")
                        .font(.system(size: 18, weight: .semibold))
                        .foregroundStyle(Color.accentColor)
                }
                .frame(width: 42, height: 42)
                .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 2) {
                    Text("Nola")
                        .font(.headline.weight(.bold))
                    Text(String(localized: "onboarding.core.brand_subtitle"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                Button(String(localized: "onboarding.guide.action.later")) {
                    completeOnboarding()
                }
                .buttonStyle(.plain)
                .font(.callout.weight(.semibold))
                .foregroundStyle(.secondary)
                .frame(minWidth: 44, minHeight: 32)
                .contentShape(Rectangle())
                .help(String(localized: "onboarding.guide.action.later"))

                Text(
                    String(
                        format: String(localized: "onboarding.guide.step_format"),
                        step + 1,
                        totalSteps
                    )
                )
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            }

            HStack(spacing: 7) {
                ForEach(0..<totalSteps, id: \.self) { index in
                    Capsule()
                        .fill(index <= step ? Color.accentColor : Color.secondary.opacity(0.17))
                        .frame(height: 4)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(String(localized: "Progress"))
            .accessibilityValue(
                String(
                    format: String(localized: "onboarding.guide.step_format"),
                    step + 1,
                    totalSteps
                )
            )
        }
        .padding(.horizontal, 30)
        .padding(.top, 24)
        .frame(maxWidth: 840)
        .frame(maxWidth: .infinity)
    }

    @ViewBuilder
    private var page: some View {
        switch step {
        case 0:
            MacGuidePage(
                eyebrow: String(localized: "onboarding.core.loop_eyebrow"),
                title: String(localized: "onboarding.core.loop_title"),
                body: String(localized: "onboarding.core.loop_body")
            ) {
                VStack(spacing: 0) {
                    MacFlowRow(
                        icon: "rectangle.stack.fill",
                        color: .blue,
                        title: String(localized: "onboarding.core.source_title"),
                        detail: String(localized: "onboarding.core.source_detail")
                    )
                    MacFlowConnector()
                    MacFlowRow(
                        icon: "text.bubble.fill",
                        color: .teal,
                        title: String(localized: "onboarding.core.answer_title"),
                        detail: String(localized: "onboarding.core.answer_detail")
                    )
                    MacFlowConnector()
                    MacFlowRow(
                        icon: "arrow.triangle.2.circlepath",
                        color: .orange,
                        title: String(localized: "onboarding.core.learn_title"),
                        detail: String(localized: "onboarding.core.learn_detail")
                    )
                }
                .macGuideCard()

                MacGuideNote(
                    icon: "lock.shield",
                    text: String(localized: "onboarding.core.local_note")
                )
            }
        case 1:
            MacGuidePage(
                eyebrow: String(localized: "onboarding.guide.runtime.eyebrow"),
                title: String(localized: "onboarding.guide.runtime.title"),
                body: String(localized: "onboarding.guide.runtime.body")
            ) {
                VStack(spacing: 10) {
                    MacChoiceRow(
                        id: "automatic",
                        icon: "wand.and.stars",
                        title: String(localized: "onboarding.guide.runtime.auto"),
                        detail: String(localized: "onboarding.guide.runtime.auto_detail"),
                        selection: $runtimePreference
                    )
                    MacChoiceRow(
                        id: "device",
                        icon: "desktopcomputer",
                        title: String(localized: "onboarding.guide.runtime.device"),
                        detail: String(localized: "onboarding.guide.runtime.device_detail"),
                        selection: $runtimePreference
                    )
                    MacChoiceRow(
                        id: "host",
                        icon: "network",
                        title: String(localized: "onboarding.guide.runtime.host"),
                        detail: String(localized: "onboarding.guide.runtime.host_detail"),
                        selection: $runtimePreference
                    )
                }

                MacGuideNote(
                    icon: "checkmark.shield",
                    text: String(localized: "onboarding.guide.runtime.note")
                )
            }
        case 2:
            MacGuidePage(
                eyebrow: String(localized: "onboarding.core.sources_eyebrow"),
                title: String(localized: "onboarding.guide.sources.title"),
                body: String(localized: "onboarding.guide.sources.body")
            ) {
                VStack(spacing: 10) {
                    MacSourceRow(
                        id: "workspace",
                        icon: "folder",
                        color: .blue,
                        title: String(localized: "onboarding.guide.source.workspace"),
                        detail: String(localized: "onboarding.guide.source.workspace_detail"),
                        selectedSources: $selectedSources
                    )
                    MacSourceRow(
                        id: "web",
                        icon: "globe",
                        color: .teal,
                        title: String(localized: "onboarding.guide.source.web"),
                        detail: String(localized: "onboarding.guide.source.web_detail"),
                        selectedSources: $selectedSources
                    )
                    MacSourceRow(
                        id: "iphone",
                        icon: "iphone",
                        color: .red,
                        title: String(localized: "onboarding.guide.source.iphone"),
                        detail: String(localized: "onboarding.guide.source.iphone_detail"),
                        selectedSources: $selectedSources
                    )
                }

                MacGuideNote(
                    icon: "hand.raised",
                    text: String(localized: "onboarding.guide.sources.permission_note")
                )
            }
        default:
            MacGuidePage(
                eyebrow: String(localized: "onboarding.guide.ready.eyebrow"),
                title: String(localized: "onboarding.guide.ready.title"),
                body: String(localized: "onboarding.guide.ready.body")
            ) {
                VStack(spacing: 10) {
                    MacTaskRow(
                        icon: "hammer",
                        title: String(localized: "onboarding.guide.task.project"),
                        detail: String(localized: "onboarding.guide.task.project_detail")
                    )
                    MacTaskRow(
                        icon: "doc.text.magnifyingglass",
                        title: String(localized: "onboarding.guide.task.document"),
                        detail: String(localized: "onboarding.guide.task.document_detail")
                    )
                    MacTaskRow(
                        icon: "globe",
                        title: String(localized: "onboarding.guide.task.research"),
                        detail: String(localized: "onboarding.guide.task.research_detail")
                    )
                }

                MacGuideNote(
                    icon: "brain.head.profile",
                    text: String(localized: "onboarding.guide.ready.memory_note")
                )
            }
        }
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Group {
                if step > 0 {
                    Button {
                        navigationDirection = -1
                        step -= 1
                    } label: {
                        Image(systemName: "chevron.left")
                            .frame(width: 42, height: 42)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .help(String(localized: "onboarding.guide.back"))
                    .keyboardShortcut(.cancelAction)
                    .transition(.opacity)
                } else {
                    Color.clear
                        .frame(width: 44, height: 44)
                        .accessibilityHidden(true)
                }
            }

            Button {
                if step < totalSteps - 1 {
                    navigationDirection = 1
                    step += 1
                } else {
                    completeOnboarding()
                }
            } label: {
                HStack {
                    Text(
                        step == totalSteps - 1
                            ? String(localized: "onboarding.guide.action.start")
                            : String(localized: "onboarding.guide.action.next")
                    )
                    .fontWeight(.semibold)
                    Spacer()
                    Image(systemName: step == totalSteps - 1 ? "checkmark" : "arrow.right")
                }
                .padding(.horizontal, 16)
                .frame(height: 44)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .tint(Color.accentColor)
            .frame(maxWidth: 360)
            .keyboardShortcut(.defaultAction)

            Color.clear
                .frame(width: 44, height: 44)
                .accessibilityHidden(true)
        }
        .padding(.vertical, 20)
        .padding(.horizontal, 30)
        .frame(maxWidth: 840)
        .frame(maxWidth: .infinity)
        .background(.ultraThinMaterial)
    }

    private func completeOnboarding() {
        plannedSources = selectedSources.sorted().joined(separator: ",")
        onComplete()
    }
}

private struct MacGuidePage<Content: View>: View {
    let eyebrow: String
    let title: String
    let bodyText: String
    let content: Content

    init(
        eyebrow: String,
        title: String,
        body: String,
        @ViewBuilder content: () -> Content
    ) {
        self.eyebrow = eyebrow
        self.title = title
        self.bodyText = body
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            VStack(alignment: .leading, spacing: 9) {
                Text(eyebrow)
                    .font(.caption.weight(.bold))
                    .tracking(1)
                    .foregroundStyle(Color.accentColor)
                Text(title)
                    .font(.system(.largeTitle, design: .rounded, weight: .bold))
                    .fixedSize(horizontal: false, vertical: true)
                Text(bodyText)
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .lineSpacing(3)
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct MacFlowRow: View {
    let icon: String
    let color: Color
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(color)
                .frame(width: 38, height: 38)
                .background(color.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(detail).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
        }
    }
}

private struct MacFlowConnector: View {
    var body: some View {
        Rectangle()
            .fill(Color.secondary.opacity(0.18))
            .frame(width: 1, height: 15)
            .padding(.leading, 18)
            .accessibilityHidden(true)
    }
}

private struct MacChoiceRow: View {
    let id: String
    let icon: String
    let title: String
    let detail: String
    @Binding var selection: String
    @State private var isHovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button {
            selection = id
        } label: {
            HStack(spacing: 13) {
                Image(systemName: icon)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(selection == id ? Color.accentColor : Color.secondary)
                    .frame(width: 38, height: 38)
                    .background(Color.accentColor.opacity(selection == id ? 0.12 : 0.05), in: RoundedRectangle(cornerRadius: 10))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 3) {
                    Text(title).font(.subheadline.weight(.semibold))
                    Text(detail).font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Image(systemName: selection == id ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(selection == id ? Color.accentColor : Color.secondary.opacity(0.45))
                    .accessibilityHidden(true)
            }
            .padding(14)
            .contentShape(Rectangle())
            .background(
                selection == id
                    ? MacDesignColor.selected
                    : Color(nsColor: .controlBackgroundColor).opacity(isHovering ? 0.94 : 0.78),
                in: RoundedRectangle(cornerRadius: 16, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(Color.secondary.opacity(0.12))
            }
        }
        .buttonStyle(MacOnboardingChoiceButtonStyle())
        .onHover { hovering in
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.16)) {
                isHovering = hovering
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(title)
        .accessibilityValue(
            selection == id
                ? String(localized: "onboarding.guide.selection.selected")
                : String(localized: "onboarding.guide.selection.not_selected")
        )
        .accessibilityAddTraits(selection == id ? .isSelected : [])
    }
}

private struct MacSourceRow: View {
    let id: String
    let icon: String
    let color: Color
    let title: String
    let detail: String
    @Binding var selectedSources: Set<String>
    @State private var isHovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button {
            if selectedSources.contains(id) {
                selectedSources.remove(id)
            } else {
                selectedSources.insert(id)
            }
        } label: {
            HStack(spacing: 13) {
                Image(systemName: icon)
                    .foregroundStyle(color)
                    .frame(width: 38, height: 38)
                    .background(color.opacity(0.11), in: RoundedRectangle(cornerRadius: 10))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 3) {
                    Text(title).font(.subheadline.weight(.semibold))
                    Text(detail).font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Image(systemName: selectedSources.contains(id) ? "checkmark.square.fill" : "square")
                    .foregroundStyle(selectedSources.contains(id) ? Color.accentColor : Color.secondary.opacity(0.45))
                    .accessibilityHidden(true)
            }
            .padding(14)
            .contentShape(Rectangle())
            .background(
                selectedSources.contains(id)
                    ? MacDesignColor.selected
                    : Color(nsColor: .controlBackgroundColor).opacity(isHovering ? 0.94 : 0.78),
                in: RoundedRectangle(cornerRadius: 16, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(
                        selectedSources.contains(id) ? Color.accentColor.opacity(0.5) : Color.secondary.opacity(0.12)
                    )
            }
        }
        .buttonStyle(MacOnboardingChoiceButtonStyle())
        .onHover { hovering in
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.16)) {
                isHovering = hovering
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(title)
        .accessibilityValue(
            selectedSources.contains(id)
                ? String(localized: "onboarding.guide.selection.selected")
                : String(localized: "onboarding.guide.selection.not_selected")
        )
        .accessibilityAddTraits(selectedSources.contains(id) ? .isSelected : [])
    }
}

private struct MacTaskRow: View {
    let icon: String
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 13) {
            Image(systemName: icon)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 38, height: 38)
                .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(detail).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(14)
        .macPanel(cornerRadius: 16)
        .accessibilityElement(children: .combine)
    }
}

private struct MacGuideNote: View {
    let icon: String
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            Image(systemName: icon)
                .foregroundStyle(Color.accentColor)
                .accessibilityHidden(true)
            Text(text)
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 13)
        .padding(.vertical, 11)
        .background(
            Color.accentColor.opacity(0.075),
            in: RoundedRectangle(cornerRadius: 14, style: .continuous)
        )
    }
}

private extension View {
    func macGuideCard() -> some View {
        padding(17)
            .macPanel(cornerRadius: 20)
    }
}

private struct MacOnboardingChoiceButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed && !reduceMotion ? 0.992 : 1)
            .opacity(configuration.isPressed ? 0.88 : 1)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: configuration.isPressed)
    }
}
