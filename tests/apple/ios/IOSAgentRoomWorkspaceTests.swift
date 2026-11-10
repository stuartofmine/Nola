// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import UIKit
@testable import Nola

@MainActor final class IOSAgentRoomWorkspaceTests: XCTestCase {
    private func participant(_ name: String, active: Bool = true, owner: String = UUID().uuidString) -> IOSRoomParticipant {
        IOSRoomParticipant(id: name, agentID: name, agentRevision: 1, displayName: name, description: "",
            contextConversationID: owner, active: active)
    }

    private func projection(_ owner: String, revision: Int = 1, messageIDs: [String] = ["same-id"], role: String = "user", markdown: String = "private fact", state: String = "completed") throws -> IOSConversationProjectionV1 {
        let fields: [String: Any] = ["contract_version": 1, "execution_ownership": ["kind": "user"],  "conversation_id": owner,
            "revision": revision, "title": "Fixture", "messages": messageIDs.map { ["message_id": $0, "role": role,
                "state": state, "revision": revision, "created_at_ms": 1,
                "parts": [["kind": "text", "part_id": "text", "revision": revision,
                    "markdown": markdown, "is_final": true]]] },
            "context_usage": ["used_tokens": 0, "context_window_tokens": 128000,
                "prompt_token_limit": 120000, "estimated": true, "measured_at_revision": revision]]
        return try JSONDecoder().decode(IOSConversationProjectionV1.self,
            from: JSONSerialization.data(withJSONObject: fields))
    }

    func testExactNativeControlRefreshesPendingMetadataWithoutReplacingLiveJournalBodies() throws {
        let workspace = IOSAgentRoomWorkspace(), member = participant("Expert")
        let ownerID = member.contextConversationID
        let pending = try projection(ownerID, role: "assistant", markdown: "", state: "pending")
        try workspace.install(.init(revision: 1, room: .init(id: "room", title: "Room", participants: [member])),
            projections: [ownerID: pending], eventSequences: [ownerID: 10])
        let owner = IOSCommittedAssistantOwner(conversationID: ownerID, assistantMessageID: "same-id", generation: 7)
        func snapshot(_ value: IOSConversationProjectionV1, sequence: UInt64) throws -> IOSCommittedAssistantProjection {
            try XCTUnwrap(.init(window: .init(projection: value, eventSequence: sequence,
                totalMessageCount: 1, beforeOrdinal: nil, hasMore: false, assistantJournalHeads: []), owner: owner))
        }
        let initial = try snapshot(pending, sequence: 10)
        func needsRefresh(_ value: IOSCommittedAssistantProjection?, session: String? = nil,
                          run: String = "run", generation: UInt64 = 7) -> Bool {
            owner.needsLocalMetadataRefresh(sessionID: session ?? ownerID, runID: run,
                expectedRunID: "run", currentGeneration: generation, projection: value)
        }
        XCTAssertTrue(needsRefresh(nil)); XCTAssertTrue(needsRefresh(initial))
        XCTAssertFalse(needsRefresh(initial, session: UUID().uuidString))
        XCTAssertFalse(needsRefresh(initial, run: "other-child-run"))
        XCTAssertFalse(needsRefresh(initial, generation: 8))
        let foreignOwner = IOSCommittedAssistantOwner(conversationID: ownerID, assistantMessageID: "other-message", generation: 7)
        XCTAssertFalse(foreignOwner.needsLocalMetadataRefresh(sessionID: ownerID, runID: "run",
            expectedRunID: "run", currentGeneration: 7, projection: initial))

        workspace.applyJournalContent(.init(cursor: .init(identity: .init(source_store_epoch: "epoch",
            conversation_id: ownerID, run_id: "run", assistant_message_id: "same-id"), sequence: "12",
            record_digest: String(repeating: "a", count: 64)), parts: [
                .reasoning(.init(partID: "reasoning", revision: 1, state: .running, title: "Thinking", summary: "Actual live public reasoning"))]), owner: ownerID)
        XCTAssertEqual(workspace.rows.first?.message.metadata[ChatConversationSurfaceMeta.stateKey], "pending")
        let running = try snapshot(projection(ownerID, revision: 2, role: "assistant", markdown: "", state: "running"), sequence: 13)
        workspace.apply(owner: ownerID, sequence: running.eventSequence, updates: [running.message],
            run: running.activeRun, surface: running.surface)
        XCTAssertEqual(workspace.rows.first?.message.metadata[ChatConversationSurfaceMeta.stateKey], "running")
        XCTAssertEqual(workspace.rows.first?.projected.state, .running)
        XCTAssertEqual(workspace.rows.first?.message.thinking, "Actual live public reasoning")
        XCTAssertFalse(needsRefresh(running), "Already-running metadata must not be reread on every body delta")
        workspace.apply(owner: ownerID, sequence: initial.eventSequence, updates: [initial.message],
            run: initial.activeRun, surface: initial.surface)
        XCTAssertEqual(workspace.rows.first?.message.metadata[ChatConversationSurfaceMeta.stateKey], "running")
    }

