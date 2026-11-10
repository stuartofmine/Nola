// SPDX-License-Identifier: Apache-2.0
// Adapter contract tests, not model/Host execution acceptance tests.
import XCTest
@testable import Nola

final class IOSExpertMessageAdmissionContractTests: XCTestCase {
    func testCapturePreviewRequiresSameSnapshotEffectiveParameters() throws {
        var participant: [String: Any] = [
            "participant": ["participant_id": "p", "agent_id": "a", "agent_revision": 1,
                "context_conversation_id": "context"],
            "context_configuration_revision": 7, "defaults": [:], "client_model_presentation": NSNull(),
            "model_declaration": ["profile_id": "profile", "profile_revision": 3,
                "provider": "cloud", "model_name": "model", "runtime": "cloud", "parameters": [:]],
            "effective_parameters": ["status": "unavailable", "reason": "local_baseline_unavailable"],
        ]
        let decode = { (entry: [String: Any]) throws in
            try JSONDecoder().decode(AppleExpertCapturePreview<AgentCoreJSONValue>.self,
                from: JSONSerialization.data(withJSONObject: ["participants": [entry]]))
        }
        let preview = try decode(participant)
        XCTAssertEqual(preview.participants[0].effective_parameters,
            .object(["status": .string("unavailable"), "reason": .string("local_baseline_unavailable")]))
        participant.removeValue(forKey: "effective_parameters")
        XCTAssertThrowsError(try decode(participant))
    }

