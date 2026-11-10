// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationStoreTests logic in the project module.
// - Interfaces:
//   - NotificationStoreTests
// - APIs:
//   - none
//
//
//  NotificationStoreTests.swift
//  NolaTests
//
//  Tests for Notification store implementations (in-memory behaviors).
//

import Testing
@testable import Nola
import Foundation

@Suite("NotificationStore Tests")
struct NotificationStoreTests {

    @Test("Today notification projection reads on a background executor")
    func backgroundProjection() async throws {
        let store = InMemoryNotificationStore()
        let entity = NotificationEntity(requestId: "background-projection", title: "Read off main")
        try store.save(entity)
        let result = try await Task.detached {
            (Thread.isMainThread, try store.fetchAll())
        }.value
        #expect(!result.0)
        #expect(result.1.map(\.id) == [entity.id])
    }

    @Test("Save and fetch pending")
    func saveAndFetchPending() throws {
        let store = InMemoryNotificationStore()
        let e = NotificationEntity(requestId: "r1", dedupeKey: "k1")
        try store.save(e)

        let pending = try store.fetchPending()
        #expect(pending.count == 1)
        #expect(pending.first?.id == e.id)
    }

    @Test("Mark sent removes from pending and updates sentAt")
    func markSentUpdates() throws {
        let store = InMemoryNotificationStore()
        let e = NotificationEntity(requestId: "r2", dedupeKey: "k2")
        try store.save(e)

        try store.markSent(id: e.id)

        let pending = try store.fetchPending()
        #expect(pending.count == 0)

        let all = try store.fetchAll()
        #expect(all.count == 1)
        #expect(all.first?.status == .sent)
        #expect(all.first?.sentAt != nil)
    }

    @Test("Find by dedupe key returns matches")
    func findByDedupeKey() throws {
        let store = InMemoryNotificationStore()
        let a = NotificationEntity(requestId: "ra", dedupeKey: "dup")
        let b = NotificationEntity(requestId: "rb", dedupeKey: "dup")
        try store.save(a)
        try store.save(b)

        let found = try store.findByDedupeKey("dup")
        #expect(found.count == 2)
    }

    @Test("Fetch history by date")
    func fetchHistoryByDate() throws {
        let store = InMemoryNotificationStore()
        let day = Date()
        let e = NotificationEntity(requestId: "rh", dedupeKey: "h1", createdAt: day)
        try store.save(e)

        let hist = try store.fetchHistory(on: day)
        #expect(hist.count == 1)
        #expect(hist.first?.id == e.id)
    }

    @Test("Default is unread and markRead works")
    func markReadBehavior() throws {
        let store = InMemoryNotificationStore()
        let e = NotificationEntity(requestId: "r3", dedupeKey: "k3")
        try store.save(e)
        let all = try store.fetchAll()
        #expect(all.first?.isRead == false)
        try store.markRead(id: e.id)
        let after = try store.fetchAll()
        #expect(after.first?.isRead == true)
    }

    @Test("Mark all read works")
    func markAllRead() throws {
        let store = InMemoryNotificationStore()
        let a = NotificationEntity(requestId: "ra", dedupeKey: "d1")
        let b = NotificationEntity(requestId: "rb", dedupeKey: "d2")
        try store.save(a); try store.save(b)
        try store.markAllRead()
        let all = try store.fetchAll()
        #expect(all.allSatisfy { $0.isRead })
    }
}

