// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS experts, child history, role configuration, template host, dispatch contracts, and agent room workspace/recovery suite.

import AppKit
import Combine
import CryptoKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

@MainActor final class MacExpertTemplateHostTests: XCTestCase {
    func testExpertInstructionProducerAndCaptureSelection() throws {
        let definitions = MacExpertInstructionProducer.definitions(style: .detailed,
            customSystemPrompt: "  My precise model instructions.\n")
        XCTAssertEqual(definitions.map(\.component_id), ["macos:response-depth", "macos:user-custom-instructions"])
        XCTAssertEqual(definitions[0].content, MacResponseStyle.detailed.responseInstruction)
        XCTAssertEqual(definitions[1].content, "My precise model instructions.")
        for definition in definitions {
            XCTAssertNil(definition.definition_revision)
            XCTAssertEqual(definition.platform, .string("macos"))
            XCTAssertEqual(definition.content_sha256, SHA256.hash(data: Data(definition.content.utf8))
                .map { String(format: "%02x", $0) }.joined())
        }

        let blankDefinitions = MacExpertInstructionProducer.definitions(style: .balanced, customSystemPrompt: " \n\t")
        XCTAssertEqual(blankDefinitions.count, 1)
        XCTAssertEqual(blankDefinitions[0].component_id, "macos:response-depth")
        XCTAssertEqual(blankDefinitions[0].content, MacResponseStyle.balanced.responseInstruction)

        let selection = AppleExpertCaptureSelection<MacCanonicalJSONValue>(
            context_conversation_id: "child", context_configuration_revision: 7,
            model_profile: .init(profile_id: "profile", admitted_revision: 11),
            source: .noTemplate, template_id: nil,
            host_user_instructions: MacExpertInstructionProducer.definitions(style: .concise, customSystemPrompt: "Original"))
        let value = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(selection)) as? [String: Any])
        let model = try XCTUnwrap(value["model_profile"] as? [String: Any])
        XCTAssertEqual(model["profile_id"] as? String, "profile")
        XCTAssertEqual(model["admitted_revision"] as? Int, 11)
        XCTAssertTrue(value["template_id"] is NSNull)
        XCTAssertEqual((value["host_user_instructions"] as? [[String: Any]])?.count, 2)
    }
}

final class MacMainExpertDispatchContractTests: XCTestCase {
    func testInlineMentionsUseCursorAndPreserveFrozenIdentity() throws {
        let text = "👩🏽‍💻请@测帮我"
        let caret = "👩🏽‍💻请@测".utf16.count
        let query = try XCTUnwrap(ExpertMentionQuery.atCaret(in: text, selection: NSRange(location: caret, length: 0)))
        let expert = AppleMainExpertSelectionEntry(agentID: "a", agentRevision: 3)
        let inserted = AppleDraftMention.insertion(text: text, range: query.range, name: "测试", expert: expert, mentions: [], sourceID: nil, epoch: nil)
        XCTAssertEqual(inserted.text, "👩🏽‍💻请@测试 帮我")
        let again = AppleDraftMention.insertion(text: inserted.text, range: NSRange(location: inserted.text.utf16.count, length: 0),
            name: "测试", expert: expert, mentions: inserted.mentions, sourceID: nil, epoch: nil)
        var wire = try XCTUnwrap(AppleDraftMention.segments(again.mentions, in: again.text))
        XCTAssertEqual(wire.segments.compactMap(\.agentID), ["a", "a"])
        wire.selectedExperts = try AppleDraftMention.selected(again.mentions, sourceID: nil, epoch: nil)
        XCTAssertEqual(wire.selectedExperts, [expert])
        XCTAssertEqual(wire.runs(in: again.text), again.mentions)
        XCTAssertNil(wire.runs(in: "stale"))
        XCTAssertEqual(AppleDraftMention.replacing(again.mentions, range: again.mentions[0].range, with: "@测试").count, 1)
        XCTAssertNil(ExpertMentionQuery.atEnd(of: "hello@example"))
        XCTAssertThrowsError(try AppleDraftMention.selected(again.mentions, sourceID: "different-host", epoch: "1"))
    }

