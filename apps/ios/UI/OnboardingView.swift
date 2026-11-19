// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Offers model setup or direct entry to chat on first launch.

import SwiftUI

struct OnboardingView: View {
    let chooseModel: () -> Void
    let skip: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(verbatim: "Nola")
                    .font(.largeTitle.bold())
                    .accessibilityAddTraits(.isHeader)
                Text("onboarding.first_chat.body")
                    .font(.title3)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: 480, alignment: .leading)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 24)
            .padding(.vertical, 48)
        }
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 8) {
                Button(action: chooseModel) {
                    Text("onboarding.first_chat.choose_model")
                        .font(.headline)
                        .frame(maxWidth: .infinity, minHeight: 50)
                        .foregroundStyle(DesignTokens.onActionColor)
                        .background(DesignTokens.actionColor, in: RoundedRectangle(cornerRadius: 16))
                }
                .buttonStyle(NolaPressableButtonStyle())
                .accessibilityIdentifier("onboarding.chooseModel")
                Button(action: skip) {
                    Text("onboarding.guide.action.later")
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("onboarding.skip")
            }
            .frame(maxWidth: 480)
            .padding(.horizontal, 24)
            .padding(.vertical, 12)
        }
        .background(DesignTokens.backgroundColor.ignoresSafeArea())
    }
}
