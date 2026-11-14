// SPDX-License-Identifier: Apache-2.0
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
    private func peerView(name: String? = "Frozen source expert", kind: String = "request") -> AppleExpertPeerView {
        let source = AppleExpertParticipantReference(participant_id: "source-member", agent_id: "source-agent", agent_revision: 3, context_conversation_id: "source-context")
        let target = AppleExpertParticipantReference(participant_id: "target-member", agent_id: "target-agent", agent_revision: 5, context_conversation_id: "target-context")
        return .init(version: 1, exchangeID: "exchange-run", roomID: "room", parentConversationID: "parent",
            source: .legacyExpert(source), target: target, sourceRunID: "source-run", sourceCallID: "source-call",
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
        let page = AppleExpertPeerExchanges(version: 1, conversationID: "target-context", exchanges: [exchange], hasMore: false, nextOffset: nil)
        XCTAssertNoThrow(try page.validate(contextID: "target-context", offset: 0))
        XCTAssertThrowsError(try page.validate(contextID: "foreign-context", offset: 0))
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertPeerExchanges.self, from: JSONEncoder().encode(page)), page)
    }
}
