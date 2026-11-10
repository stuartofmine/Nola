// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationManager logic in the iOS service layer.
// - Interfaces:
//   - NotificationManager
// - APIs:
//   - none
//
import Foundation
import Combine
import UserNotifications

@MainActor
public final class NotificationManager: ObservableObject {
    private let store: NotificationStore
    @Published public private(set) var unreadCount: Int = 0
    private var notificationObserver: NSObjectProtocol? = nil

    public init(store: NotificationStore) {
        self.store = store
        refreshUnread()
        // Observe store-level updates to refresh unread count
        notificationObserver = NotificationCenter.default.addObserver(forName: .nolaNotificationsUpdated, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.refreshUnread()
            }
        }
    }

    public convenience init() {
        self.init(store: InMemoryNotificationStore())
    }

    deinit {
        if let obs = notificationObserver { NotificationCenter.default.removeObserver(obs) }
    }

    private func refreshUnread() {
        do {
            let all = try store.fetchAll()
            self.unreadCount = all.filter { !$0.isRead }.count
        } catch {
            self.unreadCount = 0
        }
        updateAppBadge()
    }

    private func updateAppBadge() {
        UNUserNotificationCenter.current().setBadgeCount(max(0, unreadCount)) { _ in }
    }

    @discardableResult
    public func post(candidate: NotificationCandidate) throws -> NotificationEntity {
        // If a notification with the same dedupeKey already exists, return it (idempotent)
        if !candidate.dedupeKey.isEmpty {
            let found = try store.findByDedupeKey(candidate.dedupeKey)
            if let first = found.first { return first }
        }

        let e = NotificationEntity(requestId: UUID().uuidString,
                                   dedupeKey: candidate.dedupeKey,
                                   source: candidate.event.source.rawValue,
                                   title: candidate.text,
                                   body: nil,
                                   status: .pending,
                                   isRead: false)
        try store.save(e)
        refreshUnread()
        return e
    }

    public func markAsSent(id: String) throws {
        try store.markSent(id: id)
        // sent does not affect unread count directly
        refreshUnread()
    }

    public func fetchHistory(limit: Int = 50, offset: Int = 0, unreadOnly: Bool = false, query: String? = nil) throws -> [NotificationEntity] {
        var all = try store.fetchAll()
        // Apply unread filter
        if unreadOnly {
            all = all.filter { !$0.isRead }
        }
        // Apply query filter (search in title/body/source/dedupeKey)
        if let q = query, !q.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            let low = q.lowercased()
            all = all.filter { e in
                (e.title?.lowercased().contains(low) ?? false) ||
                (e.body?.lowercased().contains(low) ?? false) ||
                (e.source?.lowercased().contains(low) ?? false) ||
                (e.dedupeKey?.lowercased().contains(low) ?? false)
            }
        }
        all.sort { $0.createdAt > $1.createdAt }
        guard offset < all.count else { return [] }
        let end = min(offset + limit, all.count)
        return Array(all[offset..<end])
    }

    public func markAsRead(id: String) throws {
        try store.markRead(id: id)
        refreshUnread()
    }

    @discardableResult
    public func markMatchingAsRead(key: String) throws -> Int {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return 0 }

        var ids = Set<String>()
        if let direct = try store.findById(trimmed) {
            ids.insert(direct.id)
        }
        for entity in try store.findByDedupeKey(trimmed) {
            ids.insert(entity.id)
        }

        for id in ids {
            try store.markRead(id: id)
        }
        refreshUnread()
        return ids.count
    }

    public func markAllRead() throws {
        try store.markAllRead()
        refreshUnread()
    }

    /// Cancel any pending UN notifications associated with a dedupeKey by removing their requests (uses persisted entity id)
    public func cancelPendingRequests(forDedupeKey dedupeKey: String) {
        do {
            let matches = try store.findByDedupeKey(dedupeKey)
            let ids = matches.map { $0.id }
            let center = UNUserNotificationCenter.current()
            center.removePendingNotificationRequests(withIdentifiers: ids)
            // mark them as cancelled in store
            for id in ids { try? store.markCancelled(id: id) }
        } catch {
            // ignore errors
        }
    }
}
