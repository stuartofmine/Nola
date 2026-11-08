import Foundation
import Testing
@testable import Nola

struct URLHistoryCanonicalEndToEndTests {
    @Test
    @MainActor
    func appendDeduplicatesAndSurvivesColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-url-history-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let entry = URLDataSource.HistoryEntry(
            timestamp: Date(timeIntervalSince1970: 1_750_000_000),
            dedupeKey: "article-42",
            title: "Canonical history",
            eventId: "event-42"
        )
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let entryData = try encoder.encode(entry)

        weak var firstRuntime: IOSApplicationRuntime?
        try autoreleasepool {
            let store = try IOSApplicationRuntime(databaseURL: database)
            firstRuntime = store
            let history = URLHistoryStore(applicationRuntime: store)
            #expect(history.append(sourceID: "source-1", entryData: entryData))
            #expect(history.append(sourceID: "source-1", entryData: entryData))
            #expect(history.load(sourceID: "source-1").count == 1)
        }

        // Release the MainActor-owned history wrapper on its owning executor,
        // and verify a cold reopen is not racing the first writer's teardown.
        #expect(firstRuntime == nil)

        let reopenedStore = try IOSApplicationRuntime(databaseURL: database)
        let reopened = URLHistoryStore(applicationRuntime: reopenedStore)
            .load(sourceID: "source-1")
        #expect(reopened.count == 1)
        #expect(reopened.first?.dedupeKey == entry.dedupeKey)
        #expect(reopened.first?.eventId == entry.eventId)
        #expect(reopened.first?.timestamp == entry.timestamp)
    }
}
