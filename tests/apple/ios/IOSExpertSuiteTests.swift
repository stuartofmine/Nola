// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS expert child history, configuration drafts, host mutation validation, message admission, native task integration, peer host, presentation, and main expert dispatch suite.

import Foundation
import XCTest
@testable import Nola

@MainActor final class IOSExpertChildHistoryTests: XCTestCase {
    private let parent = UUID().uuidString
    private let room = UUID().uuidString
    private func child(_ id: String = UUID().uuidString, current: Bool = true, active: Bool = true, revision: Int64? = 4) -> AppleExpertChildConversation {
        .init(conversationID: id, revision: 1, title: "Expert", participantID: UUID().uuidString,
              agentID: "expert", agentRevision: revision, isCurrent: current, active: active,
              createdAtMs: 1, updatedAtMs: 2, messageCount: 0)
    }
    private func page(_ children: [AppleExpertChildConversation], next: Int? = nil, parent: String? = nil, revision: Int64 = 1) -> AppleExpertChildConversations {
        .init(version: 1, conversationID: parent ?? self.parent, roomID: room, roomRevision: revision,
              children: children, hasMore: next != nil, nextOffset: next)
    }
    func testBranchRemainsUnreadUntilExpandedAndAppendsOnlyExplicitPage() async throws {
        let first = child(), second = child()
        let pages = [0: page([first], next: 1), 1: page([second])]
        let state = IOSExpertChildrenState(parentID: parent) { _, offset in try XCTUnwrap(pages[offset]) }
        XCTAssertFalse(state.loaded); XCTAssertTrue(state.children.isEmpty)
        await state.load()
        XCTAssertEqual(state.children, [first]); XCTAssertEqual(state.nextOffset, 1)
        await state.load()
        XCTAssertEqual(state.children, [first, second]); XCTAssertNil(state.nextOffset)
        await state.load()
        XCTAssertEqual(state.children, [first, second]); XCTAssertNil(state.error)
    }
    func testForeignParentAndChangedRoomPageCannotReplaceConfirmedBranch() async throws {
        let first = child(), second = child()
        let pages = [0: page([first], next: 1), 1: page([second], revision: 2)]
        let state = IOSExpertChildrenState(parentID: parent) { _, offset in try XCTUnwrap(pages[offset]) }
        await state.load(); await state.load()
        XCTAssertEqual(state.children, [first]); XCTAssertNotNil(state.error)
        let foreign = page([second], parent: UUID().uuidString)
        let foreignState = IOSExpertChildrenState(parentID: parent) { _, _ in foreign }
        await foreignState.load()
        XCTAssertTrue(foreignState.children.isEmpty); XCTAssertFalse(foreignState.loaded); XCTAssertNotNil(foreignState.error)
    }
    func testMentionHistoryMatchesFrozenContextAcrossPagesAndRejectsChangedRevision() async throws {
        let old = child(current: false, revision: nil)
        let current = child()
        let entry = AppleMainExpertSelectionEntry(agentID: old.agentID, agentRevision: 3,
            participantID: old.participantID, contextConversationID: old.id)
        var offsets: [Int] = []
        let (matchedRoom, matchedChild) = try await AppleExpertChildConversations.child(for: entry, parentID: parent) { offset in
            offsets.append(offset)
            return offset == 0 ? self.page([current], next: 1) : self.page([old])
        }
        XCTAssertEqual(offsets, [0, 1])
        XCTAssertEqual(matchedRoom, room)
        XCTAssertEqual(matchedChild, old)
        do {
            _ = try await AppleExpertChildConversations.child(for: entry, parentID: parent) { offset in
                offset == 0 ? self.page([current], next: 1) : self.page([old], revision: 2)
            }
            XCTFail("Changing the Room snapshot during lookup must invalidate the lookup")
        } catch { XCTAssertEqual((error as NSError).domain, NSCocoaErrorDomain) }
    }

    func testDuplicateContextAcrossPagesIsRejectedWithoutLosingHistory() async throws {
        let first = child()
        let page0 = page([first], next: 1), page1 = page([first])
        let state = IOSExpertChildrenState(parentID: parent) { _, offset in offset == 0 ? page0 : page1 }
        await state.load(); await state.load()
        XCTAssertEqual(state.children, [first]); XCTAssertNotNil(state.error)
    }
    func testContinueAcceptsOnlyExactCurrentMemberSnapshot() throws {
        let current = child()
        let member = IOSRoomParticipant(id: current.participantID, agentID: current.agentID,
            agentRevision: 4, displayName: "Expert", description: "", contextConversationID: current.id, active: true)
        let association = IOSConversationRoomAssociation(conversationID: parent, roomID: room, revision: 1,
            room: .init(revision: 1, room: .init(id: room, title: "Room", participants: [member])))
        let request = IOSExpertHistoryContinuation(parentID: try XCTUnwrap(UUID(uuidString: parent)), roomID: room, child: current)
        XCTAssertEqual(try request.participant(in: association).id, current.participantID)
        for old in [child(current.id, current: false), child(current.id, active: false), child(current.id, revision: nil), child()] {
            let rejected = IOSExpertHistoryContinuation(parentID: request.parentID, roomID: room, child: old)
            XCTAssertThrowsError(try rejected.participant(in: association))
        }
        XCTAssertThrowsError(try IOSExpertHistoryContinuation(parentID: UUID(), roomID: room, child: current).participant(in: association))
    }
}

extension IOSExpertChildHistoryTests {
    private func peerView(name: String? = "Frozen source expert", kind: String = "request", mainSource: String? = nil) -> AppleExpertPeerView {
        let source = AppleExpertParticipantReference(participant_id: "source-member", agent_id: "source-agent", agent_revision: 3, context_conversation_id: "source-context")
        let target = AppleExpertParticipantReference(participant_id: "target-member", agent_id: "target-agent", agent_revision: 5, context_conversation_id: "target-context")
        return .init(version: 1, exchangeID: "exchange-run", roomID: "room", parentConversationID: "parent",
            source: mainSource.map { .mainAgent(contextConversationID: $0) } ?? .legacyExpert(source),
            target: target, sourceRunID: "source-run", sourceCallID: "source-call",
            sourceDisplayName: name, targetDisplayName: "Frozen target expert", kind: kind, text: "Identical ordinary text",
            inReplyTo: nil, rootExchangeID: "exchange-run", hop: 0, createdAtMs: 1,
            clarifications: [.init(messageID: "clarification-message", source: target, target: source,
                text: "A clarification", createdAtMs: 2)])
    }
    func testPeerAuthorsUseExactCanonicalMessageIDsAndFrozenParticipantNames() throws {
        let exchange = peerView()
        let target = try IOSExpertPeerInputAuthor.map([exchange], ownerID: "target-context", parentID: "parent", roomID: "room")
        XCTAssertEqual(target["exchange-run:user"]?.name, "Frozen source expert")
        XCTAssertEqual(target["exchange-run:user"]?.participant, exchange.source)
        XCTAssertNil(target["ordinary-user-message"], "Same message text cannot establish expert provenance")
        XCTAssertNil(target["exchange-run:user:suffix"])
        let source = try IOSExpertPeerInputAuthor.map([exchange], ownerID: "source-context", parentID: "parent", roomID: "room")
        XCTAssertEqual(source["clarification-message"]?.name, "Frozen target expert")
        XCTAssertNil(source["exchange-run:user"], "Outgoing request cannot be presented as incoming user text")
    }
    func testMissingPeerNameUsesExpertLabelAndForeignProvenanceIsRejected() throws {
        let exchange = peerView(name: nil)
        let authors = try IOSExpertPeerInputAuthor.map([exchange], ownerID: "target-context", parentID: "parent", roomID: "room")
        XCTAssertEqual(authors["exchange-run:user"]?.name, String(localized: "expert.history.peerName"))
        XCTAssertThrowsError(try IOSExpertPeerInputAuthor.map([exchange], ownerID: "target-context", parentID: "other-parent", roomID: "room"))
        XCTAssertThrowsError(try IOSExpertPeerInputAuthor.map([exchange], ownerID: "target-context", parentID: "parent", roomID: "other-room"))
        XCTAssertThrowsError(try IOSExpertPeerInputAuthor.map([peerView(kind: "unknown")], ownerID: "target-context", parentID: "parent", roomID: "room"))
        let page = AppleExpertPeerExchanges(version: 1, conversationID: "parent", exchanges: [exchange], hasMore: false, nextOffset: nil)
        XCTAssertNoThrow(try page.validate(parentID: "parent", offset: 0))
        XCTAssertThrowsError(try page.validate(parentID: "target-context", offset: 0))
        let foreign = AppleExpertPeerExchanges(version: 1, conversationID: "target-context", exchanges: [exchange], hasMore: false, nextOffset: nil)
        XCTAssertThrowsError(try foreign.validate(parentID: "target-context", offset: 0))
        func mainPage(_ owner: String) -> AppleExpertPeerExchanges {
            .init(version: 1, conversationID: "parent", exchanges: [peerView(mainSource: owner)], hasMore: false, nextOffset: nil)
        }
        XCTAssertNoThrow(try mainPage("parent").validate(parentID: "parent", offset: 0))
        XCTAssertThrowsError(try mainPage("different-owner").validate(parentID: "parent", offset: 0))
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertPeerExchanges.self, from: JSONEncoder().encode(page)), page)
    }
}

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

