// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSRoomSubmissionWorkspaceTests: XCTestCase {
    func testRemoteIntentReferencesExactBoundRequestWithoutNativeRecapture() throws {
        let identity = IOSRemoteSubmissionIdentity(source_host_id: "paired-host", client_id: "paired-client",
            conversation_id: "private-context", submission_id: "original-execution",
            user_message_id: "original-user", assistant_message_id: "original-assistant")
        let original = IOSAgentRoomService.ExecutionIntent(execution_id: identity.submission_id,
            envelope: .remoteHost(.init(kind: "remote_host", identity: identity, request_digest: String(repeating: "a", count: 64))))
        let encoded = try JSONEncoder().encode(original)
        let restored = try JSONDecoder().decode(IOSAgentRoomService.ExecutionIntent.self, from: encoded)
        XCTAssertEqual(restored, original)
        XCTAssertNil(restored.envelope.native)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        let envelope = try XCTUnwrap(object["envelope"] as? [String: Any])
        XCTAssertEqual(Set(envelope.keys), ["kind", "identity", "request_digest"])
    }
    func testNativeIntentRoundTripRetainsOriginalIdentityAndFrozenConfiguration() throws {
        let original = IOSAgentRoomService.ExecutionIntent(execution_id: UUID().uuidString.lowercased(),
            envelope: .init(kind: "native", platform: "ios",
                conversation_configuration_revision: 7, model_profile_id: "profile", model_profile_revision: 11,
                expected_parent_message_id: "parent", text: "继续分析 current architecture"))
        let restored = try JSONDecoder().decode(IOSAgentRoomService.ExecutionIntent.self,
            from: JSONEncoder().encode(original))
        XCTAssertEqual(restored, original)
        XCTAssertEqual(restored.envelope.native?.expected_parent_message_id, "parent")
    }

    func testExecutionLookupNeverAcceptsAnotherOwner() throws {
        let service = IOSAgentRoomService(execute: { _ in
            .object(["conversation_id": .string("foreign"), "run_id": .string("run"),
                     "assistant_message_id": .string("run:assistant")])
        })
        XCTAssertThrowsError(try service.executionExists(contextID: "owner", executionID: "run"))
        let absent = IOSAgentRoomService(execute: { _ in .null })
        XCTAssertFalse(try absent.executionExists(contextID: "owner", executionID: "run"))
    }
    func testRoutingRejectsInventedIdentityAndExtraInstructions() throws {
        XCTAssertEqual(try IOSRoomRoutingService.validatedParticipant(
            .object(["participant_id": .string("成员-2")]), candidates: ["成员-1", "成员-2"]), "成员-2")
        XCTAssertThrowsError(try IOSRoomRoutingService.validatedParticipant(
            .object(["participant_id": .string("search")]), candidates: ["成员-1"]))
        XCTAssertThrowsError(try IOSRoomRoutingService.validatedParticipant(
            .object(["participant_id": .string("成员-1"), "tool": .string("web")]), candidates: ["成员-1"]))
    }
    func testExecutionClaimKeepsCanonicalIdentityAndRefusesReplay() throws {
        let executionID = UUID().uuidString.lowercased()
        let input = IOSRoomSubmissionInput(roomID: UUID().uuidString, submissionID: UUID().uuidString,
            expectedRoomRevision: 2, text: "请分析", participantID: "member", atMs: 1)
        let inputValue = try JSONDecoder().decode(AgentCoreJSONValue.self,
            from: JSONEncoder().encode(input))
        let value: AgentCoreJSONValue = .object([
            "revision": .signedInteger(3), "input": inputValue, "state": .string("executing"),
            "selected_participant_id": .string("member"), "context_conversation_id": .string("context"),
            "execution_id": .string(executionID)])
        let receipt = try JSONDecoder().decode(IOSRoomSubmission.self, from: JSONEncoder().encode(value))
        let service = IOSAgentRoomService(execute: { envelope in
            guard case .object(let root) = envelope, case .object(let command) = root["command"] else {
                throw CocoaError(.coderInvalidValue)
            }
            XCTAssertEqual(command["type"], .string("claim_execution"))
            XCTAssertEqual(command["expected_revision"], .signedInteger(3))
            return .object(["submission": value, "dispatch_permitted": .bool(false)])
        })
        let claim = try service.claimExecution(receipt)
        XCTAssertFalse(claim.dispatchPermitted)
        XCTAssertEqual(claim.submission.executionID, executionID)
        XCTAssertEqual(claim.submission.input, input)
    }
    func testMentionRequiresExplicitSelectionAndDoesNotTreatEmailAsRecipient() {
        var draft = IOSRoomDraft(text: "Contact someone@example.com")
        XCTAssertNil(draft.mentionQuery)
        XCTAssertNil(draft.participantID)
        draft.text = "请分析 @研究"
        XCTAssertEqual(draft.mentionQuery, "研究")
        XCTAssertNil(draft.participantID)
        draft.mention("stable-member")
        XCTAssertEqual(draft.participantID, "stable-member")
        XCTAssertEqual(draft.text, "请分析 ")
    }

    func testLateAcknowledgementCannotClearNewRoomDraft() {
        var draft = IOSRoomDraft(text: "first", participantID: "member", revision: 3)
        draft.text = "next"; draft.revision = 4
        draft.acknowledge(revision: 3)
        XCTAssertEqual(draft.text, "next")
        XCTAssertEqual(draft.participantID, "member")
        draft.acknowledge(revision: 4)
        XCTAssertEqual(draft.text, "")
        XCTAssertNil(draft.participantID)
    }

    func testSubmissionSendsStableParticipantIdentityAndReusesReceiptID() throws {
        let input = IOSRoomSubmissionInput(roomID: UUID().uuidString, submissionID: UUID().uuidString,
            expectedRoomRevision: 7, text: "search current architecture", participantID: nil, atMs: 1)
        let service = IOSAgentRoomService(execute: { envelope in
            guard case .object(let root) = envelope, case .object(let command) = root["command"],
                  let value = command["input"] else { throw CocoaError(.coderInvalidValue) }
            XCTAssertEqual(root["type"], .string("agent_room"))
            XCTAssertEqual(command["type"], .string("submit"))
            return .object(["revision": .signedInteger(1), "input": value, "state": .string("accepted"),
                            "context_conversation_id": .null, "selected_participant_id": .null])
        })
        let first = try service.submit(input)
        let retry = try service.submit(input)
        XCTAssertEqual(first.id, retry.id)
        XCTAssertEqual(first.input, input)
        XCTAssertNil(first.contextConversationID)
    }
}
