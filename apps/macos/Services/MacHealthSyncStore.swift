// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Observes iCloud KVS for aggregate Health snapshots published by Nola on iPhone.

import Foundation
import Observation

@MainActor
@Observable
final class MacHealthSyncStore {
    static let shared = MacHealthSyncStore()

    var snapshot: HealthSummarySyncPayload?
    var isSyncing = false
    let cloudSyncAvailable = HealthSummarySync.cloudSyncAvailable

    @ObservationIgnored private var notificationToken: NSObjectProtocol?

    private init() {
        snapshot = HealthSummarySync.latest()
        guard cloudSyncAvailable else { return }
        notificationToken = NotificationCenter.default.addObserver(
            forName: NSUbiquitousKeyValueStore.didChangeExternallyNotification,
            object: NSUbiquitousKeyValueStore.default,
            queue: .main
        ) { _ in
            Task { @MainActor in MacHealthSyncStore.shared.reload() }
        }
        HealthSummarySync.synchronize()
    }

    deinit {
        if let notificationToken { NotificationCenter.default.removeObserver(notificationToken) }
    }

    func reload() {
        snapshot = HealthSummarySync.latest()
    }

    func syncNow() async {
        guard cloudSyncAvailable, !isSyncing else { return }
        isSyncing = true
        defer { isSyncing = false }
        HealthSummarySync.synchronize()
        try? await Task.sleep(for: .milliseconds(500))
        reload()
    }
}
