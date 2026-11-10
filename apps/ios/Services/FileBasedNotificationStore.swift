// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements FileBasedNotificationStore logic in the iOS service layer.
// - Interfaces:
//   - FileBasedNotificationStore
// - APIs:
//   - none
//
import Foundation

public final class FileBasedNotificationStore: NotificationStore {
    private let fileURL: URL
    // Mutations and background read projections share this lock.
    nonisolated(unsafe) private var db: [String: NotificationEntity] = [:]
    nonisolated private let lock = NSLock()

    public init(filename: String = "notification_store.json") throws {
        let fm = FileManager.default
        guard let appSupport = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else {
            throw NSError(domain: "FileBasedNotificationStore", code: 1, userInfo: [NSLocalizedDescriptionKey: "Unable to access Application Support directory"])
        }
        let dir = appSupport.appendingPathComponent(Bundle.main.bundleIdentifier ?? "Nola", isDirectory: true)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true, attributes: nil)
        self.fileURL = dir.appendingPathComponent(filename)

        // Load if exists
        try loadUnlocked()
    }

    private func loadUnlocked() throws {
        let fm = FileManager.default
        guard fm.fileExists(atPath: fileURL.path) else { db = [:]; return }
        let data = try Data(contentsOf: fileURL)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let arr = try decoder.decode([NotificationEntity].self, from: data)
        db = Dictionary(uniqueKeysWithValues: arr.map { ($0.id, $0) })
    }

    private func saveUnlocked() throws {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let arr = Array(db.values)
        let data = try encoder.encode(arr)
        try data.write(to: fileURL, options: [.atomic])
    }

    private func withLockedWrite(_ operation: () throws -> Void) throws {
        var didPost = false
        lock.lock(); defer {
            lock.unlock()
            if didPost { AppEventBus.postNotificationsUpdated() }
        }
        try operation()
        try saveUnlocked()
        didPost = true
    }

    public func save(_ entity: NotificationEntity) throws {
        try withLockedWrite {
            db[entity.id] = entity
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
        try withLockedWrite {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applySent()
            db[id] = e
        }
    }

    public func markPending(id: String) throws {
        try withLockedWrite {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applyPending()
            db[id] = e
        }
    }
    public func markCancelled(id: String) throws {
        try withLockedWrite {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applyCancelled()
            db[id] = e
        }
    }
    public func markRead(id: String) throws {
        try withLockedWrite {
            guard var e = db[id] else { throw notificationStoreNotFoundError(id: id) }
            e.applyRead()
            db[id] = e
        }
    }

    public func markAllRead() throws {
        try withLockedWrite {
            for (k, var e) in db {
                e.applyRead()
                db[k] = e
            }
        }
    }

    public func remove(id: String) throws {
        try withLockedWrite {
            db.removeValue(forKey: id)
        }
    }
}