@MainActor final class IOSExpertNativeTaskIntegrationTests: XCTestCase {
    func testMainAgentDispatchUsesCommonTaskPumpOwnModelAndSealedReopenDoesNotRerun() async throws {
        try await run(cancel: false)
    }
    func testMainAgentCancellationWaitsForActualTargetTransportStop() async throws {
        try await run(cancel: true)
    }

    func testCommittedPeerAdmissionLostAcknowledgementRecoversOriginalIntent() async throws {
        try await run(cancel: false, lostAcknowledgement: "admit_peer")
    }
    func testPreparedPeerTaskLostAcknowledgementRecoversFrozenConfiguration() async throws {
        try await run(cancel: false, lostAcknowledgement: "prepare_peer_task")
    }

    func testBusyStreamingExpertCancellationJoinsTransportAndSealsCanonicalResult() async throws {
        try await run(cancel: true, busyStreaming: true)
    }

    func testOwnerTransportFailureStopsIndependentBusyProducerWithoutInventingTerminalReceipt() async throws {
        try await run(cancel: true, busyStreaming: true, ownerTransportFailure: true)
    }

    func testTwoMainRunsInSameDatabaseMayReuseProviderCallIDs() async throws {
        try await run(cancel: false, repeatsMainRun: true)
    }

    private func run(cancel: Bool, lostAcknowledgement: String? = nil, busyStreaming: Bool = false,
                     ownerTransportFailure: Bool = false, repeatsMainRun: Bool = false) async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-native-peer-pump-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let fault = NativePeerAcknowledgementFault(kind: lostAcknowledgement, closeOwnerPump: ownerTransportFailure)
        defer { fault.cancelObserver() }
        var runtime = try IOSApplicationRuntime(databaseURL: database, expertHostAcknowledgementObserver: fault.observe,
            childPumpObserver: fault.observePump)
        NativePeerHTTPFixture.reset(runtime: runtime, cancel: cancel, busy: busyStreaming)
        URLProtocol.registerClass(NativePeerHTTPFixture.self)
        defer { URLProtocol.unregisterClass(NativePeerHTTPFixture.self); NativePeerHTTPFixture.clear() }
        let anonymous = try JSONSerialization.jsonObject(with: JSONEncoder().encode(IOSCloudCredentialBinding.anonymous))
        let source: [String: Any] = ["baseURL": "https://ios-native-peer-pump.invalid/v1", "wireProtocol": "responses", "cloudCredential": anonymous, "capabilities": ["chat"],
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true, "samplingParameters": true],
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        for model in ["source", "target"] {
            _ = try runtime.upsertModelProfileCAS(.init(id: model + "-profile", displayName: model, provider: "openai-compatible", modelName: model + "-model",
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: #"{"maxContextLength":131072,"maxOutputTokens":1024}"#, selected: model == "source", enabled: true,
                revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil), expectedRevision: 0)
        }
        let service = IOSAgentRoomService(runtime: runtime)
        var directory = try service.directory()
        directory = try service.createAgent(name: "target", description: "Native Task fixture", defaults: .object([
            "modelSelectionID": .string("configured:target-profile"), "expertReasoningEffort": .string("high")]), directory: directory)
        let targetAgent = try XCTUnwrap(directory.agents.first { $0.name == "target" })
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let parent = try store.createConversation(title: "Main shared Task", language: nil)
        let joined = try service.joinMention(agentID: targetAgent.id, expectedAgentRevision: targetAgent.revision, conversationID: parent.id.uuidString)
        let target = try XCTUnwrap(joined.room.room.participants.first { $0.agentID == targetAgent.id })
        // A user-selected existing snapshot remains pinned after directory edits.
        // The target must still execute its old own model/high effort, never the new source-model/low defaults.
        _ = try service.updateAgent(targetAgent, name: "New directory target", description: "New definition", availability: "active",
            directory: service.directory(), defaults: .object(["modelSelectionID": .string("configured:source-profile"), "expertReasoningEffort": .string("low")]))
        NativePeerHTTPFixture.targetParticipantID = target.id
        NativePeerHTTPFixture.parentConversationID = parent.id.uuidString
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = store
        let manager = ChatManager(container: container, restorePersistedState: false)
        let profile = try XCTUnwrap(runtime.modelProfile(id: "source-profile"))
        let model = try XCTUnwrap(ModelSelectionManager.modelInfo(from: profile))
        var parameters = ModelParameters.webAlignedCloudDefaults
        // Keep dispatch/cancellation tests independent of full tool-catalog input
        // budgeting; canonical blocked-budget presentation has separate coverage.
        parameters.maxContextLength = 131072; parameters.maxOutputTokens = 1024; parameters.reasoningPreference = "low"
        try store.selectConversationModel(conversationID: parent.id, model: model, initialParameters: parameters)
        manager.currentConversationId = parent.id
        manager.selectModel(model, persistAsDefault: false)
        let originalText = "Perform the bounded expert collaboration"
        do {
            _ = try await manager.processMessage(originalText, selectedExperts: [
                .init(agentID: targetAgent.id, agentRevision: target.agentRevision,
                    participantID: target.id, contextConversationID: target.contextConversationID)])
            XCTAssertNil(lostAcknowledgement, "The injected lost ACK must leave the original source operation unresolved")
        } catch IOSApplicationRuntimeError.outcomePending {
            XCTAssertNotNil(lostAcknowledgement)
            XCTAssertTrue(fault.dropped)
            let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .expertHost,
                validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
            let transaction = try journal.beginTransaction()
            let retained = try transaction.load(); transaction.close()
            XCTAssertEqual(retained.count, 1, "Commit alone must not discard the original uncertain intent")
            let original = try XCTUnwrap(retained.values.first)
            let operation = try AppleExpertHostIntentScope.operation(in: original)
            XCTAssertEqual(operation["kind"] as? String, lostAcknowledgement)
            let active = try XCTUnwrap(runtime.loadConversationProjectionV1(id: parent.id.uuidString).activeRun)
            runtime = try IOSApplicationRuntime(databaseURL: database, expertHostAcknowledgementObserver: fault.observe)
            NativePeerHTTPFixture.runtime = runtime
            let request = try XCTUnwrap(fault.sourceRequest)
            let recoveredRuntime = runtime
            let result = try await IOSLocalTaskHost.prepareExpert(request, contextID: parent.id.uuidString,
                runID: active.runID, store: runtime, expertTargetConfiguration: { material in
                    if lostAcknowledgement == "prepare_peer_task" {
                        XCTFail("Prepared configuration must be loaded from the canonical Task, never captured again")
                    }
                    return try await IOSExpertTaskTargetFactory.configuration(material: material,
                        runtime: recoveredRuntime, skills: [], hostExecutor: IOSAgentHostExecutor())
                })
            XCTAssertEqual(result.objectFields?["response"]?.objectFields?["type"], .string("task_promoted"))
            XCTAssertEqual(try taskRows(runtime).count, 1)
            XCTAssertEqual(try taskRows(runtime).first?["child_session_id"], .string(target.contextConversationID))
            XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 0, "Preparing/recovering cannot execute before source ACK")
            let settled = try journal.beginTransaction()
            let remaining = try settled.load(); settled.close()
            XCTAssertTrue(remaining.isEmpty, "Only the original validated receipt may retire the uncertainty")
            XCTAssertEqual(fault.replayedRequestIDs.count, 2)
            XCTAssertEqual(fault.replayedRequestIDs.first, original["request_id"] as? String)
            XCTAssertEqual(fault.replayedRequestIDs.first, fault.replayedRequestIDs.last,
                "The original durable request ID, not a new mutation, resolves the acknowledgement")
            XCTAssertEqual(fault.replayedOperations.first, fault.replayedOperations.last,
                "Replay must preserve the original authority and capture operation")
            if lostAcknowledgement == "prepare_peer_task" {
                XCTAssertEqual(fault.preparedConfigurations.count, 2)
                XCTAssertEqual(fault.preparedConfigurations.first, fault.preparedConfigurations.last,
                    "Retry must retain the first frozen target configuration")
            }
            XCTAssertEqual(try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString)).messages.filter { $0.role == "user" }.count, 1)
            return
        }
        let mainHistory = try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString))
        XCTAssertEqual(mainHistory.messages.filter { $0.role == "user" }.count, 1, "Selection cannot bypass Main or create a duplicate user turn")
        XCTAssertTrue(NativePeerHTTPFixture.failures.isEmpty, NativePeerHTTPFixture.failures.joined(separator: "\n"))
        let tasks = try taskRows(runtime)
        XCTAssertEqual(tasks.count, 1, "The peer must use one common Task rather than a parallel Room runner")
        let task = try XCTUnwrap(tasks.first)
        let taskID = try XCTUnwrap(task["id"]?.stringScalar)
        XCTAssertEqual(task["child_session_id"], .string(target.contextConversationID))
        XCTAssertEqual(task["owner_session_id"], .string(parent.id.uuidString))
        if ownerTransportFailure {
            XCTAssertTrue(fault.ownerPumpClosed)
            XCTAssertTrue(NativePeerHTTPFixture.targetStopped, "Owner failure must join the actual independent provider")
            XCTAssertGreaterThanOrEqual(NativePeerHTTPFixture.targetBusyFrames, 3)
            XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 1)
            let receipt = try runtime.executeAgentCommand(.object(["type": .string("runtime_task"),
                "command": .object(["type": .string("terminal_receipt"), "task_id": .string(taskID)])]))
            XCTAssertEqual(receipt, .null, "Transport failure and physical stop cannot invent a canonical terminal receipt")
            return
        }
        XCTAssertEqual(task["status"], .string("sealed"))
        XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 1, "The target's own provider executes exactly once")
        let sent = try XCTUnwrap(NativePeerHTTPFixture.targetRequests.first)
        XCTAssertEqual(sent["model"] as? String, "target-model")
        XCTAssertEqual((sent["reasoning"] as? [String: Any])?["effort"] as? String, "high")
        XCTAssertEqual(sent["max_output_tokens"] as? Int, 1024)
        XCTAssertEqual(NativePeerHTTPFixture.sourceEfforts, Set(["low"]))
        if cancel {
            let childTerminal = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
                "conversation_id": .string(target.contextConversationID), "run_id": .string(taskID)]))
            let childResult = childTerminal.objectFields?["result"]?.objectFields?["value"]?.objectFields
            XCTAssertEqual(childResult?["kind"], .string("cancelled"))
            XCTAssertEqual(childResult?["unknown_effect_refs"], .array([]))
            XCTAssertTrue(NativePeerHTTPFixture.targetStopped, "A persisted cancel request alone is not transport stop evidence")
            if busyStreaming { XCTAssertGreaterThanOrEqual(NativePeerHTTPFixture.targetBusyFrames, 3) }
        }
        else {
            let history = try service.childHistory(conversationID: parent.id.uuidString, childID: target.contextConversationID)
            let workspace = IOSRoomSubmissionWorkspace(service: service)
            await workspace.restoreConversation(parent.id.uuidString)
            XCTAssertNil(workspace.errors[joined.roomID])
            XCTAssertEqual(workspace.peerExchanges[joined.roomID]?.map(\.exchangeID), [taskID])
            let standalone = try service.create(title: "Independent room")
            await workspace.select(standalone, resumeExecutions: false)
            XCTAssertNil(workspace.errors[standalone.room.id])
            XCTAssertEqual(workspace.peerExchanges[standalone.room.id], [])
            let authors = try IOSExpertPeerInputAuthor.map(history.expertExchanges, ownerID: target.contextConversationID,
                parentID: parent.id.uuidString, roomID: joined.roomID)
            XCTAssertEqual(authors[taskID + ":user"]?.participant, .mainAgent(contextConversationID: parent.id.uuidString))
            XCTAssertTrue(IOSConversationProjectionPresenter.messages(from: history.window.projection).contains { $0.content.contains("target-own-high") })
        }
        let before = NativePeerHTTPFixture.targetRequests.count
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let reopenedContainer = ServiceContainer(applicationRuntime: reopened)
        _ = reopenedContainer.chatManager
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertEqual(try taskRows(reopened).first?["status"], task["status"])
        XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, before, "A sealed Task must not infer again on cold reopen")
        XCTAssertEqual(try IOSWorkspaceReadFacts.conversationDirectory(runtime: reopened).map(\.id), [parent.id])
        if repeatsMainRun {
            let firstRunID = try XCTUnwrap(mainHistory.messages.first(where: { $0.role == "assistant" })?.runID)
            let firstTerminal = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
                "conversation_id": .string(parent.id.uuidString), "run_id": .string(firstRunID)]))
            XCTAssertEqual(firstTerminal.objectFields?["result"]?.objectFields?["value"]?.objectFields?["kind"], .string("answered"))
            NativePeerHTTPFixture.sourceCount = 0
            let secondUserID = UUID()
            _ = try await manager.processMessage("Continue the same expert context", userMessageID: secondUserID, selectedExperts: [
                .init(agentID: targetAgent.id, agentRevision: target.agentRevision,
                    participantID: target.id, contextConversationID: target.contextConversationID)])
            XCTAssertTrue(NativePeerHTTPFixture.failures.isEmpty, NativePeerHTTPFixture.failures.joined(separator: "\n"))
            let secondRows = try taskRows(runtime)
            let secondIdentity = IOSCanonicalAgentTurnIdentity(userMessageID: secondUserID)
            let secondTerminal = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
                "conversation_id": .string(parent.id.uuidString), "run_id": .string(secondIdentity.submissionID)]))
            let diagnostic = XCTAttachment(string: String(decoding: try JSONEncoder().encode(AgentCoreJSONValue.object([
                "source_requests": .number(Double(NativePeerHTTPFixture.sourceCount)), "terminal": secondTerminal,
                "projection": try runtime.executeAgentCommand(.object(["type": .string("load_conversation_projection_v1"),
                    "conversation_id": .string(parent.id.uuidString)]))
            ])), as: UTF8.self))
            diagnostic.name = "same-main-second-run-canonical-diagnostic"
            diagnostic.lifetime = .keepAlways; add(diagnostic)
            XCTAssertEqual(secondTerminal.objectFields?["result"]?.objectFields?["value"]?.objectFields?["kind"], .string("answered"),
                "Sealed expert Tasks cannot hide a failed Main wait or final response")
            let projection = try runtime.loadConversationProjectionV1(id: parent.id.uuidString)
            let toolParts = projection.messages.filter { $0.role == .assistant }.flatMap { message in
                message.parts.compactMap { part -> IOSConversationProjectionV1.ToolPart? in
                    if case .tool(let tool) = part { return tool }; return nil
                }
            }
            for originalCallID in ["peer-request-call", "peer-wait-call"] {
                let repeats = toolParts.filter { $0.toolCallID == originalCallID }
                XCTAssertEqual(repeats.count, 2, "The provider's original call ID must be preserved in each Run")
                XCTAssertEqual(Set(repeats.map(\.partID)).count, 2, "Each acknowledged model attempt owns a distinct Part")
            }
            XCTAssertEqual(secondRows.count, 2)
            XCTAssertTrue(secondRows.allSatisfy { $0["status"] == .string("sealed") })
            XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 2)
            XCTAssertEqual(try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString)).messages.filter { $0.role == "user" }.count, 2)
            let secondBody = try XCTUnwrap(NativePeerHTTPFixture.targetRequests.last)
            XCTAssertEqual(secondBody["model"] as? String, "target-model")
            XCTAssertEqual((secondBody["reasoning"] as? [String: Any])?["effort"] as? String, "high")
            XCTAssertTrue(String(decoding: try JSONSerialization.data(withJSONObject: secondBody), as: UTF8.self).contains("target-own-high"),
                "The next Task must preserve its expert's existing canonical history")
        }
    }
    private func taskRows(_ runtime: IOSApplicationRuntime) throws -> [[String: AgentCoreJSONValue]] {
        let value = try runtime.executeAgentCommand(.object(["type": .string("runtime_task"), "command": .object(["type": .string("list_all")])]))
        guard case .array(let rows) = value else { throw IOSApplicationRuntimeError.invalidResponse }
        return try rows.map { try XCTUnwrap($0.objectFields) }
    }
}

