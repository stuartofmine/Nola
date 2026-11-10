// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduledActionsOptionsView logic in the iOS UI layer.
// - Interfaces:
//   - ScheduledActionsOptionsView
// - APIs:
//   - none
//
import SwiftUI
import UIKit

/// Advanced options entry for scheduled actions.
struct ScheduledActionsOptionsView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss

    private var backgroundRefreshStatus: UIBackgroundRefreshStatus {
        UIApplication.shared.backgroundRefreshStatus
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    VStack(alignment: .leading, spacing: 10) {
                        if backgroundRefreshStatus != .available {
                            Text(L10n.backgroundRefreshDisabledMessage)
                                .font(.subheadline)
                                .foregroundColor(.secondary)

                            if ProcessInfo.processInfo.isLowPowerModeEnabled {
                                Text(L10n.backgroundRefreshLowPowerModeNote)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }

                            Button(L10n.backgroundRefreshOpenSystemSettings) {
                                guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
                                UIApplication.shared.open(url)
                            }
                            .buttonStyle(.bordered)

                            Text(L10n.backgroundRefreshHint)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        } else {
                            Text(L10n.backgroundRefreshSectionTitle)
                                .font(.subheadline)
                            Text(NSLocalizedString("scheduled.actions.options.ok", value: "OK", comment: "Background refresh ready status"))
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                    .padding(.vertical, 4)
                } header: {
                    Text(L10n.backgroundRefreshSectionTitle)
                }

                Section {
                    NavigationLink {
                        URLDataSourcesView()
                            .environmentObject(container)
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(L10n.scheduledActionsAdvancedManageTitle)
                            Text(L10n.scheduledActionsAdvancedManageSubtitle)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                } header: {
                    Text(L10n.scheduledActionsAdvancedOptionsTitle)
                }
            }
            .navigationTitle(L10n.scheduledActionsOptions)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.done) { dismiss() }
                }
            }
        }
    }
}