    @MainActor func testScopedExpertHistoryLoadsJournalOnlyFromItsSourceWindow() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "child-history", runID: "run", assistantMessageID: "assistant")
        _ = try fixture.append("专家回复。")
        _ = try fixture.finish()
        let source = try MacApplicationPort(databaseURL: fixture.databaseURL)
        let window = try source.loadConversationWindow(fixture.conversationID)
        let head = try XCTUnwrap(window.assistantJournalHeads.first)
        let page = MacExpertChildHistoryReader.Page(version: 1, conversationID: "parent", childConversationID: fixture.conversationID,
            roomID: "room", roomRevision: 1, sourceOwner: .local, window: window, expertExchanges: [])
        let reader = MacExpertChildHistoryReader(parentID: "parent", childID: fixture.conversationID, readJournal: { cursor in
            XCTAssertEqual(cursor, head)
            let snapshot = try source.openAssistantJournalSnapshot(at: cursor)
            defer { try? source.releaseAssistantJournalSnapshot(snapshot) }
            return try await MacJournalTextRead.publicMarkdown(at: cursor, parts: { after in
                try source.assistantJournalParts(snapshot: snapshot, afterPartID: after)
            }, range: { part, body, offset in
                try source.assistantJournalRange(snapshot: snapshot, partID: part, content: body, offsetBytes: offset)
            })
        }, read: { _ in page })
        await reader.load()
        XCTAssertNil(reader.error)
        XCTAssertEqual(reader.messages.first(where: { $0.role == .assistant })?.scopedJournalText, "专家回复。")
        XCTAssertThrowsError(try MacCanonicalConversationRepository.projectMessages(window, disclosedJournalText: ["someone-else": "foreign"]))
    }

    func testSelectedChipsBlockedBudgetRemotePayloadAndTaskSourceContracts() throws {
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

        let blocked = try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"context.input_budget_exceeded","detail":null},"resolution_condition":"opaque budget condition","blocking_fact_refs":["budget"]}"#.utf8))
        XCTAssertNotNil(blocked.blockedFeedback)
        XCTAssertNotEqual(blocked.blockedFeedback, "opaque budget condition")
        let unrelated = try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"permission.denied","detail":"context.input_budget_exceeded"},"resolution_condition":"Grant the requested permission","blocking_fact_refs":["permission"]}"#.utf8))
        XCTAssertEqual(unrelated.blockedFeedback, "Grant the requested permission")
        let answered = try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: Data(#"{"kind":"answered","answer_part_refs":["answer"]}"#.utf8))
        XCTAssertNil(answered.blockedFeedback)

        let selected = AppleMainExpertSelectionContext(selectedExperts: [.init(agentID: "expert", agentRevision: 4,
            participantID: "member", contextConversationID: "child")], expectedSourceStoreEpoch: "observed-store")
        let component = MacCoreContextComponent(kind: "main_expert_selection", id: "nola.main-expert-selection",
            content: try JSONDecoder().decode(MacCoreJSONValue.self, from: JSONEncoder().encode(selected)), source: nil)
        let ordinary = MacCoreContextComponent(kind: "user_preference", id: "preference", content: "Keep concise")
        let context = try MacRemoteContextPayload.encode([ordinary, component])
        XCTAssertEqual(context.count, 1)
        XCTAssertEqual(context.first?["id"] as? String, "preference")
        XCTAssertEqual(try MacRemoteContextPayload.selectionEpoch([ordinary, component]), "observed-store")
        let entries = try XCTUnwrap(MacRemoteContextPayload.selectedExperts([ordinary, component]))
        XCTAssertEqual(entries.first?["agent_id"] as? String, "expert")
        XCTAssertEqual(entries.first?["participant_id"] as? String, "member")
        XCTAssertThrowsError(try MacRemoteContextPayload.selectedExperts([component, component]))

        let source = try JSONDecoder().decode(AppleExpertTaskSource.self, from: Data(#"{"kind":"main_agent","context_conversation_id":"main"}"#.utf8))
        XCTAssertTrue(source.isMainAgent)
        XCTAssertEqual(source.context_conversation_id, "main")
        let legacy = AppleExpertParticipantReference(participant_id: "member", agent_id: "expert", agent_revision: 3, context_conversation_id: "old")
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertTaskSource.self, from: JSONEncoder().encode(legacy)), .legacyExpert(legacy))
        let page = AppleExpertRoomHostResponse(hostID: "host", sourceStoreEpoch: "store", sourceMatches: true, result: MacCanonicalJSONValue.array([]))
        XCTAssertNoThrow(try page.validate(hostID: "host", expectedEpoch: "store"))
        XCTAssertThrowsError(try page.validate(hostID: "other", expectedEpoch: "store"))
        XCTAssertThrowsError(try page.validate(hostID: "host", expectedEpoch: "different-store"))
    }

    @MainActor func testNativeTemplateCaptureAndRemoteSelectionEpochIsolation() async throws {
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

@MainActor final class MacExpertChildHistoryTests: XCTestCase {
    func testPageRejectsWrongParentVersionMissingRoomAndNonAdvancingCursor() throws {
        let page = try JSONDecoder().decode(AppleExpertChildConversations.self, from: Data(Self.pageJSON.utf8))
        try page.validate(parentID: "parent", offset: 0)
        XCTAssertThrowsError(try page.validate(parentID: "other", offset: 0))
        for (source, replacement) in [("\"version\":1", "\"version\":2"),
            ("\"room_id\":\"room\"", "\"room_id\":null"),
            ("\"room_revision\":1", "\"room_revision\":null"),
            ("\"has_more\":false,\"next_offset\":null", "\"has_more\":true,\"next_offset\":0"),
            ("\"message_count\":0", "\"message_count\":-1")] {
            let invalid = try JSONDecoder().decode(AppleExpertChildConversations.self,
                from: Data(Self.pageJSON.replacingOccurrences(of: source, with: replacement).utf8))
            XCTAssertThrowsError(try invalid.validate(parentID: "parent", offset: 0))
        }
    }

    func testReadUsesExactParentAndCursorAndRejectsCrossOwnerResponse() throws {
        let service = MacAgentRoomService(execute: { request in
            guard case .object(let envelope) = request, case .object(let command) = envelope["command"] else {
                throw MacApplicationPortError.invalidResponse
            }
            XCTAssertEqual(command["type"], .string("child_conversations"))
            XCTAssertEqual(command["conversation_id"], .string("parent"))
            XCTAssertEqual(command["offset"], .signedInteger(4))
            return try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(Self.pageJSON.utf8))
        })
        XCTAssertEqual(try service.childConversations(conversationID: "parent", offset: 4).children.first?.id, "child")
        let wrong = MacAgentRoomService(execute: { _ in
            try JSONDecoder().decode(MacCanonicalJSONValue.self,
                from: Data(Self.pageJSON.replacingOccurrences(of: "\"conversation_id\":\"parent\"", with: "\"conversation_id\":\"other\"").utf8))
        })
        XCTAssertThrowsError(try wrong.childConversations(conversationID: "parent"))
    }

    func testCollapsedTreeDoesNotReadAndLateCollapsedResponseDoesNotPublish() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        var reads = 0
        var pending: CheckedContinuation<AppleExpertChildConversations, Error>?
        let host = NSHostingView(rootView: MacExpertChildrenDisclosure(parentID: "parent", refreshRevision: .distantPast,
            load: { _, _ in
                reads += 1
                return try await withCheckedThrowingContinuation { pending = $0 }
            }, open: { _ in XCTFail("Reading must not open a child automatically") })
            .environment(\.accessibilityEnabled, true))
        host.frame = NSRect(x: 0, y: 0, width: 320, height: 500)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(reads, 0)
        let toggle = try XCTUnwrap(element(host, "expert.children.toggle.parent"))
        _ = toggle.accessibilityPerformPress?()
        for _ in 0..<100 where pending == nil { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(reads, 1)
        _ = toggle.accessibilityPerformPress?()
        let continuation = try XCTUnwrap(pending)
        continuation.resume(returning: try JSONDecoder().decode(AppleExpertChildConversations.self, from: Data(Self.pageJSON.utf8)))
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertNil(element(host, "expert.child.child"), "A collapsed request must not mount a late child row")
        XCTAssertEqual(reads, 1)
    }

    func testCanonicalRejoinKeepsOldHistoryUnderParentAndNeverEnablesOldFollowup() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-child-history-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("isolated.sqlite"))
        let repository = MacCanonicalConversationRepository(store: port)
        let service = MacAgentRoomService(runtime: port)
        let conversation = MacConversation(title: "Parent", preview: "", updatedAt: .now, isPinned: false)
        _ = try repository.ensureConversation(conversation)
        let directory = try service.createAgent(name: "Expert", description: "Read only fixture", defaults: .object([:]), directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let association = try service.ensureForConversation(conversation.id)
        let joined = try service.join(agent: agent, room: association.room)
        let old = try XCTUnwrap(try service.childConversations(conversationID: conversation.id).children.first)
        let updatedDirectory = try service.updateAgent(agent, name: agent.name, description: "Updated snapshot",
            availability: "active", directory: service.directory())
        let updated = try XCTUnwrap(updatedDirectory.agents.first)
        _ = try service.rejoin(agent: updated, room: joined)
        let page = try service.childConversations(conversationID: conversation.id)
        XCTAssertEqual(page.children.count, 2)
        let previous = try XCTUnwrap(page.children.first { $0.id == old.id })
        XCTAssertFalse(previous.isCurrent)
        XCTAssertFalse(previous.active)
        XCTAssertNil(previous.agentRevision, "Historical context must not infer the directory revision")
        let current = try XCTUnwrap(page.children.first { $0.isCurrent })
        let workspace = MacAgentRoomWorkspace(service: service)
        do { try await workspace.selectCurrentChild(previous, parentID: conversation.id); XCTFail("Old histories are read only") }
        catch { }
        try await workspace.selectCurrentChild(current, parentID: conversation.id)
        XCTAssertTrue(try service.submissions(roomID: association.roomID).isEmpty)
        XCTAssertFalse(try repository.loadSidebar().contains { $0.conversation.id == current.id || $0.conversation.id == previous.id })
    }

    func testPeerExchangesUseParentAndRoomForRemoteRestoreRefreshAndStandaloneRoom() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let repository = MacCanonicalConversationRepository(store: port)
        let local = MacAgentRoomService(execute: { request in
            if case .object(let envelope) = request, case .object(let command) = envelope["command"] {
                XCTAssertNotEqual(command["type"], .string("peer_exchanges"), "Selecting a standalone room has no main owner")
            }
            return try port.executeAgentRoomEnvelope(request)
        })
        let standalone = try local.create(title: "Standalone")
        let workspace = MacAgentRoomWorkspace(service: local)
        let execution = MacRoomExecutionService(repository: repository,
            localRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        workspace.executionService = execution
        workspace.mainIsRemote = { true }

        var associatedParent = "parent"
        var pageParent = "parent"
        var exchangeParent = "parent"
        var exchangeRoom = "room"
        var requests = [String]()
        workspace.mainCommand = { owner, request in
            guard case .object(let fields) = request, case .string(let kind) = fields["type"] else {
                throw MacApplicationPortError.invalidResponse
            }
            XCTAssertEqual(owner, "parent")
            XCTAssertEqual(fields["conversation_id"], .string("parent"))
            requests.append(kind)
            switch kind {
            case "for_conversation":
                let participants = ["source", "target"].map {
                    MacRoomParticipant(id: $0 + "-member", agentID: $0 + "-agent", agentRevision: 1,
                        displayName: $0, description: "", contextConversationID: $0, active: true)
                }
                let association = MacConversationRoomAssociation(conversationID: associatedParent, roomID: "room", revision: 1,
                    room: .init(revision: 1, room: .init(id: "room", title: "Owner", participants: participants)))
                return try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(association))
            case "peer_exchanges":
                let text = Self.peerJSON.replacingOccurrences(of: #""parent_conversation_id":"parent""#,
                    with: #""parent_conversation_id":"\#(exchangeParent)""#)
                    .replacingOccurrences(of: #""room_id":"room""#, with: #""room_id":"\#(exchangeRoom)""#)
                return .object(["version": .signedInteger(1), "conversation_id": .string(pageParent),
                    "exchanges": .array([try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(text.utf8))]),
                    "has_more": .bool(false), "next_offset": .null])
            default: throw MacApplicationPortError.invalidResponse
            }
        }
        let initialIDs = try repository.conversationIDs()
        await workspace.restoreConversation("parent")
        XCTAssertEqual(workspace.peerExchanges.map(\.exchangeID), ["exchange"])
        XCTAssertTrue(execution.owners.isEmpty, "A read from a remote Host must not create a local child conversation")
        XCTAssertEqual(try repository.conversationIDs(), initialIDs)
        execution.observe(contextID: "target")
        await workspace.refreshPeerExchanges(for: "parent")
        XCTAssertTrue(workspace.transcriptOwners.isEmpty, "A local child with the same ID cannot supply a remote transcript")

        let initialRequests = requests
        await workspace.select(standalone, resumeExecutions: false)
        XCTAssertEqual(workspace.selectedRoom?.room.id, standalone.room.id)
        XCTAssertNil(workspace.errors[standalone.room.id])
        XCTAssertEqual(requests, initialRequests)

        pageParent = "foreign"
        await workspace.restoreConversation("parent")
        XCTAssertTrue(workspace.peerExchanges.isEmpty)
        pageParent = "parent"
        exchangeParent = "foreign"
        await workspace.refreshPeerExchanges(for: "parent")
        XCTAssertTrue(workspace.peerExchanges.isEmpty)
        exchangeParent = "parent"
        exchangeRoom = "foreign-room"
        await workspace.refreshPeerExchanges(for: "parent")
        XCTAssertTrue(workspace.peerExchanges.isEmpty)
        exchangeRoom = "room"
        await workspace.refreshPeerExchanges(for: "parent")
        XCTAssertEqual(workspace.peerExchanges.map(\.exchangeID), ["exchange"])
        associatedParent = "foreign"
        let count = requests.count
        await workspace.restoreConversation("parent")
        XCTAssertNil(workspace.conversationAssociation)
        XCTAssertTrue(workspace.peerExchanges.isEmpty)
        XCTAssertEqual(Array(requests.dropFirst(count)), ["for_conversation"])
    }

    func testScopedPeerAuthorsUseFrozenNamesAndRejectForeignOrUnmappedMessages() throws {
        let exchange = try JSONDecoder().decode(AppleExpertPeerView.self, from: Data(Self.peerJSON.utf8))
        let result = AppleExpertPeerExchanges(version: 1, conversationID: "parent", exchanges: [exchange],
            hasMore: false, nextOffset: nil)
        let wire = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(result))
        let service = MacAgentRoomService(execute: { _ in wire })
        XCTAssertEqual(try service.peerExchanges(conversationID: "parent").exchanges, [exchange])
        XCTAssertThrowsError(try service.peerExchanges(conversationID: "source"))
        XCTAssertThrowsError(try service.peerExchanges(conversationID: "target"))
        let authors = try MacExpertChildHistoryReader.authors([exchange], parentID: "parent", childID: "target", userMessageIDs: ["exchange:user"])
        XCTAssertEqual(authors["exchange:user"], "Frozen reviewer")
        XCTAssertThrowsError(try MacExpertChildHistoryReader.authors([exchange], parentID: "other", childID: "target", userMessageIDs: ["exchange:user"]))
        XCTAssertThrowsError(try MacExpertChildHistoryReader.authors([exchange], parentID: "parent", childID: "sibling", userMessageIDs: ["exchange:user"]))
        XCTAssertThrowsError(try MacExpertChildHistoryReader.authors([exchange], parentID: "parent", childID: "target", userMessageIDs: []))
        XCTAssertThrowsError(try MacExpertChildHistoryReader.authors([exchange, exchange], parentID: "parent", childID: "target", userMessageIDs: ["exchange:user"]))
    }

    func testScopedReaderUsesParentAggregateAndDiscardsInvalidatedLatePage() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-scoped-child-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("isolated.sqlite"))
        let repository = MacCanonicalConversationRepository(store: port), service = MacAgentRoomService(runtime: port)
        let conversation = MacConversation(title: "Parent", preview: "", updatedAt: .now, isPinned: false)
        _ = try repository.ensureConversation(conversation)
        let directory = try service.createAgent(name: "Expert", description: "Scoped fixture", defaults: .object([:]), directory: service.directory())
        let association = try service.ensureForConversation(conversation.id)
        _ = try service.join(agent: XCTUnwrap(directory.agents.first), room: association.room)
        let child = try XCTUnwrap(try service.childConversations(conversationID: conversation.id).children.first)
        let page = try service.childHistory(parentID: conversation.id, childID: child.id)
        XCTAssertEqual(page.window.projection.conversationID, child.id)
        var pending: CheckedContinuation<MacExpertChildHistoryReader.Page, Error>?
        let reader = MacExpertChildHistoryReader(parentID: conversation.id, childID: child.id) { before in
            XCTAssertNil(before)
            return try await withCheckedThrowingContinuation { pending = $0 }
        }
        let loading = Task { await reader.load() }
        for _ in 0..<100 where pending == nil { await Task.yield() }
        let continuation = try XCTUnwrap(pending)
        reader.invalidate(); continuation.resume(returning: page); await loading.value
        XCTAssertNil(reader.sourceOwner); XCTAssertTrue(reader.messages.isEmpty)
        let visible = MacExpertChildHistoryReader(parentID: conversation.id, childID: child.id) { _ in page }
        await visible.load()
        XCTAssertNil(visible.error); XCTAssertEqual(visible.sourceOwner, .local)
        XCTAssertTrue(try service.submissions(roomID: association.roomID).isEmpty)
    }

    func testReadonlyPeerTranscriptShowsFrozenAuthorWithoutEditingAndCapturesThemes() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let question = MacChatMessage(role: .user, text: "Please review the cancellation and recovery behavior.", createdAt: .now, canonicalMessageID: "exchange:user")
        let answer = MacChatMessage(role: .assistant, text: "The worker preserves its task identity when recovering. Cancellation remains pending until execution has stopped.", createdAt: .now, deliveryState: .completed)
        for scheme in [ColorScheme.light, .dark] {
            let host = NSHostingView(rootView: MacTranscriptView(conversationID: "target", messages: [question, answer], isRunning: false,
                liveActivities: [], searchText: "", bottomClearance: 0, onSendTaskGuidance: { _, _, _ in XCTFail("Readonly") },
                onRequestTaskCancellation: { _, _ in XCTFail("Readonly") }, permitsFollowupUIActions: false,
                sourceAuthors: ["exchange:user": "Frozen reviewer"])
                .background(MacDesignColor.canvas)
                .environment(\.macTranscriptReadOnly, true).environment(\.accessibilityEnabled, true).environment(\.colorScheme, scheme))
            host.frame = NSRect(x: 0, y: 0, width: 640, height: 520)
            let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await Task.sleep(for: .milliseconds(180))
            XCTAssertNotNil(element(host, "expert.message.author.exchange:user"))
            XCTAssertNil(element(host, "mac-message-edit-" + question.id.uuidString))
            host.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            window.effectiveAppearance.performAsCurrentDrawingAppearance { host.cacheDisplay(in: host.bounds, to: bitmap) }
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            let image = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
            image.name = "expert-child-readonly-" + (scheme == .dark ? "dark" : "light"); image.lifetime = .keepAlways; add(image)
        }
    }

    nonisolated private static let peerJSON = #"{"version":1,"exchange_id":"exchange","room_id":"room","parent_conversation_id":"parent","source":{"participant_id":"source-member","agent_id":"source-agent","agent_revision":1,"context_conversation_id":"source"},"target":{"participant_id":"target-member","agent_id":"target-agent","agent_revision":1,"context_conversation_id":"target"},"source_run_id":"source-run","source_call_id":"source-call","source_display_name":"Frozen reviewer","target_display_name":"Target","kind":"request","text":"Source text is not an author label","in_reply_to":null,"root_exchange_id":"exchange","hop":1,"created_at_ms":1,"clarifications":[]}"#

    private func element(_ root: AnyObject, _ id: String) -> AnyObject? {
        var queue = [root], seen = Set<ObjectIdentifier>(), index = 0
        while index < queue.count, index < 6000 {
            let value = queue[index]; index += 1
            guard seen.insert(ObjectIdentifier(value)).inserted else { continue }
            if (value.accessibilityIdentifier?() ?? nil) == id { return value }
            let children = value.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = value as? NSView { queue += view.subviews }
        }
        return nil
    }

    nonisolated private static let pageJSON = #"{"version":1,"conversation_id":"parent","room_id":"room","room_revision":1,"children":[{"conversation_id":"child","revision":0,"title":"Expert","participant_id":"member","agent_id":"agent","agent_revision":null,"is_current":false,"active":false,"created_at_ms":1,"updated_at_ms":1,"message_count":0}],"has_more":false,"next_offset":null}"#
}