private nonisolated final class NativePeerHTTPFixture: URLProtocol, @unchecked Sendable {
    @MainActor static var runtime: IOSApplicationRuntime?
    @MainActor static var targetParticipantID = ""
    @MainActor static var parentConversationID = ""
    @MainActor static var sourceCount = 0
    @MainActor static var sourceEfforts = Set<String>()
    @MainActor static var targetRequests: [[String: Any]] = []
    @MainActor static var targetStopped = false
    @MainActor static var cancel = false
    @MainActor static var busy = false
    @MainActor static var targetBusyFrames = 0
    @MainActor static var failures: [String] = []
    private let operationLock = NSLock()
    private var operation: Task<Void, Never>?
    private var isTarget = false
    @MainActor static func reset(runtime: IOSApplicationRuntime, cancel: Bool, busy: Bool) {
        self.runtime = runtime; self.cancel = cancel; self.busy = busy; targetBusyFrames = 0
        targetParticipantID = ""; sourceCount = 0
        sourceEfforts = []; targetRequests = []; targetStopped = false; failures = []
    }
    @MainActor static func clear() { runtime = nil }
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "ios-native-peer-pump.invalid" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {
        operationLock.withLock { operation?.cancel() }
        Task { @MainActor in if self.isTarget { Self.targetStopped = true } }
    }
    override func startLoading() {
        let request = request
        let work = Task { @MainActor in
            var headersSent = false
            do {
                var bytes = request.httpBody ?? Data()
                if bytes.isEmpty, let stream = request.httpBodyStream {
                    stream.open(); defer { stream.close() }
                    var chunk = [UInt8](repeating: 0, count: 4096)
                    while stream.hasBytesAvailable {
                        let count = stream.read(&chunk, maxLength: chunk.count)
                        guard count > 0 else { break }; bytes.append(contentsOf: chunk.prefix(count))
                    }
                }
                let body = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
                let model = try XCTUnwrap(body["model"] as? String)
                let events: [[String: Any]]
                if model == "target-model" {
                    self.isTarget = true; Self.targetRequests.append(body)
                    let names = (body["tools"] as? [[String: Any]] ?? []).compactMap { ($0["name"] as? String) ?? (($0["function"] as? [String: Any])?["name"] as? String) }
                    guard !names.contains("delegate_expert"), !names.contains("expert_exchange"),
                          !names.contains("spawn_agent"), !names.contains("request_user_input") else {
                        throw IOSApplicationRuntimeError.rejected("Expert inference advertised dispatch or direct human interaction")
                    }
                    if Self.cancel, Self.busy {
                        let url = try XCTUnwrap(request.url)
                        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil,
                            headerFields: ["Content-Type": "text/event-stream"])!, cacheStoragePolicy: .notAllowed)
                        headersSent = true
                        for index in 0..<1800 {
                            let event: [String: Any] = ["type": "response.output_text.delta", "delta": "busy-\(index) "]
                            let frame = "data: " + String(decoding: try JSONSerialization.data(withJSONObject: event), as: UTF8.self) + "\n\n"
                            client?.urlProtocol(self, didLoad: Data(frame.utf8))
                            Self.targetBusyFrames += 1
                            try await Task.sleep(for: .milliseconds(25))
                        }
                    }
                    if Self.cancel, !Self.busy { try await Task.sleep(for: .seconds(45)) }
                    events = Self.text("target-own-high")
                } else {
                    guard model == "source-model" else { throw IOSApplicationRuntimeError.invalidResponse }
                    Self.sourceCount += 1
                    if let effort = (body["reasoning"] as? [String: Any])?["effort"] as? String { Self.sourceEfforts.insert(effort) }
                    if Self.sourceCount == 1 {
                        events = try Self.tool(call: "peer-request-call", args: ["action": "request", "target_participant_id": Self.targetParticipantID, "text": "Check this implementation with your own model"])
                    } else if Self.sourceCount == 2 || (Self.cancel && Self.sourceCount == 3) {
                        if Self.cancel && Self.sourceCount == 2 {
                            for _ in 0..<500 where Self.targetRequests.isEmpty || (Self.busy && Self.targetBusyFrames < 3) { try await Task.sleep(for: .milliseconds(10)) }
                            guard !Self.targetRequests.isEmpty, !Self.busy || Self.targetBusyFrames >= 3 else { throw IOSApplicationRuntimeError.rejected("Target transport did not start before cancellation") }
                        }
                        let runtime = try XCTUnwrap(Self.runtime)
                        let value = try runtime.executeAgentCommand(.object(["type": .string("runtime_task"), "command": .object(["type": .string("list_all")])]))
                        let currentRun = try XCTUnwrap(runtime.loadConversationProjectionV1(id: Self.parentConversationID).activeRun?.runID)
                        guard case .array(let tasks) = value else { throw IOSApplicationRuntimeError.invalidResponse }
                        let currentTask = try tasks.first { task in
                            guard let metadata = task.objectFields?["metadata_json"]?.stringScalar else { return false }
                            return try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(metadata.utf8))
                                .objectFields?["expert_target"]?.objectFields?["source_run_id"] == .string(currentRun)
                        }
                        guard let id = currentTask?.objectFields?["id"]?.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }
                        let action = Self.cancel && Self.sourceCount == 2 ? "cancel" : "wait"
                        var arguments: [String: Any] = ["action": action, "exchange_id": id]
                        if action == "wait" { arguments["timeout_ms"] = 30000 }
                        events = try Self.tool(call: "peer-" + action + "-call", args: arguments)
                    } else { events = Self.text("source-finished") }
                }
                try Task.checkCancellation()
                let payload = try events.reduce(into: Data()) { data, event in
                    data.append(Data("data: ".utf8)); data.append(try JSONSerialization.data(withJSONObject: event)); data.append(Data("\n\n".utf8))
                }
                let url = try XCTUnwrap(request.url)
                if !headersSent {
                    client?.urlProtocol(self, didReceive: HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "text/event-stream"])!, cacheStoragePolicy: .notAllowed)
                }
                client?.urlProtocol(self, didLoad: payload); client?.urlProtocolDidFinishLoading(self)
            } catch is CancellationError { client?.urlProtocol(self, didFailWithError: URLError(.cancelled)) }
            catch { Self.failures.append(error.localizedDescription); client?.urlProtocol(self, didFailWithError: error) }
        }
        operationLock.withLock { operation = work }
    }
    @MainActor static func text(_ value: String) -> [[String: Any]] {
        [["type": "response.output_text.delta", "delta": value], ["type": "response.completed", "response": ["id": "response-\(UUID())", "status": "completed", "output": []]]]
    }
    @MainActor static func tool(call: String, args: [String: Any]) throws -> [[String: Any]] {
        [["type": "response.completed", "response": ["id": "response-\(UUID())", "status": "completed", "output": [["type": "function_call", "call_id": call, "name": "delegate_expert", "arguments": String(decoding: try JSONSerialization.data(withJSONObject: args), as: UTF8.self)]]]]]
    }
}