    private func install(_ workspace: IOSAgentRoomWorkspace, participants: [IOSRoomParticipant],
                         roomID: String = "room", revision: Int64 = 1) throws {
        let projections = try Dictionary(uniqueKeysWithValues: participants.map {
            ($0.contextConversationID, try projection($0.contextConversationID))
        })
        try workspace.install(.init(revision: revision, room: .init(id: roomID, title: "Room", participants: participants)),
            projections: projections, eventSequences: projections.mapValues { _ in 10 })
    }

    func testRoomTranscriptRendersCompactAndWideInBothAppearances() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("room-visual-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        let workspace = IOSAgentRoomWorkspace()
        let a = participant("研究助手 · 跨语言资料核查"), b = participant("写作助手 · 结构与表达")
        let projections = [
            a.contextConversationID: try projection(a.contextConversationID, role: "assistant",
                markdown: "## 已核对资料\n这是一条隔离测试回复，不是模型生成。\n\n- 保留每位成员的独立上下文\n- 消息过程默认折叠，授权操作单独呈现"),
            b.contextConversationID: try projection(b.contextConversationID, role: "assistant",
                markdown: "建议先说明结论，再列出证据。A longer English sentence verifies wrapping at compact widths without changing the conversation owner.")]
        try workspace.install(.init(revision: 1, room: .init(id: "visual-room", title: "青山讨论室", participants: [a, b])),
            projections: projections, eventSequences: projections.mapValues { _ in 1 })
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for width in [CGFloat(390), CGFloat(834)] {
            for dark in [false, true] {
                let view = NavigationStack {
                    IOSAgentRoomTimeline(workspace: workspace).navigationTitle("青山讨论室")
                }.environmentObject(container).environment(\.locale, Locale(identifier: "zh-Hans"))
                    .environment(\.colorScheme, dark ? .dark : .light)
                let controller = UIHostingController(rootView: view)
                let window = UIWindow(windowScene: scene)
                window.frame = CGRect(x: 0, y: 0, width: width, height: width == 390 ? 844 : 1112)
                window.overrideUserInterfaceStyle = dark ? .dark : .light
                window.rootViewController = controller
                window.makeKeyAndVisible()
                controller.view.frame = window.bounds
                controller.view.layoutIfNeeded()
                RunLoop.main.run(until: Date().addingTimeInterval(0.2))
                let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                    window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
                }
                XCTAssertGreaterThan(try XCTUnwrap(image.pngData()).count, 20_000)
                let attachment = XCTAttachment(image: image)
                attachment.name = "room-transcript-\(Int(width))-\(dark ? "dark" : "light")"
                attachment.lifetime = .keepAlways
                add(attachment)
                window.isHidden = true
                window.rootViewController = nil
            }
        }
    }

    func testJournalContentStreamsWithoutMetadataRevisionAndRejectsStaleOwners() throws {
        let workspace = IOSAgentRoomWorkspace(), a = participant("A")
        let owner = a.contextConversationID
        let projection = try projection(owner, role: "assistant", markdown: "")
        try workspace.install(.init(revision: 1, room: .init(id: "room", title: "Room", participants: [a])),
            projections: [owner: projection], eventSequences: [owner: 10])
        func content(_ sequence: String, _ text: String, conversation: String? = nil) -> IOSJournalMessageContent {
            .init(cursor: .init(identity: .init(source_store_epoch: "epoch", conversation_id: conversation ?? owner,
                run_id: "run", assistant_message_id: "same-id"), sequence: sequence,
                record_digest: String(repeating: "a", count: 64)), parts: [
                    .reasoning(.init(partID: "reasoning", revision: 1, state: .running, title: "Thinking", summary: text)),
                    .text(.init(partID: "text", revision: 1, markdown: text, isFinal: false, review: nil))])
        }
        workspace.applyJournalContent(content("11", "first"), owner: owner)
        XCTAssertEqual(workspace.rows.first?.message.content, "first")
        XCTAssertEqual(workspace.rows.first?.message.thinking, "first")
        workspace.applyJournalContent(content("12", "first second"), owner: owner)
        XCTAssertEqual(workspace.rows.first?.projected.revision, 1)
        XCTAssertEqual(workspace.rows.first?.message.content, "first second")
        workspace.applyJournalContent(content("11", "stale"), owner: owner)
        workspace.applyJournalContent(content("13", "foreign", conversation: UUID().uuidString), owner: owner)
        XCTAssertEqual(workspace.rows.first?.message.thinking, "first second")
    }

    func testCombinedRowsPreserveIndependentContextIdentity() throws {
        let workspace = IOSAgentRoomWorkspace(), a = participant("A"), b = participant("B")
        try install(workspace, participants: [a, b])
        XCTAssertEqual(workspace.rows.count, 2)
        XCTAssertEqual(Set(workspace.rows.map(\.id)).count, 2)
        XCTAssertEqual(Set(workspace.rows.map(\.participant.contextConversationID)),
                       Set([a.contextConversationID, b.contextConversationID]))
    }

    func testEqualTimestampPreservesCanonicalContextOrderRatherThanRandomIDs() throws {
        let workspace = IOSAgentRoomWorkspace(), a = participant("A"), b = participant("B")
        let projections = [a.contextConversationID: try projection(a.contextConversationID, messageIDs: ["z-user", "a-reply"]),
                           b.contextConversationID: try projection(b.contextConversationID, messageIDs: ["y-user", "b-reply"])]
        try workspace.install(.init(revision: 1, room: .init(id: "room", title: "Room", participants: [a, b])),
            projections: projections, eventSequences: projections.mapValues { _ in 10 })
        XCTAssertEqual(workspace.rows.filter { $0.participant.id == a.id }.map(\.projected.messageID), ["z-user", "a-reply"])
        XCTAssertEqual(workspace.rows.filter { $0.participant.id == b.id }.map(\.projected.messageID), ["y-user", "b-reply"])
    }

    func testDeactivationPreservesHistoryAndRevokesRecipientAndStaleRoomCannotRestoreIt() throws {
        let workspace = IOSAgentRoomWorkspace(), a = participant("A")
        try install(workspace, participants: [a])
        let inactive = participant("A", active: false, owner: a.contextConversationID)
        try install(workspace, participants: [inactive], revision: 2)
        XCTAssertEqual(workspace.rows.count, 1)
        XCTAssertFalse(try XCTUnwrap(workspace.room?.room.participants.first).active)
        try install(workspace, participants: [a], revision: 1)
        XCTAssertEqual(workspace.room?.revision, 2)
        XCTAssertFalse(try XCTUnwrap(workspace.room?.room.participants.first).active)
    }

    func testLateOrderedJournalPlacesUserBeforeAlreadyArrivedAssistantWithoutRegressingTokens() throws {
        let workspace = IOSAgentRoomWorkspace(), a = participant("A")
        try install(workspace, participants: [a])
        let token = try projection(a.contextConversationID, revision: 3, messageIDs: ["assistant"])
        workspace.apply(owner: a.contextConversationID, sequence: 13, updates: token.messages,
            run: nil, surface: token.surface)
        let window = try projection(a.contextConversationID, revision: 2, messageIDs: ["user", "assistant"])
        workspace.apply(owner: a.contextConversationID, sequence: 12, updates: window.messages,
            run: nil, surface: window.surface, orderedWindow: true)
        XCTAssertEqual(workspace.rows.map(\.projected.messageID), ["same-id", "user", "assistant"])
        XCTAssertEqual(workspace.rows.last?.projected.revision, 3)
    }

    func testOldRoomAndOldSequenceUpdatesCannotPolluteCurrentTimeline() throws {
        let workspace = IOSAgentRoomWorkspace(), a = participant("A"), b = participant("B")
        try install(workspace, participants: [a])
        let update = try projection(a.contextConversationID, revision: 2)
        workspace.apply(owner: a.contextConversationID, sequence: 9, updates: update.messages,
            run: nil, surface: update.surface)
        XCTAssertEqual(workspace.rows.first?.projected.revision, 1)
        try install(workspace, participants: [b], roomID: "other-room")
        workspace.apply(owner: a.contextConversationID, sequence: 99, updates: update.messages,
            run: nil, surface: update.surface)
        XCTAssertEqual(workspace.rows.map(\.participant.id), [b.id])
        XCTAssertEqual(workspace.room?.room.id, "other-room")
    }

    func testForeignProjectionIsRejectedWithoutReplacingCurrentRoom() throws {
        let workspace = IOSAgentRoomWorkspace(), a = participant("A"), b = participant("B")
        try install(workspace, participants: [a])
        XCTAssertThrowsError(try workspace.install(.init(revision: 2,
            room: .init(id: "room", title: "Room", participants: [a])),
            projections: [a.contextConversationID: projection(b.contextConversationID)],
            eventSequences: [a.contextConversationID: 11]))
        XCTAssertEqual(workspace.room?.revision, 1)
        XCTAssertEqual(workspace.rows.map(\.participant.id), [a.id])
    }

    func testBackgroundExecutionCreationDoesNotChangeSelectionOrAliasOwners() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-room-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let container = ServiceContainer(applicationRuntime: runtime)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        container.conversationStore = store
        let a = try store.createConversation(title: "A", language: nil)
        let b = try store.createConversation(title: "B", language: nil)
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.currentConversationId = a.id
        let executionA = try XCTUnwrap(manager.executionForConversation(a.id))
        let executionB = try XCTUnwrap(manager.ensureExecutionForConversation(b.id))
        XCTAssertEqual(manager.currentConversationId, a.id)
        XCTAssertEqual(executionA.currentConversationId, a.id)
        XCTAssertEqual(executionB.currentConversationId, b.id)
        XCTAssertFalse(executionA === executionB)
        XCTAssertTrue(manager.ensureExecutionForConversation(b.id) === executionB)
        XCTAssertNil(manager.ensureExecutionForConversation(UUID()))
        XCTAssertEqual(manager.currentConversationId, a.id)
    }

    func testSubmissionFailureRemainsContextOwnedAcrossCompletionAndRetry() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-room-failure-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let container = ServiceContainer(applicationRuntime: runtime)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        container.conversationStore = store
        let a = try store.createConversation(title: "A", language: nil)
        let b = try store.createConversation(title: "B", language: nil)
        let manager = ChatManager(container: container, restorePersistedState: false)
        let executionA = try XCTUnwrap(manager.ensureExecutionForConversation(a.id))
        let executionB = try XCTUnwrap(manager.ensureExecutionForConversation(b.id))
        executionA.recordSubmissionFailure(IOSApplicationRuntimeError.rejected("A admission rejected"))
        XCTAssertNotNil(executionA.submissionFailure)
        XCTAssertNil(executionB.submissionFailure)
        let failure = executionA.submissionFailure
        executionA.processingState = .completed
        XCTAssertEqual(executionA.submissionFailure, failure, "Old terminal presentation cannot erase a new submission failure")
        executionB.recordSubmissionFailure(IOSApplicationRuntimeError.rejected("B admission rejected"))
        let failureB = executionB.submissionFailure
        executionA.beginSubmissionAttempt()
        XCTAssertNil(executionA.submissionFailure)
        XCTAssertEqual(executionB.submissionFailure, failureB)
        executionA.recordSubmissionFailure(CancellationError())
        XCTAssertNil(executionA.submissionFailure)
        XCTAssertFalse(executionA.processingState.blocksInput, "Failure feedback must not block retry")
    }

    func testExitedRoomMemberPermissionAndCancellationRemainBoundToItsExecution() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-room-permission-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let container = ServiceContainer(applicationRuntime: runtime)
        let service = IOSAgentRoomService(runtime: runtime)
        var directory = try service.createAgent(name: "A", description: "", defaults: .object([:]), directory: service.directory())
        directory = try service.createAgent(name: "B", description: "", defaults: .object([:]), directory: directory)
        var room = try service.create(title: "Permission owners")
        for agent in directory.agents { room = try service.join(agent: agent, room: room) }
        let a = room.room.participants[0], b = room.room.participants[1]
        let ownerA = try XCTUnwrap(UUID(uuidString: a.contextConversationID))
        let ownerB = try XCTUnwrap(UUID(uuidString: b.contextConversationID))
        let manager = ChatManager(container: container, restorePersistedState: false)
        let executionA = try XCTUnwrap(manager.ensureExecutionForConversation(ownerA))
        let executionB = try XCTUnwrap(manager.ensureExecutionForConversation(ownerB))
        let requestA = HostAgentPermissionRequest(id: "same-request", allowConversation: true,
            toolName: "fixture", match: "A", nonce: "nonce-a")
        let requestB = HostAgentPermissionRequest(id: "same-request", allowConversation: true,
            toolName: "fixture", match: "B", nonce: "nonce-b")
        var decisionsA = 0, decisionsB = 0
        let waitA = executionA.startHostPermissionInteraction(requestA, conversationId: ownerA,
            isCurrent: { true }, send: { _ in decisionsA += 1 })
        let waitB = executionB.startHostPermissionInteraction(requestB, conversationId: ownerB,
            isCurrent: { true }, send: { _ in decisionsB += 1 })
        defer {
            waitA.cancel(); waitB.cancel()
            executionA.cancelHostPermissionInteraction(); executionB.cancelHostPermissionInteraction()
        }
        for _ in 0..<1_000 where executionA.pendingHostPermissionRequest == nil || executionB.pendingHostPermissionRequest == nil {
            await Task.yield()
        }
        XCTAssertNotNil(executionA.pendingHostPermissionRequest)
        XCTAssertNotNil(executionB.pendingHostPermissionRequest)
        let exited = try service.deactivate(participantID: a.id, room: room)
        XCTAssertFalse(try XCTUnwrap(exited.room.participants.first { $0.id == a.id }).active)
        manager.currentConversationId = ownerB
        executionA.permissionDecisionHandler(localRequest: nil, hostRequest: requestA)(requestA.id, .deny)
        for _ in 0..<1_000 where decisionsA == 0 { await Task.yield() }
        XCTAssertEqual(decisionsA, 1)
        XCTAssertEqual(decisionsB, 0)
        XCTAssertEqual(executionB.pendingHostPermissionRequest?.nonce, "nonce-b")
        executionA.processingState = .processingWithCloud
        executionB.processingState = .processingWithCloud
        executionA.cancelCurrentGeneration()
        XCTAssertTrue(executionB.processingState.blocksInput)
        XCTAssertEqual(executionB.pendingHostPermissionRequest?.nonce, "nonce-b")
        XCTAssertEqual(manager.currentConversationId, ownerB)
    }
}
