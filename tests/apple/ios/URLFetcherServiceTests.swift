// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements URLFetcherServiceTests logic in the project module.
// - Interfaces:
//   - URLDataSourceMetadataTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

private enum ServiceLifecycleTestScope {
    @TaskLocal static var marker: Int = 0
}

@MainActor
final class URLDataSourceMetadataTests: XCTestCase {
    func testSynchronousServiceReleaseWithinTaskLocalScope() throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        try ServiceLifecycleTestScope.$marker.withValue(1) {
            weak var releasedLogger: Logger?
            weak var releasedFetcher: URLFetcherService?
            weak var releasedStorage: LocalJSONStorage?
            do {
                let logger = Logger()
                let fetcher = URLFetcherService(logger: logger, applicationRuntime: runtime)
                let storage = try LocalJSONStorage(applicationRuntime: runtime)
                releasedLogger = logger
                releasedFetcher = fetcher
                releasedStorage = storage
                withExtendedLifetime((logger, fetcher, storage)) {}
            }
            // Keep this synchronous: an async test/task hop would miss the
            // old-runtime TaskLocal + isolated deinitializer failure mode.
            XCTAssertNil(releasedFetcher)
            XCTAssertNil(releasedLogger)
            XCTAssertNil(releasedStorage)
            XCTAssertEqual(ServiceLifecycleTestScope.marker, 1)
        }
    }

    private func makeApplicationRuntime() throws -> (IOSApplicationRuntime, URL) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-url-source-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return (
            try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite")),
            root
        )
    }

    func testSearchAndFeedKindsCanBeStoredExplicitly() throws {
        var source = URLDataSource(
            url: "https://www.google.com/search?q=nola",
            format: .html,
            kind: .webSearch,
            name: "Search"
        )
        source.history = [
            .init(dedupeKey: "k1", title: "Result 1", details: "A readable search result", rawResponse: "Full text")
        ]

        let decoded = try JSONDecoder().decode(URLDataSource.self, from: JSONEncoder().encode(source))

        XCTAssertEqual(decoded.kind, .webSearch)
        XCTAssertEqual(decoded.contextDisplaySubtitle, decoded.sourceDescription ?? decoded.url)
        XCTAssertTrue(decoded.capabilities.contains(.localOnly))
    }

    func testReadableContextItemsExposeHistoryWithoutFetcher() async throws {
        var source = URLDataSource(url: "https://example.com/rss.xml", format: .rss, name: "Feed")
        source.history = [
            .init(dedupeKey: "k1", title: "Older", details: "Older details"),
            .init(dedupeKey: "k2", title: "Newer", details: "Newer details")
        ]

        let items = try await source.readContextItems(limit: 1)

        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?.title, "Newer")
        XCTAssertEqual(items.first?.metadata["kind"], DataSourceKind.feed.rawValue)
    }

    func testLocalFileSourceReadsTextWithoutNetwork() async throws {
        let fileURL = try makeTemporaryFile(name: "notes.md", contents: "# Notes\nLocal file data source.")
        var source = URLDataSource(url: fileURL.absoluteString, format: .html, kind: .file, name: "Notes")
        source.securityBookmarkData = try fileURL.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
        let fetcher = makeFetcher()

        let events = try await fetcher.readSourceForValidation(from: source)

        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events.first?.title, "Notes")
        XCTAssertEqual(events.first?.metadata["sourceKind"], DataSourceKind.file.rawValue)
        XCTAssertTrue(events.first?.notes?.contains("Local file data source") == true)
    }

    func testLocalFolderSourceReadsMultipleTextFiles() async throws {
        let folderURL = try makeTemporaryFolder()
        try "First file".write(to: folderURL.appendingPathComponent("a.md"), atomically: true, encoding: .utf8)
        try "Second file".write(to: folderURL.appendingPathComponent("b.txt"), atomically: true, encoding: .utf8)
        let source = URLDataSource(url: folderURL.absoluteString, format: .html, kind: .folder, name: "Project Folder")
        let fetcher = makeFetcher()

        let events = try await fetcher.readSourceForValidation(from: source)

        XCTAssertEqual(events.count, 2)
        XCTAssertEqual(Set(events.map(\.metadata["sourceKind"])), [DataSourceKind.folder.rawValue])
        XCTAssertTrue(events.contains { $0.title == "a.md" || $0.title == "b.txt" })
    }

    func testValidationPersistsAReadableContractWithoutRawContent() async throws {
        let fileURL = try makeTemporaryFile(name: "private-notes.txt", contents: "secret-value-should-not-be-persisted")
        let source = URLDataSource(
            url: fileURL.absoluteString,
            format: .html,
            kind: .file,
            name: "Private Notes"
        )
        let fetcher = makeFetcher()

        let result = await fetcher.validateSource(source)

        guard case .success(let count, let contract) = result else {
            return XCTFail("Expected successful validation")
        }
        XCTAssertEqual(count, 1)
        XCTAssertEqual(contract.sourceID, source.id)
        XCTAssertEqual(contract.connectorID, "ios.local-readable")
        XCTAssertTrue(contract.isUsable)
        XCTAssertEqual(contract.readRecipe?.method, .localFileRead)
        XCTAssertEqual(contract.readRecipe?.operationID, "ios.local-readable.read")
        XCTAssertNotNil(contract.validationDigest)
        XCTAssertTrue(contract.schemaFields.contains("title"))
        XCTAssertFalse(contract.redactedSample.contains("secret-value"))
    }

    func testPolicyAndValidationContractRoundTrip() throws {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        var source = URLDataSource(url: "https://example.com/feed.json", format: .json, name: "Feed")
        source.accessPolicy = .defaultEnabled
        var validationContract = URLDataSourceValidationContract(
            sourceID: source.id,
            connectorID: "ios.url-session",
            connectorVersion: 2,
            readerDescription: "ios.url-session -> json -> RawEvent",
            schemaFields: ["id", "title"],
            redactedSample: "<redacted json record: title>",
            recordCount: 1,
            maximumAgeMinutes: 60,
            validationState: .verified,
            validatedAt: now,
            observedAt: now
        )
        validationContract.configureReadRecipe(
            method: .httpGet,
            operationID: "ios.url-session.get",
            configurationKeys: ["url", "maximumBytes"]
        )
        source.validationContract = validationContract

        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let decoded = try decoder.decode(URLDataSource.self, from: encoder.encode(source))

        XCTAssertEqual(decoded.accessPolicy, .defaultEnabled)
        XCTAssertEqual(decoded.validationContract, source.validationContract)
        XCTAssertTrue(decoded.validationContract?.isUsable == true)
    }

    func testConversationPolicyResolutionDistinguishesMissingAndExplicitChoices() {
        var source = URLDataSource(url: "https://example.com", format: .html, name: "Website")

        source.accessPolicy = .defaultEnabled
        XCTAssertTrue(source.isEnabledForConversation(explicitOverride: nil))
        XCTAssertFalse(source.isEnabledForConversation(explicitOverride: false))

        source.accessPolicy = .conversationOptIn
        XCTAssertFalse(source.isEnabledForConversation(explicitOverride: nil))
        XCTAssertTrue(source.isEnabledForConversation(explicitOverride: true))

        source.accessPolicy = .askWhenRelevant
        XCTAssertFalse(source.isEnabledForConversation(explicitOverride: nil))

        source.accessPolicy = .blocked
        XCTAssertFalse(source.isEnabledForConversation(explicitOverride: true))
    }

    func testUsageProvenancePersistsAndIsBoundedToSource() async throws {
        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let suite = "URLDataSourceUsageTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let source = URLDataSource(url: "https://example.com", format: .html, name: "Website")
        let fetcher = URLFetcherService(
            logger: Logger(),
            maxURLs: 20,
            userDefaults: defaults,
            applicationRuntime: applicationRuntime
        )
        try fetcher.addSource(source)
        fetcher.recordUsage(URLDataSourceUsageRecord(
            sourceID: source.id,
            sourceTitle: source.name,
            consumer: "chat",
            purpose: "conversation_context",
            conversationID: UUID(),
            disclosedToCloud: nil
        ))
        fetcher.waitForPersistence()

        let restored = URLFetcherService(
            logger: Logger(),
            maxURLs: 20,
            userDefaults: defaults,
            applicationRuntime: try IOSApplicationRuntime(
                databaseURL: root.appendingPathComponent("nola.sqlite")
            )
        )

        XCTAssertEqual(restored.usageRecords.count, 1)
        XCTAssertEqual(restored.usageRecords.first?.sourceID, source.id)
        XCTAssertNil(restored.usageRecords.first?.disclosedToCloud)
    }

    func testStaleSourceListsCannotDeleteAnotherServicesSources() throws {
        let (firstRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let secondRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let first = URLFetcherService(logger: Logger(), applicationRuntime: firstRuntime)
        let second = URLFetcherService(logger: Logger(), applicationRuntime: secondRuntime)
        let firstSource = URLDataSource(url: "https://example.invalid/a", format: .json, name: "First")
        let otherSource = URLDataSource(url: "https://example.invalid/b", format: .json, name: "Other")
        let thirdSource = URLDataSource(url: "https://example.invalid/c", format: .json, name: "Third")

        try first.addSource(firstSource)
        first.waitForPersistence()
        try second.addSource(otherSource)
        second.waitForPersistence()
        XCTAssertEqual(Set(try firstRuntime.listDataSources().map(\.id)), [firstSource.id.uuidString, otherSource.id.uuidString])
        let otherRevision = try XCTUnwrap(secondRuntime.listDataSources().first { $0.id == otherSource.id.uuidString }).revision

        try first.addSource(thirdSource)
        first.waitForPersistence()
        first.removeSource(id: firstSource.id)
        first.waitForPersistence()

        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let remaining = try reopened.listDataSources()
        XCTAssertEqual(Set(remaining.map(\.id)), [otherSource.id.uuidString, thirdSource.id.uuidString])
        XCTAssertEqual(remaining.first { $0.id == otherSource.id.uuidString }?.revision, otherRevision)
    }

    private func makeFetcher() -> URLFetcherService {
        let defaults = UserDefaults(suiteName: "URLDataSourceMetadataTests-\(UUID().uuidString)")!
        return URLFetcherService(logger: Logger(), maxURLs: 20, userDefaults: defaults)
    }

    func testEditingAnotherSourceDoesNotOverwriteNewerCanonicalContent() throws {
        let (firstRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let first = URLFetcherService(logger: Logger(), applicationRuntime: firstRuntime)
        let sourceA = URLDataSource(url: "https://example.invalid/a", format: .json, name: "A")
        let sourceB = URLDataSource(url: "https://example.invalid/b", format: .json, name: "B")
        try first.addSource(sourceA)
        try first.addSource(sourceB)
        first.waitForPersistence()

        let secondRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let second = URLFetcherService(logger: Logger(), applicationRuntime: secondRuntime)
        var editedA = try XCTUnwrap(first.sources.first { $0.id == sourceA.id })
        editedA.name = "New canonical A"
        first.updateSource(editedA)
        first.waitForPersistence()
        let committedA = try XCTUnwrap(firstRuntime.listDataSources().first { $0.id == sourceA.id.uuidString })
        XCTAssertEqual(committedA.name, "New canonical A")

        var editedB = try XCTUnwrap(second.sources.first { $0.id == sourceB.id })
        editedB.name = "New canonical B"
        second.updateSource(editedB)
        second.waitForPersistence()

        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let records = try reopened.listDataSources()
        XCTAssertEqual(records.first { $0.id == sourceA.id.uuidString }, committedA)
        XCTAssertEqual(records.first { $0.id == sourceB.id.uuidString }?.name, "New canonical B")
    }

    private func makeTemporaryFolder() throws -> URL {
        let folderURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-url-source-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folderURL, withIntermediateDirectories: true)
        return folderURL
    }

    func testStaleSameSourceUpdateIsRejectedByApplication() throws {
        let (firstRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let source = URLDataSource(url: "https://example.invalid/source", format: .json, name: "Original")
        let initial = try firstRuntime.saveDataSource(XCTUnwrap(source.canonicalSubmission))
        let secondRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let observed = try XCTUnwrap(secondRuntime.listDataSources().first)

        var updated = source
        updated.name = "Committed edit"
        let committed = try firstRuntime.saveDataSource(
            XCTUnwrap(updated.canonicalSubmission), expectedRevision: initial.revision
        )
        var stale = source
        stale.name = "Stale edit"
        XCTAssertThrowsError(try secondRuntime.saveDataSource(
            XCTUnwrap(stale.canonicalSubmission), expectedRevision: observed.revision
        ))
        XCTAssertThrowsError(try secondRuntime.saveDataSource(XCTUnwrap(stale.canonicalSubmission)))

        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        XCTAssertEqual(try reopened.listDataSources(), [committed])
    }

    func testStaleServiceCannotOverwriteAnotherServicesEditToSameSource() throws {
        let (firstRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let first = URLFetcherService(logger: Logger(), applicationRuntime: firstRuntime)
        let source = URLDataSource(url: "https://example.invalid/source", format: .json, name: "Original")
        try first.addSource(source)
        first.waitForPersistence()
        let secondRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let second = URLFetcherService(logger: Logger(), applicationRuntime: secondRuntime)

        var firstEdit = try XCTUnwrap(first.sources.first)
        firstEdit.name = "First edit"
        first.updateSource(firstEdit)
        first.waitForPersistence()
        let committed = try XCTUnwrap(firstRuntime.listDataSources().first)
        var secondEdit = try XCTUnwrap(second.sources.first)
        secondEdit.name = "Stale second edit"
        second.updateSource(secondEdit)
        second.waitForPersistence()
        XCTAssertEqual(try secondRuntime.listDataSources(), [committed])
    }

    func testDeletionRequiresTheObservedCanonicalRevision() throws {
        let (firstRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        var source = URLDataSource(url: "https://example.invalid/source", format: .json, name: "Original")
        let original = try firstRuntime.saveDataSource(XCTUnwrap(source.canonicalSubmission))
        source.name = "Updated"
        let committed = try firstRuntime.saveDataSource(
            XCTUnwrap(source.canonicalSubmission), expectedRevision: original.revision
        )
        let second = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        XCTAssertThrowsError(try second.deleteDataSource(
            id: original.id, expectedRevision: original.revision, deletedAtMs: now
        ))
        XCTAssertEqual(try second.listDataSources(), [committed])
        XCTAssertTrue(try second.deleteDataSource(
            id: committed.id, expectedRevision: committed.revision, deletedAtMs: now
        ))
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        XCTAssertTrue(try reopened.listDataSources().isEmpty)
    }

    func testStaleServiceCannotDeleteAnotherServicesUpdatedSource() throws {
        let (firstRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let first = URLFetcherService(logger: Logger(), applicationRuntime: firstRuntime)
        let source = URLDataSource(url: "https://example.invalid/source", format: .json, name: "Original")
        try first.addSource(source)
        first.waitForPersistence()
        let secondRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let second = URLFetcherService(logger: Logger(), applicationRuntime: secondRuntime)
        var update = try XCTUnwrap(first.sources.first)
        update.name = "Keep this newer edit"
        first.updateSource(update)
        first.waitForPersistence()
        let committed = try XCTUnwrap(firstRuntime.listDataSources().first)

        second.removeSource(id: source.id)
        second.waitForPersistence()
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        XCTAssertEqual(try reopened.listDataSources(), [committed])
    }

    private func makeTemporaryFile(name: String, contents: String) throws -> URL {
        let folderURL = try makeTemporaryFolder()
        let fileURL = folderURL.appendingPathComponent(name)
        try contents.write(to: fileURL, atomically: true, encoding: .utf8)
        return fileURL
    }

    func testAwaitedStaleDeleteReportsConflictAndPreservesBothProjections() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let first = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let original = try await first.saveSource(URLDataSource(
            url: "https://delete.example.invalid/source", format: .json, name: "Original"
        ))
        let secondRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let second = URLFetcherService(logger: Logger(), applicationRuntime: secondRuntime)
        var edited = original
        edited.name = "Keep newer version"
        let committed = try await first.saveSource(edited)
        do {
            try await second.removeSourceConfirmed(id: original.id, expectedRevision: original.canonicalRevision)
            XCTFail("Stale deletion must return an error receipt")
        } catch {
            XCTAssertEqual(second.sources.map(\.id), [original.id])
            XCTAssertEqual(second.sources.first?.name, "Original", "A rejection cannot remove the local row")
        }
        XCTAssertEqual(try secondRuntime.listDataSources().first?.revision, committed.canonicalRevision)
        XCTAssertEqual(try secondRuntime.listDataSources().first?.name, "Keep newer version")
    }

    func testAwaitedDeleteReceiptPublishesOnlyAfterCanonicalCommit() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let source = try await fetcher.saveSource(URLDataSource(
            url: "https://delete.example.invalid/source", format: .json, name: "Delete me"
        ))
        XCTAssertEqual(fetcher.sources.map(\.id), [source.id])
        try await fetcher.removeSourceConfirmed(id: source.id, expectedRevision: source.canonicalRevision)
        XCTAssertTrue(fetcher.sources.isEmpty)
        XCTAssertTrue(try runtime.listDataSources().isEmpty)
        do {
            try await fetcher.removeSourceConfirmed(id: source.id)
            XCTFail("An already absent row must not return a new successful deletion receipt")
        } catch { XCTAssertTrue(fetcher.sources.isEmpty) }
    }

    func testAwaitedEditorSavePublishesCommittedProjection() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let source = URLDataSource(url: "https://example.invalid/source", format: .json, name: "Saved")
        let committed = try await fetcher.saveSource(source)
        XCTAssertEqual(fetcher.sources.map(\.id), [committed.id])
        XCTAssertEqual(fetcher.sources.first?.enabled, committed.enabled)
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        XCTAssertEqual(try reopened.listDataSources().first?.iosURLSource?.name, committed.name)
    }

    func testAwaitedEditorConflictDoesNotPublishUnsavedInput() async throws {
        let (firstRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let first = URLFetcherService(logger: Logger(), applicationRuntime: firstRuntime)
        let source = URLDataSource(url: "https://example.invalid/source", format: .json, name: "Original")
        let original = try await first.saveSource(source)
        let secondRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let second = URLFetcherService(logger: Logger(), applicationRuntime: secondRuntime)
        var current = original
        current.name = "Committed"
        _ = try await first.saveSource(current)
        var stale = original
        stale.name = "Not saved"
        do {
            _ = try await second.saveSource(stale)
            XCTFail("Expected a stale revision rejection")
        } catch {
            XCTAssertEqual(second.sources.first?.name, "Original")
            XCTAssertEqual(stale.name, "Not saved")
        }
        XCTAssertEqual(try secondRuntime.listDataSources().first?.name, "Committed")
    }

    func testTwoEditorDraftsInOneServiceCannotOverwriteEachOther() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let original = try await fetcher.saveSource(URLDataSource(
            url: "https://example.invalid/source", format: .json, name: "Original"
        ))
        var firstDraft = original
        var secondDraft = original
        firstDraft.name = "First editor committed"
        let committed = try await fetcher.saveSource(firstDraft)
        XCTAssertNotEqual(original.canonicalRevision, committed.canonicalRevision)
        secondDraft.name = "Stale second editor"
        do {
            _ = try await fetcher.saveSource(secondDraft)
            XCTFail("A stale editor must not use the service's newer revision")
        } catch {
            XCTAssertEqual(fetcher.sources.first?.name, committed.name)
            XCTAssertEqual(fetcher.sources.first?.canonicalRevision, committed.canonicalRevision)
        }
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        XCTAssertEqual(try reopened.listDataSources().first?.name, committed.name)
    }

    func testCanonicalRevisionComesFromApplicationNotEmbeddedMetadata() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let committed = try await fetcher.saveSource(URLDataSource(
            url: "https://example.invalid/source", format: .json, name: "Original"
        ))
        XCTAssertNotNil(committed.canonicalRevision)
        let data = try JSONEncoder().encode(committed)
        let decoded = try JSONDecoder().decode(URLDataSource.self, from: data)
        XCTAssertNil(decoded.canonicalRevision)
        XCTAssertEqual(try runtime.listDataSources().first?.iosURLSource?.canonicalRevision, committed.canonicalRevision)
    }

    func testCanonicalValidationKeepsFractionalDatesAndOriginalDigest() throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        var source = URLDataSource(url: "https://example.invalid/source", format: .json, name: "Precise contract")
        let timestamp = Date(timeIntervalSince1970: 1_788_640_800.123)
        var contract = URLDataSourceValidationContract(
            sourceID: source.id,
            connectorID: "ios.url-session",
            connectorVersion: 2,
            readerDescription: "HTTP JSON",
            schemaFields: ["title"],
            redactedSample: "<redacted>",
            recordCount: 1,
            maximumAgeMinutes: 60,
            validationState: .verified,
            validatedAt: timestamp,
            observedAt: timestamp
        )
        contract.configureReadRecipe(method: .httpGet, operationID: "ios.url-session.get", configurationKeys: ["url"])
        XCTAssertTrue(contract.isUsable)
        source.validationContract = contract
        _ = try runtime.saveDataSource(XCTUnwrap(source.canonicalSubmission))

        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let record = try XCTUnwrap(reopened.listDataSources().first)
        let projected = try XCTUnwrap(record.iosURLSource?.validationContract)
        XCTAssertTrue(projected.isUsable)
        XCTAssertEqual(projected.validationDigest, contract.validationDigest)
        XCTAssertEqual(projected.validatedAt.timeIntervalSince1970, timestamp.timeIntervalSince1970, accuracy: 0.000_01)
        XCTAssertEqual(try XCTUnwrap(projected.observedAt).timeIntervalSince1970, timestamp.timeIntervalSince1970, accuracy: 0.000_01)
    }
}