private nonisolated final class NativePeerAcknowledgementFault: @unchecked Sendable {
    private let lock = NSLock()
    let kind: String?
    private let closeOwnerPump: Bool
    private var didCloseOwnerPump = false
    private var pumpObserverTask: Task<Void, Never>?
    private var didDrop = false
    private var configurations: [Data] = []
    private var originalRequest: AgentCoreJSONValue?
    private var requestIDs: [String] = []
    private var operations: [Data] = []
    init(kind: String?, closeOwnerPump: Bool = false) { self.kind = kind; self.closeOwnerPump = closeOwnerPump }
    var ownerPumpClosed: Bool { lock.withLock { didCloseOwnerPump } }
    func cancelObserver() { lock.withLock { pumpObserverTask?.cancel() } }
    func observePump(_ pump: AppleChildPump) {
        guard closeOwnerPump else { return }
        let observer = Task { @MainActor in
            for _ in 0..<1000 {
                if Task.isCancelled { return }
                if !NativePeerHTTPFixture.targetRequests.isEmpty, NativePeerHTTPFixture.targetBusyFrames >= 3 { break }
                do { try await Task.sleep(for: .milliseconds(10)) } catch { return }
            }
            if Task.isCancelled { return }
            self.lock.withLock { self.didCloseOwnerPump = true }
            pump.close()
        }
        lock.withLock { pumpObserverTask = observer }
    }
    var dropped: Bool { lock.withLock { didDrop } }
    var sourceRequest: AgentCoreJSONValue? { lock.withLock { originalRequest } }
    var replayedRequestIDs: [String] { lock.withLock { requestIDs } }
    var replayedOperations: [Data] { lock.withLock { operations } }
    var preparedConfigurations: [Data] { lock.withLock { configurations } }
    func observe(_ operation: Data, _ reply: Data) throws {
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: operation) as? [String: Any])
        let current = fields["kind"] as? String
        let response = try XCTUnwrap(JSONSerialization.jsonObject(with: reply) as? [String: Any])
        let shouldDrop = try lock.withLock {
            if current == kind, let id = response["request_id"] as? String {
                requestIDs.append(id); operations.append(operation)
            }
            if current == "derive_peer_authority", let request = fields["request"] {
                originalRequest = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: request))
            }
            if current == "prepare_peer_task", let config = fields["target_configuration"] {
                configurations.append(try JSONSerialization.data(withJSONObject: config, options: [.sortedKeys]))
            }
            guard current == kind, !didDrop else { return false }
            didDrop = true
            return true
        }
        if shouldDrop { throw IOSApplicationRuntimeError.invalidResponse }
    }
}

