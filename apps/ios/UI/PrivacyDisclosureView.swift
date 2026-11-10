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
            Section {
                Text(NSLocalizedString(
                    "privacy.disclosure.intro",
                    value: "Nola is open-source and local-first. Data is stored on this device by default.",
                    comment: "Privacy disclosure intro"
                ))
                .font(.subheadline)
                .foregroundColor(.secondary)
            } header: {
                Text(NSLocalizedString("privacy.disclosure.header", value: "Privacy", comment: "Privacy disclosure header"))
            }

            Section {
                bullet(NSLocalizedString("privacy.disclosure.principle.1", value: "Chat history and settings stay local by default.", comment: ""))
                bullet(NSLocalizedString("privacy.disclosure.principle.2", value: "Only the API requests you explicitly configure are sent to your chosen endpoint.", comment: ""))
                bullet(NSLocalizedString("privacy.disclosure.principle.3", value: "You can clear local data at any time from Settings.", comment: ""))
            } header: {
                Text(NSLocalizedString("privacy.disclosure.principles.header", value: "Principles", comment: ""))
            }
        }
        .navigationTitle(NSLocalizedString("privacy.disclosure.nav.title", value: "Privacy", comment: ""))
        .navigationBarTitleDisplayMode(.inline)
    }

    @ViewBuilder
    private func bullet(_ text: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text("•").foregroundColor(.secondary)
            Text(text).font(.footnote).foregroundColor(.secondary)
        }
    }
}

#Preview {
    NavigationStack { PrivacyDisclosureView() }
}
