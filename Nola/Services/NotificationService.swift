// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationService logic in the iOS service layer.
// - Interfaces:
//   - NotificationPolicy
//   - NotificationPayload
//   - NotificationRequest
//   - NotificationServiceError
//   - NotificationServiceProtocol
//   - NotificationService
//   - sendNotification
// - APIs:
//   - none
//
import Foundation
import UserNotifications

// Minimal, JSON-serializable types for notification subsystem.
public struct NotificationPolicy: Codable {
    public var method: String // e.g. "localNotification" | "eventKit"
    public var throttleWindowMinutes: Int?
    public var allowQuiet: Bool?
    public init(method: String, throttleWindowMinutes: Int? = nil, allowQuiet: Bool? = nil) {
        self.method = method
        self.throttleWindowMinutes = throttleWindowMinutes
        self.allowQuiet = allowQuiet
    }
}

public struct NotificationPayload: Codable {
    public var title: String
    public var body: String?
    public var category: String?
    public var userInfo: [String: String]?
}

public struct NotificationRequest: Codable {
    public var id: String
    public var sourceRecordId: String?
    public var dedupeKey: String?
    public var priority: Double?
    public var deliveryHint: String? // now | schedule | eventKit
    public var scheduledAt: Date?
    public var notificationPolicy: NotificationPolicy?
    public var payload: NotificationPayload

    public init(id: String = UUID().uuidString,
                sourceRecordId: String? = nil,
                dedupeKey: String? = nil,
                priority: Double? = nil,
                deliveryHint: String? = nil,
                scheduledAt: Date? = nil,
                notificationPolicy: NotificationPolicy? = nil,
                payload: NotificationPayload) {
        self.id = id
        self.sourceRecordId = sourceRecordId
        self.dedupeKey = dedupeKey
        self.priority = priority
        self.deliveryHint = deliveryHint
        self.scheduledAt = scheduledAt
        self.notificationPolicy = notificationPolicy
        self.payload = payload
    }
}

public enum NotificationServiceError: Error {
    case permissionDenied
    case schedulingFailed(Error)
    case storeError
}

/// Public protocol for the notification subsystem.
/// NOTE: currently unused — retained as reference for a future orchestrator refactor.
/// Delete once callers adopt UnifiedNotificationService exclusively.
public protocol NotificationServiceProtocol {
    /// Enqueue a notification request. Returns notification id persisted.
    func enqueue(request: NotificationRequest) async throws -> String

    func cancel(notificationId: String) async throws
    func cancelByDedupeKey(_ dedupeKey: String) async throws
    func reschedule(notificationId: String, at date: Date) async throws
    func handleUserAction(notificationId: String, actionIdentifier: String)
    func getHistory(for date: Date) async -> [String]
}

//
//  NotificationService.swift
//  Nola
//
//  Manages local notifications with priority, deduplication, and action buttons.
//

import Foundation
import UserNotifications

/// Notification service for sending and managing local notifications.
public final class NotificationService {
    
    /// Notification priority levels
    public enum Priority: String, Codable {
        case high       // Urgent, immediate attention needed
        case medium     // Important but not urgent
        case low        // Nice to know, can be delayed
    }
    
    /// Configuration for notification behavior
    public struct Config {
        public var enableSound: Bool
        public var enableBadge: Bool
        public var maxNotificationsPerDay: Int
        public var deduplicationWindow: TimeInterval // seconds
        
        public static let `default` = Config(
            enableSound: true,
            enableBadge: true,
            maxNotificationsPerDay: 10,
            deduplicationWindow: 24 * 3600 // 24 hours
        )
    }
    
    private let config: Config
    private let logger: Logger
    private var sentNotifications: [String: Date] = [:] // dedupeKey -> sent time
    private var notificationMergeMap: [String: [String]] = [:] // dedupeKey -> merged keys

    // Persistence + synchronization: dedup window (24h default) must survive app relaunch,
    // and the in-memory dictionary must be safe to mutate from multiple async callers
    // (Scheduler runs on cooperative pool, drainAndReplayCandidates runs on @MainActor).
    // All access to sentNotifications / notificationMergeMap goes through stateQueue.
    private static let sentNotificationsDefaultsKey = "nola.notifications.sent.v1"
    private let stateQueue = DispatchQueue(label: "nola.notificationService.state", qos: .utility)

    public init(config: Config = .default, logger: Logger) {
        self.config = config
        self.logger = logger
        loadPersistedSentNotifications()
    }

