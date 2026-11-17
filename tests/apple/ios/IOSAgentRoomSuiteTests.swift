// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS agent room history, service, workspace, composer interactions, full-page layout, recovery boundaries, and submission workspace suite.

import Foundation
import SwiftUI
import UIKit
import XCTest
@testable import Nola

@MainActor final class IOSAgentRoomHistoryTests: XCTestCase {
    func testCanonicalOwnerSurvivesReopenWithoutLeakingIntoOrdinaryHistoryOrClientEncoding() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-room-history-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("nola.sqlite")
        var roomID = ""
        var contextID = UUID()
        var ordinaryID = UUID()
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let store = try LocalConversationStore(applicationRuntime: runtime)
            ordinaryID = try store.createConversation(title: "Ordinary", language: nil).id
            let service = IOSAgentRoomService(runtime: runtime)
            let directory = try service.createAgent(name: "Research", description: "", defaults: .object([:]), directory: service.directory())
            let room = try service.join(agent: XCTUnwrap(directory.agents.first), room: service.create(title: "Discussion"))
            roomID = room.room.id
            contextID = try XCTUnwrap(UUID(uuidString: XCTUnwrap(room.room.participants.first).contextConversationID))
            let all = try store.getAllConversations()
            XCTAssertEqual(Set(all.map(\.id)), Set([ordinaryID, contextID]))
            XCTAssertEqual(all.filter(\.appearsInOrdinaryHistory).map(\.id), [ordinaryID])
            XCTAssertEqual(ChatManager.coldStartConversation(from: all,
                fallbackModel: nil, activeRun: { _ in nil })?.id, ordinaryID)
            let context = try XCTUnwrap(all.first { $0.id == contextID })
            XCTAssertEqual(context.agentRoomOwner?.roomID, roomID)
            XCTAssertEqual(context.agentRoomOwner?.agentID, directory.agents.first?.id)
            let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(context)) as? [String: Any])
            XCTAssertNil(encoded["agentRoomOwner"])
        }
        let reopened = try LocalConversationStore(applicationRuntime: IOSApplicationRuntime(databaseURL: database))
        let all = try reopened.getAllConversations()
        XCTAssertEqual(all.first { $0.id == contextID }?.agentRoomOwner?.roomID, roomID)
        XCTAssertEqual(all.filter(\.appearsInOrdinaryHistory).map(\.id), [ordinaryID])
    }

    func testMalformedOwnershipIsRejectedRatherThanExposedAsOrdinaryConversation() {
        XCTAssertThrowsError(try ConversationAgentRoomOwner.read(NSNull()))
        XCTAssertThrowsError(try ConversationAgentRoomOwner.read(["room_id": "invalid"]))
        XCTAssertThrowsError(try ConversationAgentRoomOwner.read([
            "room_id": UUID().uuidString, "participant_id": UUID().uuidString,
            "agent_id": "agent", "old_alias": "unsupported"
        ]))
    }
}

