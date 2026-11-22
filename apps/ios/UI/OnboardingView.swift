// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Offers model setup or direct entry to chat on first launch.

import SwiftUI

struct OnboardingView: View {
    let chooseModel: () -> Void
    let skip: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                VStack(spacing: 14) {
                    Text(verbatim: "n")
                        .font(.system(size: 32, weight: .semibold, design: .serif))
                        .italic()
                        .foregroundStyle(DesignTokens.actionColor)
                        .frame(width: 64, height: 64)
                        .background(DesignTokens.accentSubtleColor, in: Circle())
                        .overlay(Circle().stroke(DesignTokens.borderColor, lineWidth: 1))
                        .accessibilityHidden(true)

                    Text(verbatim: "Nola")
                        .font(.system(.largeTitle, design: .serif, weight: .semibold))
                        .foregroundStyle(DesignTokens.contentColor)
                        .accessibilityAddTraits(.isHeader)

                    Text("onboarding.first_chat.body")
                        .font(.body)
                        .foregroundStyle(DesignTokens.secondaryContentColor)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity)
                .padding(.top, 12)

                VStack(alignment: .leading, spacing: 12) {
                    capabilityRow(
                        icon: "iphone",
                        title: NSLocalizedString("onboarding.capability.local.title", value: "Private on-device models", comment: "Onboarding local model capability title"),
                        detail: NSLocalizedString("onboarding.capability.local.detail", value: "Download and run models directly on this device with zero cloud dependency.", comment: "Onboarding local model capability detail")
                    )
                    capabilityRow(
                        icon: "cloud",
                        title: NSLocalizedString("onboarding.capability.cloud.title", value: "Cloud API providers", comment: "Onboarding cloud API capability title"),
                        detail: NSLocalizedString("onboarding.capability.cloud.detail", value: "Connect OpenAI, Anthropic, DeepSeek, OpenRouter, or your own endpoint.", comment: "Onboarding cloud API capability detail")
                    )
                    capabilityRow(
                        icon: "desktopcomputer",
                        title: NSLocalizedString("onboarding.capability.host.title", value: "Nola Server on Mac", comment: "Onboarding Mac Host capability title"),
                        detail: NSLocalizedString("onboarding.capability.host.detail", value: "Pair with your Mac for project tools, skills, and longer-running work.", comment: "Onboarding Mac Host capability detail")
                    )
                }
            }
            .frame(maxWidth: 480, alignment: .leading)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 24)
            .padding(.vertical, 36)
        }
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 8) {
                Button(action: chooseModel) {
                    Text("onboarding.first_chat.choose_model")
                        .font(.headline)
                        .frame(maxWidth: .infinity, minHeight: 50)
                }
                .buttonStyle(NolaPrimaryButtonStyle(cornerRadius: 16))
                .accessibilityIdentifier("onboarding.chooseModel")
                Button(action: skip) {
                    Text("onboarding.guide.action.later")
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundStyle(DesignTokens.secondaryContentColor)
                .accessibilityIdentifier("onboarding.skip")
            }
            .frame(maxWidth: 480)
            .padding(.horizontal, 24)
            .padding(.vertical, 12)
        }
        .background(DesignTokens.backgroundColor.ignoresSafeArea())
    }

    private func capabilityRow(icon: String, title: String, detail: String) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(DesignTokens.actionColor)
                .frame(width: 36, height: 36)
                .background(DesignTokens.accentSubtleColor, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(DesignTokens.contentColor)
                Text(detail)
                    .font(.footnote)
                    .foregroundStyle(DesignTokens.secondaryContentColor)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(DesignTokens.surfaceColor, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}
