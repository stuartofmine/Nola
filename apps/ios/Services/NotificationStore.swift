// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationStore logic in the iOS service layer.
// - Interfaces:
//   - NotificationEntityStatus
//   - NotificationEntity
//   - NotificationStore
//   - notificationStoreNotFoundError
//   - normalizedNotificationOrder
//   - InMemoryNotificationStore
// - APIs:
//   - none
//
import Foundation

nonisolated public enum NotificationEntityStatus: String, Codable, Sendable {
    case pending
    case sent
    case failed
    case cancelled
}

nonisolated public struct NotificationEntity: Codable, Identifiable, Sendable {
    public var id: String
    public var requestId: String
    public var recordId: String?
    public var dedupeKey: String?
    public var source: String?
    public var title: String?
    public var body: String?
    public var status: NotificationEntityStatus
    public var attempts: Int
    public var sentAt: Date?
    public var deliveredAt: Date?
    public var isRead: Bool
    public var ekIdentifier: String?
    public var userAction: String?
    public var createdAt: Date

    public init(id: String = UUID().uuidString,
                requestId: String,
                recordId: String? = nil,
                dedupeKey: String? = nil,
                source: String? = nil,
                title: String? = nil,
                body: String? = nil,
                status: NotificationEntityStatus = .pending,
                attempts: Int = 0,
                sentAt: Date? = nil,
                deliveredAt: Date? = nil,
                isRead: Bool = false,
                ekIdentifier: String? = nil,
                userAction: String? = nil,
                createdAt: Date = Date()) {
        self.id = id
        self.requestId = requestId
        self.recordId = recordId
        self.dedupeKey = dedupeKey
        self.source = source
        self.title = title
        self.body = body
        self.status = status
        self.attempts = attempts
        self.sentAt = sentAt
        self.deliveredAt = deliveredAt
        self.isRead = isRead
        self.ekIdentifier = ekIdentifier
        self.userAction = userAction
        self.createdAt = createdAt
    }

}

public protocol NotificationStore {
    func save(_ entity: NotificationEntity) throws
    func fetchPending() throws -> [NotificationEntity]
    func fetchHistory(on date: Date) throws -> [NotificationEntity]
    func markSent(id: String) throws
    func markPending(id: String) throws
    func markRead(id: String) throws
    func markAllRead() throws
    func remove(id: String) throws
    nonisolated func fetchAll() throws -> [NotificationEntity]
    func findByDedupeKey(_ dedupeKey: String) throws -> [NotificationEntity]
    func findById(_ id: String) throws -> NotificationEntity?
    func markCancelled(id: String) throws
}

func notificationStoreNotFoundError(id: String) -> NSError {
    NSError(
        domain: "NotificationStore",
        code: 0,
        userInfo: [NSLocalizedDescriptionKey: "Notification not found: \(id)"]
    )
}

nonisolated func normalizedNotificationOrder(_ entities: [NotificationEntity]) -> [NotificationEntity] {
    entities.sorted {
        if $0.createdAt != $1.createdAt {
            return $0.createdAt > $1.createdAt
        }
        return $0.id > $1.id
    }
}

extension NotificationEntity {
    mutating func applySent(now: Date = Date()) {
        status = .sent
        sentAt = now
        deliveredAt = now
        attempts += 1
    }

    mutating func applyPending() {
        status = .pending
        sentAt = nil
        deliveredAt = nil
    }

    mutating func applyCancelled() {
        status = .cancelled
    }

    mutating func applyRead() {
        isRead = true
    }
}

/// Simple in-memory store useful for tests and early integration.
public final class InMemoryNotificationStore: NotificationStore {
    // All access, including the background read projection, is protected by lock.
    nonisolated(unsafe) private var db: [String: NotificationEntity] = [:]
    nonisolated private let lock = NSLock()

    public init() {}

    private func postNotificationsUpdated() {
        AppEventBus.postNotificationsUpdated()
    }

    private func mutateAndNotify(_ mutation: () throws -> Bool) throws {
        var didPost = false
        lock.lock(); defer {
            lock.unlock()
            if didPost { postNotificationsUpdated() }
        }
        didPost = try mutation()
    }

    public func save(_ entity: NotificationEntity) throws {
        try mutateAndNotify {
            db[entity.id] = entity
            return true
        }
    }

    public func fetchPending() throws -> [NotificationEntity] {
        lock.lock(); defer { lock.unlock() }
        return normalizedNotificationOrder(db.values.filter { $0.status == .pending })
    }

    public func fetchHistory(on date: Date) throws -> [NotificationEntity] {
        lock.lock(); defer { lock.unlock() }
        let cal = Calendar.current
        return normalizedNotificationOrder(db.values.filter { cal.isDate($0.createdAt, inSameDayAs: date) })
    }

    nonisolated public func fetchAll() throws -> [NotificationEntity] {
        lock.lock(); defer { lock.unlock() }
        return normalizedNotificationOrder(Array(db.values))
    }

    public func findByDedupeKey(_ dedupeKey: String) throws -> [NotificationEntity] {
        lock.lock(); defer { lock.unlock() }
        return normalizedNotificationOrder(db.values.filter { $0.dedupeKey == dedupeKey })
    }

    public func findById(_ id: String) throws -> NotificationEntity? {
        lock.lock(); defer { lock.unlock() }
        return db[id]
    }

    public func markSent(id: String) throws {
        try mutateAndNotify {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applySent()
            db[id] = e
            return true
        }
    }

    public func markPending(id: String) throws {
        try mutateAndNotify {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applyPending()
            db[id] = e
            return true
        }
    }

    public func markCancelled(id: String) throws {
        try mutateAndNotify {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applyCancelled()
            db[id] = e
            return true
        }
    }

    public func markRead(id: String) throws {
        try mutateAndNotify {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applyRead()
            db[id] = e
            return true
        }
    }

    public func markAllRead() throws {
        try mutateAndNotify {
            for (k, var e) in db {
                e.applyRead()
                db[k] = e
            }
            return true
        }
    }

    public func remove(id: String) throws {
        try mutateAndNotify {
            db.removeValue(forKey: id)
            return true
        }
    }
}