@MainActor
final class MacExpertRoleConfigurationTests: XCTestCase {
    func testEffortSelectionPreservesOpaqueDefaultsAndPerModelOverrides() throws {
        let original: MacCanonicalJSONValue = .object([
            "modelSelectionID": .string("configured:model"), "expertReasoningEffort": .string("high"),
            "modelOverrides": .object(["configured:model": .object(["temperature": .number(0.2), "future": .bool(true)])]),
            "futureDefault": .array([.string("keep"), .null])
        ])
        var draft = try AppleExpertConfigurationDraft(defaults: original)
        XCTAssertEqual(draft.reasoningEffort, "high")
        XCTAssertEqual(try draft.applying(to: original), original)
        draft.reasoningEffort = "max"
        guard case .object(let changed) = try draft.applying(to: original), case .object(let old) = original else {
            return XCTFail("Expected defaults")
        }
        XCTAssertEqual(changed["expertReasoningEffort"], .string("max"))
        XCTAssertEqual(changed["modelOverrides"], old["modelOverrides"])
        XCTAssertEqual(changed["futureDefault"], old["futureDefault"])
        draft.reasoningEffort = "automatic"
        guard case .object(let inherited) = try draft.applying(to: original) else { return XCTFail("Expected defaults") }
        XCTAssertNil(inherited["expertReasoningEffort"])
        XCTAssertEqual(inherited["modelOverrides"], old["modelOverrides"])
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: .object([:])).reasoningEffort, "automatic")
        XCTAssertThrowsError(try AppleExpertConfigurationDraft(defaults: .object(["expertReasoningEffort": .bool(false)])))
    }

    func testCoreRoleCatalogContainsReadablePromptsToolPacksAndRealSkillIdentities() throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("roles.sqlite"))
        let catalog = try MacAgentRoomService(runtime: port).roleTemplates()
        XCTAssertEqual(catalog.version, 1)
        XCTAssertEqual(catalog.templates.count, 7)
        XCTAssertTrue(catalog.features.reasoningEffort)
        XCTAssertTrue(catalog.features.explicitGenerationReasoning)
        XCTAssertTrue(catalog.features.rejoin)
        let descriptors = MacBuiltInSkillRegistry.descriptors(overridesRootURL: root.appendingPathComponent("no-overrides"))
        for role in catalog.templates {
            XCTAssertNotNil(UUID(uuidString: role.id))
            XCTAssertFalse(role.prompt.isEmpty)
            XCTAssertFalse(role.toolPack.tools.isEmpty)
            XCTAssertFalse(role.localizedName(locale: Locale(identifier: "zh_Hans_CN")).isEmpty)
            for skill in role.skills {
                XCTAssertNotNil(MacBuiltInSkillID(rawValue: skill.id), "Role must reference a registered real bundle Skill")
                let resource = try XCTUnwrap(descriptors.first { $0.id.rawValue == skill.id })
                XCTAssertFalse(try String(contentsOf: resource.instructionURL, encoding: .utf8).isEmpty)
                XCTAssertFalse(skill.instructions.isEmpty)
            }
        }
        let data = try JSONEncoder().encode(catalog)
        var document = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        document["version"] = 2
        XCTAssertThrowsError(try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self,
            from: JSONSerialization.data(withJSONObject: document)))
        document["version"] = 1
        if var templates = document["templates"] as? [[String: Any]], var duplicate = templates.first {
            duplicate["id"] = catalog.templates[0].id.uppercased()
            templates.append(duplicate)
            document["templates"] = templates
            XCTAssertThrowsError(try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self,
                from: JSONSerialization.data(withJSONObject: document)))
            document["templates"] = Array(templates.dropLast())
        }
        document.removeValue(forKey: "features")
        let legacy = try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self,
            from: JSONSerialization.data(withJSONObject: document))
        XCTAssertFalse(legacy.features.reasoningEffort)
        XCTAssertFalse(legacy.features.rejoin)
    }

    func testEffortOptionsUseOnlyDeclaredSelectedOrInheritedModel() throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("models.sqlite"))
        var source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture", "endpointName": "Fixture",
            "baseURL": "https://fixture.invalid/v1", "endpointOrder": 0, "endpointEnabled": true,
            "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
            "wireProtocol": "responses", "credentialAccount": "fixture-account",
            "reasoning": ["control": "effort", "efforts": ["low", "medium", "high", "max"]]]
        for id in ["declared", "unknown"] {
            if id == "unknown" { source.removeValue(forKey: "reasoning") }
            try port.upsertModelProfile(.init(id: id, displayName: id, provider: "openai-compatible", modelName: id,
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        }
        let repository = MacCanonicalConversationRepository(store: port)
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: "configured:declared"), ["low", "medium", "high", "max"])
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: "configured:unknown"), [])
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: "configured:missing"), [])
        let preference = MacDefaultChatModelPresentation(store: .init(application: port))
        preference.select(.profile("declared"))
        XCTAssertNil(preference.error)
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: ""), ["low", "medium", "high", "max"])
        preference.select(.profile("unknown"))
        XCTAssertEqual(try repository.agentReasoningEfforts(modelSelectionID: ""), [])
    }

    func testBlankComposerMentionValidatesExactRevisionWithoutCreatingRoom() async throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("mentions.sqlite"))
        let service = MacAgentRoomService(runtime: port)
        let directory = try service.createAgent(name: "Writer", description: "Code", defaults: .object([:]), directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let workspace = MacAgentRoomWorkspace(service: service)
        try await workspace.prepareExpertMention(agentID: agent.id, expectedRevision: agent.revision,
            conversationID: "unsaved-composer", isPersisted: false)
        XCTAssertTrue(try service.list().isEmpty)
        _ = try service.updateAgent(agent, name: "Writer edited", description: "New configuration",
            availability: "active", directory: directory, defaults: agent.defaults)
        do {
            try await workspace.prepareExpertMention(agentID: agent.id, expectedRevision: agent.revision,
                conversationID: "unsaved-composer", isPersisted: false)
            XCTFail("A stale observed candidate must not silently upgrade")
        } catch { }
        XCTAssertTrue(try service.list().isEmpty)
    }

    func testExistingConversationRequiresExplicitReintroductionAndRetainsOldContext() async throws {
        let root = temporaryRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("rejoin.sqlite"))
        let source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture", "endpointName": "Fixture",
            "baseURL": "https://fixture.invalid/v1", "endpointOrder": 0, "endpointEnabled": true,
            "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
            "wireProtocol": "responses", "credentialAccount": "fixture-account",
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        try port.upsertModelProfile(.init(id: "declared", displayName: "Declared", provider: "openai-compatible", modelName: "declared",
            runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        let repository = MacCanonicalConversationRepository(store: port)
        let conversation = MacConversation(title: "Existing", preview: "", updatedAt: .now, isPinned: false)
        _ = try repository.ensureConversation(conversation)
        let service = MacAgentRoomService(runtime: port)
        let directory = try service.createAgent(name: "Writer", description: "Code",
            defaults: .object(["modelSelectionID": .string("configured:declared"), "expertReasoningEffort": .string("low")]),
            directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let association = try service.ensureForConversation(conversation.id)
        let joined = try service.join(agent: agent, room: association.room)
        let previous = try XCTUnwrap(joined.room.participants.first)
        let oldSettings = try port.loadConversationMetadata(previous.contextConversationID)?.settingsJSON
        let updated = try service.updateAgent(agent, name: "Writer", description: "Code", availability: "active", directory: directory,
            defaults: .object(["modelSelectionID": .string("configured:declared"), "expertReasoningEffort": .string("high")]))
        let latest = try XCTUnwrap(updated.agents.first)
        let workspace = MacAgentRoomWorkspace(service: service)
        do {
            try await workspace.prepareExpertMention(agentID: latest.id, expectedRevision: latest.revision,
                conversationID: conversation.id, isPersisted: true)
            XCTFail("An updated member must not silently replace its previous context")
        } catch MacExpertMentionPreparationError.reintroductionRequired { }
        let unchanged = try XCTUnwrap(service.forConversation(conversation.id))
        XCTAssertEqual(unchanged.room.room.participants.count, 1)
        try await workspace.prepareExpertMention(agentID: latest.id, expectedRevision: latest.revision,
            conversationID: conversation.id, isPersisted: true, allowReintroduction: true)
        let refreshed = try XCTUnwrap(service.forConversation(conversation.id))
        XCTAssertEqual(refreshed.room.room.participants.count, 1)
        XCTAssertNil(refreshed.room.room.participants.first { $0.id == previous.id })
        let active = try XCTUnwrap(refreshed.room.room.participants.first { $0.active })
        XCTAssertEqual(active.agentRevision, latest.revision)
        XCTAssertNotEqual(active.contextConversationID, previous.contextConversationID)
        XCTAssertEqual(try port.loadConversationMetadata(previous.contextConversationID)?.settingsJSON, oldSettings)
    }

    func testBuiltinLibraryBrowsesWithoutWritesAndCreatesEditableRoleCopy() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let before = try fixture.service.directory()
        let catalog = try fixture.service.roleTemplates()
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        workspace.agentModelOptions = { try fixture.repository.agentModelOptions() }
        workspace.agentReasoningEfforts = { try fixture.repository.agentReasoningEfforts(modelSelectionID: $0) }
        let host = NSHostingView(rootView: MacExpertDirectoryManagementView(workspace: workspace)
            .environment(\.accessibilityEnabled, true))
        let window = makeWindow(host, width: 980, height: 780)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let role = try XCTUnwrap(catalog.templates.first)
        try await wait { self.element(host, "mac-expert-builtin-" + role.id) != nil }
        XCTAssertEqual(catalog.templates.count, 7)
        attach(host, name: "expert-library-before-selection")
        for builtin in catalog.templates.dropFirst() + catalog.templates.prefix(1) {
            let button = try XCTUnwrap(element(host, "mac-expert-builtin-" + builtin.id))
            XCTAssertTrue(button.accessibilityPerformPress?() == true, "Could not press \(builtin.id)")
            let description = builtin.localizedDescription()
            try await wait {
                guard let preview = self.element(host, "mac-expert-builtin-preview") else { return false }
                return self.elements(preview).contains { node in
                    let value: Any? = node.accessibilityValue?() ?? nil
                    return (value as? String) == description || node.accessibilityLabel?() == description
                }
            }
            attach(host, name: "expert-preview-" + builtin.localizedName(locale: Locale(identifier: "en")))
        }
        XCTAssertEqual(try fixture.service.directory().agents.count, before.agents.count,
            "Browsing built-in roles must not create persistent agents")
        _ = element(host, "mac-expert-builtin-" + role.id)?.accessibilityPerformPress?()
        try await Task.sleep(for: .milliseconds(100))
        _ = element(host, "mac-expert-customize-builtin")?.accessibilityPerformPress?()
        try await wait { self.element(host, "mac-expert-save") != nil }
        _ = element(host, "mac-expert-save")?.accessibilityPerformPress?()
        try await wait { (try? fixture.service.directory().agents.count) == before.agents.count + 1 }
        let saved = try XCTUnwrap(fixture.service.directory().agents.first { agent in !before.agents.contains { $0.id == agent.id } })
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).templateID, role.id)
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).reasoningEffort, "automatic")
        XCTAssertEqual(try fixture.service.roleTemplates().templates.first?.prompt, role.prompt)
        XCTAssertNotNil(element(host, "mac-expert-edit-" + saved.id))
    }

    func testNativeExpertEditorSavesAndShowsRealRoleDetailsInBothAppearances() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        workspace.agentModelOptions = { try fixture.repository.agentModelOptions() }
        workspace.agentReasoningEfforts = { try fixture.repository.agentReasoningEfforts(modelSelectionID: $0) }
        let host = NSHostingView(rootView: MacExpertDirectoryManagementView(workspace: workspace).background(MacDesignColor.canvas)
            .environment(\.accessibilityEnabled, true).environment(\.colorScheme, .light).preferredColorScheme(.light))
        let window = makeWindow(host, width: 980, height: 780)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await wait { self.element(host, "mac-expert-edit-" + fixture.expert.id) != nil }
        _ = element(host, "mac-expert-edit-" + fixture.expert.id)?.accessibilityPerformPress?()
        try await wait { self.element(host, "mac-expert-role-prompt") != nil }
        XCTAssertNotNil(element(host, "mac-expert-reasoning-effort"))
        XCTAssertNotNil(element(host, "mac-expert-role"))
        XCTAssertNotNil(element(host, "mac-expert-change-template"))
        XCTAssertNil(element(host, "mac-expert-template"), "An applied role should not repeat its template selector")
        let name = try XCTUnwrap(elements(host).compactMap { $0 as? NSTextField }.first { $0.stringValue == fixture.expert.name })
        name.selectText(nil)
        let textEditor = try XCTUnwrap(name.currentEditor() as? NSTextView)
        textEditor.insertText("Native edited expert", replacementRange: NSRange(location: 0, length: textEditor.string.utf16.count))
        window.makeFirstResponder(host)
        try await Task.sleep(for: .milliseconds(100))
        _ = element(host, "mac-expert-save")?.accessibilityPerformPress?()
        try await wait { (try? fixture.service.directory().agents.first?.name) == "Native edited expert" }
        let saved = try XCTUnwrap(fixture.service.directory().agents.first)
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).reasoningEffort, "high")
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: saved.defaults).templateID, fixture.role.id)
        _ = element(host, "mac-expert-role-prompt")?.accessibilityPerformPress?()
        _ = element(host, "mac-expert-role-tools")?.accessibilityPerformPress?()
        for scheme: ColorScheme in [.light, .dark] {
            host.rootView = MacExpertDirectoryManagementView(workspace: workspace).background(MacDesignColor.canvas)
                .environment(\.accessibilityEnabled, true).environment(\.colorScheme, scheme).preferredColorScheme(scheme)
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            host.appearance = window.appearance
            try await Task.sleep(for: .milliseconds(150))
            attach(host, name: "expert-role-editor-\(scheme == .dark ? "dark" : "light")")
        }
        _ = element(host, "mac-expert-change-template")?.accessibilityPerformPress?()
        try await wait { self.element(host, "mac-expert-template") != nil }
        XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: fixture.service.directory().agents[0].defaults).templateID, fixture.role.id)
    }

    func testNativeAtSelectionBindsDirectoryExpertByIDWithoutModelRequest() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        let model = MacWorkspaceModel(canonicalRepository: fixture.repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: fixture.port)
        let owner = try XCTUnwrap(model.selectedConversationID)
        model.presentationDidAttach()
        await model.loadSelectedConversationIfNeeded()
        await workspace.restoreConversation(owner)
        let host = NSHostingView(rootView: ExpertMentionUITestRoot(model: model, workspace: workspace)
            .environment(\.accessibilityEnabled, true).environment(\.colorScheme, .light).preferredColorScheme(.light))
        let window = makeWindow(host, width: 980, height: 760)
        defer { model.presentationDidDetach(); window.orderOut(nil); window.contentView = nil; window.close() }
        try await Task.sleep(for: .milliseconds(200))
        let input = try XCTUnwrap(views(host).compactMap { $0 as? MacAttachmentPasteTextView }.first)
        XCTAssertTrue(window.makeFirstResponder(input))
        input.insertText("Review @Editor", replacementRange: NSRange(location: NSNotFound, length: 0))
        try await wait { self.element(host, "chat.mention." + fixture.expert.id) != nil }
        _ = element(host, "chat.mention." + fixture.expert.id)?.accessibilityPerformPress?()
        try await wait { model.draftMentions(owner).map { $0.expert.agentID } == [fixture.expert.id] && model.draft == "Review @"+fixture.expert.name+" " }
        XCTAssertNil(try fixture.service.forConversation(owner), "Selecting a candidate must only edit the draft")
        for scheme: ColorScheme in [.light, .dark] {
            host.rootView = ExpertMentionUITestRoot(model: model, workspace: workspace)
                .environment(\.accessibilityEnabled, true).environment(\.colorScheme, scheme).preferredColorScheme(scheme)
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            host.appearance = window.appearance
            try await Task.sleep(for: .milliseconds(150))
            attach(host, name: "expert-at-selected-\(scheme == .dark ? "dark" : "light")")
        }
    }

    func testJoinedSnapshotPinsMainSelectionAndDirectFanoutRequiresMainTurn() async throws {
        let fixture = try nativeFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let conversation = MacConversation(title: "Snapshot continuity", preview: "", updatedAt: .now, isPinned: false)
        _ = try fixture.repository.ensureConversation(conversation)
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        await workspace.restoreConversation(conversation.id)
        try await workspace.prepareExpertMention(agentID: fixture.expert.id, expectedRevision: fixture.expert.revision,
            conversationID: conversation.id, isPersisted: true)
        XCTAssertNil(try fixture.service.forConversation(conversation.id))
        let room = try fixture.service.ensureForConversation(conversation.id)
        _ = try fixture.service.join(agent: fixture.expert, room: room.room)
        let joined = try XCTUnwrap(fixture.service.forConversation(conversation.id))
        let original = try XCTUnwrap(joined.room.room.participants.first)
        _ = try fixture.service.updateAgent(fixture.expert, name: fixture.expert.name, description: "A newer directory definition",
            availability: "active", directory: fixture.service.directory(),
            defaults: .object(["modelSelectionID": .string("configured:native-fixture"), "expertReasoningEffort": .string("low")]))
        let messageID = UUID().uuidString.lowercased()
        let admitted = await workspace.sendExperts(conversationID: conversation.id, text: "Continue the selected snapshot",
            agentIDs: [fixture.expert.id], messageID: messageID)
        XCTAssertNil(admitted)
        XCTAssertTrue(workspace.admissionError?.contains("requires_main_agent_turn") == true)
        XCTAssertNil(try fixture.service.expertMessageSource(conversationID: conversation.id, messageID: messageID))
        let selected = try await workspace.mainExpertSelection(conversationID: conversation.id, agentIDs: [fixture.expert.id])
        XCTAssertEqual(selected.first?.agentRevision, fixture.expert.revision)
        XCTAssertEqual(selected.first?.contextConversationID, original.contextConversationID)
        let current = try XCTUnwrap(fixture.service.forConversation(conversation.id))
        XCTAssertEqual(current.room.room.participants.first?.id, original.id)
        XCTAssertEqual(current.room.room.participants.first?.agentRevision, original.agentRevision)
        // No execution owner was installed: only Core admission, never inference.
        XCTAssertTrue((workspace.submissions[current.roomID] ?? []).allSatisfy { $0.state != "executing" })
    }

    private func nativeFixture() throws -> (root: URL, port: MacApplicationPort, repository: MacCanonicalConversationRepository,
        service: MacAgentRoomService, expert: MacPersistentAgent, role: AppleExpertRoleTemplate) {
        let root = temporaryRoot()
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("native.sqlite"))
        let source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture", "endpointName": "Fixture",
            "baseURL": "https://fixture.invalid/v1", "endpointOrder": 0, "endpointEnabled": true,
            "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
            "wireProtocol": "responses", "credentialAccount": "fixture-account",
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true,
                "samplingParameters": false, "samplingRequiresNoReasoning": false, "chatOutputTokenField": "auto"],
            "reasoning": ["control": "effort", "efforts": ["minimal", "low", "medium", "high", "max"]]]
        try port.upsertModelProfile(.init(id: "native-fixture", displayName: "Declared five levels", provider: "openai-compatible", modelName: "fixture",
            runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        let service = MacAgentRoomService(runtime: port)
        let role = try XCTUnwrap(service.roleTemplates().templates.first)
        let directory = try service.createAgent(name: "Editor expert", description: "Native control test",
            defaults: .object(["modelSelectionID": .string("configured:native-fixture"),
                               "selectedPromptTemplateID": .string(role.id), "expertReasoningEffort": .string("high")]),
            directory: service.directory())
        return (root, port, MacCanonicalConversationRepository(store: port), service, try XCTUnwrap(directory.agents.first), role)
    }

    private func makeWindow<Content: View>(_ host: NSHostingView<Content>, width: CGFloat, height: CGFloat) -> NSWindow {
        host.frame = NSRect(x: 0, y: 0, width: width, height: height)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        return window
    }
    private func attach(_ host: NSView, name: String) {
        host.layoutSubtreeIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return XCTFail("Native bitmap unavailable") }
        (host.window?.effectiveAppearance ?? NSAppearance.current).performAsCurrentDrawingAppearance {
            host.cacheDisplay(in: host.bounds, to: bitmap)
        }
        guard let data = bitmap.representation(using: .png, properties: [:]) else { return XCTFail("PNG unavailable") }
        let image = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
        image.name = name; image.lifetime = .keepAlways; add(image)
    }
    private func wait(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(8)
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(30)) }
        XCTAssertTrue(condition())
    }
    private func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(views) }
    private func elements(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], seen = Set<ObjectIdentifier>(), index = 0, result: [AnyObject] = []
        while index < queue.count, index < 6000 {
            let object = queue[index]; index += 1
            guard seen.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = object as? NSView { queue += view.subviews }
        }
        return result
    }
    private func element(_ root: AnyObject, _ id: String) -> AnyObject? {
        elements(root).first { ($0.accessibilityIdentifier?() ?? nil) == id }
    }

    private func temporaryRoot() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("expert-role-\(UUID())")
    }
}

