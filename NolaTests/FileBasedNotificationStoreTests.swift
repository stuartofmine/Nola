// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements FileBasedNotificationStoreTests logic in the project module.
// - Interfaces:
//   - FileBasedNotificationStoreTests
// - APIs:
//   - none
//
import Testing
@testable import Nola
import Foundation

@Suite("FileBasedNotificationStore Tests")
struct FileBasedNotificationStoreTests {

    @Test("Today file notification projection reads off the UI executor")
    func backgroundProjection() async throws {
        let filename = "notification_background_test_\(UUID().uuidString).json"
        let root = try #require(FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first)
        let url = root.appendingPathComponent(Bundle.main.bundleIdentifier ?? "Nola").appendingPathComponent(filename)
        defer { try? FileManager.default.removeItem(at: url) }
        let store = try FileBasedNotificationStore(filename: filename)
        let entity = NotificationEntity(requestId: "background-file-projection", title: "Saved notification")
        try store.save(entity)
        let result = try await Task.detached {
            (Thread.isMainThread, try store.fetchAll())
        }.value
        #expect(!result.0)
        #expect(result.1.map(\.id) == [entity.id])
    }

    @Test("Save and reload persists data")
    func saveAndReload() throws {
        // use a unique filename to avoid collisions
        let filename = "notification_store_test_\(UUID().uuidString).json"
        let store = try FileBasedNotificationStore(filename: filename)
        let e = NotificationEntity(requestId: "r1", dedupeKey: "dk-save", title: "T", body: "B")
        try store.save(e)

        // Create a new instance that loads from the same file
        let reload = try FileBasedNotificationStore(filename: filename)
        let all = try reload.fetchAll()
        #expect(all.count == 1)
        #expect(all.first?.dedupeKey == "dk-save")
    }
}
