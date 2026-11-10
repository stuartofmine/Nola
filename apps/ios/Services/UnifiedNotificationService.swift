// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements UnifiedNotificationService logic in the iOS service layer.
// - Interfaces:
//   - UnifiedNotificationService
//   - DefaultUnifiedNotificationService
//   - JSONValue
// - APIs:
//   - none
//
import Foundation
import UserNotifications

// Minimal unified notification service skeleton. Integrates with UNUserNotificationCenter
// and performs simple deduplication based on dedupeKey.

public protocol UnifiedNotificationService {
    // Schedule a notification. Returns true on success, false on failure or if deduped.
    func scheduleNotification(for event: ProcessedEvent, at date: Date?) async -> Bool
    func cancelNotifications(for dedupeKey: String)
}

public final class DefaultUnifiedNotificationService: UnifiedNotificationService {
    private let center: UNUserNotificationCenter
    private let logger = Logger.shared
    // In-memory dedupe set for this process; persistent dedupe should be added.
    private var scheduledKeys = Set<String>()

    public init(center: UNUserNotificationCenter = .current()) {
        self.center = center
    }

    private func ensureAuthorization() async -> Bool {
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .authorized, .provisional, .ephemeral:
            return true
        case .notDetermined:
            do {
                let granted = try await center.requestAuthorization(options: [.alert, .sound, .badge])
                return granted
            } catch {
                logger.log(.warn, "UnifiedNotificationService", "Notification authorization request failed", metadata: ["error": error.localizedDescription])
                return false
            }
        case .denied:
            logger.log(.info, "UnifiedNotificationService", "Notification authorization denied")
            return false
        @unknown default:
            logger.log(.warn, "UnifiedNotificationService", "Unknown notification authorization status")
            return false
        }
    }

    public func scheduleNotification(for event: ProcessedEvent, at date: Date? = nil) async -> Bool {
        if scheduledKeys.contains(event.dedupeKey) { return false }
        let authorized = await ensureAuthorization()
        guard authorized else { return false }

        let content = UNMutableNotificationContent()
        content.title = event.title
        content.body = event.provenance?["summary"]?.stringValue ?? event.title
        content.sound = .default
        content.badge = NSNumber(value: 1)
        content.userInfo = ["dedupeKey": event.dedupeKey, "eventId": event.id]

        let trigger: UNNotificationTrigger? = {
            if let d = date {
                let comps = Calendar.current.dateComponents([.year,.month,.day,.hour,.minute,.second], from: d)
                return UNCalendarNotificationTrigger(dateMatching: comps, repeats: false)
            }
            return nil
        }()

        let request = UNNotificationRequest(identifier: event.dedupeKey, content: content, trigger: trigger)

        let didSchedule: Bool = await withCheckedContinuation { cont in
            center.add(request) { error in
                if let e = error {
                    // Log or handle error
                    self.logger.log(.warn, "UnifiedNotificationService", "Failed to schedule notification", metadata: ["error": e.localizedDescription])
                    cont.resume(returning: false)
                    return
                }
                cont.resume(returning: true)
            }
        }
        if didSchedule {
            scheduledKeys.insert(event.dedupeKey)
        }
        return didSchedule
    }

    public func cancelNotifications(for dedupeKey: String) {
        center.removePendingNotificationRequests(withIdentifiers: [dedupeKey])
        scheduledKeys.remove(dedupeKey)
    }
}

// JSONValue helper extension for convenience
private extension JSONValue {
    var stringValue: String? {
        switch self {
        case .string(let s): return s
        case .number(let n): return String(n)
        case .bool(let b): return b ? "true" : "false"
        case .null: return nil
        case .array, .object: return nil
        }
    }
}