extension URLDataSourceMetadataTests {
    private func fixtureConversation(_ runtime: IOSApplicationRuntime) throws -> String {
        let id = UUID().uuidString
        try runtime.createConversation(id: id, title: "Data source fixture", settingsJSON: nil,
                                       createdAtMs: 1, updatedAtMs: 1)
        return id
    }

    func testBoundedBodyAcceptsExactLimitWithoutContentLength() async throws {
        let bytes = AsyncStream<UInt8> { continuation in
            for byte: UInt8 in [1, 2, 3, 4] { continuation.yield(byte) }
            continuation.finish()
        }
        let body = try await IOSBoundedDataSourceBody.collect(bytes, limit: 4)
        XCTAssertEqual(body, Data([1, 2, 3, 4]))
    }

    func testBoundedBodyRejectsUnadvertisedOverflow() async {
        let bytes = AsyncStream<UInt8> { continuation in
            for _ in 0..<5 { continuation.yield(1) }
            continuation.finish()
        }
        do {
            _ = try await IOSBoundedDataSourceBody.collect(bytes, limit: 4)
            XCTFail("Unknown Content-Length must not bypass the streamed byte limit")
        } catch {
            XCTAssertEqual((error as? URLError)?.code, .dataLengthExceedsMaximum)
        }
    }