private struct ExpertMentionUITestRoot: View {
    let model: MacWorkspaceModel
    @ObservedObject var workspace: MacAgentRoomWorkspace
    @FocusState private var focused: Bool
    var body: some View {
        MacConversationCanvas(model: model, composerFocused: $focused,
            expertWorkspace: workspace)
            .background(MacDesignColor.canvas).tint(MacDesignColor.brand).onAppear { focused = true }
    }
}

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

    func testProductionExpertProjectionInlinesDelegatedPeerExchangesAfterSourceAssistantRun() throws {
        let user = transcriptMessage("parent-a", role: .user, time: 1)
        let assistant = transcriptMessage("run-main:assistant", role: .assistant, time: 2)
        let exchangeChild = transcriptMessage("exchange-1:assistant", role: .assistant, time: 3)
        let exchangeJSON = """
        {
          "version": 1,
          "exchange_id": "exchange-1",
          "room_id": "room",
          "parent_conversation_id": "chat",
          "source": { "kind": "main_agent", "context_conversation_id": "chat" },
          "target": {
            "participant_id": "a",
            "agent_id": "agent-a",
            "agent_revision": 1,
            "context_conversation_id": "context-a"
          },
          "source_run_id": "run-main",
          "source_call_id": "call-delegate-1",
          "source_display_name": "Nola",
          "target_display_name": "Expert a",
          "kind": "request",
          "text": "Review module",
          "in_reply_to": null,
          "root_exchange_id": "exchange-1",
          "hop": 0,
          "created_at_ms": 100,
          "clarifications": []
        }
        """
        let exchange = try JSONDecoder().decode(AppleExpertPeerView.self, from: Data(exchangeJSON.utf8))
        let rows = MacExpertTranscriptProjector.rows(
            conversationID: "chat",
            parentMessages: [user, assistant],
            association: transcriptAssociation(),
            receipts: [],
            contexts: ["context-a": [exchangeChild]],
            peerExchanges: [exchange]
        )
        XCTAssertEqual(rows.map { $0.message.canonicalMessageID }, ["parent-a", "run-main:assistant", "exchange-1:assistant"])
        XCTAssertEqual(rows.last?.name, "Expert a")
        XCTAssertEqual(rows.last?.context, "context-a")
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

@MainActor final class MacRoomRecoveryBoundaryTests: XCTestCase {
    func testPersistedClaimReplaysOriginalIntentWithoutRecapturing() async throws {
        let fixture = MacRoomBoundaryFixture()
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { receipt, intent in
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
        let fixture = MacRoomBoundaryFixture(existingRun: true)
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, _ in XCTFail("Existing Run must not dispatch") }
        workspace.prepareOwner = { _ in XCTFail("Must use persisted intent"); return fixture.original }
        workspace.recoverOwner = { receipt, intent in
            XCTAssertEqual(receipt.executionID, intent.execution_id)
            fixture.record("recover")
        }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        XCTAssertTrue(fixture.events.contains("recover"))
    }

    func testCancelledClaimCleansPreparedOwnerWithoutDispatch() async throws {
        let fixture = MacRoomBoundaryFixture(claimState: "cancelled")
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, _ in XCTFail("Cancelled claim cannot dispatch") }
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
        let fixture = MacRoomBoundaryFixture(persisted: false, prepareRejected: true)
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, _ in XCTFail("Failed prepare cannot dispatch") }
        workspace.prepareOwner = { _ in fixture.record("capture"); return fixture.original }
        workspace.cancelPreparedOwner = { _ in fixture.record("cleanup") }
        await workspace.startSelected(fixture.receipt(state: "selected"))
        XCTAssertEqual(fixture.events.filter { $0 == "capture" }.count, 1)
        XCTAssertTrue(fixture.events.contains("cleanup"))
        XCTAssertFalse(fixture.events.contains("claim_execution"))
    }

    func testLateReceiptCannotRegressCancellationOrOverwriteAnotherRoom() async throws {
        let fixture = MacRoomBoundaryFixture()
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
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
        let fixture = MacRoomBoundaryFixture(remote: true)
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, intent in
            XCTAssertEqual(intent, fixture.original)
            fixture.record("source-observe")
        }
        workspace.prepareOwner = { _ in XCTFail("Must not rebuild remote body"); return fixture.original }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        XCTAssertTrue(fixture.events.contains("source-observe"))
        XCTAssertFalse(fixture.events.contains("load_exact_run_invocation"))
    }
}

