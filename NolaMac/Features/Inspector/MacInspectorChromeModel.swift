// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Adapts Chrome messaging lifecycle and actions into inspector-owned observable state.

import Foundation
import Observation

protocol MacInspectorChromeMessaging: Sendable {
    func snapshot(now: Date) -> MacChromeBridgeSnapshot
    func reinstall() throws
    @discardableResult func openChromeExtensions() -> Bool
}

extension MacChromeMessagingService: MacInspectorChromeMessaging {}

@MainActor
@Observable
final class MacInspectorChromeModel {
    private let service: any MacInspectorChromeMessaging
    private(set) var snapshot: MacChromeBridgeSnapshot

    init(service: any MacInspectorChromeMessaging) {
        self.service = service
        snapshot = service.snapshot(now: .now)
    }

    func refresh() {
        snapshot = service.snapshot(now: .now)
    }

    func installAndOpenExtensions() {
        try? service.reinstall()
        _ = service.openChromeExtensions()
        refresh()
    }

    func openExtensions() {
        _ = service.openChromeExtensions()
        refresh()
    }
}