    func testBoundedBodyRejectsAdvertisedOverflowBeforeReading() async {
        let bytes = AsyncStream<UInt8> { $0.finish() }
        do {
            _ = try await IOSBoundedDataSourceBody.collect(bytes, advertisedLength: 5, limit: 4)
            XCTFail("Oversized Content-Length must be rejected even with an empty body")
        } catch {
            XCTAssertEqual((error as? URLError)?.code, .dataLengthExceedsMaximum)
        }
    }
}

import Network

/// Real TCP fixture: no Content-Length, and a body much larger than the client budget.
private final class OversizedSourceHTTPFixture: @unchecked Sendable {
    private let listener: NWListener
    private let queue = DispatchQueue(label: "nola.tests.oversized-source-http")
    private var connection: NWConnection?
    private var ready: CheckedContinuation<UInt16, Error>?
    private var sentBytes = 0
    private var disconnected = false
    private let totalBytes: Int
    private let chunkDelay: TimeInterval

    init(totalBytes: Int = 32 * 1_048_576, chunkDelay: TimeInterval = 0.005) throws {
        self.totalBytes = totalBytes
        self.chunkDelay = chunkDelay
        listener = try NWListener(using: .tcp, on: .any)
    }

    func start() async throws -> UInt16 {
        try await withCheckedThrowingContinuation { continuation in
            ready = continuation
            listener.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready:
                    if let port = self.listener.port {
                        let pending = self.ready
                        self.ready = nil
                        pending?.resume(returning: port.rawValue)
                    }
                case .failed(let error):
                    let pending = self.ready
                    self.ready = nil
                    pending?.resume(throwing: error)
                default: break
                }
            }
            listener.newConnectionHandler = { [weak self] connection in
                guard let self else { connection.cancel(); return }
                self.connection = connection
                connection.start(queue: self.queue)
                connection.receive(minimumIncompleteLength: 1, maximumLength: 16_384) { [weak self] _, _, _, error in
                    guard let self, error == nil else { connection.cancel(); return }
                    let header = Data("HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n".utf8)
                    connection.send(content: header, completion: .contentProcessed { [weak self] error in
                        guard let self else { return }
                        if error != nil { self.disconnected = true; return }
                        self.sendChunk(connection)
                    })
                }
            }
            listener.start(queue: queue)
        }
    }

    private func sendChunk(_ connection: NWConnection) {
        guard sentBytes < totalBytes else {
            connection.send(content: nil, contentContext: .finalMessage, isComplete: true, completion: .contentProcessed { _ in connection.cancel() })
            return
        }
        let chunk = Data(repeating: 32, count: min(65_536, totalBytes - sentBytes))
        connection.send(content: chunk, completion: .contentProcessed { [weak self] error in
            guard let self else { return }
            if error != nil {
                self.disconnected = true
                connection.cancel()
                return
            }
            self.sentBytes += chunk.count
            self.queue.asyncAfter(deadline: .now() + self.chunkDelay) { [weak self] in
                self?.sendChunk(connection)
            }
        })
    }

    func snapshot() async -> (bytes: Int, disconnected: Bool) {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: (self.sentBytes, self.disconnected)) }
        }
    }

    func stop() {
        queue.async {
            self.connection?.cancel()
            self.listener.cancel()
        }
    }
}

