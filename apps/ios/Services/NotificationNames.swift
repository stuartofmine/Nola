// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationNames logic in the iOS service layer.
// - Interfaces:
//   - Notification
// - APIs:
//   - none
//
import Foundation

extension Notification.Name {
    static let showTaskConfirmation = Notification.Name("showTaskConfirmation")
    static let nolaOpenChatTab = Notification.Name("nola.openChatTab")
    static let nolaModelConfigured = Notification.Name("nola.modelConfigured")
    static let nolaOpenModelTab = Notification.Name("nola.openModelTab")
    static let nolaOpenSettingsTab = Notification.Name("nola.openSettingsTab")
    // Posted when a local identity token is refreshed.
    static let nolaDidVerifyReceipt = Notification.Name("nola.did.verifyReceipt")
    // Posted when the client requests config refresh.
    static let nolaDidRequestServerConfigSync = Notification.Name("nola.did.requestServerConfigSync")

    // Scheduled action runs updated (e.g., remote push result persisted)
    static let nolaScheduledActionRunsUpdated = Notification.Name("nola.scheduledActionRuns.updated")
}
