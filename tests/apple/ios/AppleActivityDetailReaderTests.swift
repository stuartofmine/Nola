// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class AppleActivityDetailReaderTests: XCTestCase {
    func testLateResultAndLateErrorCannotReplaceAnotherActivity() async throws {
        for fails in [false, true] {
            let reader = AppleActivityDetailReader()
            let a = try entry("a"), b = try entry("b")
            var pending: CheckedContinuation<AppleActivityDetail?, Error>?
            let old = Task {
                await reader.load(a) { _ in try await withCheckedThrowingContinuation { pending = $0 } }
            }
            for _ in 0..<100 where pending == nil { await Task.yield() }
            let continuation = try XCTUnwrap(pending)
            await reader.load(b) { _ in try self.detail(b, text: "B result") }
            if fails { continuation.resume(throwing: FixtureFailure.failed) }
            else { continuation.resume(returning: try detail(a, text: "late A result")) }
            await old.value
            XCTAssertNil(reader.value(for: a))
            XCTAssertEqual(reader.value(for: b)?.result_text, "B result")
            XCTAssertNil(reader.error(for: b))
            XCTAssertFalse(reader.isLoading(b))
        }
    }

    func testRefreshFailurePreservesOnlyTheSameOwnersLastSuccessfulResult() async throws {
        let reader = AppleActivityDetailReader(), a = try entry("a"), b = try entry("b")
        await reader.load(a) { _ in try self.detail(a, text: "saved A result") }
        await reader.load(a) { _ in throw FixtureFailure.failed }
        XCTAssertEqual(reader.value(for: a)?.result_text, "saved A result")
        XCTAssertNotNil(reader.error(for: a))
        // Even before B's asynchronous task starts, its view cannot read A.
        XCTAssertNil(reader.value(for: b))
        XCTAssertNil(reader.error(for: b))
        XCTAssertTrue(reader.isLoading(b))
    }

    func testDetailMustMatchAllThreeCanonicalOwnerFields() async throws {
        let a = try entry("a")
        for key in ["run_id", "conversation_id", "assistant_message_id"] {
            let reader = AppleActivityDetailReader()
            var payload = fields(a)
            payload[key] = "foreign"
            let foreign = try JSONDecoder().decode(AppleActivityEntry.self, from: JSONSerialization.data(withJSONObject: payload))
            await reader.load(a) { _ in try self.detail(foreign, text: "must not be shown") }
            XCTAssertNil(reader.value(for: a), key)
            XCTAssertNotNil(reader.error(for: a), key)
        }
    }

    private enum FixtureFailure: Error { case failed }
    private func entry(_ id: String) throws -> AppleActivityEntry {
        try JSONDecoder().decode(AppleActivityEntry.self, from: JSONSerialization.data(withJSONObject: [
            "run_id": id, "conversation_id": "conversation-" + id, "assistant_message_id": "assistant-" + id,
            "source": "agent", "title": id, "created_at_ms": 1, "completed_at_ms": 2,
            "status": "completed", "needs_response": false, "is_current": true]))
    }
    private func detail(_ entry: AppleActivityEntry, text: String) throws -> AppleActivityDetail {
        return try JSONDecoder().decode(AppleActivityDetail.self, from: JSONSerialization.data(withJSONObject: [
            "entry": fields(entry), "result_text": text, "result_has_more": false,
            "source_conversation_id": entry.conversation_id, "review_in_source": false,
            "context_scope": "conversation"]))
    }
    private func fields(_ entry: AppleActivityEntry) -> [String: Any] {
        ["run_id": entry.run_id, "conversation_id": entry.conversation_id, "assistant_message_id": entry.assistant_message_id,
         "source": entry.source, "title": entry.title, "created_at_ms": entry.created_at_ms,
         "completed_at_ms": entry.completed_at_ms.map { $0 as Any } ?? NSNull(), "status": entry.status,
         "needs_response": entry.needs_response, "is_current": entry.is_current]
    }
}
