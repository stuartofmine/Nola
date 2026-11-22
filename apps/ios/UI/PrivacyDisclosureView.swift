// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements PrivacyDisclosureView logic in the iOS UI layer.
// - Interfaces:
//   - PrivacyDisclosureView
// - APIs:
//   - none
//
import SwiftUI

struct PrivacyDisclosureView: View {
    var body: some View {
        Form {
            Group {
                Section {
                    Text("privacy.disclosure.intro")
                }
                Section {
                    Text("privacy.disclosure.principle.1")
                    Text("privacy.disclosure.principle.2")
                    Text("privacy.disclosure.principle.3")
                } header: {
                    Text("privacy.disclosure.principles.header")
                }
            }
            .font(.body)
            .foregroundStyle(DesignTokens.contentColor)
            .listRowBackground(DesignTokens.surfaceColor)
            .listRowSeparatorTint(DesignTokens.borderColor)
        }
        .nolaGroupedSurface()
        .navigationTitle(NSLocalizedString("privacy.disclosure.nav.title", value: "Privacy", comment: ""))
        .navigationBarTitleDisplayMode(.inline)
    }

}

#Preview {
    NavigationStack { PrivacyDisclosureView() }
}
