// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacMainExpertDispatchContractTests: XCTestCase {
    func testSelectedChipsRemainVisibleWithoutDirectoryAndKeepExactJoinedName() {
        let old = MacRoomParticipant(id: "member", agentID: "expert", agentRevision: 1,
            displayName: "Frozen name", description: "", contextConversationID: "old-context", active: true)
        let latest = MacPersistentAgent(id: "expert", name: "Latest name", description: "", revision: 2,
            availability: "active", defaults: .object([:]))
        XCTAssertEqual(MacExpertSelectionChip.rows(agentIDs: ["expert"], observedRevisions: ["expert": 1],
            directory: [], joined: [old]), [.init(id: "expert", name: "Frozen name")])
        XCTAssertEqual(MacExpertSelectionChip.rows(agentIDs: ["expert"], observedRevisions: ["expert": 1],
            directory: [latest], joined: [old]), [.init(id: "expert", name: "Frozen name")])
        XCTAssertEqual(MacExpertSelectionChip.rows(agentIDs: ["expert"], observedRevisions: ["expert": 1],
            directory: [latest], joined: [], fallbackName: "Expert"), [.init(id: "expert", name: "Expert")])
        XCTAssertEqual(MacExpertSelectionChip.rows(agentIDs: ["expert"], observedRevisions: ["expert": 2],
            directory: [latest], joined: [old]), [.init(id: "expert", name: "Latest name")])
    }

    func testCanonicalBlockedBudgetNeverLooksLikeAnsweredAndDoesNotMatchArbitraryDetail() throws {
        let blocked = try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"context.input_budget_exceeded","detail":null},"resolution_condition":"opaque budget condition","blocking_fact_refs":["budget"]}"#.utf8))
        XCTAssertNotNil(blocked.blockedFeedback)
        XCTAssertNotEqual(blocked.blockedFeedback, "opaque budget condition")
        let unrelated = try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"permission.denied","detail":"context.input_budget_exceeded"},"resolution_condition":"Grant the requested permission","blocking_fact_refs":["permission"]}"#.utf8))
        XCTAssertEqual(unrelated.blockedFeedback, "Grant the requested permission")
        let answered = try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: Data(#"{"kind":"answered","answer_part_refs":["answer"]}"#.utf8))
        XCTAssertNil(answered.blockedFeedback)
    }

    @MainActor func testNativeTemplateCaptureUsesExactStoreDefaultAndOnlySelectedBuiltin() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let first = try MacApplicationPort(databaseURL: root.appendingPathComponent("first.sqlite"))
        let second = try MacApplicationPort(databaseURL: root.appendingPathComponent("second.sqlite"))
        let builtins = BuiltInPromptTemplates.templates()
        XCTAssertGreaterThanOrEqual(builtins.count, 2)
        let firstID = builtins[0].id, secondID = builtins[1].id
        try MacPromptLibrary.setDefaultTemplateID(firstID, using: first)
        try MacPromptLibrary.setDefaultTemplateID(secondID, using: second)
        XCTAssertEqual(try MacPromptLibrary.defaultTemplateID(using: first), firstID)
        XCTAssertEqual(try MacPromptLibrary.defaultTemplateID(using: second), secondID)
        let reference = AppleExpertParticipantReference(participant_id: "member", agent_id: "expert", agent_revision: 1, context_conversation_id: "context")
        let declaration = AppleExpertModelDeclaration<MacCanonicalJSONValue>(profile_id: "profile", profile_revision: 2,
            provider: "fixture", model_name: "model", runtime: "remote", parameters: .object([:]))
        let preview = AppleExpertCapturePreview<MacCanonicalJSONValue>(participants: [.init(participant: reference,
            context_configuration_revision: 3, defaults: .object([:]), client_model_presentation: .object([:]),
            model_declaration: declaration, effective_parameters: .object([:]))])
        let a = try MacExpertInstructionProducer.capture(preview, using: first)
        let b = try MacExpertInstructionProducer.capture(preview, using: second)
        XCTAssertEqual(a.selections.map(\.template_id), [firstID.uuidString])
        XCTAssertEqual(b.selections.map(\.template_id), [secondID.uuidString])
        XCTAssertEqual(a.builtins.map(\.template_id), [firstID.uuidString])
        XCTAssertEqual(b.builtins.map(\.template_id), [secondID.uuidString])
        XCTAssertEqual(a.builtins.first?.content, builtins[0].content)
        XCTAssertLessThan(try JSONEncoder().encode(a).count, 2 * 1024 * 1024)
    }

    func testRemoteDispatchSeparatesSelectionFromPromptContextAndRetainsOriginalEpoch() throws {
        let selected = AppleMainExpertSelectionContext(selectedExperts: [.init(agentID: "expert", agentRevision: 4,
            participantID: "member", contextConversationID: "child")], expectedSourceStoreEpoch: "observed-store")
        let component = MacCoreContextComponent(kind: "main_expert_selection", id: "nola.main-expert-selection",
            content: try JSONDecoder().decode(MacCoreJSONValue.self, from: JSONEncoder().encode(selected)), source: nil)
        let ordinary = MacCoreContextComponent(kind: "user_preference", id: "preference", content: "Keep concise")
        let context = try MacRemoteContextPayload.encode([ordinary, component])
        XCTAssertEqual(context.count, 1); XCTAssertEqual(context.first?["id"] as? String, "preference")
        XCTAssertEqual(try MacRemoteContextPayload.selectionEpoch([ordinary, component]), "observed-store")
        let entries = try XCTUnwrap(MacRemoteContextPayload.selectedExperts([ordinary, component]))
        XCTAssertEqual(entries.first?["agent_id"] as? String, "expert")
        XCTAssertEqual(entries.first?["participant_id"] as? String, "member")
        XCTAssertThrowsError(try MacRemoteContextPayload.selectedExperts([component, component]))
    }

    func testMainAndLegacySourcesRemainDistinctAndForeignHostNeverLooksEmpty() throws {
        let source = try JSONDecoder().decode(AppleExpertTaskSource.self, from: Data(#"{"kind":"main_agent","context_conversation_id":"main"}"#.utf8))
        XCTAssertTrue(source.isMainAgent); XCTAssertEqual(source.context_conversation_id, "main")
        let legacy = AppleExpertParticipantReference(participant_id: "member", agent_id: "expert", agent_revision: 3, context_conversation_id: "old")
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertTaskSource.self, from: JSONEncoder().encode(legacy)), .legacyExpert(legacy))
        let page = AppleExpertRoomHostResponse(hostID: "host", sourceStoreEpoch: "store", sourceMatches: true, result: MacCanonicalJSONValue.array([]))
        XCTAssertNoThrow(try page.validate(hostID: "host", expectedEpoch: "store"))
        XCTAssertThrowsError(try page.validate(hostID: "other", expectedEpoch: "store"))
        XCTAssertThrowsError(try page.validate(hostID: "host", expectedEpoch: "different-store"))
    }
    @MainActor func testRemoteSelectionRetainsOriginalSourceAndNeverFallsBackToLocalDirectory() async throws {
        let service = MacAgentRoomService(execute: { _ in
            XCTFail("A Host selection must never consult the local expert store")
            return .null
        })
        let workspace = MacAgentRoomWorkspace(service: service)
        var epoch = "first-store"
        var identity = "first-connection"
        workspace.mainIsRemote = { true }
        workspace.mainSourceEpoch = { _ in epoch }
        workspace.mainSourceIdentity = { _ in identity }
        workspace.mainCommand = { _, command in
            guard case .object(let fields) = command else { throw MacRemoteHostError.invalidResponse }
            switch fields["type"] {
            case .string("directory"):
                return .object(["revision": .signedInteger(1), "agents": .array([.object([
                    "id": .string("expert"), "name": .string("Target"), "description": .string(""), "defaults": .object([:]),
                    "availability": .string("active"), "revision": .signedInteger(3)])])])
            case .string("for_conversation"): return .null
            default: XCTFail("Selecting an expert must not create a Room or execute it"); throw MacRemoteHostError.invalidResponse
            }
        }
        try await workspace.prepareExpertMention(agentID: "expert", expectedRevision: 3, conversationID: "main", isPersisted: false)
        let selection = try await workspace.mainExpertSelection(conversationID: "main", agentIDs: ["expert"])
        XCTAssertEqual(selection, [.init(agentID: "expert", agentRevision: 3)])
        XCTAssertEqual(try workspace.capturedMainSourceEpoch(owner: "main", agentIDs: ["expert"]), "first-store")
        epoch = "new-store"; identity = "new-connection"
        do {
            _ = try await workspace.mainExpertSelection(conversationID: "main", agentIDs: ["expert"])
            XCTFail("A reconnected directory cannot silently rebase the original choice")
        } catch { }
    }

}
