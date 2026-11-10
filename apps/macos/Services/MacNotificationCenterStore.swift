// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents delivered and pending Nola notifications as a native desktop inbox.

import AppKit
import Foundation
import Observation
import UserNotifications

nonisolated struct MacNotificationInboxItem: Identifiable, Codable, Hashable, Sendable {
    enum DeliveryState: String, Codable, Hashable, Sendable {
        case delivered
        case scheduled
        case history
    }

    let identifier: String
    let title: String
    let body: String
    let date: Date
    let deliveryState: DeliveryState
    let conversationID: String?
    var isRead: Bool

    var id: String { identifier }
}

extension Notification.Name {
    nonisolated static let nolaMacNotificationsDidChange = Notification.Name("nola.mac.notifications.didChange")
}

actor MacNotificationHistoryRepository {
    static let shared = MacNotificationHistoryRepository()

    private let fileManager = FileManager.default
    private let maximumItems = 500

    func items() -> [MacNotificationInboxItem] {
        guard let data = try? Data(contentsOf: fileURL),
              let decoded = try? JSONDecoder().decode([MacNotificationInboxItem].self, from: data) else {
            return []
        }
        return decoded.sorted { $0.date > $1.date }
    }

    func record(_ item: MacNotificationInboxItem) {
        var values = items()
        values.removeAll { $0.identifier == item.identifier }
        values.insert(item, at: 0)
        persist(Array(values.prefix(maximumItems)))
        NotificationCenter.default.post(name: .nolaMacNotificationsDidChange, object: nil)
    }

    /// Durable notification consumers must not acknowledge a swallowed write error.
    func recordCommitted(_ item: MacNotificationInboxItem) throws {
        var values: [MacNotificationInboxItem] = []
        if fileManager.fileExists(atPath: fileURL.path) {
            values = try JSONDecoder().decode([MacNotificationInboxItem].self, from: Data(contentsOf: fileURL))
        }
        if values.contains(where: { $0.identifier == item.identifier }) { return }
        values.insert(item, at: 0)
        try fileManager.createDirectory(at: fileURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONEncoder().encode(Array(values.prefix(maximumItems))).write(to: fileURL, options: .atomic)
        NotificationCenter.default.post(name: .nolaMacNotificationsDidChange, object: nil)
    }

    func markRead(identifier: String) {
        var values = items()
        guard let index = values.firstIndex(where: { $0.identifier == identifier }) else { return }
        values[index].isRead = true
        persist(values)
        NotificationCenter.default.post(name: .nolaMacNotificationsDidChange, object: nil)
    }

    func markAllRead() {
        var values = items()
        for index in values.indices { values[index].isRead = true }
        persist(values)
        NotificationCenter.default.post(name: .nolaMacNotificationsDidChange, object: nil)
    }

    func remove(identifier: String) {
        var values = items()
        values.removeAll { $0.identifier == identifier }
        persist(values)
        NotificationCenter.default.post(name: .nolaMacNotificationsDidChange, object: nil)
    }

    func clear() {
        persist([])
        NotificationCenter.default.post(name: .nolaMacNotificationsDidChange, object: nil)
    }

    func replaceAll(_ restored: [MacNotificationInboxItem]) {
        let values = Array(restored.sorted { $0.date > $1.date }.prefix(maximumItems))
        persist(values)
        NotificationCenter.default.post(name: .nolaMacNotificationsDidChange, object: nil)
    }

    private func persist(_ values: [MacNotificationInboxItem]) {
        do {
            try fileManager.createDirectory(at: fileURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(values).write(to: fileURL, options: .atomic)
        } catch {
            let message = error.localizedDescription
            Task { @MainActor in
                Logger.shared.log(.warn, "MacNotificationHistory", "Unable to persist notification history", metadata: [
                    "error": message
                ])
            }
        }
    }

    private var fileURL: URL {
        (fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fileManager.temporaryDirectory)
            .appendingPathComponent("Nola", isDirectory: true)
            .appendingPathComponent("Notifications", isDirectory: true)
            .appendingPathComponent("history.json")
    }
}

@MainActor
@Observable
final class MacNotificationCenterStore {
    var items: [MacNotificationInboxItem] = []
    var authorizationStatus: UNAuthorizationStatus = .notDetermined
    var isLoading = false
    var errorMessage: String?

    private let center = UNUserNotificationCenter.current()

    var delivered: [MacNotificationInboxItem] {
        items.filter { $0.deliveryState == .delivered }
    }

    var scheduled: [MacNotificationInboxItem] {
        items.filter { $0.deliveryState == .scheduled }
    }

    var unreadCount: Int {
        items.filter { $0.deliveryState != .scheduled && !$0.isRead }.count
    }

    func refresh() async {
        guard !isLoading else { return }
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }

        let settings = await center.notificationSettings()
        let deliveredNotifications = await center.deliveredNotifications()
        let pendingRequests = await center.pendingNotificationRequests()
        let historyItems = await MacNotificationHistoryRepository.shared.items()
        authorizationStatus = settings.authorizationStatus

        let readByIdentifier = Dictionary(uniqueKeysWithValues: historyItems.map { ($0.identifier, $0.isRead) })

        let deliveredItems = deliveredNotifications.map { notification in
            makeItem(
                request: notification.request,
                date: notification.date,
                state: .delivered,
                isRead: readByIdentifier[notification.request.identifier] ?? false
            )
        }
        let pendingItems = pendingRequests.map { request in
            makeItem(
                request: request,
                date: scheduledDate(for: request.trigger),
                state: .scheduled,
                isRead: true
            )
        }
        let deliveredIdentifiers = Set(deliveredItems.map(\.identifier))
        let retainedHistory = historyItems
            .filter { !deliveredIdentifiers.contains($0.identifier) }
            .map {
                MacNotificationInboxItem(
                    identifier: $0.identifier,
                    title: $0.title,
                    body: $0.body,
                    date: $0.date,
                    deliveryState: .history,
                    conversationID: $0.conversationID,
                    isRead: $0.isRead
                )
            }
        items = (deliveredItems + retainedHistory + pendingItems).sorted { $0.date > $1.date }
        updateDockBadge()
    }

    func requestAuthorization() async {
        do {
            _ = try await center.requestAuthorization(options: [.alert, .badge, .sound])
            await refresh()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func remove(_ item: MacNotificationInboxItem) async {
        switch item.deliveryState {
        case .delivered, .history:
            center.removeDeliveredNotifications(withIdentifiers: [item.identifier])
        case .scheduled:
            center.removePendingNotificationRequests(withIdentifiers: [item.identifier])
        }
        await MacNotificationHistoryRepository.shared.remove(identifier: item.identifier)
        items.removeAll { $0.id == item.id }
        updateDockBadge()
    }

    func clearDelivered() async {
        center.removeAllDeliveredNotifications()
        await MacNotificationHistoryRepository.shared.clear()
        items.removeAll { $0.deliveryState != .scheduled }
        updateDockBadge()
    }

    func markRead(_ item: MacNotificationInboxItem) async {
        guard item.deliveryState != .scheduled else { return }
        await MacNotificationHistoryRepository.shared.markRead(identifier: item.identifier)
        if let index = items.firstIndex(where: { $0.identifier == item.identifier }) {
            items[index].isRead = true
        }
        updateDockBadge()
    }

    func markAllRead() async {
        await MacNotificationHistoryRepository.shared.markAllRead()
        for index in items.indices where items[index].deliveryState != .scheduled {
            items[index].isRead = true
        }
        updateDockBadge()
    }

    private func makeItem(
        request: UNNotificationRequest,
        date: Date,
        state: MacNotificationInboxItem.DeliveryState,
        isRead: Bool
    ) -> MacNotificationInboxItem {
        let content = request.content
        let title = content.title.isEmpty ? String(localized: "Nola notification") : content.title
        return MacNotificationInboxItem(
            identifier: request.identifier,
            title: title,
            body: content.body,
            date: date,
            deliveryState: state,
            conversationID: content.userInfo["conversation_id"] as? String,
            isRead: isRead
        )
    }

    private func updateDockBadge() {
        NSApplication.shared.dockTile.badgeLabel = unreadCount > 0 ? String(unreadCount) : nil
    }

    private func scheduledDate(for trigger: UNNotificationTrigger?) -> Date {
        if let calendarTrigger = trigger as? UNCalendarNotificationTrigger {
            return calendarTrigger.nextTriggerDate() ?? .now
        }
        if let intervalTrigger = trigger as? UNTimeIntervalNotificationTrigger {
            return Date().addingTimeInterval(intervalTrigger.timeInterval)
        }
        return .now
    }
}
