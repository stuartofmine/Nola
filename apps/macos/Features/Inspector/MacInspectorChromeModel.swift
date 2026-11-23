// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Adapts Chrome messaging lifecycle and actions into inspector-owned observable state.

import Foundation
import Observation

protocol MacInspectorChromeMessaging: Sendable {
    func snapshot(now: Date) -> MacChromeBridgeSnapshot
    func reinstall() throws
    @discardableResult func openChromeExtensions() -> Bool
    @discardableResult func openBrowser() -> Bool
    func checkConnection(timeout: TimeInterval) -> MacChromeConnectionCheck
}

extension MacInspectorChromeMessaging {
    @discardableResult
    func openBrowser() -> Bool {
        openChromeExtensions()
    }

    func checkConnection(timeout: TimeInterval) -> MacChromeConnectionCheck {
        let current = snapshot(now: .now)
        if current.isExtensionConnected {
            return MacChromeConnectionCheck(
                succeeded: true,
                message: String(localized: "Browser extension is connected.")
            )
        }
        return MacChromeConnectionCheck(
            succeeded: false,
            message: String(localized: "Browser extension did not respond yet.")
        )
    }
}

extension MacChromeMessagingService: MacInspectorChromeMessaging {}

@MainActor
@Observable
final class MacInspectorChromeModel {
    private let service: any MacInspectorChromeMessaging
    private(set) var snapshot: MacChromeBridgeSnapshot
    private(set) var actionError: String?
    private(set) var isCheckingConnection = false

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

    func openBrowser() {
        actionError = service.openBrowser() ? nil : String(localized: "Chrome could not be opened. Install Chrome and try again.", table: "ConversationInspector")
        refresh()
    }

    func checkConnection() {
        guard !isCheckingConnection else { return }
        actionError = nil
        isCheckingConnection = true
        let service = self.service
        Task.detached(priority: .userInitiated) { [weak self] in
            let result = service.checkConnection(timeout: 5)
            await MainActor.run {
                guard let self else { return }
                self.isCheckingConnection = false
                self.actionError = result.succeeded ? nil : result.message
                self.refresh()
            }
        }
    }
}
