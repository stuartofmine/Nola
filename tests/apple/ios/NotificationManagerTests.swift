// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationManagerTests logic in the project module.
// - Interfaces:
//   - NotificationManagerTests
// - APIs:
//   - none
//
import Testing
@testable import Nola
import Foundation

nonisolated private final class LockedNotificationCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var storage = 0

    func increment() {
        lock.lock()
        storage += 1
        lock.unlock()
    }

    var value: Int {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }
}

@Suite("NotificationManager Tests")
@MainActor
struct NotificationManagerTests {

    @Test("Post candidate increases unreadCount and fetchHistory returns it")
    func postCandidate() async throws {
        let store = InMemoryNotificationStore()
        let nm = NotificationManager(store: store)
        let ev = ProcessedEvent(id: "p1", source: .reminder, sourceType: .manual, sourceId: "s1", title: "Test", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "dk1", provenance: nil, metadata: [:], notes: nil, hash: 1)
        let candidate = NotificationCandidate(event: ev, dedupeKey: "dk1", text: "Test notification", actionType: .openCalendar)
        let _ = try nm.post(candidate: candidate)
        #expect(nm.unreadCount == 1)
        let hist = try nm.fetchHistory(limit:10, offset:0)
        #expect(hist.count == 1)
        #expect(hist.first?.dedupeKey == "dk1")
    }

    @Test("Mark as read reduces unreadCount")
    func markAsRead() async throws {
        let store = InMemoryNotificationStore()
        let nm = NotificationManager(store: store)
        let ev = ProcessedEvent(id: "p2", source: .reminder, sourceType: .manual, sourceId: "s2", title: "T2", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "dk2", provenance: nil, metadata: [:], notes: nil, hash: 2)
        let candidate = NotificationCandidate(event: ev, dedupeKey: "dk2", text: "T2", actionType: .openCalendar)
        let e = try nm.post(candidate: candidate)
        #expect(nm.unreadCount == 1)
        try nm.markAsRead(id: e.id)
        #expect(nm.unreadCount == 0)
    }

    @Test("Mark matching key reads notification by id or dedupeKey")
    func markMatchingAsRead() async throws {
        let store = InMemoryNotificationStore()
        let nm = NotificationManager(store: store)
        let ev = ProcessedEvent(id: "p2b", source: .url, sourceType: .manual, sourceId: "s2b", title: "Push", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "push_summary:result-1", provenance: nil, metadata: [:], notes: nil, hash: 22)
        let candidate = NotificationCandidate(event: ev, dedupeKey: "push_summary:result-1", text: "Push summary", actionType: .openCalendar)
        let e = try nm.post(candidate: candidate)

        #expect(nm.unreadCount == 1)
        #expect(try nm.markMatchingAsRead(key: "push_summary:result-1") == 1)
        #expect(nm.unreadCount == 0)

        let otherEvent = ProcessedEvent(id: "p2c", source: .url, sourceType: .manual, sourceId: "s2c", title: "Other", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "other", provenance: nil, metadata: [:], notes: nil, hash: 23)
        let another = try nm.post(candidate: NotificationCandidate(event: otherEvent, dedupeKey: "other", text: "Other", actionType: .openCalendar))
        #expect(nm.unreadCount == 1)
        #expect(try nm.markMatchingAsRead(key: another.id) == 1)
        #expect(try store.findById(e.id)?.isRead == true)
        #expect(nm.unreadCount == 0)
    }

    @Test("Fetch history with unreadOnly and query filters")
    func fetchWithFilters() async throws {
        let store = InMemoryNotificationStore()
        let nm = NotificationManager(store: store)

        let ev1 = ProcessedEvent(id: "p3", source: .url, sourceType: .manual, sourceId: "s3", title: "Hello World", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "dk3", provenance: nil, metadata: [:], notes: nil, hash: 3)
        let ev2 = ProcessedEvent(id: "p4", source: .reminder, sourceType: .manual, sourceId: "s4", title: "Other", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "dk4", provenance: nil, metadata: [:], notes: nil, hash: 4)
        let c1 = NotificationCandidate(event: ev1, dedupeKey: "dk3", text: "Hey there", actionType: .openCalendar)
        let c2 = NotificationCandidate(event: ev2, dedupeKey: "dk4", text: "Another", actionType: .openCalendar)
        let e1 = try nm.post(candidate: c1)
        let e2 = try nm.post(candidate: c2)

        // mark one as read
        try nm.markAsRead(id: e2.id)

        let unread = try nm.fetchHistory(limit: 10, offset: 0, unreadOnly: true)
        #expect(unread.count == 1)
        #expect(unread.first?.id == e1.id)
        #expect(unread.first?.dedupeKey == "dk3")

        let queryMatches = try nm.fetchHistory(limit: 10, offset: 0, unreadOnly: false, query: "Hey")
        #expect(queryMatches.count == 1)
        #expect(queryMatches.first?.dedupeKey == "dk3")
    }

    @Test("Post is idempotent by dedupeKey")
    func postIdempotent() async throws {
        let store = InMemoryNotificationStore()
        let nm = NotificationManager(store: store)
        let ev = ProcessedEvent(id: "p5", source: .calendar, sourceType: .manual, sourceId: "s5", title: "Dup", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "dup1", provenance: nil, metadata: [:], notes: nil, hash: 5)
        let c = NotificationCandidate(event: ev, dedupeKey: "dup1", text: "Dup", actionType: .openCalendar)
        let a = try nm.post(candidate: c)
        let b = try nm.post(candidate: c)
        #expect(a.id == b.id)
    }

    @Test("Cancel by dedupeKey marks persisted entries cancelled")
    func cancelByDedupeKey() async throws {
        let store = InMemoryNotificationStore()
        let nm = NotificationManager(store: store)
        let ev = ProcessedEvent(id: "px", source: .reminder, sourceType: .manual, sourceId: "sx", title: "CancelMe", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "dk_cancel", provenance: nil, metadata: [:], notes: nil, hash: 999)
        let c = NotificationCandidate(event: ev, dedupeKey: "dk_cancel", text: "Cancel this", actionType: .openCalendar)
        let e = try nm.post(candidate: c)
        #expect(try store.findById(e.id)?.status == .pending)
        nm.cancelPendingRequests(forDedupeKey: "dk_cancel")
        #expect(try store.findById(e.id)?.status == .cancelled)
    }

    @Test("Store mutation posts update notification")
    func storeEmitsUpdateNotification() async throws {
        let store = InMemoryNotificationStore()
        let counter = LockedNotificationCounter()
        let obs = NotificationCenter.default.addObserver(forName: .nolaNotificationsUpdated, object: nil, queue: .main) { _ in
            counter.increment()
        }
        defer { NotificationCenter.default.removeObserver(obs) }
        let e = NotificationEntity(requestId: "r1", dedupeKey: "d1", source: "test", title: "t", body: nil, status: .pending, isRead: false)
        try store.save(e)
        // allow notification to be delivered on main runloop
        try await Task.sleep(nanoseconds: 20_000_000)
        #expect(counter.value > 0)
    }

    @Test("NotificationManager refreshes unreadCount on store updates")
    func managerRefreshesOnStoreUpdate() async throws {
        let store = InMemoryNotificationStore()
        let nm = NotificationManager(store: store)
        let e = NotificationEntity(requestId: "r2", dedupeKey: "d2", source: "test", title: "t2", body: nil, status: .pending, isRead: false)
        try store.save(e)
        // yield twice so the Task { @MainActor in refreshUnread() } inside NotificationManager can run
        try await Task.sleep(nanoseconds: 20_000_000)
        #expect(nm.unreadCount == 1)
    }
}
