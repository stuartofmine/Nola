// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS notification stores, NotificationManager lifecycle, and Settings notification permission presentation suite.

import Foundation
import Testing
import UserNotifications
import XCTest
@testable import Nola

nonisolated private final class LockedNotificationCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var storage = 0
    func increment() { lock.lock(); storage += 1; lock.unlock() }
    var value: Int { lock.lock(); defer { lock.unlock() }; return storage }
}

@Suite("iOS Notification Stores and Manager Suite")
struct IOSNotificationStoreAndManagerSuiteTests {
    @Test("FileBasedNotificationStore persists across reopen and reads off UI executor")
    func fileBasedNotificationStorePersistenceAndBackgroundProjection() async throws {
        let filename = "notification_suite_test_\(UUID().uuidString).json"
        let root = try #require(FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first)
        let url = root.appendingPathComponent(Bundle.main.bundleIdentifier ?? "Nola").appendingPathComponent(filename)
        defer { try? FileManager.default.removeItem(at: url) }

        let store = try FileBasedNotificationStore(filename: filename)
        let entity = NotificationEntity(requestId: "r1", dedupeKey: "dk-save", title: "Saved notification", body: "B")
        try store.save(entity)

        let bgResult = try await Task.detached {
            (Thread.isMainThread, try store.fetchAll())
        }.value
        #expect(!bgResult.0)
        #expect(bgResult.1.map(\.id) == [entity.id])

        let reload = try FileBasedNotificationStore(filename: filename)
        let all = try reload.fetchAll()
        #expect(all.count == 1)
        #expect(all.first?.dedupeKey == "dk-save")
    }

    @Test("InMemoryNotificationStore pending, sent, dedupe, history, read, and background projection")
    func inMemoryNotificationStoreLifecycleAndBackgroundProjection() async throws {
        let store = InMemoryNotificationStore()
        let day = Date()
        let e1 = NotificationEntity(requestId: "r1", dedupeKey: "dup", title: "First", createdAt: day)
        let e2 = NotificationEntity(requestId: "r2", dedupeKey: "dup", title: "Second", createdAt: day)
        try store.save(e1)
        try store.save(e2)

        let bgResult = try await Task.detached {
            (Thread.isMainThread, try store.fetchAll())
        }.value
        #expect(!bgResult.0)
        #expect(bgResult.1.count == 2)

        let pendingBefore = try store.fetchPending()
        #expect(pendingBefore.count == 2)
        #expect(try store.findByDedupeKey("dup").count == 2)
        #expect(try store.fetchHistory(on: day).count == 2)

        try store.markSent(id: e1.id)
        #expect(try store.fetchPending().map(\.id) == [e2.id])
        let updatedE1 = try #require(try store.findById(e1.id))
        #expect(updatedE1.status == .sent)
        #expect(updatedE1.sentAt != nil)
        #expect(updatedE1.isRead == false)

        try store.markRead(id: e1.id)
        #expect(try store.findById(e1.id)?.isRead == true)
        #expect(try store.findById(e2.id)?.isRead == false)

        try store.markAllRead()
        #expect(try store.fetchAll().allSatisfy { $0.isRead })
    }

    @Test("NotificationManager post, idempotent dedupe, read matching, filtering, cancellation, and observer updates")
    @MainActor
    func notificationManagerEndToEndFlow() async throws {
        let store = InMemoryNotificationStore()
        let counter = LockedNotificationCounter()
        let obs = NotificationCenter.default.addObserver(forName: .nolaNotificationsUpdated, object: nil, queue: .main) { _ in
            counter.increment()
        }
        defer { NotificationCenter.default.removeObserver(obs) }

        let nm = NotificationManager(store: store)
        let ev1 = ProcessedEvent(id: "p1", source: .url, sourceType: .manual, sourceId: "s1", title: "Hello World", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "push_summary:result-1", provenance: nil, metadata: [:], notes: nil, hash: 1)
        let c1 = NotificationCandidate(event: ev1, dedupeKey: "push_summary:result-1", text: "Hey there", actionType: .openCalendar)
        let posted1 = try nm.post(candidate: c1)
        let postedDup = try nm.post(candidate: c1)
        #expect(posted1.id == postedDup.id)
        #expect(nm.unreadCount == 1)
        #expect(try nm.fetchHistory(limit: 10, offset: 0).first?.dedupeKey == "push_summary:result-1")

        let ev2 = ProcessedEvent(id: "p2", source: .reminder, sourceType: .manual, sourceId: "s2", title: "Other", date: nil, start: nil, due: nil, priority: 0.5, tags: nil, dedupeKey: "dk_cancel", provenance: nil, metadata: [:], notes: nil, hash: 2)
        let posted2 = try nm.post(candidate: NotificationCandidate(event: ev2, dedupeKey: "dk_cancel", text: "Cancel this", actionType: .openCalendar))
        #expect(nm.unreadCount == 2)

        #expect(try nm.markMatchingAsRead(key: "push_summary:result-1") == 1)
        #expect(nm.unreadCount == 1)
        #expect(try store.findById(posted1.id)?.isRead == true)

        let unread = try nm.fetchHistory(limit: 10, offset: 0, unreadOnly: true)
        #expect(unread.map(\.id) == [posted2.id])
        let queryMatches = try nm.fetchHistory(limit: 10, offset: 0, unreadOnly: false, query: "Hey")
        #expect(queryMatches.map(\.id) == [posted1.id])

        nm.cancelPendingRequests(forDedupeKey: "dk_cancel")
        #expect(try store.findById(posted2.id)?.status == .cancelled)
        try nm.markAsRead(id: posted2.id)
        #expect(nm.unreadCount == 0)

        let directEntity = NotificationEntity(requestId: "r-direct", dedupeKey: "d-direct", source: "test", title: "Direct", body: nil, status: .pending, isRead: false)
        try store.save(directEntity)
        try await Task.sleep(nanoseconds: 30_000_000)
        #expect(counter.value > 0)
        #expect(nm.unreadCount == 1)
        #expect(try nm.markMatchingAsRead(key: directEntity.id) == 1)
        #expect(nm.unreadCount == 0)
    }
}

final class SettingsNotificationPresentationTests: XCTestCase {
    func testReturningFromNotificationSettingsRespectsPermissionAndEnableIntent() {
        for status: UNAuthorizationStatus in [.authorized, .provisional, .ephemeral] {
            XCTAssertTrue(SettingsNotificationPresentation.refreshedPreference(current: false, requestedFromSettings: true, authorizationStatus: status))
            XCTAssertFalse(SettingsNotificationPresentation.refreshedPreference(current: false, requestedFromSettings: false, authorizationStatus: status))
        }
        for status: UNAuthorizationStatus in [.denied, .notDetermined] {
            XCTAssertFalse(SettingsNotificationPresentation.refreshedPreference(current: true, requestedFromSettings: true, authorizationStatus: status))
        }
    }

    func testSettingsNotificationAuthorizationPresentationPolicy() {
        XCTAssertTrue(SettingsNotificationPresentation.requiresSystemSettings(requestedEnabled: true, authorizationStatus: .denied))
        XCTAssertFalse(SettingsNotificationPresentation.requiresSystemSettings(requestedEnabled: true, authorizationStatus: .authorized))
        XCTAssertFalse(SettingsNotificationPresentation.requiresSystemSettings(requestedEnabled: true, authorizationStatus: .notDetermined))
        XCTAssertFalse(SettingsNotificationPresentation.requiresSystemSettings(requestedEnabled: false, authorizationStatus: .denied))
    }
}