extension URLDataSourceMetadataTests {
    func testRealHTTPSourceOverflowDisconnectsBeforeFullDownload() async throws {
        let server = try OversizedSourceHTTPFixture()
        let port = try await server.start()
        defer { server.stop() }
        let session = URLSession(configuration: .ephemeral)
        defer { session.invalidateAndCancel() }
        let url = try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/source.json"))
        var request = URLRequest(url: url)
        request.timeoutInterval = 10
        do {
            _ = try await IOSBoundedDataSourceBody.load(request, using: session)
            XCTFail("The actual HTTP reader must reject the oversized source")
        } catch {
            XCTAssertEqual((error as? URLError)?.code, .dataLengthExceedsMaximum)
        }
        var observation = await server.snapshot()
        for _ in 0..<100 where !observation.disconnected {
            try await Task.sleep(for: .milliseconds(20))
            observation = await server.snapshot()
        }
        XCTAssertTrue(observation.disconnected, "Overflow must cancel the underlying HTTP task")
        XCTAssertLessThan(observation.bytes, 32 * 1_048_576, "The server must not finish sending the oversized body")
        let evidence = XCTAttachment(string: "Server bytes sent: \(observation.bytes); disconnected: \(observation.disconnected); client limit: \(IOSBoundedDataSourceBody.maximumBytes)")
        evidence.lifetime = .keepAlways
        add(evidence)
    }
}