    private func input() -> AppleAdmitExpertConversationMessage {
        .init(conversation_id: "parent", expected_revision: 4, room_id: "11111111-1111-4111-8111-111111111111",
            expected_room_revision: 2, submission_id: "22222222-2222-4222-8222-222222222222",
            user_message_id: "user", text: "Compare the alternatives", selected_participants: [
                .init(participant_id: "a", agent_id: "expert-a", agent_revision: 3, context_conversation_id: "context-a"),
                .init(participant_id: "b", agent_id: "expert-b", agent_revision: 7, context_conversation_id: "context-b"),
            ], created_at_ms: 100)
    }
    private func manifest(_ input: AppleAdmitExpertConversationMessage, subset: Bool) throws -> AgentCoreJSONValue {
        let raw = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(input))
        let entries = (subset ? Array(input.selected_participants.prefix(1)) : input.selected_participants).enumerated().map { index, member -> AgentCoreJSONValue in
            let child = index == 0 ? "33333333-3333-4333-8333-333333333333" : "44444444-4444-4444-8444-444444444444"
            return .object([
                "participant": .object(["participant_id": .string(member.participant_id), "agent_id": .string(member.agent_id),
                    "agent_revision": .signedInteger(member.agent_revision), "context_conversation_id": .string(member.context_conversation_id)]),
                "display_name": .string(member.agent_id), "description": .string(""), "defaults": .object([:]),
                "model_declaration": .object(["profile_id": .string("profile"), "profile_revision": .signedInteger(1),
                    "provider": .string("cloud"), "model_name": .string("model"), "runtime": .string("cloud"), "parameters": .object([:])]),
                "context_configuration_revision": .signedInteger(2), "child_submission_id": .string(child),
                "child_user_message_id": .string(child + ":user"), "child_assistant_message_id": .string(child + ":assistant"),
                "expected_prompt_definition": .null, "model_witness": .null, "instruction_witness": .null,
            ])
        }
        return .object(["version": .signedInteger(1), "input": raw, "entries": .array(entries)])
    }
    func testCompleteCorePlanIsPassedAsOneCommandAndSubsetIsRejected() throws {
        let input = input(), response = try manifest(input, subset: false)
        let decoded = try JSONDecoder().decode(AppleExpertMessageManifest<AgentCoreJSONValue>.self, from: JSONEncoder().encode(response))
        let service = IOSAgentRoomService(execute: { _ in XCTFail("Expert admission must use the private Host factory"); return .null },
            admitExpert: { request in XCTAssertEqual(request, input); return decoded })
        XCTAssertEqual(try service.admitExperts(input).entries.count, 2)
        let incomplete = try manifest(input, subset: true)
        let partial = try JSONDecoder().decode(AppleExpertMessageManifest<AgentCoreJSONValue>.self, from: JSONEncoder().encode(incomplete))
        XCTAssertThrowsError(try IOSAgentRoomService(execute: { _ in .null }, admitExpert: { _ in partial }).admitExperts(input))
    }
    func testReadingOriginalManifestNeverAdmitsOrDispatches() throws {
        let input = input(), response = try manifest(input, subset: false)
        let service = IOSAgentRoomService(execute: { _ in XCTFail("Template-bearing reads require the private Host boundary"); return .null },
            expertHost: { operation in
                guard case .loadManifest(let conversationID, let messageID, let authority) = operation else { throw CocoaError(.coderInvalidValue) }
                XCTAssertEqual(conversationID, input.conversation_id); XCTAssertEqual(messageID, input.user_message_id)
                XCTAssertEqual(authority.actor, AgentCoreResourceAccess.localPrincipal()); XCTAssertNil(authority.request_host_id)
                return response
            })
        XCTAssertEqual(try service.expertManifest(conversationID: input.conversation_id,
            messageID: input.user_message_id)?.input, input)
    }

    func testCurrentSparseAndNullableProfileRetainsExplicitScalarsWithoutReadingSelection() throws {
        let sparse = try ModelParameters.decodeProfile(#"{"temperature":0,"maxContextLength":64000}"#)
        XCTAssertEqual(sparse.temperature, 0)
        XCTAssertEqual(sparse.maxContextLength, 64000)
        XCTAssertEqual(sparse.maxOutputTokens, ModelParameters.webAlignedCloudDefaults.maxOutputTokens)
        XCTAssertTrue(sparse.showThinking)
        var fixed = sparse; fixed.topK = 17; fixed.seed = 4
        let cleared = try ModelParameters.decodeProfile(#"{"temperature":null,"maxContextLength":null,"topK":null,"seed":null}"#, inheriting: fixed)
        XCTAssertEqual(cleared.temperature, sparse.temperature)
        XCTAssertEqual(cleared.maxContextLength, sparse.maxContextLength)
        XCTAssertNil(cleared.topK); XCTAssertNil(cleared.seed)
        XCTAssertThrowsError(try ModelParameters.decodeProfile(#"{"temperature":"invalid"}"#))
        let overridden = try ConversationParameterSnapshot.applyingOverrides(
            #"{"modelOverrides":{"configured:original":{"temperature":null,"topP":0.25,"seed":{"kind":"value","value":"0"}}}}"#,
            modelKey: "configured:original", to: fixed)
        XCTAssertEqual(overridden.temperature, fixed.temperature)
        XCTAssertEqual(overridden.topP, 0.25)
        XCTAssertEqual(overridden.maxContextLength, fixed.maxContextLength)
        XCTAssertEqual(overridden.seed, 0)
    }

    func testExpertChildUsesDurableRunAndUserIdentity() {
        let run = "33333333-3333-4333-8333-333333333333"
        let identity = IOSCanonicalAgentTurnIdentity(queuedSubmissionID: run, userMessageID: run + ":user")
        XCTAssertEqual(identity.submissionID, run)
        XCTAssertEqual(identity.userMessageID, run + ":user")
        XCTAssertEqual(identity.assistantMessageID, run + ":assistant")
    }
}

extension IOSExpertMessageAdmissionContractTests {
    @MainActor func testRecoveryPresentationRechecksWholeManifestAfterReceiptUpdate() async throws {
        let facts = ExpertRecoveryReadFacts()
        let (service, receipts) = try recoveryReadService(facts)
        let workspace = IOSRoomSubmissionWorkspace(service: service)
        await workspace.restoreConversation("parent")
        XCTAssertNil(workspace.errors[input().room_id])
        await workspace.refreshVisibleExpertReceipts(conversationID: "parent", messageIDs: ["user"])
        XCTAssertEqual(workspace.recoverableExpertManifests["user"]?.entries.count, 2)

        // Completing/admitting one peer must keep the other missing peer visible.
        facts.setExisting([receipts[0].id])
        workspace.applyReceipt(revisedRecoveryReceipt(receipts[0], revision: 2))
        for _ in 0..<100 where facts.invocationReads < 4 { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertNotNil(workspace.recoverableExpertManifests["user"])

        facts.setExisting(Set(receipts.map(\.id)))
        workspace.applyReceipt(revisedRecoveryReceipt(receipts[1], revision: 2))
        for _ in 0..<100 where workspace.recoverableExpertManifests["user"] != nil {
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertNil(workspace.recoverableExpertManifests["user"])
        XCTAssertGreaterThanOrEqual(facts.invocationReads, 6)
        XCTAssertTrue(facts.commandTypes.allSatisfy {
            ["agent_room", "get_product_setting", "private_load_manifest", "load_exact_run_invocation"].contains($0)
        }, "A presentation refresh cannot admit, prepare or dispatch a peer")
    }

    @MainActor func testLateRecoveryFactReadCannotRestorePreviousConversationCard() async throws {
        let facts = ExpertRecoveryReadFacts()
        let (service, _) = try recoveryReadService(facts)
        let workspace = IOSRoomSubmissionWorkspace(service: service)
        await workspace.restoreConversation("parent")
        XCTAssertNil(workspace.errors[input().room_id])
        facts.blockManifestRead()
        let read = Task { await workspace.refreshVisibleExpertReceipts(conversationID: "parent", messageIDs: ["user"]) }
        for _ in 0..<100 where !facts.manifestReadStarted { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(facts.manifestReadStarted)
        await workspace.restoreConversation("other-parent")
        facts.releaseManifestRead()
        await read.value
        XCTAssertTrue(facts.manifestReadReleasedBySignal, "The old read must remain blocked until after the actual owner switch")
        XCTAssertNil(workspace.conversationAssociation)
        XCTAssertTrue(workspace.recoverableExpertManifests.isEmpty)
    }

    private func revisedRecoveryReceipt(_ receipt: IOSRoomSubmission, revision: Int64) -> IOSRoomSubmission {
        .init(revision: revision, input: receipt.input, state: receipt.state,
            selectedParticipantID: receipt.selectedParticipantID, failureCode: nil,
            executionID: receipt.executionID, terminal: nil, contextConversationID: receipt.contextConversationID)
    }

    private func recoveryReadService(_ facts: ExpertRecoveryReadFacts) throws -> (IOSAgentRoomService, [IOSRoomSubmission]) {
        let original = input()
        let manifestValue = try manifest(original, subset: false)
        let manifest = try JSONDecoder().decode(AppleExpertMessageManifest<AgentCoreJSONValue>.self,
            from: JSONEncoder().encode(manifestValue))
        let members = original.selected_participants.map {
            IOSRoomParticipant(id: $0.participant_id, agentID: $0.agent_id, agentRevision: $0.agent_revision,
                displayName: $0.agent_id, description: "", contextConversationID: $0.context_conversation_id, active: true)
        }
        let room = IOSAgentRoomSnapshot(revision: 2,
            room: .init(id: original.room_id, title: "Contract fixture", participants: members))
        let association = IOSConversationRoomAssociation(conversationID: "parent", roomID: original.room_id,
            revision: 1, room: room)
        let receipts = manifest.entries.map { entry in
            IOSRoomSubmission(revision: 1, input: .init(roomID: original.room_id,
                submissionID: entry.child_submission_id, expectedRoomRevision: 2, text: original.text,
                participantID: entry.participant.participant_id, atMs: 100, parentMessageID: "user", conversationID: "parent"),
                state: "selected", selectedParticipantID: entry.participant.participant_id,
                failureCode: nil, executionID: entry.child_submission_id, terminal: nil,
                contextConversationID: entry.participant.context_conversation_id)
        }
        func json<T: Encodable>(_ value: T) throws -> AgentCoreJSONValue {
            try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value))
        }
        let associationValue = try json(association), roomValue = try json(room)
        let receiptValues = try Dictionary(uniqueKeysWithValues: receipts.map { ($0.id, try json($0)) })
        let allReceipts = AgentCoreJSONValue.array(receipts.compactMap { receiptValues[$0.id] })
        let service = IOSAgentRoomService(execute: { envelope in
            guard case .object(let root) = envelope, case .string(let type)? = root["type"] else {
                throw CocoaError(.coderInvalidValue)
            }
            facts.record(type)
            switch type {
            case "get_product_setting": return .null
            case "load_exact_run_invocation":
                guard case .string(let run)? = root["expected_run_id"],
                      let conversation = root["conversation_id"] else { throw CocoaError(.coderInvalidValue) }
                guard facts.exists(run) else { return .null }
                return .object(["conversation_id": conversation, "run_id": .string(run),
                    "assistant_message_id": .string(run + ":assistant")])
            case "agent_room":
                guard case .object(let command)? = root["command"], case .string(let kind)? = command["type"] else {
                    throw CocoaError(.coderInvalidValue)
                }
                switch kind {
                case "for_conversation": return command["conversation_id"] == .string("parent") ? associationValue : .null
                case "read": return roomValue
                case "submissions": return allReceipts
                case "submission":
                    guard case .string(let id)? = command["submission_id"], let receipt = receiptValues[id] else {
                        throw CocoaError(.coderInvalidValue)
                    }
                    return receipt
                default: throw CocoaError(.coderInvalidValue)
                }
            default: throw CocoaError(.coderInvalidValue)
            }
        }, expertHost: { operation in
            guard case .loadManifest = operation else { throw CocoaError(.coderInvalidValue) }
            facts.record("private_load_manifest")
            facts.waitForManifestReadIfRequested()
            return manifestValue
        })
        return (service, receipts)
    }
}

private nonisolated final class ExpertRecoveryReadFacts: @unchecked Sendable {
    private let lock = NSLock()
    private var existing = Set<String>()
    private var types: [String] = []
    private var shouldBlock = false
    private var blocked = false
    private var releasedBySignal = false
    private let gate = DispatchSemaphore(value: 0)
    var commandTypes: [String] { lock.withLock { types } }
    var invocationReads: Int { lock.withLock { types.filter { $0 == "load_exact_run_invocation" }.count } }
    var manifestReadStarted: Bool { lock.withLock { blocked } }
    var manifestReadReleasedBySignal: Bool { lock.withLock { releasedBySignal } }
    func setExisting(_ ids: Set<String>) { lock.withLock { existing = ids } }
    func exists(_ id: String) -> Bool { lock.withLock { existing.contains(id) } }
    func record(_ type: String) { lock.withLock { types.append(type) } }
    func blockManifestRead() { lock.withLock { shouldBlock = true } }
    func releaseManifestRead() { gate.signal() }
    func waitForManifestReadIfRequested() {
        let wait = lock.withLock { () -> Bool in
            guard shouldBlock else { return false }
            shouldBlock = false; blocked = true; return true
        }
        if wait {
            let released = gate.wait(timeout: .now() + 2) == .success
            lock.withLock { releasedBySignal = released }
        }
    }
}
