// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements AppEventBus logic in the iOS service layer.
// - Interfaces:
//   - AppEventBus
//   - Notification
// - APIs:
//   - none
//
import Foundation

/// Centralized app-wide event posting facade.
///
/// Benefits:
/// - avoid scattered raw notification names
/// - keep cross-module flow triggers consistent
/// - reduce duplicated NotificationCenter boilerplate
public enum AppEventBus {
    public static func postUpgradeRequested(syncConfigFirst: Bool = true) {
        if syncConfigFirst {
            NotificationCenter.default.post(name: .nolaDidRequestServerConfigSync, object: nil)
        }
        NotificationCenter.default.post(name: .nolaOpenSettingsTab, object: nil)
    }

    public static func postProcessedEventsUpdated(dedupeKey: String, processedEventData: Data? = nil) {
        var userInfo: [String: Any] = ["dedupeKey": dedupeKey]
        if let processedEventData {
            userInfo["processedEvent"] = processedEventData
        }
        NotificationCenter.default.post(name: .processedEventsUpdated, object: nil, userInfo: userInfo)
    }

    public static func postProcessedEventsRemoved(dedupeKey: String) {
        NotificationCenter.default.post(name: .processedEventsRemoved, object: nil, userInfo: ["dedupeKey": dedupeKey])
    }

    public static func postRemoteNotification(_ userInfo: [AnyHashable: Any]) {
        NotificationCenter.default.post(name: .remoteNotificationReceived, object: nil, userInfo: userInfo)
    }

    public static func postNotificationResponse(_ userInfo: [AnyHashable: Any]) {
        NotificationCenter.default.post(name: .notificationResponseReceived, object: nil, userInfo: userInfo)
    }

    public static func postNotificationsUpdated() {
        NotificationCenter.default.post(name: .nolaNotificationsUpdated, object: nil)
    }
}

public extension Notification.Name {
    static let processedEventsUpdated = Notification.Name("com.nola.processedEvents.updated")
    static let processedEventsRemoved = Notification.Name("com.nola.processedEvents.removed")
    static let remoteNotificationReceived = Notification.Name("com.nola.remote.notification")
    static let notificationResponseReceived = Notification.Name("com.nola.notification.response")
}