extension URLDataSourceMetadataTests {
    private func createValidatedReadFixture(runtime: IOSApplicationRuntime, root: URL) async throws -> (URLDataSource, IOSCanonicalDataSource) {
        let file = root.appendingPathComponent("source.txt")
        try Data("Canonical source content".utf8).write(to: file)
        var source = URLDataSource(url: file.absoluteString, format: .html, kind: .file, name: "Read fixture")
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        guard case .success(_, let contract) = await fetcher.validateSource(source) else {
            throw URLError(.cannotParseResponse)
        }
        source.validationContract = contract
        let submission = try XCTUnwrap(source.canonicalSubmission)
        return (source, try runtime.saveDataSource(submission))
    }

    func testCanonicalReadCommitsWithoutRevalidationAndSurvivesReopen() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let pending = try runtime.requestDataSourceRefresh(id: initial.id, expectedRevision: initial.revision, reason: "test read", requestedAtMs: now)
        XCTAssertEqual(pending.readState.status, .pending)
        let token = try XCTUnwrap(pending.readState.readToken)
        XCTAssertThrowsError(try runtime.requestDataSourceRefresh(id: initial.id, expectedRevision: pending.revision, reason: "duplicate", requestedAtMs: now))
        let reference = IOSCanonicalDataSourceContentReference(uri: root.appendingPathComponent("source.txt").absoluteString, mimeType: "text/plain", byteCount: 24, contentVersion: "fixture-1")
        let completed = try runtime.recordDataSourceRead(id: initial.id, readToken: token, result: .succeeded(contentReference: reference, observedAtMs: now, schemaVersion: 1, recordCount: 1), recordedAtMs: now)
        XCTAssertEqual(completed.readState.status, .ready)
        XCTAssertNil(completed.readState.readToken)
        XCTAssertEqual(completed.readState.contentReference, reference)
        XCTAssertEqual(completed.validation.completedAtMs, initial.validation.completedAtMs)
        XCTAssertEqual(completed.configuration, initial.configuration)
        XCTAssertThrowsError(try runtime.recordDataSourceRead(id: initial.id, readToken: token, result: .empty(observedAtMs: now, schemaVersion: 1), recordedAtMs: now))
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        XCTAssertEqual(try reopened.listDataSources().first, completed)
    }

    func testConfigurationChangeRejectsPriorReadToken() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        var (source, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let pending = try runtime.requestDataSourceRefresh(id: initial.id, expectedRevision: initial.revision, reason: "test read", requestedAtMs: now)
        let token = try XCTUnwrap(pending.readState.readToken)
        source.name = "Updated configuration"
        let updated = try runtime.saveDataSource(XCTUnwrap(source.canonicalSubmission), expectedRevision: pending.revision)
        XCTAssertThrowsError(try runtime.recordDataSourceRead(id: initial.id, readToken: token, result: .empty(observedAtMs: now, schemaVersion: 1), recordedAtMs: now))
        XCTAssertEqual(try runtime.listDataSources().first, updated)
    }
}

extension URLDataSourceMetadataTests {
    func testRealSourceRefreshStoresContentWithoutConfigurationWrite() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let source = try XCTUnwrap(fetcher.sources.first)
        let events = try await fetcher.fetchEvents(from: source)
        XCTAssertEqual(events.count, 1)
        let committed = try XCTUnwrap(runtime.listDataSources().first)
        XCTAssertEqual(committed.configuration, initial.configuration)
        XCTAssertEqual(committed.validation.completedAtMs, initial.validation.completedAtMs)
        XCTAssertEqual(committed.readState.status, .ready)
        let reference = try XCTUnwrap(committed.readState.contentReference)
        let url = try XCTUnwrap(URL(string: reference.uri))
        let stored = try Data(contentsOf: url)
        XCTAssertEqual(UInt64(stored.count), reference.byteCount)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .deferredToDate
        XCTAssertEqual(try decoder.decode([RawEvent].self, from: stored), events)
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let restored = URLFetcherService(logger: Logger(), applicationRuntime: reopened)
        XCTAssertNotNil(restored.sources.first?.lastFetchDate)
        XCTAssertEqual(restored.sources.first?.canonicalRevision, committed.revision)
        XCTAssertEqual(restored.sources.first?.isRefreshing, false)
    }

    func testFailedSourceRefreshSettlesTokenAndCanRetry() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let file = root.appendingPathComponent("source.txt")
        try FileManager.default.removeItem(at: file)
        do {
            _ = try await fetcher.fetchEvents(from: XCTUnwrap(fetcher.sources.first))
            XCTFail("Missing file must fail the read")
        } catch { }
        let failed = try XCTUnwrap(runtime.listDataSources().first)
        XCTAssertEqual(failed.readState.status, .failed)
        XCTAssertNil(failed.readState.readToken)
        XCTAssertEqual(failed.configuration, initial.configuration)
        XCTAssertNotNil(fetcher.sources.first?.lastError)
        XCTAssertEqual(fetcher.sources.first?.isRefreshing, false)
        try Data("Recovered source".utf8).write(to: file)
        _ = try await fetcher.fetchEvents(from: XCTUnwrap(fetcher.sources.first))
        let recovered = try XCTUnwrap(runtime.listDataSources().first)
        XCTAssertEqual(recovered.readState.status, .ready)
        XCTAssertNil(fetcher.sources.first?.lastError)
        XCTAssertEqual(recovered.configuration, initial.configuration)
    }
}