    private func loadPersistedSentNotifications() {
        guard let raw = UserDefaults.standard.dictionary(forKey: Self.sentNotificationsDefaultsKey) as? [String: Double] else { return }
        let cutoff = Date().timeIntervalSince1970 - config.deduplicationWindow
        var restored: [String: Date] = [:]
        for (key, ts) in raw where ts >= cutoff {
            restored[key] = Date(timeIntervalSince1970: ts)
        }
        stateQueue.sync {
            self.sentNotifications = restored
        }
        if restored.count != raw.count {
            persistSentNotifications()
        }
    }

    /// Snapshot the dedup map (queue-protected) and asynchronously write it to UserDefaults.
    private func persistSentNotifications() {
        stateQueue.async {
            let snapshot = self.sentNotifications.mapValues { $0.timeIntervalSince1970 }
            UserDefaults.standard.set(snapshot, forKey: Self.sentNotificationsDefaultsKey)
        }
    }

    private func recordSent(_ dedupeKey: String, at date: Date = Date()) {
        stateQueue.sync {
            sentNotifications[dedupeKey] = date
        }
        persistSentNotifications()
    }

    private func sentDate(forKey key: String) -> Date? {
        return stateQueue.sync { sentNotifications[key] }
    }

    private func allSentDates() -> [Date] {
        return stateQueue.sync { Array(sentNotifications.values) }
    }

    private func removeSent(forKey key: String) {
        stateQueue.sync { _ = sentNotifications.removeValue(forKey: key) }
        persistSentNotifications()
    }

    private func removeAllSent() {
        stateQueue.sync { sentNotifications.removeAll() }
        persistSentNotifications()
    }

    // MARK: - Permissions

    private func ensureAuthorization(_ center: UNUserNotificationCenter) async throws {
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .authorized, .provisional, .ephemeral:
            return
        case .notDetermined:
            // Best-effort: request permission. In background this may fail silently.
            let granted = try await center.requestAuthorization(options: [.alert, .sound, .badge])
            if !granted {
                throw NotificationServiceError.permissionDenied
            }
        case .denied:
            throw NotificationServiceError.permissionDenied
        @unknown default:
            throw NotificationServiceError.permissionDenied
        }
    }
    
    // MARK: - Deduplication and Merging
    
    /// Check if notification should be skipped due to recent send
    private func shouldSkipDueToDuplication(_ dedupeKey: String) -> Bool {
        if let lastSent = sentDate(forKey: dedupeKey) {
            let elapsed = Date().timeIntervalSince(lastSent)
            return elapsed < config.deduplicationWindow
        }
        return false
    }
    
    /// Get all keys that have been merged into this key
    public func getMergedKeys(for dedupeKey: String) -> [String] {
        return stateQueue.sync { notificationMergeMap[dedupeKey] ?? [dedupeKey] }
    }
    
    /// Merge multiple dedupeKeys into one (elevates priority)
    public func mergeNotifications(_ keys: [String]) -> String {
        let mainKey = keys.first ?? "unknown"
        stateQueue.sync { notificationMergeMap[mainKey] = keys }
        return mainKey
    }
    
    // MARK: - Public Methods
    
    /// Send a notification for a candidate event
