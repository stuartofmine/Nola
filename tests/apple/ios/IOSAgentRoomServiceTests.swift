// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSAgentRoomServiceTests: XCTestCase {
    func testJoinUsesCapturedAgentAndRoomRevisions() throws {
        let fixture = #"{"revision":3,"room":{"id":"room","title":"Discussion","participants":[]}}"#
        let response = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(fixture.utf8))
        let room = try JSONDecoder().decode(IOSAgentRoomSnapshot.self, from: Data(fixture.utf8))
        let agent = IOSPersistentAgent(id: "a", name: "A", description: "", revision: 7, availability: "active", defaults: .object([:]))
        let service = IOSAgentRoomService(execute: { command in
            guard case .object(let envelope) = command,
                  case .object(let body) = envelope["command"],
                  case .object(let input) = body["input"] else {
                throw NSError(domain: "Invalid command", code: 1)
            }
            XCTAssertEqual(envelope["type"], .string("agent_room"))
            XCTAssertEqual(body["type"], .string("join"))
            XCTAssertEqual(input["room_id"], .string("room"))
            XCTAssertEqual(input["agent_id"], .string("a"))
            XCTAssertEqual(input["expected_room_revision"], .signedInteger(3))
            XCTAssertEqual(input["expected_agent_revision"], .signedInteger(7))
            if case .signedInteger(let timestamp) = input["at_ms"] {
                XCTAssertGreaterThan(timestamp, 0)
            } else { XCTFail("Missing integer timestamp") }
            return response
        })
        XCTAssertEqual(try service.join(agent: agent, room: room), room)
    }

    func testAbsentRoomIsNotRebuiltOrReplacedWithDefault() throws {
        let service = IOSAgentRoomService(execute: { _ in .null })
        XCTAssertNil(try service.load(id: "missing"))
    }

    func testMutationFailureIsNotSilentlyReportedAsSuccess() {
        let service = IOSAgentRoomService(execute: { _ in
            throw NSError(domain: "room_revision_conflict", code: 1)
        })
        XCTAssertThrowsError(try service.create(title: "Discussion"))
    }
}


extension IOSAgentRoomServiceTests {
    func testOrdinaryConversationReadDoesNotCreateOrExecute() throws {
        let service = IOSAgentRoomService(execute: { envelope in
            guard case .object(let root) = envelope,
                  case .object(let body) = root["command"] else { throw CocoaError(.coderInvalidValue) }
            XCTAssertEqual(root["type"], .string("agent_room"))
            XCTAssertEqual(body["type"], .string("for_conversation"))
            XCTAssertEqual(body["conversation_id"], .string("chat"))
            return .null
        })
        XCTAssertNil(try service.forConversation("chat"))
    }

    func testExpertAnchorRejectsHalfPairBeforeDispatch() throws {
        let service = IOSAgentRoomService(execute: { _ in
            XCTFail("Invalid ownership must not dispatch")
            throw CocoaError(.coderInvalidValue)
        })
        var input = IOSRoomSubmissionInput(roomID: "room", submissionID: "stable-retry",
            expectedRoomRevision: 1, text: "question", participantID: "expert", atMs: 1)
        input.parentMessageID = "user-message"
        XCTAssertThrowsError(try service.submit(input))
        input.conversationID = "chat"
        let restored = try JSONDecoder().decode(IOSRoomSubmissionInput.self, from: JSONEncoder().encode(input))
        XCTAssertEqual(restored, input)
        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(input)) as! [String: Any]
        XCTAssertEqual(object["parent_message_id"] as? String, "user-message")
        XCTAssertEqual(object["conversation_id"] as? String, "chat")
        XCTAssertEqual(object["submission_id"] as? String, "stable-retry")
    }

    func testConversationAssociationRejectsForeignOwner() throws {
        let value = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(#"{"conversation_id":"foreign","room_id":"room","revision":1,"room":{"revision":1,"room":{"id":"room","title":"","participants":[]}}}"#.utf8))
        let service = IOSAgentRoomService(execute: { _ in value })
        XCTAssertThrowsError(try service.forConversation("chat"))
        XCTAssertThrowsError(try service.ensureForConversation("chat"))
    }
}


extension IOSAgentRoomServiceTests {
    func testFrozenAdmissionReloadPreservesRetryIdentityWithoutDispatch() throws {
        let input = IOSAgentRoomService.UserMessageAdmission(conversationID: "chat", messageID: "stable", text: "Original", createdAtMs: 42)
        let draft = IOSAgentRoomService.ExpertAdmissionDraft(conversationID: "chat", input: input, expertIDs: ["expert"])
        let data = try JSONEncoder().encode(draft)
        let setting = IOSCanonicalProductSetting(key: "apple.chat.expert-admission/chat",
            valueJSON: String(decoding: data, as: UTF8.self), revision: 3, updatedAtMs: 42)
        let value = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(setting))
        let service = IOSAgentRoomService(execute: { envelope in
            guard case .object(let fields) = envelope else { throw CocoaError(.coderInvalidValue) }
            XCTAssertEqual(fields["type"], .string("get_product_setting"))
            XCTAssertEqual(fields["key"], .string("apple.chat.expert-admission/chat"))
            return value
        })
        let restored = try XCTUnwrap(service.loadExpertAdmission(conversationID: "chat"))
        XCTAssertEqual(restored.0.input?.messageID, "stable")
        XCTAssertEqual(restored.0.input?.text, "Original")
        XCTAssertEqual(restored.0.input?.createdAtMs, 42)
        XCTAssertEqual(restored.0.expertIDs, ["expert"])
        XCTAssertEqual(restored.1, 3)
    }
}


extension IOSAgentRoomServiceTests {
    @MainActor
    func testReopeningPendingExpertMessageDoesNotAdmitOrExecute() async throws {
        let input = IOSAgentRoomService.UserMessageAdmission(conversationID: "chat", messageID: "stable", text: "Original", createdAtMs: 42)
        let draft = IOSAgentRoomService.ExpertAdmissionDraft(conversationID: "chat", input: input, expertIDs: ["expert"])
        let setting = IOSCanonicalProductSetting(key: "apple.chat.expert-admission/chat",
            valueJSON: String(decoding: try JSONEncoder().encode(draft), as: UTF8.self), revision: 3, updatedAtMs: 42)
        let value = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(setting))
        let service = IOSAgentRoomService(execute: { envelope in
            guard case .object(let fields) = envelope else { throw CocoaError(.coderInvalidValue) }
            if fields["type"] == .string("get_product_setting") { return value }
            guard fields["type"] == .string("agent_room"), case .object(let command) = fields["command"],
                  command["type"] == .string("for_conversation") else {
                XCTFail("Reopening must only read state")
                throw CocoaError(.coderInvalidValue)
            }
            return .null
        })
        let workspace = IOSRoomSubmissionWorkspace(service: service)
        await workspace.restoreConversation("chat")
        XCTAssertEqual(workspace.pendingExpertIDs, ["expert"])
        XCTAssertNotNil(workspace.admissionError)
        XCTAssertFalse(workspace.isAdmitting)
    }
}
