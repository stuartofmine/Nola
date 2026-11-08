// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Delivers optional local notifications when a background agent run finishes.

import AppKit
import Foundation
import UserNotifications

extension Notification.Name {
    nonisolated static let nolaMacOpenConversation = Notification.Name("nola.mac.openConversation")
}

final class MacNotificationRouter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = MacNotificationRouter()
    private static let pendingConversationKey = "nola.mac.notifications.pendingConversation"

    func consumePendingConversationID() -> String? {
        let defaults = UserDefaults.standard
        guard let value = defaults.string(forKey: Self.pendingConversationKey), !value.isEmpty else { return nil }
        defaults.removeObject(forKey: Self.pendingConversationKey)
        return value
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        defer { completionHandler() }
        guard let conversationID = response.notification.request.content.userInfo["conversation_id"] as? String,
              !conversationID.isEmpty else { return }
        UserDefaults.standard.set(conversationID, forKey: Self.pendingConversationKey)
        DispatchQueue.main.async {
            NSApplication.shared.activate(ignoringOtherApps: true)
            NotificationCenter.default.post(
                name: .nolaMacOpenConversation,
                object: nil,
                userInfo: ["conversation_id": conversationID]
            )
        }
    }
}

nonisolated enum MacRunNotificationPreferenceKeys {
    static let notificationsEnabled = "enableNotifications"
    static let stepGuidance = "enableStepReminders"
    static let aggregateCalendar = "aggregateCalendarNotifications"
    static let frequency = "notificationFrequency"
    static let agentCompletion = "nola.mac.notifications.agentCompletion"

    static var areNotificationsEnabled: Bool {
        UserDefaults.standard.object(forKey: notificationsEnabled) as? Bool ?? true
    }
}

actor MacRunNotificationService {
    static let shared = MacRunNotificationService()

    nonisolated struct AgentCompletionNotification: Sendable, Equatable {
        let title: String
        let body: String
        let conversationID: String
    }

    private let completionNotificationsEnabled: @Sendable () -> Bool
    private let deliverAgentCompletion: @Sendable (AgentCompletionNotification) async -> Void

    init(
        completionNotificationsEnabled: @escaping @Sendable () -> Bool = {
            MacRunNotificationPreferenceKeys.areNotificationsEnabled
                && UserDefaults.standard.bool(forKey: MacRunNotificationPreferenceKeys.agentCompletion)
        },
        deliverAgentCompletion: @escaping @Sendable (AgentCompletionNotification) async -> Void = {
            await MacRunNotificationService.deliverSystemAgentCompletion($0)
        }
    ) {
        self.completionNotificationsEnabled = completionNotificationsEnabled
        self.deliverAgentCompletion = deliverAgentCompletion
    }

    func setNotificationsEnabled(_ enabled: Bool) async -> Bool {
        guard enabled else {
            UserDefaults.standard.set(false, forKey: MacRunNotificationPreferenceKeys.notificationsEnabled)
            return false
        }
        do {
            let granted = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])
            UserDefaults.standard.set(granted, forKey: MacRunNotificationPreferenceKeys.notificationsEnabled)
            return granted
        } catch {
            UserDefaults.standard.set(false, forKey: MacRunNotificationPreferenceKeys.notificationsEnabled)
            return false
        }
    }

    func setAgentCompletionEnabled(_ enabled: Bool) async -> Bool {
        guard enabled else {
            UserDefaults.standard.set(false, forKey: MacRunNotificationPreferenceKeys.agentCompletion)
            return false
        }
        guard MacRunNotificationPreferenceKeys.areNotificationsEnabled else { return false }
        do {
            let granted = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])
            UserDefaults.standard.set(granted, forKey: MacRunNotificationPreferenceKeys.agentCompletion)
            return granted
        } catch {
            UserDefaults.standard.set(false, forKey: MacRunNotificationPreferenceKeys.agentCompletion)
            return false
        }
    }

    func notifyAgentCompletion(conversationTitle: String, conversationID: String, status: MacRunCompletionStatus) async {
        // Cancellation is a distinct canonical outcome, not an attention/failure alert.
        guard status != .cancelled, completionNotificationsEnabled() else { return }
        let title = switch status {
        case .answered: String(localized: "Nola answered")
        case .actionCompleted: String(localized: "Nola completed the action")
        case .delegationAccepted: String(localized: "Nola delegated the task")
        case .incomplete: String(localized: "Nola has unfinished work")
        case .blocked: String(localized: "Nola is blocked")
        case .failed: String(localized: "Nola encountered an error")
        case .cancelled: String(localized: "Cancelled")
        }
        await deliverAgentCompletion(AgentCompletionNotification(
            title: title,
            body: conversationTitle,
            conversationID: conversationID
        ))
    }

    private static func deliverSystemAgentCompletion(_ notification: AgentCompletionNotification) async {
        let content = UNMutableNotificationContent()
        content.title = notification.title
        content.body = notification.body
        content.sound = .default
        content.categoryIdentifier = "NOLA_AGENT_COMPLETION"
        content.userInfo = ["conversation_id": notification.conversationID]
        let request = UNNotificationRequest(
            identifier: "agent-completion-\(UUID().uuidString)",
            content: content,
            trigger: nil
        )
        await MacNotificationHistoryRepository.shared.record(MacNotificationInboxItem(
            identifier: request.identifier,
            title: content.title,
            body: content.body,
            date: .now,
            deliveryState: .history,
            conversationID: notification.conversationID,
            isRead: false
        ))
        try? await UNUserNotificationCenter.current().add(request)
    }

    func notifyAutomationCompletion(name: String, receipt: MacAutomationExecutionReceipt) async throws {
        let content = UNMutableNotificationContent()
        content.title = switch receipt.terminal.result.value.kind {
        case .answered: String(localized: "Nola answered")
        case .actionCompleted: String(localized: "Nola completed the action")
        case .delegationAccepted: String(localized: "Nola delegated the task")
        case .incomplete: String(localized: "Nola has unfinished work")
        case .blocked: String(localized: "Nola is blocked")
        case .failed: String(localized: "Nola encountered an error")
        case .cancelled: String(localized: "Cancelled")
        }
        content.body = name
        content.sound = .default
        content.categoryIdentifier = "NOLA_AUTOMATION_COMPLETION"
        content.userInfo = ["conversation_id": receipt.binding.conversationID]
        let request = UNNotificationRequest(
            identifier: receipt.notificationID,
            content: content,
            trigger: nil
        )
        try await MacNotificationHistoryRepository.shared.recordCommitted(MacNotificationInboxItem(
            identifier: request.identifier,
            title: content.title,
            body: content.body,
            date: .now,
            deliveryState: .history,
            conversationID: receipt.binding.conversationID,
            isRead: false
        ))
        // ACK means committed local inbox delivery, not that the user saw an OS alert.
        if MacRunNotificationPreferenceKeys.areNotificationsEnabled {
            try? await UNUserNotificationCenter.current().add(request)
        }
    }
}