final class IOSExpertPeerHostTests: XCTestCase {
    private func request(session: String = "source-context", run: String = "source-run", call: String = "source-call", input: [String: AgentCoreJSONValue] = ["action": .string("directory")]) -> AgentCoreJSONValue {
        .object(["session_id": .string(session), "turn_id": .string(run), "payload": .object([
            "call_id": .string(call), "capability_id": .string("delegate_expert"), "input": .object(input)])])
    }
    func testPeerSourceIdentityComesFromExactHostEnvelopeAndRejectsArgumentInjection() throws {
        let bound = try IOSExpertPeerHostRequest(request: request(), expectedContextID: "source-context", expectedRunID: "source-run")
        XCTAssertEqual(bound.contextID, "source-context"); XCTAssertEqual(bound.runID, "source-run"); XCTAssertEqual(bound.callID, "source-call")
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(session: "foreign"), expectedContextID: "source-context", expectedRunID: "source-run"))
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(run: "foreign"), expectedContextID: "source-context", expectedRunID: "source-run"))
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(call: ""), expectedContextID: "source-context", expectedRunID: "source-run"))
        for key in ["source_context_id", "source_run_id", "call_id", "api_key", "model"] {
            XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(input: ["action": .string("directory"), key: .string("spoofed")]),
                expectedContextID: "source-context", expectedRunID: "source-run"))
        }
    }
    func testTimeoutIsOnlyAdmittedForWaitActions() throws {
        for action in ["read", "cancel"] {
            let arguments: [String: AgentCoreJSONValue] = ["action": .string(action), "exchange_id": .string("exchange")]
            XCTAssertNoThrow(try IOSExpertPeerHostRequest(request: request(input: arguments),
                expectedContextID: "source-context", expectedRunID: "source-run"))
            var invalid = arguments; invalid["timeout_ms"] = .number(30_000)
            XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(input: invalid),
                expectedContextID: "source-context", expectedRunID: "source-run"))
        }
        XCTAssertNoThrow(try IOSExpertPeerHostRequest(request: request(input: ["action": .string("wait"),
            "exchange_id": .string("exchange"), "timeout_ms": .number(30_000)]),
            expectedContextID: "source-context", expectedRunID: "source-run"))
    }

    func testPeerHostOperationsKeepPrivateMutationAndExactClarificationIdentity() throws {
        let first = AppleExpertPeerClarificationInput(sourceContextID: "context", sourceRunID: "run", callID: "call", exchangeID: "exchange", text: "First")
        let second = AppleExpertPeerClarificationInput(sourceContextID: "context", sourceRunID: "run", callID: "other-call", exchangeID: "exchange", text: "Second")
        func object(_ input: AppleExpertPeerClarificationInput) throws -> [String: Any] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertHostOperation<AgentCoreJSONValue>.clarifyPeer(input: input, authority: .local()))) as? [String: Any])
        }
        XCTAssertNotEqual(try AppleExpertHostIntentScope.stableIdentity(object(first)), try AppleExpertHostIntentScope.stableIdentity(object(second)))
        XCTAssertFalse(AppleExpertHostOperation<AgentCoreJSONValue>.clarifyPeer(input: first, authority: .local()).isReadOnly)
        XCTAssertTrue(AppleExpertHostOperation<AgentCoreJSONValue>.loadPeerForSubmission(roomID: "room", exchangeID: "exchange", authority: .local()).isReadOnly)
        XCTAssertTrue(AppleExpertHostOperation<AgentCoreJSONValue>.peerDirectory(sourceContextID: "context", sourceRunID: "run", authority: .local()).isReadOnly)
    }
}

extension IOSExpertPeerHostTests {
    @MainActor func testOnlyMainPlanAdvertisesCoreDelegationSchema() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-expert-capability-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let identity = AgentCoreExpectedIdentity(hostId: "ios-local", clientId: "fixture", sessionId: "context")
        let ordinary = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal, expectedIdentity: identity,
            isForeground: true, applicationRuntime: runtime)
        XCTAssertFalse((ordinary.output.enabledTools + ordinary.output.disabledTools).contains { $0.toolId == "delegate_expert" })
        let main = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal, expectedIdentity: identity,
            isForeground: true, applicationRuntime: runtime, includeExpertTasks: true)
        let tool = try XCTUnwrap(main.output.enabledTools.first { $0.toolId == "delegate_expert" })
        let canonical = try runtime.executeAgentCommand(.object(["type": .string("read_expert_exchange_capability")]))
        let canonicalTool = try XCTUnwrap(canonical.objectFields?["tool"]?.objectFields)
        var expectedBoundSchema = canonicalTool
        expectedBoundSchema["x_nola"] = .object(["permission_contract": .object(["risk": .string("ordinary_read")])])
        XCTAssertEqual(tool.schema, .object(expectedBoundSchema), "Only the registered internal dispatcher receives this contract")
        XCTAssertEqual(tool.schema.objectFields?["function"], canonicalTool["function"], "The canonical model tool schema is unchanged")
        let properties = tool.schema.objectFields?["function"]?.objectFields?["parameters"]?.objectFields?["properties"]?.objectFields
        XCTAssertNotNil(properties?["timeout_ms"]); XCTAssertNil(properties?["timeout_sec"])
        XCTAssertNil(properties?["source_context_id"]); XCTAssertNil(properties?["source_run_id"])
    }

    func testPreparePeerTaskRetryIdentityRetainsFirstTargetConfiguration() throws {
        let request = request()
        func operation(_ marker: String) throws -> [String: Any] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertHostOperation<AgentCoreJSONValue>.preparePeerTask(
                request: request, exchangeID: "exchange", targetConfiguration: .object(["manifest_nonce": .string(marker)]),
                targetInvocation: nil, projectRoot: "fixture-root", authority: .local()))) as? [String: Any])
        }
        XCTAssertEqual(try AppleExpertHostIntentScope.stableIdentity(operation("first")), try AppleExpertHostIntentScope.stableIdentity(operation("second")))
    }

    func testCoreTaskTargetProofBindsOwnContextWithoutGrantingDelegation() throws {
        let task = UUID().uuidString.lowercased()
        let raw: [String: Any] = ["version": 1, "task_id": task, "exchange_id": task,
            "parent_conversation_id": "main", "room_id": UUID().uuidString,
            "source": ["participant_id": "source", "agent_id": "source-agent", "agent_revision": 1, "context_conversation_id": "source-context"],
            "target": ["participant_id": "target", "agent_id": "target-agent", "agent_revision": 2, "context_conversation_id": "target-context"],
            "source_run_id": UUID().uuidString, "source_call_id": "call"]
        let proof = try JSONDecoder().decode(AppleExpertTaskTargetV1.self, from: JSONSerialization.data(withJSONObject: raw))
        XCTAssertNoThrow(try proof.validate(taskID: task, contextID: "target-context", runID: task))
        XCTAssertThrowsError(try proof.validate(taskID: "other", contextID: "target-context", runID: task))
        XCTAssertThrowsError(try proof.validate(taskID: task, contextID: "source-context", runID: task))
        XCTAssertThrowsError(try proof.validate(taskID: task, contextID: "target-context", runID: "another-run"))
    }
}

