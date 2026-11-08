// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationExtensions logic in the iOS service layer.
// - Interfaces:
//   - Notification
// - APIs:
//   - none
//
import Foundation

extension Notification.Name {
    /// Posted whenever a NotificationStore mutates its contents
    static let nolaNotificationsUpdated = Notification.Name("nola.notifications.updated")
    /// Posted whenever the custom URL data sources are changed (add/edit/delete/toggle)
    static let nolaCustomSourcesUpdated = Notification.Name("nola.custom_sources.updated")
}