extension IOSAgentRoomHistoryTests {
    func testChildDirectoryRetainsRejoinedHistoryAndDoesNotCreateUnopenedRoom() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("expert-child-directory-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let service = IOSAgentRoomService(runtime: runtime)
        let parent = try store.createConversation(title: "Main conversation", language: nil)
        let untouched = try service.childConversations(conversationID: parent.id.uuidString)
        XCTAssertTrue(untouched.children.isEmpty); XCTAssertNil(untouched.roomID)
        XCTAssertNil(try service.forConversation(parent.id.uuidString), "Reading an unopened branch must not create a room")
        let agents = try service.createAgent(name: "Expert", description: "Original", defaults: .object([:]), directory: service.directory())
        let originalAgent = try XCTUnwrap(agents.agents.first)
        let oldRoom = try service.joinMention(agentID: originalAgent.id, expectedAgentRevision: originalAgent.revision,
            conversationID: parent.id.uuidString)
        let oldMember = try XCTUnwrap(oldRoom.room.room.participants.first)
        let updated = try service.updateAgent(originalAgent, name: "Expert", description: "Updated",
            availability: "active", directory: service.directory())
        let newAgent = try XCTUnwrap(updated.agents.first)
        let newRoom = try service.rejoinMention(agentID: newAgent.id, expectedAgentRevision: newAgent.revision,
            conversationID: parent.id.uuidString)
        let newMember = try XCTUnwrap(newRoom.room.room.participants.first)
        let first = try service.childConversations(conversationID: parent.id.uuidString, limit: 1)
        XCTAssertEqual(first.children.count, 1); XCTAssertTrue(first.hasMore)
        let second = try service.childConversations(conversationID: parent.id.uuidString, limit: 1, offset: try XCTUnwrap(first.nextOffset))
        let all = first.children + second.children
        XCTAssertEqual(Set(all.map(\.conversationID)), [oldMember.contextConversationID, newMember.contextConversationID])
        let old = try XCTUnwrap(all.first { $0.conversationID == oldMember.contextConversationID })
        let current = try XCTUnwrap(all.first { $0.conversationID == newMember.contextConversationID })
        XCTAssertFalse(old.isCurrent); XCTAssertTrue(current.isCurrent)
        XCTAssertEqual(current.agentRevision, newMember.agentRevision)
        XCTAssertEqual(try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime).map(\.id), [parent.id], "Neither old nor current expert context is top-level")
        XCTAssertEqual(try service.forConversation(parent.id.uuidString)?.revision, newRoom.revision, "Read-only child enumeration cannot mutate the association")
        XCTAssertNil(try runtime.loadConversationProjectionV1(id: old.conversationID).activeRun)
        XCTAssertNil(try runtime.loadConversationProjectionV1(id: current.conversationID).activeRun)
        let oldHistory = try service.childHistory(conversationID: parent.id.uuidString, childID: old.id)
        let currentHistory = try service.childHistory(conversationID: parent.id.uuidString, childID: current.id)
        XCTAssertEqual(oldHistory.window.projection.conversationID, old.id)
        XCTAssertEqual(currentHistory.sourceOwner, .local)
        XCTAssertTrue(currentHistory.expertExchanges.isEmpty)
        XCTAssertThrowsError(try service.childHistory(conversationID: UUID().uuidString, childID: current.id))
        let container = ServiceContainer(applicationRuntime: runtime)
        let currentID = try XCTUnwrap(UUID(uuidString: current.conversationID))
        XCTAssertNil(container.chatManager.executionForConversation(currentID))
        let selectedBeforeRead = container.chatManager.currentConversationId
        let readOnly = IOSAgentRoomWorkspace()
        let snapshot = IOSAgentRoomSnapshot(revision: newRoom.room.revision,
            room: .init(id: newRoom.roomID, title: "Current child", participants: [newMember]))
        try readOnly.installReadOnlyHistory(snapshot, window: currentHistory.window)
        XCTAssertNil(container.chatManager.executionForConversation(currentID), "Reading child history must not allocate an execution owner")
        XCTAssertEqual(container.chatManager.currentConversationId, selectedBeforeRead)
        readOnly.close()
    }
}

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

