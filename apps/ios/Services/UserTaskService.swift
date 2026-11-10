// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements UserTaskService logic in the iOS service layer.
// - Interfaces:
//   - UserTaskService
// - APIs:
//   - none
//
import Foundation
import UserNotifications

public final class UserTaskService {
    private let storage: LocalJSONStorage
    private let unifiedStorage: UnifiedStorageService
    private let notificationService: UnifiedNotificationService
    private let notificationManager: NotificationManager?

    public init(storage: LocalJSONStorage, unifiedStorage: UnifiedStorageService, notificationService: UnifiedNotificationService, notificationManager: NotificationManager? = nil) {
        self.storage = storage
        self.unifiedStorage = unifiedStorage
        self.notificationService = notificationService
        self.notificationManager = notificationManager
    }

    public func loadAllTasks() throws -> [UserTask] {
        return try storage.getAllUserTasks()
    }

    public func saveTask(_ task: UserTask) throws {
        // Persist task to local JSON storage
        try storage.saveUserTask(task)

        // Convert to ProcessedEvent for unified storage & notification scheduling
        let dedupe = "task:\(task.id.uuidString)"
        let metadata: [String: String] = ["taskType": task.type.rawValue]
        let pe = ProcessedEvent(
            id: UUID().uuidString,
            source: EventSource.reminder,
            sourceType: .manual,
            sourceId: task.id.uuidString,
            title: task.title,
            date: task.targetDate,
            start: nil,
            due: task.dueDate.map { ISO8601DateFormatter().string(from: $0) },
            priority: 0.5,
            tags: nil,
            dedupeKey: dedupe,
            provenance: ["task": .string(task.description ?? "")],
            metadata: metadata,
            notes: task.description,
            hash: task.title.hashValue
        )

        try unifiedStorage.saveProcessedEvent(pe)
        // Debug: log that we've saved the ProcessedEvent
        Logger().log(.info, "UserTaskService", "Saved ProcessedEvent", metadata: ["title": pe.title, "dedupe": pe.dedupeKey, "sourceType": pe.sourceType.rawValue])

        // Notify listeners so Today/Home view can refresh processed events immediately
        do {
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            let data = try encoder.encode(pe)
            AppEventBus.postProcessedEventsUpdated(dedupeKey: dedupe, processedEventData: data)
            Logger().log(.info, "UserTaskService", "Posted processedEvents.updated notification", metadata: ["dedupe": dedupe])
        } catch {
            // Fallback: still post basic notification with dedupeKey only
            AppEventBus.postProcessedEventsUpdated(dedupeKey: dedupe)
            Logger().log(.info, "UserTaskService", "Posted processedEvents.updated notification (fallback)", metadata: ["dedupe": dedupe])
        }

        if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil {
            Logger().log(.debug, "UserTaskService", "Skipping notification scheduling during XCTest", metadata: ["dedupe": dedupe])
            return
        }

        // Schedule notifications according to task type
        Task {
            switch task.type {
            case .reminder:
                if let date = task.targetDate {
                    // Persist via NotificationManager first if available
                    if let nm = self.notificationManager {
                        do {
                            let candidate = NotificationCandidate(event: pe, dedupeKey: pe.dedupeKey, text: pe.title, actionType: .openReminder)
                            _ = try nm.post(candidate: candidate)
                        } catch {
                            Logger().log(.warn, "UserTaskService", "Failed to persist scheduled task notification", metadata: ["error": error.localizedDescription])
                        }
                    }
                    _ = await notificationService.scheduleNotification(for: pe, at: date)
                } else if task.isRecurring, task.recurrenceRule != nil {
                    // schedule next occurrence simplistically
                    if let next = calculateNextRecurrence(for: task) {
                        // Persist candidate and schedule
                        if let nm = self.notificationManager {
                            do {
                                let candidate = NotificationCandidate(event: pe, dedupeKey: pe.dedupeKey, text: pe.title, actionType: .openReminder)
                                _ = try nm.post(candidate: candidate)
                            } catch {
                                Logger().log(.warn, "UserTaskService", "Failed to persist recurring task notification", metadata: ["error": error.localizedDescription])
                            }
                        }
                        _ = await notificationService.scheduleNotification(for: pe, at: next)
                    }
                }
            case .birthday:
                if let date = task.targetDate {
                    let calendar = Calendar.current
                    if let threeBefore = calendar.date(byAdding: .day, value: -3, to: date) {
                        _ = await notificationService.scheduleNotification(for: pe, at: threeBefore)
                    }
                    _ = await notificationService.scheduleNotification(for: pe, at: date)
                }
            case .debt:
                if let due = task.dueDate {
                    let calendar = Calendar.current
                    if let oneBefore = calendar.date(byAdding: .day, value: -1, to: due) {
                        _ = await notificationService.scheduleNotification(for: pe, at: oneBefore)
                    }
                    _ = await notificationService.scheduleNotification(for: pe, at: due)
                }
            default:
                if let date = task.targetDate {
                    _ = await notificationService.scheduleNotification(for: pe, at: date)
                }
            }
        }
    }

    public func deleteTask(_ taskId: UUID) throws {
        try storage.deleteUserTask(id: taskId)
        // remove any ProcessedEvent and cancel notifications
        let dedupe = "task:\(taskId.uuidString)"
        do {
            try unifiedStorage.deleteProcessedEvent(withDedupeKey: dedupe)
            // notify the UI that this processed event is removed
            AppEventBus.postProcessedEventsRemoved(dedupeKey: dedupe)
        } catch {
            // Log and continue to ensure notifications are cancelled even on storage failure
            Logger().log(.warn, "UserTaskService", "Failed to remove processed event from storage", metadata: ["error": error.localizedDescription])
        }
        // Prefer cancelling persisted scheduled requests by entity id if available
        if let nm = self.notificationManager {
            nm.cancelPendingRequests(forDedupeKey: dedupe)
        } else {
            notificationService.cancelNotifications(for: dedupe)
        }
    }

    private func calculateNextRecurrence(for task: UserTask) -> Date? {
        guard let last = task.targetDate, let rule = task.recurrenceRule else { return nil }
        var next = last
        let calendar = Calendar.current
        switch rule.frequency {
        case .daily: next = calendar.date(byAdding: .day, value: rule.interval, to: next) ?? next
        case .weekly: next = calendar.date(byAdding: .weekOfYear, value: rule.interval, to: next) ?? next
        case .monthly: next = calendar.date(byAdding: .month, value: rule.interval, to: next) ?? next
        case .yearly: next = calendar.date(byAdding: .year, value: rule.interval, to: next) ?? next
        }
        if let end = rule.endDate, next > end { return nil }
        return next
    }
}