extension IOSExpertPeerHostTests {
    @MainActor func testTargetFactoryFreezesOwnModelEffortAndSkillsWithoutStartingRun() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-expert-target-factory-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let anonymous = try JSONSerialization.jsonObject(with: JSONEncoder().encode(IOSCloudCredentialBinding.anonymous))
        let source: [String: Any] = ["baseURL": "https://fixture.invalid/v1", "wireProtocol": "responses", "cloudCredential": anonymous, "capabilities": ["chat"],
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true, "samplingParameters": true],
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        for (id, model) in [("source-profile", "source-model"), ("target-profile", "target-model")] {
            _ = try runtime.upsertModelProfileCAS(.init(id: id, displayName: id, provider: "openai-compatible", modelName: model,
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: #"{"maxContextLength":16384,"maxOutputTokens":1024}"#, selected: id == "source-profile", enabled: true,
                revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil), expectedRevision: 0)
        }
        let service = IOSAgentRoomService(runtime: runtime)
        let directory = try service.createAgent(name: "Target", description: "", defaults: .object([
            "modelSelectionID": .string("configured:target-profile"), "expertReasoningEffort": .string("high")]), directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let parent = try LocalConversationStore(applicationRuntime: runtime).createConversation(title: "Target factory", language: nil)
        let association = try service.joinMention(agentID: agent.id, expectedAgentRevision: agent.revision, conversationID: parent.id.uuidString)
        // Factory input is a typed fixture built from the actual joined target;
        // the separate native Main pump test validates real capture/admission.
        let member = try XCTUnwrap(association.room.room.participants.first)
        let participant = AppleExpertParticipantReference(participant_id: member.id, agent_id: member.agentID,
            agent_revision: member.agentRevision, context_conversation_id: member.contextConversationID)
        let parentRecord = try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString))
        let runID = UUID().uuidString
        let previewInput = AppleAdmitExpertConversationMessage(conversation_id: parent.id.uuidString,
            expected_revision: parentRecord.revision, room_id: association.roomID, expected_room_revision: association.room.revision,
            submission_id: runID, user_message_id: runID + ":user", text: "Preview own target configuration",
            selected_participants: [participant], created_at_ms: 1)
        let preview: AppleExpertCapturePreview<AgentCoreJSONValue> = try runtime.expertHostOperation(
            .preview(input: previewInput, authority: .local()))
        let resolved = try XCTUnwrap(preview.participants.first)
        let entry = AppleExpertMessageEntry<AgentCoreJSONValue>(participant: participant, display_name: "Target", description: "", defaults: resolved.defaults,
            model_declaration: resolved.model_declaration,
            context_configuration_revision: resolved.context_configuration_revision, child_submission_id: runID,
            child_user_message_id: runID + ":user", child_assistant_message_id: runID + ":assistant", expected_prompt_definition: nil,
            model_witness: nil, instruction_witness: nil)
        let view = AppleExpertPeerView(version: 1, exchangeID: entry.child_submission_id, roomID: association.roomID,
            parentConversationID: parent.id.uuidString, source: .mainAgent(contextConversationID: parent.id.uuidString), target: entry.participant, sourceRunID: UUID().uuidString,
            sourceCallID: "call", sourceDisplayName: "Source", targetDisplayName: "Target", kind: "request", text: "Capture target",
            inReplyTo: nil, rootExchangeID: entry.child_submission_id, hop: 1, createdAtMs: 1, clarifications: [])
        let skill = IOSAgentSkillSnapshot(id: "target-skill", name: "Target Skill", summary: "", instructions: "Frozen target instructions",
            version: "1", requiredTools: [], validationSteps: [])
        let config = try await IOSExpertTaskTargetFactory.configuration(material: .init(view: view, entry: entry, promptDefinitions: []),
            runtime: runtime, skills: [skill], hostExecutor: IOSAgentHostExecutor())
        let identity = try XCTUnwrap(config.objectFields?["turn_identity"]?.objectFields)
        let invocation = try XCTUnwrap(identity["host_invocation"]?.objectFields)
        XCTAssertEqual(invocation["primary"]?.objectFields?["model"], .string("target-model"))
        XCTAssertEqual(invocation["generation"]?.objectFields?["reasoning_selection"]?.objectFields?["value"], .string("high"))
        XCTAssertEqual(identity["host_dependencies"], IOSPreparedHostInvocation.skillIdentity([skill]))
        XCTAssertNotNil(identity["delegation"]?.objectFields?["project_root"])
        let tools: [AgentCoreJSONValue]
        if case .array(let values)? = config.objectFields?["tools"] { tools = values } else { throw IOSApplicationRuntimeError.invalidResponse }
        let toolNames = tools.compactMap { tool in
            tool.objectFields?["function"]?.objectFields?["name"]?.stringScalar ?? tool.objectFields?["name"]?.stringScalar
        }
        for control in ["delegate_expert", "expert_exchange", "spawn_agent", "request_user_input"] {
            XCTAssertFalse(toolNames.contains(control), "Expert capture must leave human interaction and dispatch to Main: \(control)")
        }
        XCTAssertNil(try runtime.loadConversationProjectionV1(id: entry.participant.context_conversation_id).activeRun)
        XCTAssertTrue(try runtime.executeAgentCommand(.object(["type": .string("runtime_task"), "command": .object(["type": .string("list_all")])])) == .array([]),
            "Producing target configuration must not create a worker or Task")
    }
}

extension IOSExpertPeerHostTests {
    func testUncertainPreparationNeverCompletesOriginalSourceToolAsFailed() throws {
        for error: Error in [IOSApplicationRuntimeError.outcomePending("original-command-receipt"),
            IOSExpertTaskPreparationError(exchangeID: "existing-task", reason: "The promotion ACK is not confirmed", taskMayExist: true), CancellationError()] {
            XCTAssertThrowsError(try DeviceAgentCoordinator.expertPreparationFailureResponse(callID: "original-source-call", error: error))
        }
        let rejected = try DeviceAgentCoordinator.expertPreparationFailureResponse(callID: "original-source-call",
            error: IOSExpertTaskPreparationError(exchangeID: "aborted-capture", reason: "Target configuration unsupported", preparationAborted: true))
        XCTAssertEqual(rejected.objectFields?["call_id"], .string("original-source-call"))
        XCTAssertEqual(rejected.objectFields?["outcome"], .string("failed"))
        XCTAssertEqual(rejected.objectFields?["output"]?.objectFields?["execution"]?.objectFields?["state"], .string("not_started"))
        XCTAssertEqual(rejected.objectFields?["output"]?.objectFields?["data"]?.objectFields?["execution_started"], .bool(false))
        XCTAssertEqual(rejected.objectFields?["output"]?.objectFields?["data"]?.objectFields?["preparation_aborted"], .bool(true))
    }
}