extension URLDataSourceMetadataTests {
    func testCancellingRealHTTPRefreshSettlesApplicationRead() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let server = try OversizedSourceHTTPFixture()
        let port = try await server.start()
        defer { server.stop() }
        var source = URLDataSource(url: "http://127.0.0.1:\(port)/source.json", format: .html, name: "Cancellation fixture")
        var contract = URLDataSourceValidationContract(
            sourceID: source.id, connectorID: "ios.url-session", connectorVersion: 1,
            readerDescription: "HTTP source fixture", schemaFields: ["title"],
            redactedSample: "<redacted>", recordCount: 1, maximumAgeMinutes: 60,
            validationState: .verified, validatedAt: Date(), observedAt: Date()
        )
        contract.configureReadRecipe(method: .httpGet, operationID: "ios.url-session.read", configurationKeys: ["url", "maximumBytes"])
        source.validationContract = contract
        let initial = try runtime.saveDataSource(XCTUnwrap(source.canonicalSubmission))
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let observed = try XCTUnwrap(fetcher.sources.first)
        let task = Task { try await fetcher.fetchEvents(from: observed) }
        defer { task.cancel() }
        var network = await server.snapshot()
        for _ in 0..<100 where network.bytes == 0 {
            try await Task.sleep(for: .milliseconds(20))
            network = await server.snapshot()
        }
        XCTAssertGreaterThan(network.bytes, 0, "Cancel only after real body transmission begins")
        XCTAssertEqual(try runtime.listDataSources().first?.readState.status, .pending)
        task.cancel()
        do {
            _ = try await task.value
            XCTFail("Cancelled HTTP refresh must not report success")
        } catch {
            XCTAssertTrue(error is CancellationError || (error as? URLError)?.code == .cancelled)
        }
        let settled = try XCTUnwrap(runtime.listDataSources().first)
        XCTAssertEqual(settled.readState.status, .failed)
        XCTAssertNil(settled.readState.readToken)
        XCTAssertTrue(settled.readState.diagnostic?.hasPrefix("cancelled:") == true)
        XCTAssertEqual(settled.configuration, initial.configuration)
        XCTAssertEqual(settled.validation.completedAtMs, initial.validation.completedAtMs)
        XCTAssertEqual(fetcher.sources.first?.isRefreshing, false)
        XCTAssertNotNil(fetcher.sources.first?.lastError)
    }
}

extension URLDataSourceMetadataTests {
    func testConcurrentSourceReadsSettleIndependentlyWithoutBatchWrites() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, first) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let secondRoot = root.appendingPathComponent("second", isDirectory: true)
        try FileManager.default.createDirectory(at: secondRoot, withIntermediateDirectories: true)
        let (_, second) = try await createValidatedReadFixture(runtime: runtime, root: secondRoot)
        try FileManager.default.removeItem(at: secondRoot.appendingPathComponent("source.txt"))
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let sources = fetcher.sources
        let outcomes = await withTaskGroup(of: Bool.self, returning: [Bool].self) { group in
            for source in sources {
                group.addTask {
                    do { _ = try await fetcher.fetchEvents(from: source); return true }
                    catch { return false }
                }
            }
            var results: [Bool] = []
            for await result in group { results.append(result) }
            return results
        }
        XCTAssertEqual(outcomes.filter { $0 }.count, 1)
        XCTAssertEqual(outcomes.filter { !$0 }.count, 1)
        let records = try runtime.listDataSources()
        let success = try XCTUnwrap(records.first { $0.id == first.id })
        let failure = try XCTUnwrap(records.first { $0.id == second.id })
        XCTAssertEqual(success.readState.status, .ready)
        XCTAssertEqual(failure.readState.status, .failed)
        XCTAssertNil(success.readState.readToken)
        XCTAssertNil(failure.readState.readToken)
        XCTAssertEqual(success.configuration, first.configuration)
        XCTAssertEqual(failure.configuration, second.configuration)
        XCTAssertEqual(success.validation.completedAtMs, first.validation.completedAtMs)
        XCTAssertEqual(failure.validation.completedAtMs, second.validation.completedAtMs)
        XCTAssertTrue(fetcher.sources.allSatisfy { !$0.isRefreshing })
    }

    func testConcurrentReadsOfSameRevisionDoNotCreateTwoOwners() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        _ = try await createValidatedReadFixture(runtime: runtime, root: root)
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let source = try XCTUnwrap(fetcher.sources.first)
        let successes = await withTaskGroup(of: Bool.self, returning: Int.self) { group in
            for _ in 0..<2 {
                group.addTask {
                    do { _ = try await fetcher.fetchEvents(from: source); return true }
                    catch { return false }
                }
            }
            var count = 0
            for await result in group { if result { count += 1 } }
            return count
        }
        XCTAssertEqual(successes, 1)
        let record = try XCTUnwrap(runtime.listDataSources().first)
        XCTAssertEqual(record.readState.status, .ready)
        XCTAssertNil(record.readState.readToken)
        XCTAssertEqual(fetcher.sources.first?.canonicalRevision, record.revision)
    }
}

