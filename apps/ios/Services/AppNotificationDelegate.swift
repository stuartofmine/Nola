// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements AppNotificationDelegate logic in the iOS service layer.
// - Interfaces:
//   - AppNotificationDelegate
// - APIs:
//   - none
//
import Foundation
import UserNotifications

/// App-scoped UNUserNotificationCenter delegate to present notifications while app is foreground.
public final class AppNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    public static let shared = AppNotificationDelegate()

    private override init() { super.init() }

    // Present notifications as banners/sounds even when app is foreground
    public func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        // Debug: log willPresent invocation to help diagnose missing banners
        NSLog("[AppNotificationDelegate] willPresent notification id=%@ userInfo=%@", notification.request.identifier, String(describing: notification.request.content.userInfo))
        completionHandler([.banner, .sound, .badge])
    }

    // Handle user's response to notification (taps, actions)
    public func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        // Forward to orchestrator hook and include original payload userInfo so app can deep-link
        var info: [String: Any] = ["id": response.notification.request.identifier, "action": response.actionIdentifier]
        // include any userInfo supplied in the notification payload (e.g., eventId, dedupeKey, url)
        let payload = response.notification.request.content.userInfo
        for (k, v) in payload {
            // APNs payload arrives as [AnyHashable: Any]. Convert keys to String for downstream consumers.
            let key = (k as? String) ?? String(describing: k)
            info[key] = v
        }
        // Debug: log didReceive so we can verify taps are forwarded
        NSLog("[AppNotificationDelegate] didReceive response id=%@ action=%@ userInfo=%@", response.notification.request.identifier, response.actionIdentifier, String(describing: response.notification.request.content.userInfo))
        NotificationCenter.default.post(name: NSNotification.Name("com.nola.notification.response"), object: nil, userInfo: info)
        completionHandler()
    }
}
