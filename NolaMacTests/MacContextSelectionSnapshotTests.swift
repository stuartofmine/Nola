import Foundation
import XCTest
@testable import Nola

private final class SnapshotSourceURLProtocol: URLProtocol, @unchecked Sendable {
    @MainActor static var handlers: [String: (URLRequest) -> Void] = [:]
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Task { @MainActor in
            Self.handlers[request.url!.host!]?(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: 200,
                httpVersion: nil, headerFields: ["Content-Type": "text/plain"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data("fetched original content".utf8))
            client?.urlProtocolDidFinishLoading(self)
        }
    }
    override func stopLoading() {}
}

@MainActor
final class MacContextSelectionSnapshotTests: XCTestCase {
    func testBrowserPreviewIdentityDoesNotDependOnFrameTimestamps() throws {
        let endpoint = try XCTUnwrap(URL(string: "http://127.0.0.1:8123/preview/session?token=fixture"))
        let identity = MacManagedBrowserDocumentIdentity(owner: "conversation", sessionID: "browser-session", endpoint: endpoint)
        XCTAssertEqual(identity, .init(owner: "conversation", sessionID: "browser-session", endpoint: endpoint))
        XCTAssertNotEqual(identity, .init(owner: "other-conversation", sessionID: "browser-session", endpoint: endpoint))
        XCTAssertNotEqual(identity, .init(owner: "conversation", sessionID: "new-browser-session", endpoint: endpoint))
    }

    func testCanonicalEstimateAndCacheDiagnosticRetainIndependentRequestSources() throws {
        let data = Data(#"{"used_tokens":2000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":true,"measured_at_revision":10,"measurement":{"request_id":"next","attempt_id":null,"context_generation":4},"prompt_cache_measurement":{"request_id":"paid","attempt_id":null,"context_generation":3},"prompt_cache_diagnostic":{"schemaVersion":1,"status":"hit","identityStable":false,"reasons":["tool_manifest_changed"],"cacheReadInputTokens":800,"cacheWriteInputTokens":null}}"#.utf8)
        let usage = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ContextUsage.self, from: data)
        let snapshot = MacConversationContextSnapshot(canonical: usage)
        XCTAssertTrue(snapshot.isEstimated)
        XCTAssertEqual(snapshot.measurement?.requestID, "next")
        XCTAssertEqual(snapshot.promptCacheMeasurement?.requestID, "paid")
        XCTAssertNil(snapshot.promptCacheMeasurement?.attemptID)
        XCTAssertEqual(snapshot.promptCacheDiagnostic?.reasons, ["tool_manifest_changed"])
        let restored = try JSONDecoder().decode(MacConversationContextSnapshot.self, from: JSONEncoder().encode(snapshot))
        XCTAssertEqual(restored, snapshot)
    }