extension URLDataSourceMetadataTests {
    func testCanonicalConversationOverrideIsIsolatedAndRestoresDefault() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        var (source, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        source.enabled = false
        let configured = try runtime.saveDataSource(XCTUnwrap(source.canonicalSubmission), expectedRevision: initial.revision)
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let firstID = try fixtureConversation(runtime)
        let secondID = try fixtureConversation(runtime)
        XCTAssertEqual(try runtime.dataSourceOverride(conversationID: firstID, sourceID: initial.id), .inherit)
        try runtime.setDataSourceOverride(conversationID: firstID, sourceID: initial.id, value: .enabled, updatedAtMs: now)
        let first = try XCTUnwrap(runtime.resolveDataSources(conversationID: firstID, nowMs: now).first)
        let second = try XCTUnwrap(runtime.resolveDataSources(conversationID: secondID, nowMs: now).first)
        XCTAssertTrue(first.enabled)
        XCTAssertTrue(first.shouldRefresh)
        XCTAssertFalse(first.usable)
        XCTAssertFalse(second.enabled)
        XCTAssertEqual(second.reason, .disabledByDefault)
        XCTAssertEqual(try runtime.listDataSources().first, configured, "Conversation overrides must not change global configuration")
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        XCTAssertEqual(try reopened.dataSourceOverride(conversationID: firstID, sourceID: initial.id), .enabled)
        try reopened.setDataSourceOverride(conversationID: firstID, sourceID: initial.id, value: .disabled, updatedAtMs: now)
        XCTAssertEqual(try reopened.resolveDataSources(conversationID: firstID, nowMs: now).first?.reason, .disabledForConversation)
        try reopened.setDataSourceOverride(conversationID: firstID, sourceID: initial.id, value: .inherit, updatedAtMs: now)
        XCTAssertEqual(try reopened.dataSourceOverride(conversationID: firstID, sourceID: initial.id), .inherit)
        XCTAssertEqual(try reopened.resolveDataSources(conversationID: firstID, nowMs: now).first?.reason, .disabledByDefault)
    }

    func testCanonicalContextSelectionTracksPendingEmptyAndExpiry() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let conversationID = try fixtureConversation(runtime)
        try runtime.setDataSourceOverride(conversationID: conversationID, sourceID: initial.id, value: .enabled, updatedAtMs: now)
        let pending = try runtime.requestDataSourceRefresh(id: initial.id, expectedRevision: initial.revision, reason: "context selection test", requestedAtMs: now)
        let during = try XCTUnwrap(runtime.resolveDataSources(conversationID: conversationID, nowMs: now).first)
        XCTAssertEqual(during.reason, .readPending)
        XCTAssertFalse(during.usable)
        XCTAssertFalse(during.shouldRefresh, "Pending reads must not trigger a second owner")
        let completed = try runtime.recordDataSourceRead(id: initial.id, readToken: XCTUnwrap(pending.readState.readToken), result: .empty(observedAtMs: now, schemaVersion: 1), recordedAtMs: now)
        let after = try XCTUnwrap(runtime.resolveDataSources(conversationID: conversationID, nowMs: now).first)
        XCTAssertEqual(after.reason, .readyEmpty)
        XCTAssertTrue(after.usable)
        XCTAssertEqual(after.sourceRevision, completed.revision)
        XCTAssertNil(after.contentReference)
        let expiredAt = now + Int64(initial.freshnessPolicy.maximumAgeMinutes) * 60_000 + 60_000
        let expired = try XCTUnwrap(runtime.resolveDataSources(conversationID: conversationID, nowMs: expiredAt).first)
        XCTAssertEqual(expired.reason, .refreshRequired)
        XCTAssertTrue(expired.shouldRefresh)
        XCTAssertFalse(expired.usable)
    }
}

extension URLDataSourceMetadataTests {
    func testConversationContextEditPersistsOnlyCanonicalCustomOverrides() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, source) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "Source context", language: "en")
        let templateID = UUID()
        try store.updateConversationMetadata(conversationId: conversation.id, language: nil, selectedPromptTemplateID: templateID, dataSourceOverrides: [source.id: true, "health": true])
        XCTAssertEqual(try runtime.dataSourceOverride(conversationID: conversation.id.uuidString, sourceID: source.id), .enabled)
        let persisted = try XCTUnwrap(runtime.loadConversation(id: conversation.id.uuidString))
        let raw = try XCTUnwrap(persisted.settingsJSON)
        let settings = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        let embedded = try XCTUnwrap(settings["dataSourceOverrides"] as? [String: Bool])
        XCTAssertNil(embedded[source.id], "Canonical custom overrides must not have a second persisted copy")
        XCTAssertEqual(embedded["health"], true)
        let reopenedRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let reopened = try LocalConversationStore(applicationRuntime: reopenedRuntime)
        let restored = try XCTUnwrap(reopened.getConversation(conversationId: conversation.id))
        XCTAssertEqual(restored.dataSourceOverrides[source.id], true)
        XCTAssertEqual(restored.selectedPromptTemplateID, templateID)
        try reopened.updateConversationMetadata(conversationId: conversation.id, language: nil, selectedPromptTemplateID: nil, dataSourceOverrides: ["health": true])
        XCTAssertEqual(try reopenedRuntime.dataSourceOverride(conversationID: conversation.id.uuidString, sourceID: source.id), .inherit)
        XCTAssertNil(try reopened.getConversation(conversationId: conversation.id)?.dataSourceOverrides[source.id])
        XCTAssertNil(try reopened.getConversation(conversationId: conversation.id)?.selectedPromptTemplateID)
        try reopened.updateConversationMetadata(conversationId: conversation.id, language: nil,
            selectedPromptTemplateID: nil, dataSourceOverrides: [:])
        let cleared = try XCTUnwrap(reopenedRuntime.loadConversation(id: conversation.id.uuidString))
        let clearedSettings = try IOSConversationSettingsObject.decode(cleared.settingsJSON)
        let clearedOverrides = try XCTUnwrap(clearedSettings["dataSourceOverrides"] as? [String: Bool])
        XCTAssertNil(clearedOverrides[source.id])
        XCTAssertNil(clearedOverrides["health"], "Removed custom keys must be reset through the sparse map patch")
    }

    func testOrdinaryConversationSaveDoesNotReplayStaleSourceOverrides() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, source) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        var stale = try store.createConversation(title: "Original", language: "en")
        stale.dataSourceOverrides[source.id] = false
        try store.updateConversationMetadata(conversationId: stale.id, language: nil, selectedPromptTemplateID: nil, dataSourceOverrides: [source.id: true])
        stale.title = "Renamed"
        try store.upsertConversation(stale)
        XCTAssertEqual(try runtime.dataSourceOverride(conversationID: stale.id.uuidString, sourceID: source.id), .enabled)
        let restored = try XCTUnwrap(store.getConversation(conversationId: stale.id))
        XCTAssertEqual(restored.title, "Renamed")
        XCTAssertEqual(restored.dataSourceOverrides[source.id], true)
    }
}

