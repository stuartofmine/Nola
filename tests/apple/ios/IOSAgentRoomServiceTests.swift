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
