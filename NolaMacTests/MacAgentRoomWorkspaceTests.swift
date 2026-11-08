// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacAgentRoomWorkspaceTests: XCTestCase {
    func testRemoteIntentReferencesExactBoundRequestWithoutNativeRecapture() throws {
        let identity = MacRemoteSubmissionIdentity(source_host_id: "paired-host", client_id: "paired-client",
            conversation_id: "private-context", submission_id: "original-execution",
            user_message_id: "original-user", assistant_message_id: "original-assistant")
        let original = MacAgentRoomService.ExecutionIntent(execution_id: identity.submission_id,
            envelope: .remoteHost(.init(kind: "remote_host", identity: identity, request_digest: String(repeating: "a", count: 64))))
        let encoded = try JSONEncoder().encode(original)
        let restored = try JSONDecoder().decode(MacAgentRoomService.ExecutionIntent.self, from: encoded)
        XCTAssertEqual(restored, original)
        XCTAssertNil(restored.envelope.native)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        let envelope = try XCTUnwrap(object["envelope"] as? [String: Any])
        XCTAssertEqual(Set(envelope.keys), ["kind", "identity", "request_digest"])
    }
    func testProductionExpertProjectionAnchorsLateChildrenAfterTheirExactParentInCoreOrder() {
        let first = transcriptMessage("parent-a", role: .user, time: 10)
        let next = transcriptMessage("parent-b", role: .user, time: 1)
        let lateA = transcriptMessage("run-a:assistant", time: 500)
        let lateB = transcriptMessage("run-b:assistant", time: 300)
        let rows = MacExpertTranscriptProjector.rows(conversationID: "chat", parentMessages: [first, next],
            association: transcriptAssociation(), receipts: [transcriptReceipt(peer: "b"), transcriptReceipt(peer: "a")],
            contexts: ["context-b": [lateB], "context-a": [lateA]])
        XCTAssertEqual(rows.map { $0.message.canonicalMessageID },
            ["parent-a", "run-a:assistant", "run-b:assistant", "parent-b"])
        XCTAssertEqual(rows.compactMap(\.name), ["Expert a", "Expert b"],
            "Core association peer order is stable regardless of arrival and timestamps")
    }

    func testProductionExpertProjectionScopesEqualUUIDsAndKeepsIdentityAcrossStreaming() {
        let sharedUIID = UUID()
        let parent = transcriptMessage("parent-a", role: .user)
        var a = transcriptMessage("run-a:assistant", id: sharedUIID)
        let b = transcriptMessage("run-b:assistant", id: sharedUIID)
        func project(_ a: MacChatMessage) -> [MacRoomTranscriptRow] {
            MacExpertTranscriptProjector.rows(conversationID: "chat", parentMessages: [parent],
                association: transcriptAssociation(), receipts: [transcriptReceipt(peer: "b"), transcriptReceipt(peer: "a")],
                contexts: ["context-b": [b], "context-a": [a]])
        }
        let rows = project(a)
        XCTAssertEqual(rows.count, 3)
        XCTAssertEqual(Set(rows.map(\.id)).count, 3, "UI UUIDs do not establish cross-context ownership")
        a.text += " streamed"
        let updated = project(a)
        XCTAssertEqual(rows.map(\.id), updated.map(\.id))
        XCTAssertEqual(updated[1].message.text, "run-a:assistant streamed")
    }

    func testProductionExpertProjectionExcludesForeignOrAmbiguousOwnershipAndPartIDAliases() {
        var parent = transcriptMessage("parent-a", role: .user)
        parent.canonicalItemIDs = ["parent-a", "part-a"]
        let a = transcriptMessage("run-a:assistant")
        for receipt in [transcriptReceipt(peer: "a", room: "foreign"),
            transcriptReceipt(peer: "a", conversation: "foreign"),
            transcriptReceipt(peer: "a", parent: "foreign"),
            transcriptReceipt(peer: "a", parent: "part-a"),
            transcriptReceipt(peer: "a", context: "context-b"),
            transcriptReceipt(peer: "a", selected: "foreign")] {
            let rows = MacExpertTranscriptProjector.rows(conversationID: "chat", parentMessages: [parent],
                association: transcriptAssociation(), receipts: [receipt], contexts: ["context-a": [a], "context-b": [a]])
            XCTAssertEqual(rows.map(\.id), [.ordinary(conversationID: "chat", messageID: parent.id)])
        }
        var optimistic = parent
        optimistic.canonicalMessageID = nil
        XCTAssertEqual(MacExpertTranscriptProjector.rows(conversationID: "chat", parentMessages: [optimistic],
            association: transcriptAssociation(), receipts: [transcriptReceipt(peer: "a")],
            contexts: ["context-a": [a]]).count, 1, "No eager UI identity may guess a canonical parent")
        var partOnly = a
        partOnly.canonicalMessageID = "foreign-assistant"
        partOnly.canonicalItemIDs = ["foreign-assistant", "run-a:assistant"]
        XCTAssertEqual(MacExpertTranscriptProjector.rows(conversationID: "chat", parentMessages: [parent],
            association: transcriptAssociation(), receipts: [transcriptReceipt(peer: "a")],
            contexts: ["context-a": [partOnly]]).count, 1, "A child part ID cannot impersonate its assistant message ID")
        let duplicate = [transcriptReceipt(peer: "a"), transcriptReceipt(peer: "a")]
        XCTAssertEqual(MacExpertTranscriptProjector.rows(conversationID: "chat", parentMessages: [parent],
            association: transcriptAssociation(), receipts: duplicate, contexts: ["context-a": [a]]).count, 1)
    }

    func testCanonicalPresentationIdentityDoesNotEnterMessageArchiveWire() throws {
        let message = transcriptMessage("canonical-user", role: .user)
        let data = try JSONEncoder().encode(message)
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertNil(fields["canonicalMessageID"])
        let restored = try JSONDecoder().decode(MacChatMessage.self, from: data)
        XCTAssertNil(restored.canonicalMessageID, "Only the current Core projection supplies the exact message identity")
        XCTAssertEqual(restored.id, message.id)
    }

    private func transcriptMessage(_ canonical: String, role: MacChatMessage.Role = .assistant,
        id: UUID = UUID(), time: TimeInterval = 1) -> MacChatMessage {
        MacChatMessage(id: id, role: role, text: canonical, createdAt: Date(timeIntervalSince1970: time),
            canonicalItemIDs: [canonical], canonicalMessageID: canonical)
    }
    private func transcriptAssociation() -> MacConversationRoomAssociation {
        .init(conversationID: "chat", roomID: "room", revision: 1,
            room: .init(revision: 1, room: .init(id: "room", title: "", participants: ["a", "b"].map {
                .init(id: $0, agentID: "agent-" + $0, agentRevision: 1, displayName: "Expert " + $0,
                    description: "", contextConversationID: "context-" + $0, active: true)
            })))
    }
    private func transcriptReceipt(peer: String, room: String = "room", conversation: String = "chat",
        parent: String = "parent-a", context: String? = nil, selected: String? = nil) -> MacRoomSubmission {
        var input = MacRoomSubmissionInput(roomID: room, submissionID: "receipt-" + peer,
            expectedRoomRevision: 1, text: "Question", participantID: peer, atMs: 1)
        input.parentMessageID = parent; input.conversationID = conversation
        return .init(revision: 1, input: input, state: "executing", selectedParticipantID: selected ?? peer,
            failureCode: nil, executionID: "run-" + peer, terminal: nil,
            contextConversationID: context ?? "context-" + peer)
    }
    func testNativeIntentRoundTripRetainsOriginalIdentityAndFrozenConfiguration() throws {
        let original = MacAgentRoomService.ExecutionIntent(execution_id: UUID().uuidString.lowercased(),
            envelope: .init(kind: "native", platform: "macos",
                conversation_configuration_revision: 7, model_profile_id: "profile", model_profile_revision: 11,
                expected_parent_message_id: "parent", text: "继续分析 current architecture"))
        let restored = try JSONDecoder().decode(MacAgentRoomService.ExecutionIntent.self,
            from: JSONEncoder().encode(original))
        XCTAssertEqual(restored, original)
        XCTAssertEqual(restored.envelope.native?.expected_parent_message_id, "parent")
    }

    func testExecutionLookupNeverAcceptsAnotherOwner() throws {
        let service = MacAgentRoomService(execute: { _ in
            .object(["conversation_id": .string("foreign"), "run_id": .string("run"),
                     "assistant_message_id": .string("run:assistant")])
        })
        XCTAssertThrowsError(try service.executionExists(contextID: "owner", executionID: "run"))
        let absent = MacAgentRoomService(execute: { _ in .null })
        XCTAssertFalse(try absent.executionExists(contextID: "owner", executionID: "run"))
    }
    func testRoutingRejectsInventedIdentityAndExtraInstructions() throws {
        XCTAssertEqual(try MacRoomRoutingService.validatedParticipant(
            .object(["participant_id": .string("成员-2")]), candidates: ["成员-1", "成员-2"]), "成员-2")
        XCTAssertThrowsError(try MacRoomRoutingService.validatedParticipant(
            .object(["participant_id": .string("search")]), candidates: ["成员-1"]))
        XCTAssertThrowsError(try MacRoomRoutingService.validatedParticipant(
            .object(["participant_id": .string("成员-1"), "tool": .string("web")]), candidates: ["成员-1"]))
    }
    func testRoutingEndpointAndUsageRemainExplicit() throws {
        XCTAssertEqual(try MacRoomRoutingService.endpoint(URL(string: "https://example.test")!, suffix: "chat/completions").path,
            "/v1/chat/completions")
        XCTAssertEqual(try MacRoomRoutingService.endpoint(URL(string: "https://example.test/v1")!, suffix: "responses").path,
            "/v1/responses")
        let usage = MacRoomRoutingService.usage(.object(["prompt_tokens": .signedInteger(12), "completion_tokens": .signedInteger(3)]))
        guard case .object(let fields) = usage, case .object(let normalized)? = fields["normalized"] else {
            return XCTFail("Expected canonical usage")
        }
        XCTAssertEqual(fields["coverage"], .string("known"))
        XCTAssertEqual(normalized["cached_input_tokens"], .null)
    }
    func testExecutionClaimKeepsCanonicalIdentityAndRefusesReplay() throws {
        let executionID = UUID().uuidString.lowercased()
        let input = MacRoomSubmissionInput(roomID: UUID().uuidString, submissionID: UUID().uuidString,
            expectedRoomRevision: 2, text: "请分析", participantID: "member", atMs: 1)
        let inputValue = try JSONDecoder().decode(MacCanonicalJSONValue.self,
            from: JSONEncoder().encode(input))
        let value: MacCanonicalJSONValue = .object([
            "revision": .signedInteger(3), "input": inputValue, "state": .string("executing"),
            "selected_participant_id": .string("member"), "context_conversation_id": .string("context"),
            "execution_id": .string(executionID)])
        let receipt = try JSONDecoder().decode(MacRoomSubmission.self, from: JSONEncoder().encode(value))
        let service = MacAgentRoomService(execute: { envelope in
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
        var draft = MacRoomDraft(text: "Contact someone@example.com")
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
        var draft = MacRoomDraft(text: "first", participantID: "member", revision: 3)
        draft.text = "next"; draft.revision = 4
        draft.acknowledge(revision: 3)
        XCTAssertEqual(draft.text, "next")
        XCTAssertEqual(draft.participantID, "member")
        draft.acknowledge(revision: 4)
        XCTAssertEqual(draft.text, "")
        XCTAssertNil(draft.participantID)
    }

    func testSubmissionSendsStableParticipantIdentityAndReusesReceiptID() throws {
        let input = MacRoomSubmissionInput(roomID: UUID().uuidString, submissionID: UUID().uuidString,
            expectedRoomRevision: 7, text: "search current architecture", participantID: nil, atMs: 1)
        let service = MacAgentRoomService(execute: { envelope in
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


extension MacAgentRoomWorkspaceTests {
    func testOrdinaryConversationReadDoesNotCreateOrExecute() throws {
        let service = MacAgentRoomService(execute: { envelope in
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
        let service = MacAgentRoomService(execute: { _ in
            XCTFail("Invalid ownership must not dispatch")
            throw CocoaError(.coderInvalidValue)
        })
        var input = MacRoomSubmissionInput(roomID: "room", submissionID: "stable-retry",
            expectedRoomRevision: 1, text: "question", participantID: "expert", atMs: 1)
        input.parentMessageID = "user-message"
        XCTAssertThrowsError(try service.submit(input))
        input.conversationID = "chat"
        let restored = try JSONDecoder().decode(MacRoomSubmissionInput.self, from: JSONEncoder().encode(input))
        XCTAssertEqual(restored, input)
        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(input)) as! [String: Any]
        XCTAssertEqual(object["parent_message_id"] as? String, "user-message")
        XCTAssertEqual(object["conversation_id"] as? String, "chat")
        XCTAssertEqual(object["submission_id"] as? String, "stable-retry")
    }

    func testConversationAssociationRejectsForeignOwner() throws {
        let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(#"{"conversation_id":"foreign","room_id":"room","revision":1,"room":{"revision":1,"room":{"id":"room","title":"","participants":[]}}}"#.utf8))
        let service = MacAgentRoomService(execute: { _ in value })
        XCTAssertThrowsError(try service.forConversation("chat"))
        XCTAssertThrowsError(try service.ensureForConversation("chat"))
    }
}


extension MacAgentRoomWorkspaceTests {
    func testFrozenAdmissionReloadPreservesRetryIdentityWithoutDispatch() throws {
        let input = MacAgentRoomService.UserMessageAdmission(conversationID: "chat", messageID: "stable", text: "Original", createdAtMs: 42)
        let draft = MacAgentRoomService.ExpertAdmissionDraft(conversationID: "chat", input: input, expertIDs: ["expert"])
        let data = try JSONEncoder().encode(draft)
        let setting = MacCanonicalProductSetting(key: "apple.chat.expert-admission/chat",
            valueJSON: String(decoding: data, as: UTF8.self), revision: 3, updatedAtMs: 42)
        let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(setting))
        let service = MacAgentRoomService(execute: { envelope in
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


extension MacAgentRoomWorkspaceTests {
    @MainActor
    func testReopeningPendingExpertMessageDoesNotAdmitOrExecute() async throws {
        let input = MacAgentRoomService.UserMessageAdmission(conversationID: "chat", messageID: "stable", text: "Original", createdAtMs: 42)
        let draft = MacAgentRoomService.ExpertAdmissionDraft(conversationID: "chat", input: input, expertIDs: ["expert"])
        let setting = MacCanonicalProductSetting(key: "apple.chat.expert-admission/chat",
            valueJSON: String(decoding: try JSONEncoder().encode(draft), as: UTF8.self), revision: 3, updatedAtMs: 42)
        let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(setting))
        let service = MacAgentRoomService(execute: { envelope in
            guard case .object(let fields) = envelope else { throw CocoaError(.coderInvalidValue) }
            if fields["type"] == .string("get_product_setting") { return value }
            guard fields["type"] == .string("agent_room"), case .object(let command) = fields["command"],
                  command["type"] == .string("for_conversation") else {
                XCTFail("Reopening must only read state")
                throw CocoaError(.coderInvalidValue)
            }
            return .null
        })
        let workspace = MacAgentRoomWorkspace(service: service)
        await workspace.restoreConversation("chat")
        XCTAssertEqual(workspace.pendingExpertIDs, ["expert"])
        XCTAssertNotNil(workspace.admissionError)
        XCTAssertFalse(workspace.isAdmitting)
    }
}