extension URLDataSourceMetadataTests {
    func testCanonicalContextReadsConversationEnabledSourceAndRestoresCache() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        var (source, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        source.enabled = false
        _ = try runtime.saveDataSource(XCTUnwrap(source.canonicalSubmission), expectedRevision: initial.revision)
        let conversationID = try fixtureConversation(runtime)
        try runtime.setDataSourceOverride(conversationID: conversationID, sourceID: initial.id, value: .enabled, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let selected = try await fetcher.canonicalContextSources(conversationID: conversationID, allowLiveFetch: true)
        XCTAssertEqual(selected.count, 1)
        XCTAssertEqual(selected.first?.events.first?.notes, "Canonical source content")
        XCTAssertEqual(try runtime.listDataSources().first?.defaultEnabled, false)
        try FileManager.default.removeItem(at: root.appendingPathComponent("source.txt"))
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let restored = URLFetcherService(logger: Logger(), applicationRuntime: reopened)
        let cached = try await restored.canonicalContextSources(conversationID: conversationID, allowLiveFetch: false)
        XCTAssertEqual(cached.first?.events, selected.first?.events)
        let other = try await restored.canonicalContextSources(conversationID: UUID().uuidString, allowLiveFetch: false)
        XCTAssertTrue(other.isEmpty)
        try reopened.setDataSourceOverride(conversationID: conversationID, sourceID: initial.id, value: .disabled, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        let disabled = try await restored.canonicalContextSources(conversationID: conversationID, allowLiveFetch: true)
        XCTAssertTrue(disabled.isEmpty)
    }

    func testCanonicalContextDoesNotUseOldHistoryWithoutUsableRead() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        var (source, initial) = try await createValidatedReadFixture(runtime: runtime, root: root)
        source.history = [.init(dedupeKey: "old", title: "Old content", details: "Must not be injected")]
        let updated = try runtime.saveDataSource(XCTUnwrap(source.canonicalSubmission), expectedRevision: initial.revision)
        let conversationID = try fixtureConversation(runtime)
        try runtime.setDataSourceOverride(conversationID: conversationID, sourceID: updated.id, value: .enabled, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let items = try await fetcher.canonicalContextSources(conversationID: conversationID, allowLiveFetch: false)
        XCTAssertTrue(items.isEmpty)
        XCTAssertEqual(try runtime.listDataSources().first?.readState.status, .neverRead)
    }
}

extension URLDataSourceMetadataTests {
    func testConversationSourceSettingControlsActualCorePayload() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let (_, source) = try await createValidatedReadFixture(runtime: runtime, root: root)
        let marker = "SOURCE_PAYLOAD_EVIDENCE"
        let excludedTail = "MUST_NOT_REACH_MODEL_TAIL"
        try Data((marker + String(repeating: "x", count: 20_000) + excludedTail).utf8)
            .write(to: root.appendingPathComponent("source.txt"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "Payload test", language: "en")
        try store.updateConversationMetadata(conversationId: conversation.id, language: nil, selectedPromptTemplateID: nil, dataSourceOverrides: [source.id: true])
        let manager = DefaultContextManager(store: store, profileStore: CanonicalChatMemoryProfileStore(applicationRuntime: runtime))
        manager.urlFetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let payload = try await manager.prepareOutboundPayload(for: conversation.id, maxTokens: 32_768, additionalComponents: [], tools: [])
        let actualText = payload.segments.map(\.content).joined(separator: "\n")
        let sourceInputs = payload.contextComponents.filter { $0.id == "data_source:\(source.id)" }
        let trace = XCTAttachment(string: "INPUT COMPONENTS:\n\(String(describing: sourceInputs))\n\nCOMPONENT METRICS:\n\(String(describing: payload.componentMetrics))\n\nFINAL MESSAGES:\n\(actualText)")
        trace.lifetime = .keepAlways
        add(trace)
        XCTAssertTrue(actualText.contains(marker))
        XCTAssertFalse(actualText.contains(excludedTail))
        XCTAssertTrue(payload.usedDataSourceIDs.contains(try XCTUnwrap(UUID(uuidString: source.id))))
        let component = try XCTUnwrap(payload.contextComponents.first { $0.id == "data_source:\(source.id)" })
        guard case .string(let content) = component.content else { return XCTFail("Expected bounded source text") }
        XCTAssertLessThan(content.count, 5_000)
        try store.updateConversationMetadata(conversationId: conversation.id, language: nil, selectedPromptTemplateID: nil, dataSourceOverrides: [source.id: false])
        let disabled = try await manager.prepareOutboundPayload(for: conversation.id, maxTokens: 32_768, additionalComponents: [], tools: [])
        XCTAssertFalse(disabled.segments.map(\.content).joined().contains(marker))
        XCTAssertTrue(disabled.usedDataSourceIDs.isEmpty)
    }

    func testDisablingSourceDuringRealHTTPReadPreventsContextInjection() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let server = try OversizedSourceHTTPFixture(totalBytes: 262_144, chunkDelay: 0.1)
        let port = try await server.start()
        defer { server.stop() }
        var source = URLDataSource(url: "http://127.0.0.1:\(port)/finite.html", format: .html, name: "Revocation fixture")
        var contract = URLDataSourceValidationContract(
            sourceID: source.id, connectorID: "ios.url-session", connectorVersion: 1,
            readerDescription: "HTTP source fixture", schemaFields: ["title"],
            redactedSample: "<redacted>", recordCount: 1, maximumAgeMinutes: 60,
            validationState: .verified, validatedAt: Date(), observedAt: Date()
        )
        contract.configureReadRecipe(method: .httpGet, operationID: "ios.url-session.read", configurationKeys: ["url", "maximumBytes"])
        source.validationContract = contract
        let initial = try runtime.saveDataSource(XCTUnwrap(source.canonicalSubmission))
        let conversationID = try fixtureConversation(runtime)
        try runtime.setDataSourceOverride(conversationID: conversationID, sourceID: initial.id, value: .enabled, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        let fetcher = URLFetcherService(logger: Logger(), applicationRuntime: runtime)
        let task = Task { try await fetcher.canonicalContextSources(conversationID: conversationID, allowLiveFetch: true) }
        defer { task.cancel() }
        var network = await server.snapshot()
        for _ in 0..<100 where network.bytes == 0 {
            try await Task.sleep(for: .milliseconds(20))
            network = await server.snapshot()
        }
        XCTAssertGreaterThan(network.bytes, 0)
        XCTAssertEqual(try runtime.listDataSources().first?.readState.status, .pending)
        try runtime.setDataSourceOverride(conversationID: conversationID, sourceID: initial.id, value: .disabled, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        let result = try await task.value
        XCTAssertTrue(result.isEmpty)
        XCTAssertEqual(try runtime.listDataSources().first?.readState.status, .ready, "Exclusion must result from revocation, not a failed network read")
    }
}