public func sendNotification(for candidate: NotificationCandidate, priority: Priority = .medium, notificationManager: NotificationManager? = nil) async throws {
        // Check deduplication
        if shouldSkipDueToDuplication(candidate.dedupeKey) {
            logger.log(.debug, "NotificationService", NSLocalizedString("autogen.NotificationService.f93b1b1c", comment: "AUTOGEN"), metadata: ["key": candidate.dedupeKey])
            return
        }

        // Check daily limit
        if hasReachedDailyLimit() {
            logger.log(.warn, "NotificationService", NSLocalizedString("autogen.NotificationService.78d2e66d", comment: "AUTOGEN"))
            return
        }

        let center = UNUserNotificationCenter.current()

        // Ensure system authorization exists (otherwise scheduling may succeed but never display)
        try await ensureAuthorization(center)

        // Persist via NotificationManager if available (idempotent)
        var persistedEntityId: String? = nil
        if let nm = notificationManager {
            do {
                let ne = try nm.post(candidate: candidate)
                persistedEntityId = ne.id
            } catch {
                logger.log(.warn, "NotificationService", "Failed to persist notification candidate", metadata: ["error": error.localizedDescription])
            }
        }

        // Create notification content
        let content = createNotificationContent(for: candidate, priority: priority)

        // Use persisted entity id as request identifier when available
        let identifier = persistedEntityId ?? candidate.dedupeKey

        // Create request
        let request = UNNotificationRequest(
            identifier: identifier,
            content: content,
            trigger: nil // Immediate delivery
        )

        // Send notification
        try await center.add(request)

        // Record sent notification
        recordSent(candidate.dedupeKey)
        if let id = persistedEntityId {
            try? notificationManager?.markAsSent(id: id)
        }
        logger.log(.info, "NotificationService", NSLocalizedString("autogen.NotificationService.7d112e59", comment: "AUTOGEN"), metadata: [
            "key": candidate.dedupeKey,
            "priority": priority.rawValue,
            "title": candidate.event.title
        ])
    }
    
    /// Send a scheduled notification at a specific time
    public func scheduleNotification(
        for candidate: NotificationCandidate,
        at date: Date,
        priority: Priority = .medium,
        notificationManager: NotificationManager? = nil
    ) async throws {
        // Check deduplication
        if shouldSkipDueToDuplication(candidate.dedupeKey) {
            logger.log(.debug, "NotificationService", NSLocalizedString("autogen.NotificationService.337bdb4b", comment: "AUTOGEN"), metadata: ["key": candidate.dedupeKey])
            return
        }

        let center = UNUserNotificationCenter.current()

        // Ensure system authorization exists (otherwise scheduling may succeed but never display)
        try await ensureAuthorization(center)

        // Persist via NotificationManager if available (idempotent)
        var persistedEntityId: String? = nil
        if let nm = notificationManager {
            do {
                let ne = try nm.post(candidate: candidate)
                persistedEntityId = ne.id
            } catch {
                logger.log(.warn, "NotificationService", "Failed to persist scheduled notification candidate", metadata: ["error": error.localizedDescription])
            }
        }

        // Create notification content
        let content = createNotificationContent(for: candidate, priority: priority)

        // Create trigger
        let components = Calendar.current.dateComponents([.year, .month, .day, .hour, .minute], from: date)
        let trigger = UNCalendarNotificationTrigger(dateMatching: components, repeats: false)

        // Use persisted id if available
        let identifier = persistedEntityId ?? candidate.dedupeKey

        // Create request
        let request = UNNotificationRequest(
            identifier: identifier,
            content: content,
            trigger: trigger
        )

        // Schedule notification
        try await center.add(request)

        // Record scheduled notification
        recordSent(candidate.dedupeKey)
        if let id = persistedEntityId {
            try? notificationManager?.markAsSent(id: id)
        }
        logger.log(.info, "NotificationService", NSLocalizedString("autogen.NotificationService.scheduled", comment: "AUTOGEN"), metadata: [
            "key": candidate.dedupeKey,
            "priority": priority.rawValue,
            "scheduledAt": ISO8601DateFormatter().string(from: date)
        ])
    }
    
    /// Cancel a pending notification
    public func cancelNotification(withKey key: String) {
        let center = UNUserNotificationCenter.current()
        center.removePendingNotificationRequests(withIdentifiers: [key])
        removeSent(forKey: key)
        logger.log(.info, "NotificationService", NSLocalizedString("autogen.NotificationService.ac4ec9aa", comment: "AUTOGEN"), metadata: ["key": key])
    }
    
    /// Cancel all pending notifications
    public func cancelAllNotifications() {
        let center = UNUserNotificationCenter.current()
        center.removeAllPendingNotificationRequests()
        removeAllSent()
        logger.log(.info, "NotificationService", NSLocalizedString("autogen.NotificationService.70ee1939", comment: "AUTOGEN"))
    }
    
    /// Clear notification history (for deduplication tracking)
    public func clearHistory() {
        removeAllSent()
        logger.log(.info, "NotificationService", NSLocalizedString("autogen.NotificationService.ccb981dd", comment: "AUTOGEN"))
    }
    
    /// Setup notification categories with action buttons
    public func setupNotificationCategories() async {
        let center = UNUserNotificationCenter.current()
        
        // Calendar action category
        let calendarCategory = createCalendarCategory()
        
        // Reminder action category
        let reminderCategory = createReminderCategory()
        
        // Contact/Birthday action category
        let contactCategory = createContactCategory()
        
        // Dismiss action category
        let dismissCategory = createDismissCategory()
        
        // Set all categories
        center.setNotificationCategories([
            calendarCategory,
            reminderCategory,
            contactCategory,
            dismissCategory
        ])
        
        logger.log(.info, "NotificationService", NSLocalizedString("autogen.NotificationService.cbd8dc66", comment: "AUTOGEN"))
    }
    
    // MARK: - Private Methods
    
    private func createNotificationContent(
        for candidate: NotificationCandidate,
        priority: Priority,
        createdAt: Date? = nil
    ) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        
        // Set title based on priority
        content.title = titleForPriority(priority)
        content.body = candidate.text
        
        // Set sound
        if config.enableSound {
            content.sound = priority == .high ? .defaultCritical : .default
        }
        
        // Set badge
        if config.enableBadge {
            content.badge = NSNumber(value: 1)
        }
        
        // Set category for action buttons
        content.categoryIdentifier = candidate.actionType.rawValue
        
        // Add user info. Use provided createdAt if available (canonical source), otherwise now.
        let createdAtDate = createdAt ?? Date()
        let createdAtString = ISO8601DateFormatter().string(from: createdAtDate)
        content.userInfo = [
            "eventId": candidate.event.id,
            "dedupeKey": candidate.dedupeKey,
            "priority": priority.rawValue,
            "source": candidate.event.source.rawValue,
            "createdAt": createdAtString
        ]
        
        // Set interruption level (iOS 15+)
        if #available(iOS 15.0, *) {
            switch priority {
            case .high:
                content.interruptionLevel = .timeSensitive
            case .medium:
                content.interruptionLevel = .active
            case .low:
                content.interruptionLevel = .passive
            }
        }
        
        return content
    }
    
    private func titleForPriority(_ priority: Priority) -> String {
        switch priority {
        case .high:
            return NSLocalizedString("autogen.NotificationService.24221727", comment: "AUTOGEN")
        case .medium:
            return NSLocalizedString("autogen.NotificationService.b92a5a6b", comment: "AUTOGEN")
        case .low:
            return NSLocalizedString("autogen.NotificationService.cd639880", comment: "AUTOGEN")
        }
    }
    
    private func hasReachedDailyLimit() -> Bool {
        let today = Calendar.current.startOfDay(for: Date())
        let todayNotifications = allSentDates().filter { sentDate in
            Calendar.current.isDate(sentDate, inSameDayAs: today)
        }
        return todayNotifications.count >= config.maxNotificationsPerDay
    }
    
    // MARK: - Action Categories
    
    private func createCalendarCategory() -> UNNotificationCategory {
        let openAction = UNNotificationAction(
            identifier: "OPEN_CALENDAR",
            title: NSLocalizedString("autogen.NotificationService.2a8dbcfb", comment: "AUTOGEN"),
            options: [.foreground]
        )
        
        let dismissAction = UNNotificationAction(
            identifier: "DISMISS",
            title: NSLocalizedString("autogen.NotificationService.d84129b8", comment: "AUTOGEN"),
            options: []
        )
        
        return UNNotificationCategory(
            identifier: ActionType.openCalendar.rawValue,
            actions: [openAction, dismissAction],
            intentIdentifiers: [],
            options: []
        )
    }
    
    private func createReminderCategory() -> UNNotificationCategory {
        let openAction = UNNotificationAction(
            identifier: "OPEN_REMINDER",
            title: NSLocalizedString("autogen.NotificationService.766ccd69", comment: "AUTOGEN"),
            options: [.foreground]
        )
        
        let dismissAction = UNNotificationAction(
            identifier: "DISMISS",
            title: NSLocalizedString("autogen.NotificationService.d84129b8", comment: "AUTOGEN"),
            options: []
        )
        
        return UNNotificationCategory(
            identifier: ActionType.openReminder.rawValue,
            actions: [openAction, dismissAction],
            intentIdentifiers: [],
            options: []
        )
    }
    
    private func createContactCategory() -> UNNotificationCategory {
        let openAction = UNNotificationAction(
            identifier: "OPEN_CALENDAR",
            title: NSLocalizedString("autogen.NotificationService.ed2cc287", comment: "AUTOGEN"),
            options: [.foreground]
        )
        
        let dismissAction = UNNotificationAction(
            identifier: "DISMISS",
            title: NSLocalizedString("autogen.NotificationService.c69a9153", comment: "AUTOGEN"),
            options: []
        )
        
        return UNNotificationCategory(
            identifier: "contact_birthday",
            actions: [openAction, dismissAction],
            intentIdentifiers: [],
            options: []
        )
    }
    
    private func createDismissCategory() -> UNNotificationCategory {
        let dismissAction = UNNotificationAction(
            identifier: "DISMISS",
            title: NSLocalizedString("autogen.NotificationService.cb63c62e", comment: "AUTOGEN"),
            options: []
        )
        
        return UNNotificationCategory(
            identifier: ActionType.dismiss.rawValue,
            actions: [dismissAction],
            intentIdentifiers: [],
            options: []
        )
    }
}

// MARK: - Notification Priority Extension

extension NotificationService {
    
    /// Determine priority based on event properties
    public static func determinePriority(for event: ProcessedEvent) -> Priority {
        // High priority conditions
        if event.priority >= 0.8 {
            return .high
        }
        
        // Check time sensitivity
        if let date = event.date {
            let days = Calendar.current.dateComponents([.day], from: Date(), to: date).day ?? 0
            
            // Within 24 hours = high priority
            if days == 0 {
                return .high
            }
            
            // Within 3 days and high score = medium
            if days <= 3 && event.priority >= 0.6 {
                return .medium
            }
        }
        
        // Default to low for everything else
        return .low
    }
}
