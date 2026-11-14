// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

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

    func testScopedPeerAuthorsUseFrozenNamesAndRejectForeignOrUnmappedMessages() throws {
        let exchange = try JSONDecoder().decode(AppleExpertPeerView.self, from: Data(Self.peerJSON.utf8))
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