@MainActor
extension IOSAgentRoomServiceTests {
    func testBuiltInRoleCatalogComesFromCoreAndCanConfigureAnExpert() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("expert-roles-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let service = IOSAgentRoomService(runtime: runtime)
        let catalog = try service.roleTemplates()
        XCTAssertEqual(catalog.version, 1)
        XCTAssertTrue(catalog.features.reasoningEffort)
        XCTAssertTrue(catalog.features.explicitGenerationReasoning)
        XCTAssertTrue(catalog.features.rejoin)
        XCTAssertEqual(catalog.templates.count, 7)
        XCTAssertEqual(Set(catalog.templates.map(\.id)).count, 7)
        for role in catalog.templates {
            XCTAssertNotNil(UUID(uuidString: role.id))
            XCTAssertFalse(role.localizedName(locale: Locale(identifier: "zh-Hans")).isEmpty)
            XCTAssertFalse(role.localizedName(locale: Locale(identifier: "en")).isEmpty)
            XCTAssertFalse(role.prompt.isEmpty)
            XCTAssertFalse(role.toolPack.tools.isEmpty)
            XCTAssertFalse(role.skills.isEmpty)
            let directory = try service.createAgent(name: role.localizedName(locale: Locale(identifier: "en")),
                description: role.localizedDescription(), defaults: .object(["selectedPromptTemplateID": .string(role.id)]),
                directory: service.directory())
            let saved = try XCTUnwrap(directory.agents.first { $0.defaults == .object(["selectedPromptTemplateID": .string(role.id)]) })
            XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).templateID, role.id)
        }
    }

    func testMentionJoinsDirectoryExpertByIdentityAndRejectsArchiveOrChangedRevision() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("expert-mention-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let service = IOSAgentRoomService(runtime: runtime)
        _ = try service.createAgent(name: "Same name", description: "First", defaults: .object([:]), directory: service.directory())
        let directory = try service.createAgent(name: "Same name", description: "Second", defaults: .object([:]), directory: service.directory())
        let selected = try XCTUnwrap(directory.agents.first { $0.description == "Second" })
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "Mention owner", language: nil)
        let joined = try service.joinMention(agentID: selected.id, expectedAgentRevision: selected.revision,
            conversationID: conversation.id.uuidString)
        XCTAssertEqual(joined.room.room.participants.filter(\.active).map(\.agentID), [selected.id])
        let same = try service.joinMention(agentID: selected.id, expectedAgentRevision: selected.revision,
            conversationID: conversation.id.uuidString)
        XCTAssertEqual(same.room.room.participants.count, 1, "Repeated selection must not add a duplicate participant")
        let updated = try service.updateAgent(selected, name: selected.name, description: "Changed",
            availability: "active", directory: service.directory())
        XCTAssertThrowsError(try service.validateMention(agentID: selected.id, expectedAgentRevision: selected.revision))
        let current = try XCTUnwrap(updated.agents.first { $0.id == selected.id })
        XCTAssertThrowsError(try service.joinMention(agentID: current.id, expectedAgentRevision: current.revision,
            conversationID: conversation.id.uuidString)) { error in
            guard case IOSExpertMentionFailure.requiresRejoin = error else { return XCTFail("Expected explicit rejoin confirmation") }
        }
        let renewed = try service.rejoinMention(agentID: current.id, expectedAgentRevision: current.revision,
            conversationID: conversation.id.uuidString)
        XCTAssertEqual(renewed.room.room.participants.filter(\.active).map(\.agentRevision), [current.revision])
        XCTAssertEqual(renewed.room.room.participants.count, 1, "The published room format keeps one roster item per expert")
        let oldParticipant = try XCTUnwrap(joined.room.room.participants.first)
        XCTAssertFalse(renewed.room.room.participants.contains { $0.id == oldParticipant.id })
        let oldContext = try XCTUnwrap(UUID(uuidString: oldParticipant.contextConversationID))
        XCTAssertNotNil(try store.getConversation(conversationId: oldContext), "Explicit rejoin must retain historical context")
        _ = try service.updateAgent(current, name: current.name, description: current.description,
            availability: "archived", directory: service.directory())
        XCTAssertThrowsError(try service.joinMention(agentID: current.id, expectedAgentRevision: current.revision,
            conversationID: conversation.id.uuidString))
    }

    func testPendingMentionRevisionRoundTripsWithoutChangingLegacyDraftDecode() throws {
        let old = Data(#"{"conversationID":"owner","input":null,"expertIDs":["agent"]}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(IOSAgentRoomService.ExpertAdmissionDraft.self, from: old).expertRevisions)
        let draft = IOSAgentRoomService.ExpertAdmissionDraft(conversationID: "owner", input: nil,
            expertIDs: ["agent"], expertRevisions: ["agent": 7])
        let restored = try JSONDecoder().decode(IOSAgentRoomService.ExpertAdmissionDraft.self, from: JSONEncoder().encode(draft))
        XCTAssertEqual(restored.expertRevisions, ["agent": 7])
    }
    func testUnknownCatalogFeaturesNeverAuthorizeRejoinMutation() throws {
        let response = try JSONDecoder().decode(AgentCoreJSONValue.self,
            from: Data(#"{"version":1,"templates":[]}"#.utf8))
        let service = IOSAgentRoomService(execute: { envelope in
            guard case .object(let root) = envelope, case .object(let command) = root["command"] else {
                throw CocoaError(.coderInvalidValue)
            }
            XCTAssertEqual(command["type"], .string("role_templates"), "Unsupported rejoin must stop at observation")
            return response
        })
        XCTAssertFalse(try service.roleTemplates().features.rejoin)
        XCTAssertThrowsError(try service.rejoinMention(agentID: "agent", expectedAgentRevision: 1, conversationID: "owner"))
    }

    func testJoinedSnapshotPinsMainSelectionAndDirectFanoutRequiresMainTurn() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("expert-old-snapshot-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture", "endpointName": "Fixture",
            "baseURL": "https://fixture.invalid/v1", "endpointOrder": 0, "endpointEnabled": true,
            "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
            "wireProtocol": "responses", "credentialAccount": "test-only-account",
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true, "samplingParameters": true],
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        _ = try runtime.upsertModelProfileCAS(.init(id: "ios-native-fixture", displayName: "Fixture", provider: "openai-compatible",
            modelName: "fixture", runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil),
            expectedRevision: 0)
        let service = IOSAgentRoomService(runtime: runtime)
        let role = try XCTUnwrap(service.roleTemplates().templates.first)
        let defaults: AgentCoreJSONValue = .object(["modelSelectionID": .string("configured:ios-native-fixture"),
            "selectedPromptTemplateID": .string(role.id), "expertReasoningEffort": .string("high")])
        let directory = try service.createAgent(name: "Old snapshot", description: "Original", defaults: defaults, directory: service.directory())
        let expert = try XCTUnwrap(directory.agents.first)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "Snapshot continuity", language: nil)
        let joined = try service.joinMention(agentID: expert.id, expectedAgentRevision: expert.revision,
            conversationID: conversation.id.uuidString)
        let original = try XCTUnwrap(joined.room.room.participants.first)
        _ = try service.updateAgent(expert, name: expert.name, description: "New directory configuration", availability: "active",
            directory: service.directory(), defaults: .object(["modelSelectionID": .string("configured:ios-native-fixture"),
                "expertReasoningEffort": .string("low")]))
        let workspace = IOSRoomSubmissionWorkspace(service: service)
        let message = UUID().uuidString.lowercased()
        let admitted = await workspace.sendExperts(conversationID: conversation.id.uuidString, text: "Continue the old snapshot",
            agentIDs: [expert.id], observedRevisions: [expert.id: expert.revision], draftTicket: nil, messageID: message)
        XCTAssertNil(admitted)
        XCTAssertTrue(workspace.admissionError?.contains("requires_main_agent_turn") == true)
        XCTAssertNil(try service.expertManifest(conversationID: conversation.id.uuidString, messageID: message))
        let selected = try AppleMainExpertSelectionContext.capture(agentIDs: [expert.id], observedRevisions: [expert.id: expert.revision], joined: [
            .init(participant_id: original.id, agent_id: original.agentID, agent_revision: original.agentRevision, context_conversation_id: original.contextConversationID)])
        XCTAssertEqual(selected.selectedExperts.first?.agentRevision, original.agentRevision)
        XCTAssertEqual(selected.selectedExperts.first?.contextConversationID, original.contextConversationID)
        XCTAssertEqual(try service.forConversation(conversation.id.uuidString)?.room.room.participants.first?.id, original.id)
        XCTAssertTrue((workspace.submissions[joined.roomID] ?? []).allSatisfy { $0.state != "executing" },
            "No execution owner was installed; this test must admit only, never run inference")
    }

    func testSparseCanonicalParametersRetainDeclaredEffortsAndExplicitValues() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("expert-sparse-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let source: [String: Any] = ["baseURL": "https://fixture.invalid/v1", "wireProtocol": "responses",
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true, "samplingParameters": true],
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        _ = try runtime.upsertModelProfileCAS(.init(id: "sparse-effort", displayName: "Sparse", provider: "openai-compatible",
            modelName: "fixture", runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: #"{"temperature":0,"seed":null}"#, selected: false, enabled: true,
            revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil), expectedRevision: 0)
        let parameters = ModelParametersStore(applicationPort: runtime)
        let snapshot = try parameters.parameterSnapshot(for: "sparse-effort")
        XCTAssertEqual(try parameters.reasoningEfforts(for: snapshot), ["low", "high"])
        XCTAssertEqual(snapshot.parameters.temperature, 0)
        XCTAssertNil(snapshot.parameters.seed)
        XCTAssertEqual(snapshot.parameters.reasoningPreference, ModelParameters.webAlignedCloudDefaults.reasoningPreference)
        XCTAssertNotEqual(snapshot.parameters.reasoningPreference, "none", "Missing reasoning cannot invent an explicit none")
        XCTAssertEqual(parameters.loadParameters(for: "sparse-effort", isLocal: false).temperature, 0)
    }

    func testUnsavedMentionObservationDoesNotCreateAConversationOrRoom() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("expert-observe-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let service = IOSAgentRoomService(runtime: runtime)
        let directory = try service.createAgent(name: "Observe", description: "", defaults: .object([:]), directory: service.directory())
        let expert = try XCTUnwrap(directory.agents.first)
        XCTAssertEqual(try service.validateMention(agentID: expert.id, expectedAgentRevision: expert.revision).id, expert.id)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        XCTAssertTrue(try store.getAllConversations().isEmpty)
        XCTAssertTrue(try service.list().isEmpty)
    }

}

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