final class IOSMainExpertDispatchContractTests: XCTestCase {
    func testSparseDeclaredCapabilitiesRetainKnownToolsAndUnknownFieldsStayConservative() throws {
        let capabilities = try JSONDecoder().decode(ModelExecutionCapabilities.self,
            from: Data(#"{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true}"#.utf8))
        XCTAssertTrue(capabilities.toolCalls)
        XCTAssertTrue(capabilities.reasoning)
        XCTAssertFalse(capabilities.imageInput)
        XCTAssertFalse(capabilities.samplingRequiresNoReasoning)
        XCTAssertEqual(try JSONDecoder().decode(ModelExecutionCapabilities.self, from: JSONEncoder().encode(capabilities)), capabilities)
        XCTAssertThrowsError(try JSONDecoder().decode(ModelExecutionCapabilities.self,
            from: Data(#"{"toolCalls":"yes","imageInput":false,"reasoning":true,"samplingParameters":true}"#.utf8)))
        XCTAssertThrowsError(try JSONDecoder().decode(ModelExecutionCapabilities.self, from: Data(#"{"reasoning":true}"#.utf8)))
        XCTAssertThrowsError(try JSONDecoder().decode(ModelExecutionCapabilities.self,
            from: Data(#"{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true,"samplingRequiresNoReasoning":null}"#.utf8)))
    }

    func testCanonicalBlockedBudgetNeverLooksLikeAnsweredAndDoesNotMatchArbitraryDetail() throws {
        let blocked = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"context.input_budget_exceeded","detail":null},"resolution_condition":"opaque budget condition","blocking_fact_refs":["budget"]}"#.utf8))
        XCTAssertNotNil(blocked.blockedFeedback)
        XCTAssertNotEqual(blocked.blockedFeedback, "opaque budget condition")
        let unrelated = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"permission.denied","detail":"context.input_budget_exceeded"},"resolution_condition":"Grant the requested permission","blocking_fact_refs":["permission"]}"#.utf8))
        XCTAssertEqual(unrelated.blockedFeedback, "Grant the requested permission")
        let answered = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: Data(#"{"kind":"answered","answer_part_refs":["answer"]}"#.utf8))
        XCTAssertNil(answered.blockedFeedback)
    }

    func testOnlyClosedProtocolRejectionsOfferGuidanceAndUnknownOutcomesStayPending() {
        let unsupported = IOSApplicationRuntimeError.rejected("main_expert_selection_requires_tool_capable_main_model")
        XCTAssertEqual(ChatManager.mainExpertRejection(unsupported), .unsupportedModel)
        XCTAssertTrue(AppleMainExpertDispatchRejection.unsupportedModel.permitsDraftRestore)
        XCTAssertFalse(AppleMainExpertDispatchRejection.sourceChanged.permitsDraftRestore)
        XCTAssertNil(ChatManager.mainExpertRejection(IOSApplicationRuntimeError.rejected("unknown outcome")))
        XCTAssertNil(ChatManager.mainExpertRejection(IOSApplicationRuntimeError.rejected("user says main_expert_selection_requires_tool_capable_main_model")))
    }

    func testSelectionUsesOneOriginalUserTurnAndSeparateTypedContext() throws {
        let entry = AppleMainExpertSelectionEntry(agentID: "expert", agentRevision: 7, participantID: "member", contextConversationID: "child")
        let selection = AppleMainExpertSelectionContext(selectedExperts: [entry])
        try selection.validate()
        let context = AgentCoreContextComponent(kind: .mainExpertSelection, id: "nola.main-expert-selection",
            content: try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(selection)), source: nil)
        let original = "当前 search / current，请继续检查"
        let input = try AgentCoreInputSubmission.make(input: original, contextComponents: [context], messageId: "original-user", submissionId: "original-run")
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: JSONEncoder().encode(input)) as? [String: Any])
        let items = try XCTUnwrap(object["items"] as? [[String: Any]])
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?["content"] as? String, original)
        XCTAssertEqual(items.first?["id"] as? String, "original-user")
        let components = try XCTUnwrap(object["turn_context_components"] as? [[String: Any]])
        XCTAssertEqual(components.first?["kind"] as? String, "main_expert_selection")
        XCTAssertEqual((components.first?["content"] as? [String: Any])?["version"] as? Int, 1)
    }

    func testRemoteSelectionRemainsMetadataAndRetainsObservedEpoch() throws {
        let text = "请继续，不改变我的输入"
        let entry = AppleMainExpertSelectionEntry(agentID: "expert", agentRevision: 2)
        let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [], userPrefs: ["message": text],
            selectedExperts: [entry], expertSelectionSourceEpoch: "observed-store")
        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        let messages = try XCTUnwrap(payload["messages"] as? [[String: Any]])
        XCTAssertEqual(messages.count, 1); XCTAssertEqual(messages.first?["content"] as? String, text)
        XCTAssertEqual(payload["expected_source_store_epoch"] as? String, "observed-store")
        let selected = try XCTUnwrap(payload["selected_experts"] as? [[String: Any]])
        XCTAssertEqual(selected.first?["agent_id"] as? String, entry.agentID)
        XCTAssertEqual(selected.first?["agent_revision"] as? Int64, entry.agentRevision)
        XCTAssertTrue(selected.first?["participant_id"] is NSNull)
        XCTAssertNil(payload["model"], "An expert choice cannot replace the Main model")
        let restored = try JSONDecoder().decode(ModelInput.self, from: JSONEncoder().encode(input))
        XCTAssertEqual(restored, input)
    }

    func testInvalidSelectionAndUnknownSourceRejectWhileLegacyHistoryStillDecodes() throws {
        XCTAssertThrowsError(try AppleMainExpertSelectionContext(selectedExperts: [
            .init(agentID: "expert", agentRevision: 1, participantID: "unpaired")]).validate())
        XCTAssertThrowsError(try AppleMainExpertSelectionContext(selectedExperts: [.init(agentID: "expert", agentRevision: 1)], expectedSourceStoreEpoch: "").validate())
        let main = Data(#"{"kind":"main_agent","context_conversation_id":"main"}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertTaskSource.self, from: main), .mainAgent(contextConversationID: "main"))
        let legacy = AppleExpertParticipantReference(participant_id: "member", agent_id: "expert", agent_revision: 1, context_conversation_id: "legacy-context")
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertTaskSource.self, from: JSONEncoder().encode(legacy)), .legacyExpert(legacy))
        XCTAssertThrowsError(try JSONDecoder().decode(AppleExpertTaskSource.self, from: Data(#"{"kind":"other","context_conversation_id":"main"}"#.utf8)))
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: .object(["session_id": .string("main"), "turn_id": .string("run"),
            "payload": .object(["call_id": .string("call"), "capability_id": .string("delegate_expert"), "input": .object(["action": .string("clarify")])])]),
            expectedContextID: "main", expectedRunID: "run"))
    }

    func testOldHostDoesNotAdvertiseMainDelegationAndUnknownCapabilitiesStayUnsupported() throws {
        let legacy = try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self, from: Data(#"{"version":1,"templates":[],"features":{"reasoning_effort":true}}"#.utf8))
        XCTAssertFalse(legacy.features.mainAgentDelegation)
        XCTAssertFalse(legacy.features.inlineExpertMentions)
        let current = try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self, from: Data(#"{"version":1,"templates":[],"features":{"main_agent_delegation":true}}"#.utf8))
        XCTAssertTrue(current.features.mainAgentDelegation)
    }

    @MainActor func testEncryptedHostDirectoryChecksSourceAndNeverRebasesAnObservedEpoch() async throws {
        for (host, epoch, matches, succeeds) in [("host", "old-epoch", true, true), ("foreign", "old-epoch", true, false), ("host", "new-epoch", true, false), ("host", "old-epoch", false, false)] {
            let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:9")!, token: "fixture-token", expectedHostId: "host", clientId: "client")) { request in
                XCTAssertTrue(request.url?.path.hasSuffix("/agent-rooms") == true)
                let encrypted = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as? [String: Any])
                let sent = try WebAgentProvider.decryptMobileJSON(encrypted, token: "fixture-token")
                XCTAssertEqual(sent["expected_source_store_epoch"] as? String, "old-epoch")
                XCTAssertEqual((sent["command"] as? [String: Any])?["type"] as? String, "directory")
                var reply = try WebAgentProvider.encryptMobileJSON(["host_id": host, "source_store_epoch": epoch,
                    "source_matches": matches, "result": ["revision": 1, "agents": []]], token: "fixture-token")
                reply["type"] = "encrypted"
                let bytes = try JSONSerialization.data(withJSONObject: reply)
                return WebAgentTransportResponse(bytes: AsyncThrowingStream { stream in bytes.forEach { stream.yield($0) }; stream.finish() },
                    response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
            }
            do {
                let value = try await provider.expertRooms(command: .object(["type": .string("directory")]), expectedEpoch: "old-epoch")
                XCTAssertTrue(succeeds); XCTAssertEqual(value.sourceStoreEpoch, "old-epoch")
            } catch { XCTAssertFalse(succeeds, error.localizedDescription) }
        }
    }
}

final class IOSExpertConfigurationMutationAndPresentationTests: XCTestCase {
    func testExpertConfigurationDraftSelectorsReasoningAndMentionPolicy() throws {
        let originals: [AgentCoreJSONValue] = [.object([:]), .object([
            "modelSelectionID": .null, "selectedPromptTemplateID": .null,
            "modelOverrides": .object(["configured:prior": .object(["temperature": .number(0.27)])]),
            "language": .string("zh"), "unknownCoreOwnedField": .array([.null, .bool(true)])])]
        for original in originals {
            let draft = try AppleExpertConfigurationDraft(defaults: original)
            XCTAssertEqual(try draft.applying(to: original), original)
        }

        let explicitOrig: AgentCoreJSONValue = .object([
            "modelSelectionID": .string("configured:old"), "selectedPromptTemplateID": .string("original-template"),
            "modelOverrides": .object(["configured:old": .object(["temperature": .number(0.25)])]),
            "language": .string("de")])
        var draft = try AppleExpertConfigurationDraft(defaults: explicitOrig)
        draft.modelID = "configured:new"
        guard case .object(var expected) = explicitOrig else { return XCTFail("Expected object") }
        expected["modelSelectionID"] = .string("configured:new")
        XCTAssertEqual(try draft.applying(to: explicitOrig), .object(expected))
        draft.modelID = ""; draft.templateID = ""
        expected.removeValue(forKey: "modelSelectionID"); expected.removeValue(forKey: "selectedPromptTemplateID")
        XCTAssertEqual(try draft.applying(to: explicitOrig), .object(expected))

        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .array([])))
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .object(["modelSelectionID": .bool(false)])))
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .object(["selectedPromptTemplateID": .signedInteger(7)])))

        let reasoningOrig: AgentCoreJSONValue = .object([
            "expertReasoningEffort": .string("high"),
            "modelOverrides": .object(["configured:model": .object(["temperature": .number(0.25)])]),
            "unknownCoreOwnedField": .array([.null, .bool(true)])
        ])
        var rDraft = try AppleExpertConfigurationDraft(defaults: reasoningOrig)
        XCTAssertEqual(rDraft.reasoningEffort, "high")
        XCTAssertEqual(try rDraft.applying(to: reasoningOrig), reasoningOrig)
        rDraft.reasoningEffort = "xhigh"
        guard case .object(var rExpected) = reasoningOrig else { return XCTFail("Expected defaults object") }
        rExpected["expertReasoningEffort"] = .string("xhigh")
        XCTAssertEqual(try rDraft.applying(to: reasoningOrig), .object(rExpected))
        rDraft.reasoningEffort = "automatic"
        rExpected.removeValue(forKey: "expertReasoningEffort")
        XCTAssertEqual(try rDraft.applying(to: reasoningOrig), .object(rExpected))
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: .object([:])).reasoningEffort, "automatic")
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .object(["expertReasoningEffort": .bool(false)])))

        let a = UUID(), b = UUID()
        XCTAssertTrue(IOSExpertMentionCompletionPolicy.accepts(owner: nil, draftOwner: .unsent(a), currentOwner: nil, currentDraftOwner: .unsent(a)))
        XCTAssertFalse(IOSExpertMentionCompletionPolicy.accepts(owner: nil, draftOwner: .unsent(a), currentOwner: nil, currentDraftOwner: .unsent(b)))
        XCTAssertFalse(IOSExpertMentionCompletionPolicy.accepts(owner: a, draftOwner: .conversation(a), currentOwner: b, currentDraftOwner: .conversation(b)))
    }

    func testExpertHostMutationReceiptValidation() throws {
        let target = AppleExpertParticipantReference(participant_id: "target", agent_id: "agent", agent_revision: 1, context_conversation_id: "expert-context")
        let exchange = AppleExpertPeerView(version: 1, exchangeID: "exchange", roomID: "room",
            parentConversationID: "main", source: .mainAgent(contextConversationID: "main"),
            target: target, sourceRunID: "previous-main-run", sourceCallID: "previous-request-call",
            sourceDisplayName: "Main", targetDisplayName: "Expert", kind: "request", text: "Original",
            inReplyTo: nil, rootExchangeID: "exchange", hop: 1, createdAtMs: 1, clarifications: [])
        let view = try JSONSerialization.jsonObject(with: JSONEncoder().encode(exchange))
        let cancelOp: [String: Any] = ["kind": "peer_task_action", "request": [
            "session_id": "main", "turn_id": "later-main-run", "payload": ["call_id": "current-cancel-call",
                "input": ["action": "cancel", "exchange_id": "exchange"]]]]
        func cancelReceipt(call: String = "current-cancel-call", task: String = "exchange", owner: String = "main") throws -> Data {
            var projected = try XCTUnwrap(view as? [String: Any])
            projected["source"] = ["kind": "main_agent", "context_conversation_id": owner]
            return try JSONSerialization.data(withJSONObject: ["terminal": false, "wait_ms": 0,
                "response": ["type": "tool_completed", "call_id": call, "output": ["exchange": projected, "task": ["id": task]]]])
        }
        XCTAssertNoThrow(try IOSExpertHostMutationValidation.validate(cancelReceipt(), operation: cancelOp))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(cancelReceipt(call: "foreign-call"), operation: cancelOp))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(cancelReceipt(task: "foreign-task"), operation: cancelOp))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(cancelReceipt(owner: "foreign-main"), operation: cancelOp))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(cancelReceipt(), operation: ["kind": "unknown_mutation"]))

        let prepTarget = AppleExpertParticipantReference(participant_id: "target", agent_id: "agent", agent_revision: 1, context_conversation_id: "expert")
        let prepView = AppleExpertPeerView(version: 1, exchangeID: "exchange", roomID: "room",
            parentConversationID: "main", source: .mainAgent(contextConversationID: "main"), target: prepTarget,
            sourceRunID: "run", sourceCallID: "call", sourceDisplayName: "Main", targetDisplayName: "Expert",
            kind: "request", text: "Request", inReplyTo: nil, rootExchangeID: "exchange", hop: 1, createdAtMs: 1, clarifications: [])
        let prepOp: [String: Any] = ["kind": "prepare_peer_task", "exchange_id": "exchange", "request": [
            "session_id": "main", "turn_id": "run", "payload": ["call_id": "call", "input": ["action": "request", "target_participant_id": "target"]]]]
        func prepReceipt(admitted: Bool = true, call: String = "call", task: String = "exchange") throws -> Data {
            try JSONSerialization.data(withJSONObject: [
                "view": JSONSerialization.jsonObject(with: JSONEncoder().encode(prepView)),
                "prepared": ["task": ["id": "exchange", "owner_session_id": "main", "child_session_id": "expert"],
                    "snapshot": ["owner_session_id": "main"], "replayed": true, "source_admitted": admitted],
                "response": ["type": "tool_completed", "call_id": call, "output": ["exchange_id": "exchange", "task_id": task, "replayed": true]]])
        }
        XCTAssertNoThrow(try IOSExpertHostMutationValidation.validate(prepReceipt(), operation: prepOp))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(prepReceipt(admitted: false), operation: prepOp))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(prepReceipt(call: "foreign"), operation: prepOp))
        XCTAssertThrowsError(try IOSExpertHostMutationValidation.validate(prepReceipt(task: "foreign"), operation: prepOp))
    }

    @MainActor
    func testExpertColorPreferencesAppearanceJSONAndWorkspaceDestinations() throws {
        let suite = "IOSExpertPresentationTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let preferences = IOSExpertColorPreferences(defaults: defaults)
        XCTAssertEqual(preferences.color(for: "agent-a"), .blue)
        XCTAssertEqual(preferences.color(for: "other-agent"), .blue)
        preferences.save(.teal, for: "agent-a")
        let data = try XCTUnwrap(defaults.data(forKey: AppleExpertAppearance.storageKey))
        XCTAssertEqual(try JSONDecoder().decode([String: String].self, from: data), ["agent-a": "teal"])
        XCTAssertEqual(AppleExpertAppearance.selected(data, agentID: "agent-a"), .teal)
        XCTAssertNil(AppleExpertAppearance.saving(.red, agentID: "", in: data))
        XCTAssertEqual(preferences.color(for: "other-agent"), .blue)
        let reloaded = IOSExpertColorPreferences(defaults: defaults)
        XCTAssertEqual(reloaded.color(for: "agent-a"), .teal)
        reloaded.save(.green, for: "agent-a")
        XCTAssertEqual(IOSExpertColorPreferences(defaults: defaults).color(for: "agent-a"), .green)

        XCTAssertEqual(Set(IOSWorkspaceDestination.allCases.map(\.rawValue)),
            Set(["sources", "knowledge", "memory", "plans", "records", "models", "settings"]))
        XCTAssertNil(IOSWorkspaceDestination(page: -1))
        XCTAssertEqual(IOSWorkspaceDestination(page: 0), .sources)
        XCTAssertEqual(IOSWorkspaceDestination(page: 3), .records)
    }
}
