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
    private(set) var actionError: String?

    init(service: any MacInspectorChromeMessaging) {
        self.service = service
        snapshot = service.snapshot(now: .now)
    }

    func refresh() {
        snapshot = service.snapshot(now: .now)
    }

    func installAndOpenExtensions() {
        actionError = nil
        do {
            try service.reinstall()
            openExtensions()
        } catch {
            actionError = String(localized: "The Chrome extension could not be installed. Check file access and try again.", table: "ConversationInspector")
            refresh()
        }
    }

    func openExtensions() {
        actionError = service.openChromeExtensions() ? nil : String(localized: "Chrome could not be opened. Install Chrome and try again.", table: "ConversationInspector")
        refresh()
    }
}