    func testCacheOnlyChangeMustRefreshInspectorWithinSameContextBucket() throws {
        // Reproduce the measured latest-conversation values without user data.
        let miss = MacConversationContextSnapshot(usedTokens: 15_455,
            contextWindowTokens: 128_000, promptTokenLimit: 111_616, isEstimated: false,
            cacheUsage: MacPromptCacheUsage(input_tokens: 15_455, read_tokens: 0,
                write_tokens: nil, hit_rate_bps: 0, is_compaction: false))
        let hit = MacConversationContextSnapshot(usedTokens: 15_485,
            contextWindowTokens: 128_000, promptTokenLimit: 111_616, isEstimated: false,
            cacheUsage: MacPromptCacheUsage(input_tokens: 15_485, read_tokens: 15_450,
                write_tokens: nil, hit_rate_bps: 9_977, is_compaction: false))
        XCTAssertNotEqual(miss.cacheUsage?.rateText, hit.cacheUsage?.rateText)
        XCTAssertEqual(miss.cacheUsage?.hit_rate_bps, 0)
        XCTAssertEqual(hit.cacheUsage?.hit_rate_bps, 9_977)
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: miss, next: hit))
    }

    func testUnknownAndMeasuredMissAreDistinctEvenWithUnchangedOccupancy() {
        let unknown = cacheSnapshot(usage: MacPromptCacheUsage(input_tokens: 1000,
            read_tokens: nil, write_tokens: nil, hit_rate_bps: nil, is_compaction: false))
        let miss = cacheSnapshot(usage: MacPromptCacheUsage(input_tokens: 1000,
            read_tokens: 0, write_tokens: nil, hit_rate_bps: 0, is_compaction: false))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: unknown, next: miss))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: miss, next: unknown))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: miss, next: cacheSnapshot()))
    }

    func testReadWriteAndDiagnosticChangesPublishIndependently() {
        let original = cacheSnapshot()
        let diagnostic = MacPromptCacheDiagnostic(schemaVersion: 1, status: "hit", identityStable: true,
            identitySHA256: nil, reasons: ["provider_reported_cache_read"], fragmentChanges: nil,
            cacheReadInputTokens: 800, cacheWriteInputTokens: nil)
        for snapshot in [cacheSnapshot(read: 0), cacheSnapshot(write: 0),
                         cacheSnapshot(read: 800), cacheSnapshot(write: 100),
                         cacheSnapshot(diagnostic: diagnostic)] {
            XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: original, next: snapshot))
            XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: snapshot, next: original))
        }
    }

    func testUnchangedCacheEvidenceStillCoalescesOccupancyWithinSameBucket() {
        let current = cacheSnapshot(used: 1000)
        XCTAssertFalse(MacContextSnapshotPublicationPolicy.shouldPublish(current: current, next: current))
        XCTAssertFalse(MacContextSnapshotPublicationPolicy.shouldPublish(current: current, next: cacheSnapshot(used: 1001)))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: current, next: cacheSnapshot(used: 1300)))
    }

    private func cacheSnapshot(used: Int = 1000, read: Int? = nil, write: Int? = nil,
                               diagnostic: MacPromptCacheDiagnostic? = nil,
                               usage: MacPromptCacheUsage? = nil) -> MacConversationContextSnapshot {
        MacConversationContextSnapshot(usedTokens: used, contextWindowTokens: 128_000,
            promptTokenLimit: 111_616, isEstimated: false, cachedInputTokens: read,
            cacheWriteInputTokens: write, promptCacheDiagnostic: diagnostic, cacheUsage: usage)
    }

    func testCacheUsageSurvivesCanonicalAndPersistedSnapshotRoundTrip() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":800,"write_tokens":100,"hit_rate_bps":8000,"is_compaction":true}}"#.utf8)
        let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ContextUsage.self, from: data)
        XCTAssertEqual(projection.cacheUsage?.hit_rate_bps, 8000)
        let snapshot = MacConversationContextSnapshot(usedTokens: 1000, contextWindowTokens: 128000, promptTokenLimit: 120000, isEstimated: false, cacheUsage: projection.cacheUsage)
        let restored = try JSONDecoder().decode(MacConversationContextSnapshot.self, from: JSONEncoder().encode(snapshot))
        XCTAssertEqual(restored.cacheUsage, projection.cacheUsage)
        XCTAssertTrue(restored.cacheUsage?.is_compaction == true)
        let unknown = MacPromptCacheUsage(input_tokens: 1000, read_tokens: nil, write_tokens: nil, hit_rate_bps: nil, is_compaction: false)
        let miss = MacPromptCacheUsage(input_tokens: 1000, read_tokens: 0, write_tokens: 1000, hit_rate_bps: 0, is_compaction: false)
        XCTAssertNotEqual(unknown.rateText, miss.rateText)
    }

    func testConversationCacheTotalsAreIndependentOfLastRequestAndOccupancy() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":900,"read_tokens":0,"write_tokens":0,"hit_rate_bps":0,"is_compaction":false},"conversation_cache_usage":{"input_tokens":1000,"read_tokens":100,"write_tokens":0,"hit_rate_bps":1000,"is_compaction":false,"reported_requests":2,"unreported_requests":1}}"#.utf8)
        let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ContextUsage.self, from: data)
        let before = MacConversationContextSnapshot(usedTokens: 1000, contextWindowTokens: 128000,
            promptTokenLimit: 120000, isEstimated: false, cacheUsage: projection.cacheUsage)
        let after = MacConversationContextSnapshot(usedTokens: 1000, contextWindowTokens: 128000,
            promptTokenLimit: 120000, isEstimated: false, cacheUsage: projection.cacheUsage,
            conversationCacheUsage: projection.conversationCacheUsage)
        XCTAssertEqual(after.cacheUsage?.hit_rate_bps, 0)
        XCTAssertEqual(after.conversationCacheUsage?.hit_rate_bps, 1000)
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: before, next: after))
        let restored = try JSONDecoder().decode(MacConversationContextSnapshot.self, from: JSONEncoder().encode(after))
        XCTAssertEqual(restored.conversationCacheUsage, after.conversationCacheUsage)
    }

    private func fixture() throws -> (URL, MacDataSourceRegistry, MacWebSourceStore, MacWebSource) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-source-snapshot-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SnapshotSourceURLProtocol.self]
        let session = URLSession(configuration: configuration)
        addTeardownBlock { session.invalidateAndCancel() }
        let store = MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry, urlSession: session)
        let source = MacWebSource(id: UUID(), name: "Original source", url: "https://\(UUID().uuidString.lowercased()).invalid/original",
            format: .text, isEnabled: true, cachedText: "original cache", lastFetchedAt: .now,
            lastError: nil, createdAt: .now)
        store.upsert(source)
        registry.recordValidation(sourceID: source.dataSourceID, connectorID: "mac.web.http",
            readerDescription: "HTTP GET", schemaFields: ["text"], redactedSample: "text",
            recordCount: 1, maximumAgeMinutes: 60, observedAt: .now, defaultPolicy: .defaultEnabled)
        return (root, registry, store, source)
    }

    func testCapturedWebSelectionDoesNotReselectNameURLCacheOrEnablement() async throws {
        let (_, _, store, source) = try fixture()
        let selection = store.captureContextSelection(conversationID: "original-conversation")
        XCTAssertEqual(selection.sources.count, 1)
        var edited = source
        edited.name = "New source"; edited.url = "https://other.invalid/new"
        edited.cachedText = "other cache"; edited.isEnabled = false
        store.upsert(edited)
        let context = await store.promptWithSources("", selection: selection)
        XCTAssertTrue(context.contains("Original source"))
        XCTAssertTrue(context.contains(source.url))
        XCTAssertTrue(context.contains("original cache"))
        XCTAssertFalse(context.contains("other cache"))
    }

    func testFetchReturnsFrozenRequestContentWithoutOverwritingEditedSource() async throws {
        let (_, _, store, original) = try fixture()
        var source = original; source.lastFetchedAt = .distantPast
        store.upsert(source)
        let selection = store.captureContextSelection(conversationID: "c")
        let host = try XCTUnwrap(URL(string: source.url)?.host)
        SnapshotSourceURLProtocol.handlers[host] = { request in
            XCTAssertEqual(request.url?.absoluteString, source.url)
            var edited = source
            edited.name = "Edited"; edited.url = "https://edited.invalid/new"; edited.cachedText = "edited cache"
            store.upsert(edited)
        }
        defer { SnapshotSourceURLProtocol.handlers[host] = nil }
        let context = await store.promptWithSources("", selection: selection)
        XCTAssertTrue(context.contains("Original source"))
        XCTAssertTrue(context.contains("fetched original content"))
        XCTAssertFalse(context.contains("edited cache"))
        XCTAssertEqual(store.sources.first?.cachedText, "edited cache")
        XCTAssertEqual(store.sources.first?.name, "Edited")
    }

    func testLivePolicyRevocationDuringFetchSuppressesCapturedContextAndCacheWrite() async throws {
        let (_, registry, store, original) = try fixture()
        var source = original; source.lastFetchedAt = .distantPast
        store.upsert(source)
        let selection = store.captureContextSelection(conversationID: "c")
        let host = try XCTUnwrap(URL(string: source.url)?.host)
        SnapshotSourceURLProtocol.handlers[host] = { _ in registry.setPolicy(.blocked, for: source.dataSourceID) }
        defer { SnapshotSourceURLProtocol.handlers[host] = nil }
        let context = await store.promptWithSources("prompt", selection: selection)
        XCTAssertEqual(context, "prompt")
        XCTAssertEqual(store.sources.first?.cachedText, "original cache")
    }

    func testPlatformSelectionFreezesEnabledSetAndStepGuidance() throws {
        let (root, registry, _, _) = try fixture()
        // Conversation overrides are canonical writes, not an in-memory UI
        // toggle. Create the actual owner before requesting its override.
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        _ = try application.createConversation(.init(id: "captured", title: "Captured",
            settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1))
        let id = MacDataSourceRegistry.platformSourceID("health")
        registry.recordValidation(sourceID: id, connectorID: "health", readerDescription: "summary",
            schemaFields: ["steps"], redactedSample: "steps", recordCount: 1,
            maximumAgeMinutes: 60, observedAt: .now, defaultPolicy: .defaultEnabled)
        let key = MacRunNotificationPreferenceKeys.stepGuidance
        let suiteName = "nola-context-selection-\(UUID().uuidString)"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { preferences.removePersistentDomain(forName: suiteName) }
        preferences.set(false, forKey: key)
        let service = MacPlatformDataService(registry: registry, startsPeriodicRefresh: false,
            preferences: preferences)
        let selection = service.captureContextSelection(conversationID: "captured")
        registry.setConversationOverride(false, sourceID: id, conversationID: "captured")
        XCTAssertNil(registry.persistenceError)
        XCTAssertEqual(registry.conversationOverride(sourceID: id, conversationID: "captured"), false)
        preferences.set(true, forKey: key)
        XCTAssertTrue(selection.kinds.contains(.health))
        XCTAssertFalse(selection.stepGuidance)
        XCTAssertEqual(selection.conversationID, "captured")
        XCTAssertFalse(service.captureContextSelection(conversationID: "captured").kinds.contains(.health))
    }
}
