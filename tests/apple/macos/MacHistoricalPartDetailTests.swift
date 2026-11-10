// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacHistoricalPartDetailTests: XCTestCase {
    private func selection(_ conversation: String = "c", sequence: UInt64 = 1) -> MacHistoricalPartSelection {
        .init(owner: .init(conversationID: conversation, messageID: "m", partID: "p"),
              canonicalRevision: 2, windowSequence: sequence)
    }
    private func data(_ selection: MacHistoricalPartSelection, state: String = "supported") throws -> Data {
        try JSONSerialization.data(withJSONObject: [
            "owner": ["conversation_id": selection.owner.conversationID,
                      "message_id": selection.owner.messageID, "part_id": selection.owner.partID],
            "canonical_revision": selection.canonicalRevision, "canonical_kind": "text",
            "schema": "message_part_view", "binding": "canonical",
            "status": ["state": state], "fields": ["markdown": ["availability": "complete", "value": "original 中😀"]]
        ])
    }
    func testPortRejectsWrongOwnerRevisionAndOversizedResponse() throws {
        let expected = selection()
        let raw = try data(expected)
        XCTAssertNotNil(try MacApplicationPort.decodeHistoricalPart(raw, owner: expected.owner, revision: 2))
        XCTAssertThrowsError(try MacApplicationPort.decodeHistoricalPart(raw, owner: selection("other").owner, revision: 2))
        XCTAssertThrowsError(try MacApplicationPort.decodeHistoricalPart(raw, owner: expected.owner, revision: 3))
        XCTAssertThrowsError(try MacApplicationPort.decodeHistoricalPart(Data(repeating: 32, count: 32769), owner: expected.owner, revision: 2))
    }
    func testLateDifferentConversationOrWindowResponseNeverOverwritesSelection() async throws {
        let gate = DetailGate()
        let model = MacHistoricalPartDetailModel(reader: { await gate.read($0) })
        let first = selection()
        let second = selection("other", sequence: 5)
        let firstValue = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(first))
        let secondValue = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(second))
        let old = Task { await model.load(first) }
        await gate.waitForCount(1)
        model.invalidate()
        let new = Task { await model.load(second) }
        await gate.waitForCount(2)
        await gate.complete(1, value: secondValue)
        await new.value
        await gate.complete(0, value: firstValue)
        await old.value
        XCTAssertEqual(model.selection, second)
        XCTAssertEqual(model.state, .loaded(secondValue))
        // Even a same-owner window cursor update invalidates an outstanding read.
        let pending = Task { await model.load(second) }
        await gate.waitForCount(3)
        model.invalidate()
        await gate.complete(2, value: secondValue)
        await pending.value
        XCTAssertNil(model.selection)
        XCTAssertEqual(model.state, .idle)
    }
    func testDeferredAndNotLoadedStayExplicitAndWrongOwnerFails() async throws {
        let expected = selection()
        for state in ["deferred", "not_loaded"] {
            let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(expected, state: state))
            let model = MacHistoricalPartDetailModel(reader: { _ in projection })
            await model.load(expected)
            XCTAssertEqual(model.state, .loaded(projection))
        }
        let wrong = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(selection("other")))
        let model = MacHistoricalPartDetailModel(reader: { _ in wrong })
        await model.load(expected)
        XCTAssertEqual(model.state, .failed)
    }

    func testUnsupportedLegacyTextUsesCanonicalPreviewForChatDisplay() throws {
        let expected = selection()
        var object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data(expected)) as? [String: Any]
        )
        object["status"] = [
            "state": "unsupported",
            "code": "schema_missingfield",
        ]
        object["canonical_text"] = [
            "text": "背诵出师表",
            "total_utf8_bytes": 15,
        ]
        let projection = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.HistoricalProjection.self,
            from: JSONSerialization.data(withJSONObject: object)
        )

        XCTAssertNil(projection.decodedText)
        XCTAssertEqual(projection.visibleText, "背诵出师表")
        XCTAssertTrue(projection.statusLabel.contains("schema_missingfield"))
    }
    func testRealWindowSelectionQueriesUnloadedPartWithoutChangingStream() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("history.db"))
        _ = try port.createConversation(.init(id: "c", title: "History", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1))
        let parts = try (0..<64).map { index -> MacCanonicalMessagePartInput in
            let id = "p\(index)"
            let raw = try JSONSerialization.data(withJSONObject: ["kind": "text", "part_id": id,
                "revision": 1, "markdown": String(repeating: "原", count: 80), "is_final": true])
            return .init(id: id, kind: "text", ordinal: Int64(index + 1), state: "completed",
                contentText: "", contentJSON: String(decoding: raw, as: UTF8.self), contentDigest: nil,
                createdAtMs: 1, updatedAtMs: 1)
        }
        _ = try port.importConversationTranscript(.init(importID: "import", source: "external_archive", conversationID: "c",
            expectedMessageCount: 0, messages: [.init(id: "m", runID: nil, role: "assistant", status: "completed",
                ordinal: 1, createdAtMs: 1, completedAtMs: 1, parts: parts)], importedAtMs: 2))
        let before = try port.loadConversationWindow("c")
        let projected = try MacCanonicalConversationRepository.projectMessages(before)
        let descriptor = try XCTUnwrap(projected.first?.historicalDetails?.last)
        XCTAssertEqual(descriptor.windowSequence, before.eventSequence)
        guard case .historical(let last) = before.projection.messages[0].parts.last else {
            return XCTFail("missing historical Part")
        }
        XCTAssertEqual(last.projection.status.state, .notLoaded)
        let model = MacHistoricalPartDetailModel(reader: { selection in
            try port.readHistoricalPart(selection.owner, expectedConversationRevision: selection.canonicalRevision)
        })
        await model.load(descriptor)
        guard case .loaded(let value) = model.state else { return XCTFail("detail unavailable") }
        XCTAssertEqual(value.status.state, .supported)
        XCTAssertEqual(value.decodedText, String(repeating: "原", count: 80))
        let after = try port.loadConversationWindow("c")
        XCTAssertEqual(after.eventSequence, before.eventSequence)
        XCTAssertEqual(after.projection, before.projection)
        XCTAssertEqual(try MacCanonicalConversationRepository.projectMessages(after), projected)
    }
    func testActualStoreHistoricalPublicReaderNeverRevealsPendingOrRejectedTextFallback() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("historical-review-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("review.db"))
        _ = try port.createConversation(.init(id: "review", title: "Review", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1))
        let reviews = ["pending", "rejected", "approved"]
        let parts = try reviews.enumerated().map { index, review -> MacCanonicalMessagePartInput in
            let marker = "HISTORICAL-\(review)-PRIVATE-OR-PUBLIC"
            let raw = try JSONSerialization.data(withJSONObject: ["kind":"text", "part_id":review,
                "revision":1, "markdown":marker, "is_final":true, "review":review])
            return .init(id:review, kind:"text", ordinal:Int64(index + 1), state:"completed",
                contentText:marker, contentJSON:String(decoding:raw,as:UTF8.self), contentDigest:nil,
                createdAtMs:1, updatedAtMs:1)
        }
        _ = try port.importConversationTranscript(.init(importID:"review-import", source:"external_archive", conversationID:"review",
            expectedMessageCount:0, messages:[.init(id:"message", runID:nil, role:"assistant", status:"completed",
                ordinal:1, createdAtMs:1, completedAtMs:1, parts:parts)], importedAtMs:2))
        let before = try port.loadConversationWindow("review")
        let revision = try XCTUnwrap(Int64(exactly:before.projection.revision))
        for review in reviews {
            let owner = MacCanonicalPartOwner(conversationID:"review", messageID:"message", partID:review)
            let projection = try XCTUnwrap(port.readHistoricalPart(owner, expectedConversationRevision:revision))
            if review == "approved" {
                XCTAssertEqual(projection.visibleText, "HISTORICAL-approved-PRIVATE-OR-PUBLIC")
            } else {
                XCTAssertNil(projection.decodedText)
                XCTAssertNil(projection.canonicalText)
                XCTAssertNil(projection.visibleText, "Neither structured text nor canonical fallback may disclose an unapproved Part")
            }
            XCTAssertThrowsError(try port.readHistoricalPart(owner, expectedConversationRevision:revision + 1))
        }
        let after = try port.loadConversationWindow("review")
        XCTAssertEqual(after.projection,before.projection)
        XCTAssertEqual(after.eventSequence,before.eventSequence)
    }

    func testActualStoreHistoricalTaskPreservesSealedAsDistinctFromCompleted() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("sealed-history-\(UUID())")
        try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        defer { try? FileManager.default.removeItem(at:directory) }
        let port=try MacApplicationPort(databaseURL:directory.appendingPathComponent("sealed.db"))
        _=try port.createConversation(.init(id:"sealed",title:"Sealed",settingsJSON:nil,createdAtMs:1,updatedAtMs:1))
        let raw=try JSONSerialization.data(withJSONObject:["kind":"task","part_id":"task-part","revision":1,
            "task_id":"task","display_key":"Task","state":"sealed","cancel_requested":false])
        _=try port.importConversationTranscript(.init(importID:"sealed-import",source:"external_archive",conversationID:"sealed",
            expectedMessageCount:0,messages:[.init(id:"message",runID:nil,role:"assistant",status:"completed",ordinal:1,
                createdAtMs:1,completedAtMs:1,parts:[.init(id:"task-part",kind:"task",ordinal:1,state:"completed",
                    contentText:"",contentJSON:String(decoding:raw,as:UTF8.self),contentDigest:nil,createdAtMs:1,updatedAtMs:1)])],importedAtMs:2))
        let window=try port.loadConversationWindow("sealed")
        let owner=MacCanonicalPartOwner(conversationID:"sealed",messageID:"message",partID:"task-part")
        let projection=try XCTUnwrap(port.readHistoricalPart(owner,expectedConversationRevision:try XCTUnwrap(Int64(exactly:window.projection.revision))))
        guard case .complete(.string(let state))?=projection.fields["state"] else { return XCTFail("Actual Store task state") }
        XCTAssertEqual(state,"sealed")
        XCTAssertNotEqual(state,"completed")
    }

}

private actor DetailGate {
    typealias Projection = MacCanonicalConversationProjectionV1.HistoricalProjection
    private var requests: [CheckedContinuation<Projection?, Never>] = []
    private var waiter: (Int, CheckedContinuation<Void, Never>)?
    func read(_ selection: MacHistoricalPartSelection) async -> Projection? {
        await withCheckedContinuation { continuation in
            requests.append(continuation)
            if let (count, continuation) = waiter, requests.count >= count {
                waiter = nil
                continuation.resume()
            }
        }
    }
    func waitForCount(_ count: Int) async {
        if requests.count >= count { return }
        await withCheckedContinuation { waiter = (count, $0) }
    }
    func complete(_ index: Int, value: Projection) { requests[index].resume(returning: value) }
}