@MainActor final class IOSRoomFullPageTests: XCTestCase {
    private func settle() async throws { try await Task.sleep(for: .milliseconds(200)) }
    private func input(in view: UIView) -> IOSRoomComposerTextView? {
        if let input = view as? IOSRoomComposerTextView { return input }
        return view.subviews.lazy.compactMap { self.input(in: $0) }.first
    }
    private func scrollViews(in view: UIView) -> [UIScrollView] {
        ((view as? UIScrollView).map { [$0] } ?? [])
            + view.subviews.flatMap { scrollViews(in: $0) }
    }

    func testFullRoomCandidatesPermissionProcessingAndRoomSwitch() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("room-full-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
        defer {
            if FileManager.default.fileExists(atPath: root.path) { try? FileManager.default.removeItem(at: root) }
        }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)
        let service = IOSAgentRoomService(runtime: runtime)
        let directory = try service.createAgent(name: "研究助手 · 资料核查", description: "核对资料与引用，不访问其他成员私有上下文",
            defaults: .object([:]), directory: service.directory())
        let first = try XCTUnwrap(directory.agents.first)
        let all = try service.createAgent(name: "写作助手 · 结构表达", description: "整理结论并检查表达",
            defaults: .object([:]), directory: directory)
        let second = try XCTUnwrap(all.agents.first { $0.id != first.id })
        let a = try service.join(agent: second, room: service.join(agent: first, room: service.create(title: "青山讨论室")))
        let b = try service.join(agent: first, room: service.create(title: "另一间独立讨论室"))
        let workspace = IOSAgentRoomWorkspace()
        try workspace.open(a, container: container)
        let member = try XCTUnwrap(a.room.participants.first)
        _ = try service.submit(.init(roomID: a.room.id, submissionID: UUID().uuidString.lowercased(),
            expectedRoomRevision: a.revision, text: "隔离样本：请比较两份设计说明。未调用模型，也不会执行工具。",
            participantID: member.id, atMs: 1))
        // No executor or routing provider is installed: sends persist a receipt only.
        let submission = IOSRoomSubmissionWorkspace(service: service)
        await submission.select(a)
        let execution = try XCTUnwrap(container.chatManager.ensureExecutionForConversation(
            try XCTUnwrap(UUID(uuidString: member.contextConversationID))))
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for width in [CGFloat(390), CGFloat(834)] {
            for dark in [false, true] {
                let view = NavigationStack {
                    IOSRoomConversationView(workspace: workspace, submission: submission)
                        .navigationTitle("青山讨论室").navigationBarTitleDisplayMode(.inline)
                }.environmentObject(container).environment(\.locale, Locale(identifier: "zh-Hans"))
                    .environment(\.colorScheme, dark ? .dark : .light)
                let controller = UIHostingController(rootView: view)
                let window = UIWindow(windowScene: scene)
                window.frame = CGRect(x: 0, y: 0, width: width, height: width == 390 ? 844 : 1112)
                window.overrideUserInterfaceStyle = dark ? .dark : .light
                window.rootViewController = controller
                window.makeKeyAndVisible()
                controller.view.frame = window.bounds
                try await settle()
                submission.edit("请核对这份隔离样本 @")
                try await settle()
                let composer = try XCTUnwrap(input(in: controller.view))
                XCTAssertTrue(composer.mentionsVisible)
                XCTAssertGreaterThanOrEqual(composer.bounds.height, 64)
                XCTAssertTrue(window.bounds.contains(composer.convert(composer.bounds, to: window)))
                capture(window, name: "room-full-candidates-\(Int(width))-\(dark ? "dark" : "light")")
                // Exercise the real mounted UITextView command and SwiftUI binding.
                composer.chooseMention(UIKeyCommand(input: UIKeyCommand.inputDownArrow, modifierFlags: [], action: #selector(IOSRoomComposerTextView.chooseMention(_:))))
                composer.chooseMention(UIKeyCommand(input: "\t", modifierFlags: [], action: #selector(IOSRoomComposerTextView.chooseMention(_:))))
                try await settle()
                XCTAssertNotNil(submission.draft.participantID)
                XCTAssertNil(submission.draft.mentionQuery)
                composer.resignFirstResponder()
                window.endEditing(true)
                // Keyboard dismissal animates independently from SwiftUI state.
                try await Task.sleep(for: .milliseconds(600))
                execution.processingState = .streaming(progress: 0)
                execution.pendingHostPermissionRequest = HostAgentPermissionRequest(id: "fixture-permission", allowConversation: false,
                    toolName: "write_file", argsPreview: "path: fixture.txt; UTF-8 bytes: 8; isolated fixture only",
                    match: "fixture.txt", reason: "隔离页面验收：请确认是否允许写入示例文件。", message: "只展示授权，不执行工具。")
                try await settle()
                let permissionComposer = try XCTUnwrap(input(in: controller.view))
                XCTAssertGreaterThanOrEqual(permissionComposer.bounds.height, 64)
                XCTAssertTrue(window.bounds.contains(permissionComposer.convert(permissionComposer.bounds, to: window)),
                    "The input must remain inside the full page while a member needs authorization")
                capture(window, name: "room-full-permission-\(Int(width))-\(dark ? "dark" : "light")")
                let permissionScroll = try XCTUnwrap(scrollViews(in: controller.view).first {
                    abs($0.bounds.width - 300) < 1 && $0.contentSize.height > $0.bounds.height
                })
                let bottom = permissionScroll.contentSize.height - permissionScroll.bounds.height
                    + permissionScroll.adjustedContentInset.bottom
                permissionScroll.setContentOffset(CGPoint(x: 0, y: bottom), animated: false)
                try await settle()
                XCTAssertEqual(permissionScroll.contentOffset.y, bottom, accuracy: 1)
                capture(window, name: "room-full-permission-actions-\(Int(width))-\(dark ? "dark" : "light")")
                execution.pendingHostPermissionRequest = nil
                execution.processingState = .idle
                if width == 390 && !dark {
                    let previous = submission.submissions[a.room.id]?.count ?? 0
                    composer.sendFromKeyboard()
                    for _ in 0..<30 where (submission.submissions[a.room.id]?.count ?? 0) == previous { try await settle() }
                    XCTAssertEqual(submission.submissions[a.room.id]?.count, previous + 1)
                    XCTAssertEqual(submission.submissions[a.room.id]?.last?.input.text, "请核对这份隔离样本 ")
                    submission.edit("A 的独立草稿")
                    try workspace.open(b, container: container)
                    try await settle()
                    XCTAssertEqual(submission.selectedRoom?.room.id, b.room.id)
                    let otherComposer = try XCTUnwrap(input(in: controller.view))
                    XCTAssertEqual(otherComposer.text, "")
                    XCTAssertFalse(otherComposer.mentionsVisible)
                    XCTAssertFalse(workspace.room?.room.participants.contains { $0.contextConversationID == member.contextConversationID } ?? true)
                    submission.edit("B 的独立草稿")
                    try workspace.open(a, container: container)
                    try await settle()
                    XCTAssertEqual(try XCTUnwrap(input(in: controller.view)).text, "A 的独立草稿")
                    XCTAssertEqual(submission.drafts[b.room.id]?.text, "B 的独立草稿")
                }
                submission.removeMention()
                window.isHidden = true
                window.rootViewController = nil
            }
        }
        workspace.close()
        try await settle()
    }

    private func capture(_ window: UIWindow, name: String) {
        window.layoutIfNeeded()
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

@MainActor final class IOSRoomRecoveryBoundaryTests: XCTestCase {
    func testPersistedClaimReplaysOriginalIntentWithoutRecapturing() async throws {
        let fixture = IOSRoomBoundaryFixture()
        let workspace = IOSRoomSubmissionWorkspace(service: fixture.service) { receipt, intent in
            XCTAssertEqual(receipt.executionID, fixture.original.execution_id)
            XCTAssertEqual(intent, fixture.original)
            fixture.record("execute")
        }
        workspace.prepareOwner = { _ in XCTFail("A persisted intent must not recapture"); return fixture.original }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        let events = fixture.events
        XCTAssertFalse(events.contains("prepare_execution"))
        XCTAssertTrue(events.contains("claim_execution"))
        XCTAssertTrue(events.contains("execute"), "False claim permission still requires original-ID recovery after absent Run")
    }

    func testExistingRunOnlyRecoversAndNeverAdmitsAgain() async throws {
        let fixture = IOSRoomBoundaryFixture(existingRun: true)
        let workspace = IOSRoomSubmissionWorkspace(service: fixture.service) { _, _ in XCTFail("Existing Run must not dispatch") }
        workspace.prepareOwner = { _ in XCTFail("Must use persisted intent"); return fixture.original }
        workspace.recoverOwner = { receipt, intent in
            XCTAssertEqual(receipt.executionID, intent.execution_id)
            fixture.record("recover")
        }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        XCTAssertTrue(fixture.events.contains("recover"))
    }

    func testCancelledClaimCleansPreparedOwnerWithoutDispatch() async throws {
        let fixture = IOSRoomBoundaryFixture(claimState: "cancelled")
        let workspace = IOSRoomSubmissionWorkspace(service: fixture.service) { _, _ in XCTFail("Cancelled claim cannot dispatch") }
        workspace.prepareOwner = { _ in fixture.original }
        workspace.cancelPreparedOwner = { receipt in
            XCTAssertEqual(receipt.state, "cancelled")
            fixture.record("cleanup")
        }
        await workspace.startSelected(fixture.receipt(state: "selected"))
        XCTAssertTrue(fixture.events.contains("cleanup"))
        XCTAssertFalse(fixture.events.contains("load_exact_run_invocation"))
    }

    func testCancellationDuringPrepareCASFailureStillCleansFrozenRemoteRequest() async throws {
        let fixture = IOSRoomBoundaryFixture(persisted: false, prepareRejected: true)
        let workspace = IOSRoomSubmissionWorkspace(service: fixture.service) { _, _ in XCTFail("Failed prepare cannot dispatch") }
        workspace.prepareOwner = { _ in fixture.record("capture"); return fixture.original }
        workspace.cancelPreparedOwner = { _ in fixture.record("cleanup") }
        await workspace.startSelected(fixture.receipt(state: "selected"))
        XCTAssertEqual(fixture.events.filter { $0 == "capture" }.count, 1)
        XCTAssertTrue(fixture.events.contains("cleanup"))
        XCTAssertFalse(fixture.events.contains("claim_execution"))
    }

    func testLateReceiptCannotRegressCancellationOrOverwriteAnotherRoom() async throws {
        let fixture = IOSRoomBoundaryFixture()
        let workspace = IOSRoomSubmissionWorkspace(service: fixture.service)
        let newer = fixture.receipt(state: "cancelled", revision: 9)
        workspace.applyReceipt(newer)
        await workspace.select(.init(revision: 1, room: .init(id: "room-b", title: "B", participants: [])))
        workspace.applyReceipt(fixture.receipt(state: "selected", revision: 3))
        let other = fixture.receipt(state: "accepted", revision: 1, roomID: "room-b")
        workspace.applyReceipt(other)
        XCTAssertEqual(workspace.submissions["room-a"]?.first?.state, "cancelled")
        XCTAssertEqual(workspace.submissions["room-b"]?.first?.state, "accepted")
        let changed = fixture.receipt(state: "executing", revision: 10, text: "foreign input")
        workspace.applyReceipt(changed)
        XCTAssertEqual(workspace.submissions["room-a"]?.first?.input.text, "original")
        XCTAssertEqual(workspace.submissions["room-a"]?.first?.revision, 9)
        XCTAssertEqual(workspace.selectedRoom?.room.id, "room-b", "Late room A callback cannot retarget selection")
    }

    func testRemoteRecoveryUsesSourcePathNotLocalInvocationLookup() async throws {
        let fixture = IOSRoomBoundaryFixture(remote: true)
        let workspace = IOSRoomSubmissionWorkspace(service: fixture.service) { _, intent in
            XCTAssertEqual(intent, fixture.original)
            fixture.record("source-observe")
        }
        workspace.prepareOwner = { _ in XCTFail("Must not rebuild remote body"); return fixture.original }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        XCTAssertTrue(fixture.events.contains("source-observe"))
        XCTAssertFalse(fixture.events.contains("load_exact_run_invocation"))
    }
}

private nonisolated final class IOSRoomBoundaryFixture: @unchecked Sendable {
    let original: IOSAgentRoomService.ExecutionIntent
    let existingRun: Bool
    let claimState: String
    let persisted: Bool
    let prepareRejected: Bool
    private let lock = NSLock()
    private var recorded: [String] = []
    var events: [String] { lock.withLock { recorded } }
    func record(_ event: String) { lock.withLock { recorded.append(event) } }

    init(existingRun: Bool = false, claimState: String = "executing", persisted: Bool = true,
         prepareRejected: Bool = false, remote: Bool = false) {
        self.existingRun = existingRun; self.claimState = claimState
        self.persisted = persisted; self.prepareRejected = prepareRejected
        let id = "00000000-0000-4000-8000-000000000001"
        if remote {
            original = .init(execution_id: id, envelope: .remoteHost(.init(kind: "remote_host",
                identity: .init(source_host_id: "host", client_id: "paired", conversation_id: "context",
                    submission_id: id, user_message_id: id, assistant_message_id: id + ":assistant"),
                request_digest: String(repeating: "a", count: 64))))
        } else {
            original = .init(execution_id: id, envelope: .init(kind: "native", platform: "ios",
                conversation_configuration_revision: 7, model_profile_id: "profile", model_profile_revision: 2,
                expected_parent_message_id: "parent", text: "original"))
        }
    }
    func receipt(state: String, revision: Int64 = 3, roomID: String = "room-a", text: String = "original") -> IOSRoomSubmission {
        .init(revision: revision, input: .init(roomID: roomID, submissionID: "submission",
            expectedRoomRevision: 1, text: text, participantID: "member", atMs: 1),
            state: state, selectedParticipantID: "member", failureCode: nil, executionID: original.execution_id,
            terminal: nil, contextConversationID: "context")
    }
    var service: IOSAgentRoomService { .init(execute: { [self] root in
        guard case .object(let fields) = root else { throw CocoaError(.coderInvalidValue) }
        let command: [String: AgentCoreJSONValue]
        if case .object(let nested)? = fields["command"] { command = nested } else { command = fields }
        guard case .string(let type)? = command["type"] else { throw CocoaError(.coderInvalidValue) }
        record(type)
        switch type {
        case "execution_intent": return persisted ? try value(original) : .null
        case "prepare_execution":
            if prepareRejected { throw CocoaError(.coderInvalidValue) }
            return try value(receipt(state: "selected", revision: 4))
        case "claim_execution":
            return .object(["submission": try value(receipt(state: claimState, revision: 5)), "dispatch_permitted": .bool(false)])
        case "load_exact_run_invocation":
            return existingRun ? .object(["conversation_id": .string("context"), "run_id": .string(original.execution_id),
                "assistant_message_id": .string(original.execution_id + ":assistant")]) : .null
        case "reconcile_execution": return try value(receipt(state: "completed", revision: 6))
        case "submission":
            guard command["room_id"] == .string("room-a"), command["submission_id"] == .string("submission") else {
                throw CocoaError(.coderInvalidValue)
            }
            return try value(receipt(state: "cancelled", revision: 8))
        case "submissions":
            if command["room_id"] == .string("room-b") { return .array([]) }
            return .array([try value(receipt(state: "cancelled", revision: 8))])
        case "read": return try value(IOSAgentRoomSnapshot(revision: 1,
            room: .init(id: "room-b", title: "B", participants: [])))
        case "get_product_setting": return .null
        default: throw CocoaError(.coderInvalidValue)
        }
    }) }
    private func value<T: Encodable>(_ value: T) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value))
    }
}

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

@MainActor final class IOSRoomComposerInteractionTests: XCTestCase {
    func testExpertTimelineAttachmentMentionQueriesAndComposerKeyboardIMEHandling() throws {
        struct Item { let id: String; let parent: String? }
        let ordinary = [Item(id: "user-1", parent: nil), Item(id: "answer-1", parent: nil), Item(id: "user-2", parent: nil)]
        let replies = [Item(id: "expert-2", parent: "user-2"), Item(id: "expert-1", parent: "user-1")]
        let merged = IOSExpertTimelineOrder.attach(ordinary: ordinary, replies: replies, ordinaryID: { $0.id }, parentID: { $0.parent })
        XCTAssertEqual(merged.map(\.id), ["user-1", "expert-1", "answer-1", "user-2", "expert-2"])
        XCTAssertEqual(IOSExpertTimelineOrder.attach(ordinary: ordinary, replies: replies, ordinaryID: { $0.id }, parentID: { $0.parent }).map(\.id), merged.map(\.id))

        for text in ["mail@example.org", "text @name continued", "@@name"] {
            XCTAssertNil(ExpertMentionQuery.atEnd(of: text))
        }
        let mentionText = "👩🏽‍💻 讨论 @研究"
        let query = try XCTUnwrap(ExpertMentionQuery.atEnd(of: mentionText))
        XCTAssertEqual(query.query, "研究")
        XCTAssertEqual(query.removing(from: mentionText), "👩🏽‍💻 讨论 ")
        XCTAssertEqual(ExpertMentionQuery.atEnd(of: "@")?.query, "")

        let chatInput = ChatPasteAwareTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 80))
        chatInput.text = "讨论 @研"
        chatInput.mentionsVisible = true
        var chatActions: [String] = []
        chatInput.mentionCommand = { chatActions.append($0); return true }
        for key in [UIKeyCommand.inputDownArrow, "\t", "\r", UIKeyCommand.inputEscape] {
            let command = try XCTUnwrap(chatInput.keyCommands?.first { $0.input == key && $0.modifierFlags.isEmpty })
            _ = chatInput.perform(command.action, with: command)
        }
        XCTAssertEqual(chatActions, ["down", "accept", "accept", "dismiss"])
        XCTAssertEqual(chatInput.text, "讨论 @研")
        chatInput.setMarkedText("研究", selectedRange: NSRange(location: 2, length: 0))
        XCTAssertFalse(chatInput.keyCommands?.contains { $0.input == "\t" && $0.modifierFlags.isEmpty } ?? false)
        XCTAssertNil(chatInput.inputAccessoryView)

        let roomInput = IOSRoomComposerTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 80))
        roomInput.text = "Discuss @"
        roomInput.mentionsVisible = true
        var roomActions: [String] = [], sends = 0
        roomInput.mentionCommand = { roomActions.append($0); return true }
        roomInput.sendMessage = { sends += 1 }
        for key in [UIKeyCommand.inputDownArrow, UIKeyCommand.inputUpArrow, "\t", "\r", UIKeyCommand.inputEscape] {
            let command = try XCTUnwrap(roomInput.keyCommands?.first { $0.input == key && $0.modifierFlags.isEmpty })
            _ = roomInput.perform(command.action, with: command)
        }
        XCTAssertEqual(roomActions, ["down", "up", "accept", "accept", "dismiss"])
        XCTAssertEqual(roomInput.text, "Discuss @")
        XCTAssertEqual(sends, 0)

        var markedCalls = 0
        roomInput.mentionCommand = { _ in markedCalls += 1; return true }
        roomInput.sendMessage = { markedCalls += 1 }
        roomInput.setMarkedText("研究", selectedRange: NSRange(location: 2, length: 0))
        XCTAssertNotNil(roomInput.markedTextRange)
        roomInput.chooseMention(UIKeyCommand(input: "\r", modifierFlags: [], action: #selector(IOSRoomComposerTextView.chooseMention(_:))))
        roomInput.sendFromKeyboard()
        XCTAssertEqual(markedCalls, 0)
        roomInput.unmarkText()
        roomInput.mentionsVisible = false
        XCTAssertFalse(roomInput.keyCommands?.contains { $0.modifierFlags.isEmpty && $0.input == "\t" } ?? false)
        roomInput.insertText("\n")
        XCTAssertTrue(roomInput.text.hasSuffix("\n"))
        roomInput.sendFromKeyboard()
        XCTAssertEqual(markedCalls, 1)
    }
}