private nonisolated final class MacRoomBoundaryFixture: @unchecked Sendable {
    let original: MacAgentRoomService.ExecutionIntent
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
            original = .init(execution_id: id, envelope: .init(kind: "native", platform: "macos",
                conversation_configuration_revision: 7, model_profile_id: "profile", model_profile_revision: 2,
                expected_parent_message_id: "parent", text: "original"))
        }
    }
    func receipt(state: String, revision: Int64 = 3, roomID: String = "room-a", text: String = "original") -> MacRoomSubmission {
        .init(revision: revision, input: .init(roomID: roomID, submissionID: "submission",
            expectedRoomRevision: 1, text: text, participantID: "member", atMs: 1),
            state: state, selectedParticipantID: "member", failureCode: nil, executionID: original.execution_id,
            terminal: nil, contextConversationID: "context")
    }
    var service: MacAgentRoomService { .init(execute: { [self] root in
        guard case .object(let fields) = root else { throw CocoaError(.coderInvalidValue) }
        let command: [String: MacCanonicalJSONValue]
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
        case "submissions":
            if command["room_id"] == .string("room-b") { return .array([]) }
            return .array([try value(receipt(state: "cancelled", revision: 8))])
        case "read": return try value(MacAgentRoomSnapshot(revision: 1,
            room: .init(id: "room-b", title: "B", participants: [])))
        case "get_product_setting": return .null
        default: throw CocoaError(.coderInvalidValue)
        }
    }) }
    private func value<T: Encodable>(_ value: T) throws -> MacCanonicalJSONValue {
        try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(value))
    }
}
