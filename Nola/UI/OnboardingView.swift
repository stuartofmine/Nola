// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents Nola's four-stage, permission-safe first-run guide on iOS.

import SwiftUI

struct OnboardingView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    @AppStorage("hasCompletedOnboarding") private var hasCompletedOnboarding = false
    @AppStorage("onboarding.guide.version") private var completedGuideVersion = 0

    @State private var step: Int
    @State private var navigationDirection = 1

    private let totalSteps = 4

    init(initialStep: Int = 0) {
        _step = State(initialValue: min(max(initialStep, 0), 3))
    }

    var body: some View {
        ZStack {
            OnboardingBackground()

            VStack(spacing: 0) {
                header

                ScrollView {
                    stepContent
                        .id(step)
                        .frame(maxWidth: 680)
                        .frame(maxWidth: .infinity)
                        .padding(.horizontal, 22)
                        .padding(.vertical, 24)
                        .transition(stepTransition)
                }
                .scrollIndicators(.hidden)

                footer
            }
        }
        .preferredColorScheme(nil)
        .tint(DesignTokens.actionColor)
        .animation(reduceMotion ? nil : DesignTokens.revealAnimation, value: step)

    }

    private var stepTransition: AnyTransition {
        if reduceMotion {
            return .opacity
        }
        return .asymmetric(
            insertion: .opacity.combined(with: .move(edge: navigationDirection > 0 ? .trailing : .leading)),
            removal: .opacity.combined(with: .move(edge: navigationDirection > 0 ? .leading : .trailing))
        )
    }

    private var header: some View {
        VStack(spacing: 15) {
            HStack(spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 13, style: .continuous)
                        .fill(Color.accentColor.opacity(0.13))
                    Image(systemName: "sparkles")
                        .font(.system(size: 19, weight: .semibold))
                        .foregroundStyle(Color.accentColor)
                }
                .frame(width: 44, height: 44)

                VStack(alignment: .leading, spacing: 2) {
                    Text("Nola")
                        .font(.headline.weight(.bold))
                    if !dynamicTypeSize.isAccessibilitySize {
                        Text(localized("onboarding.core.brand_subtitle"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Text("\(step + 1)/\(totalSteps)")
                .accessibilityLabel(
                    String(
                        format: localized("onboarding.guide.step_format"),
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
                        .fill(index <= step ? Color.accentColor : Color.secondary.opacity(0.18))
                        .frame(height: 4)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(
                String(
                    format: localized("onboarding.guide.step_format"),
                    step + 1,
                    totalSteps
                )
            )
        }
        .padding(.horizontal, 22)
        .padding(.top, 18)
        .padding(.bottom, 4)
        .frame(maxWidth: 720)
        .frame(maxWidth: .infinity)
    }

    @ViewBuilder
    private var stepContent: some View {
        switch step {
        case 0:
            GuidePage(
                eyebrow: localized("onboarding.core.loop_eyebrow"),
                title: localized("onboarding.core.loop_title"),
                body: localized("onboarding.core.loop_body")
            ) {
                VStack(spacing: 0) {
                    GuideFlowRow(
                        icon: "rectangle.stack.fill",
                        color: .blue,
                        title: localized("onboarding.core.source_title"),
                        detail: localized("onboarding.core.source_detail")
                    )
                    GuideFlowConnector()
                    GuideFlowRow(
                        icon: "text.bubble.fill",
                        color: .teal,
                        title: localized("onboarding.core.answer_title"),
                        detail: localized("onboarding.core.answer_detail")
                    )
                    GuideFlowConnector()
                    GuideFlowRow(
                        icon: "arrow.triangle.2.circlepath",
                        color: .orange,
                        title: localized("onboarding.core.learn_title"),
                        detail: localized("onboarding.core.learn_detail")
                    )
                }
                .guideCard()

                GuideNote(icon: "lock.shield", text: localized("onboarding.core.local_note"))
            }
        case 1:
            GuidePage(
                eyebrow: localized("onboarding.guide.runtime.eyebrow"),
                title: localized("onboarding.guide.runtime.title"),
                body: localized("onboarding.guide.runtime.body")
            ) {
                VStack(spacing: 10) {
                    FirstTaskRow(
                        icon: "wand.and.stars",
                        title: localized("onboarding.guide.runtime.auto"),
                        detail: localized("onboarding.guide.runtime.auto_detail")
                    )
                    FirstTaskRow(
                        icon: "iphone",
                        title: localized("onboarding.guide.runtime.device"),
                        detail: localized("onboarding.guide.runtime.device_detail")
                    )
                    FirstTaskRow(
                        icon: "desktopcomputer",
                        title: localized("onboarding.guide.runtime.host"),
                        detail: localized("onboarding.guide.runtime.host_detail")
                    )
                }

                GuideNote(
                    icon: "checkmark.shield",
                    text: localized("onboarding.guide.runtime.note")
                )
            }
        case 2:
            GuidePage(
                eyebrow: localized("onboarding.core.sources_eyebrow"),
                title: localized("onboarding.guide.sources.title"),
                body: localized("onboarding.guide.sources.body")
            ) {
                VStack(spacing: 10) {
                    FirstTaskRow(
                        icon: "doc.text",
                        title: localized("onboarding.guide.source.documents"),
                        detail: localized("onboarding.guide.source.documents_detail")
                    )
                    FirstTaskRow(
                        icon: "calendar",
                        title: localized("onboarding.guide.source.calendar"),
                        detail: localized("onboarding.core.calendar_detail")
                    )
                    FirstTaskRow(
                        icon: "heart.fill",
                        title: localized("onboarding.guide.source.health"),
                        detail: localized("onboarding.core.health_detail")
                    )
                }

                GuideNote(
                    icon: "hand.raised",
                    text: localized("onboarding.guide.sources.permission_note")
                )
            }
        default:
            GuidePage(
                eyebrow: localized("onboarding.guide.ready.eyebrow"),
                title: localized("onboarding.guide.ready.title"),
                body: localized("onboarding.guide.ready.body")
            ) {
                VStack(spacing: 10) {
                    FirstTaskRow(
                        icon: "sun.max",
                        title: localized("onboarding.guide.task.day"),
                        detail: localized("onboarding.guide.task.day_detail")
                    )
                    FirstTaskRow(
                        icon: "doc.text.magnifyingglass",
                        title: localized("onboarding.guide.task.document"),
                        detail: localized("onboarding.guide.task.document_detail")
                    )
                    FirstTaskRow(
                        icon: "globe",
                        title: localized("onboarding.guide.task.research"),
                        detail: localized("onboarding.guide.task.research_detail")
                    )
                }

                GuideNote(
                    icon: "brain.head.profile",
                    text: localized("onboarding.guide.ready.memory_note")
                )
            }
        }
    }

    private var footer: some View {
        VStack(spacing: 8) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    backButton
                        .opacity(step > 0 ? 1 : 0)
                        .disabled(step == 0)
                        .accessibilityHidden(step == 0)
                    primaryButton
                }
                VStack(spacing: 10) {
                    primaryButton
                    if step > 0 {
                        backButton
                    }
                }
            }
            if step < totalSteps - 1 {
                Button(localized("onboarding.guide.action.later"), action: completeOnboarding)
                    .buttonStyle(.plain)
                    .font(.callout)
                    .frame(minWidth: 44, minHeight: 44)
                    .accessibilityIdentifier("onboarding.skip")
            }
        }
        .padding(.horizontal, 22)
        .padding(.top, 8)
        .padding(.bottom, 18)
        .frame(maxWidth: 720)
        .frame(maxWidth: .infinity)
        .background(.ultraThinMaterial)
    }

    private var backButton: some View {
        Button {
            navigationDirection = -1
            step -= 1
        } label: {
            Image(systemName: "chevron.left")
                .font(.system(size: 16, weight: .semibold))
                .frame(minWidth: 50, minHeight: 50)
        }
        .buttonStyle(.plain)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityLabel(localized("onboarding.guide.back"))
    }

    private var primaryButton: some View {
        Button {
            advance()
        } label: {
            HStack(spacing: 12) {
                Image(systemName: step == totalSteps - 1 ? "checkmark" : "arrow.right")
                    .hidden()
                Text(primaryTitle)
                    .fontWeight(.semibold)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                Image(systemName: step == totalSteps - 1 ? "checkmark" : "arrow.right")
            }
            .padding(.horizontal, 18)
            .frame(minHeight: 50)

        }
        .buttonStyle(.borderedProminent)
        .foregroundStyle(DesignTokens.onActionColor)
        .controlSize(.large)
        .accessibilityIdentifier("onboarding.next")
        .frame(maxWidth: .infinity)
    }

    private var primaryTitle: String {
        if step == totalSteps - 1 {
            return localized("onboarding.guide.action.start")
        }
        return localized("onboarding.guide.action.next")
    }

    private func advance() {
        if step < totalSteps - 1 {
            navigationDirection = 1
            step += 1
            return
        }
        completeOnboarding()
    }

    private func completeOnboarding() {
        completedGuideVersion = 2
        hasCompletedOnboarding = true
    }

    private func localized(_ key: String) -> String {
        NSLocalizedString(key, comment: "")
    }
}

private struct OnboardingBackground: View {
    var body: some View {
        LinearGradient(
            colors: [
                Color(.systemBackground),
                Color.accentColor.opacity(0.045),
                Color(.systemBackground)
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
        .ignoresSafeArea()
        .overlay(alignment: .topTrailing) {
            Circle()
                .fill(Color.teal.opacity(0.08))
                .frame(width: 260, height: 260)
                .blur(radius: 60)
                .offset(x: 90, y: -90)
        }
    }
}

private struct GuidePage<Content: View>: View {
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
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 9) {
                Text(eyebrow)
                    .font(.caption.weight(.bold))
                    .tracking(0.9)
                    .foregroundStyle(Color.accentColor)
                Text(title)
                    .font(.system(.largeTitle, design: .rounded, weight: .bold))
                    .fixedSize(horizontal: false, vertical: true)
                Text(bodyText)
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .lineSpacing(3)
                    .fixedSize(horizontal: false, vertical: true)
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct GuideFlowRow: View {
    let icon: String
    let color: Color
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(color)
                .frame(width: 38, height: 38)
                .background(color.opacity(0.12), in: RoundedRectangle(cornerRadius: 11))
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineSpacing(2)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Spacer()
        }
        .padding(.vertical, 4)
    }
}

private struct GuideFlowConnector: View {
    var body: some View {
        Rectangle()
            .fill(Color.secondary.opacity(0.18))
            .frame(width: 1, height: 14)
            .padding(.leading, 18)
    }
}

private struct FirstTaskRow: View {
    let icon: String
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 13) {
            Image(systemName: icon)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 38, height: 38)
                .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 11))
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineSpacing(2)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Spacer()
        }
        .padding(14)
        .background(
            Color(.secondarySystemBackground).opacity(0.78),
            in: RoundedRectangle(cornerRadius: 17, style: .continuous)
        )
    }
}

private struct GuideNote: View {
    let icon: String
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            Image(systemName: icon)
                .foregroundStyle(Color.accentColor)
            Text(text)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 13)
        .padding(.vertical, 11)
        .background(
            Color.accentColor.opacity(0.075),
            in: RoundedRectangle(cornerRadius: 15, style: .continuous)
        )
    }
}

private extension View {
    func guideCard() -> some View {
        padding(16)
            .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .stroke(Color.secondary.opacity(0.12))
            }
    }
}
