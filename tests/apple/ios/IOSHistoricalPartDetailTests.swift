// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class IOSHistoricalPartDetailTests: XCTestCase {
    func testHistoricalActionUsesContentNamesInsteadOfProtocolKindOrFileExtension() {
        XCTAssertEqual(IOSHistoricalDetailPresentation.detailTitle(for: "text"), NSLocalizedString("history.detail.open_text", comment: ""))
        XCTAssertEqual(IOSHistoricalDetailPresentation.detailTitle(for: "reasoning"), NSLocalizedString("history.detail.open_reasoning", comment: ""))
        XCTAssertEqual(IOSHistoricalDetailPresentation.detailTitle(for: "future_kind.txt"), NSLocalizedString("history.detail.open_record", comment: ""))
        XCTAssertFalse(IOSHistoricalDetailPresentation.detailTitle(for: "future_kind.txt").contains("future_kind"))
    }

    private func projection() throws -> IOSConversationProjectionV1.HistoricalProjection {
        let file = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("core/tests/fixtures/historical_unloaded_conversation_projection_v1.json")
        let window = try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(contentsOf: file))
        guard case .historical(_, _, let value) = window.messages[0].parts[0] else { throw CocoaError(.coderReadCorrupt) }
        return value
    }

    func testUnavailableCanRetryAndNotLoadedRemainsExplicit() async throws {
        let value = try projection()
        let request = try XCTUnwrap(IOSHistoricalPartRequest(value))
        let state = IOSHistoricalPartDetailState()
        await state.load(request) { _ in nil }
        guard case .failed = state.phase else { return XCTFail("Missing unavailable state") }
        await state.load(request) { _ in value }
        XCTAssertEqual(state.phase, .loaded(value))
        XCTAssertEqual(value.status.state, .notLoaded)
    }

    func testDismissedLateResponseCannotPublish() async throws {
        let value = try projection()
        let request = try XCTUnwrap(IOSHistoricalPartRequest(value))
        let state = IOSHistoricalPartDetailState()
        let gate = HistoricalReadGate()
        let task = Task { await state.load(request) { _ in await gate.wait() } }
        await gate.waitUntilStarted()
        state.invalidate()
        await gate.finish(value)
        await task.value
        XCTAssertEqual(state.phase, .loading)
    }

    func testReloadFailureKeepsPreviousDetails() async throws {
        let value = try projection()
        let request = try XCTUnwrap(IOSHistoricalPartRequest(value))
        let state = IOSHistoricalPartDetailState()
        await state.load(request) { _ in value }
        await state.load(request) { _ in nil }
        XCTAssertEqual(state.previous, value)
        guard case .failed = state.phase else { return XCTFail("Expected recoverable error") }
    }

    func testStructuredContentIsValidJSONAndPreservesIntegerPrecision() throws {
        let value: AgentCoreJSONValue = .object(["count": .unsignedInteger(UInt64.max), "text": .string("quote\"\n中")])
        let output = IOSHistoricalDetailPresentation.formatted(value)
        XCTAssertTrue(output.contains("18446744073709551615"))
        XCTAssertTrue(output.contains("\n"))
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(output.utf8)), value)
    }

    private func page(_ request: IOSHistoricalPartRequest, offset: UInt64 = 0,
                      text: String = "中", next: UInt64 = 3, total: UInt64 = 6,
                      revision: Int64 = 1, end: Bool = false) -> IOSHistoricalOriginalPage {
        IOSHistoricalOriginalPage(conversationID: request.owner.conversationID,
            messageID: request.owner.messageID, partID: request.owner.partID, field: "content_json",
            revision: revision, offsetBytes: offset, nextOffsetBytes: next,
            totalUTF8Bytes: total, text: text, endOfContent: end)
    }

    func testOriginalRangeRejectsStaleRevisionMalformedOffsetsAndOversizedPage() throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let query = IOSHistoricalOriginalQuery(request, offset: 0)
        XCTAssertTrue(page(request).matches(query))
        XCTAssertFalse(page(request, revision: 2).matches(query))
        XCTAssertFalse(page(request, next: 1).matches(query)) // UTF-8 bytes, not characters.
        XCTAssertFalse(page(request, total: 2).matches(query))
        XCTAssertFalse(page(request, end: true).matches(query))
        XCTAssertFalse(page(request, text: "", next: 0).matches(query))
        XCTAssertFalse(page(request, text: String(repeating: "a", count: 8193), next: 8193, total: 8193, end: true).matches(query))
        XCTAssertFalse(page(request, offset: UInt64.max, next: 2).matches(IOSHistoricalOriginalQuery(request, offset: UInt64.max)))
    }

    func testOriginalPagingFailurePreservesVisiblePageAndCanRetry() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let first = page(request)
        let second = page(request, offset: 3, next: 6, end: true)
        let state = IOSHistoricalOriginalState()
        await state.load(request, offset: 0) { _ in first }
        await state.load(request, offset: 3) { _ in throw CocoaError(.fileReadUnknown) }
        XCTAssertEqual(state.page, first)
        XCTAssertNotNil(state.error)
        XCTAssertTrue(state.previousOffsets.isEmpty)
        await state.load(request, offset: 3) { _ in second }
        XCTAssertEqual(state.page, second)
        XCTAssertEqual(state.previousOffsets, [0])
        XCTAssertNil(state.error)
        await state.load(request, offset: 0) { _ in first }
        XCTAssertEqual(state.page, first)
        XCTAssertTrue(state.previousOffsets.isEmpty)
    }

    func testOriginalReadRejectsChangedRevisionWithoutReplacingVisiblePage() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let first = page(request)
        let stale = page(request, offset: 3, next: 6, revision: 2, end: true)
        let state = IOSHistoricalOriginalState()
        await state.load(request, offset: 0) { _ in first }
        await state.load(request, offset: 3) { _ in stale }
        XCTAssertEqual(state.page, first)
        XCTAssertNotNil(state.error)
        XCTAssertTrue(state.previousOffsets.isEmpty)
    }

    func testOriginalQueryUsesRevisionPinnedBoundedPublicContract() throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let data = try JSONEncoder().encode(IOSHistoricalOriginalQuery(request, offset: 3))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(object["type"] as? String, "read_conversation_export_range")
        XCTAssertEqual(object["field"] as? String, "content_json")
        XCTAssertEqual(object["expected_revision"] as? Int, 1)
        XCTAssertEqual(object["limit_bytes"] as? Int, 8192)
        XCTAssertEqual(object["offset_bytes"] as? Int, 3)
        XCTAssertNil(object["path"])
    }

    func testOriginalCancelledReadCannotPublish() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let value = page(request)
        let gate = OriginalReadGate()
        let state = IOSHistoricalOriginalState()
        let task = Task { await state.load(request, offset: 0) { _ in await gate.wait() } }
        await gate.waitUntilStarted()
        task.cancel()
        await gate.finish(value)
        await task.value
        XCTAssertNil(state.page)
        XCTAssertNil(state.error)
        XCTAssertFalse(state.loading)
    }

    func testOriginalLateDifferentRequestCannotReplaceNewResult() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let differentProjection = try JSONDecoder().decode(IOSConversationProjectionV1.HistoricalProjection.self,
            from: Data(#"{"owner":{"conversation_id":"other","message_id":"m","part_id":"p"},"canonical_revision":1,"canonical_kind":"text","status":{"state":"not_loaded"},"fields":{}}"#.utf8))
        let different = try XCTUnwrap(IOSHistoricalPartRequest(differentProjection))
        let first = page(request)
        let current = page(different)
        let gate = OriginalReadGate()
        let state = IOSHistoricalOriginalState()
        let task = Task { await state.load(request, offset: 0) { _ in await gate.wait() } }
        await gate.waitUntilStarted()
        await state.load(different, offset: 0) { _ in current }
        await gate.finish(first)
        await task.value
        XCTAssertEqual(state.page, current)
        XCTAssertNil(state.error)
    }
}

private actor OriginalReadGate {
    private var continuation: CheckedContinuation<IOSHistoricalOriginalPage, Never>?
    func wait() async -> IOSHistoricalOriginalPage {
        await withCheckedContinuation { continuation = $0 }
    }
    func waitUntilStarted() async {
        while continuation == nil { await Task.yield() }
    }
    func finish(_ value: IOSHistoricalOriginalPage) {
        continuation?.resume(returning: value)
        continuation = nil
    }
}

private actor HistoricalReadGate {
    private var continuation: CheckedContinuation<IOSConversationProjectionV1.HistoricalProjection?, Never>?
    func wait() async -> IOSConversationProjectionV1.HistoricalProjection? {
        await withCheckedContinuation { continuation = $0 }
    }
    func waitUntilStarted() async {
        while continuation == nil { await Task.yield() }
    }
    func finish(_ value: IOSConversationProjectionV1.HistoricalProjection) {
        continuation?.resume(returning: value)
        continuation = nil
    }
}
