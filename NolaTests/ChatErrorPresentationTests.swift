import XCTest
@testable import Nola

@MainActor
final class ChatErrorPresentationTests: XCTestCase {
    func testFailedReloadKeepsOnlyConfirmedSameConversationPresentationAndReportsError() {
        let owner = UUID(), other = UUID()
        let confirmed = [ChatMessage(role: .assistant, content: "Already displayed")]
        var recovery = ChatConversationLoadRecovery()
        recovery.didLoad(owner)
        let retained = recovery.didFail(owner, message: "temporary read failure", existing: confirmed, submitted: [])
        XCTAssertEqual(retained.map(\.id), confirmed.map(\.id))
        XCTAssertEqual(recovery.failure?.message, "temporary read failure")
        XCTAssertEqual(recovery.failure?.conversationID, owner)
        let otherDraft = ChatMessage(role: .user, content: "Other conversation draft")
        let switched = recovery.didFail(other, message: "unavailable", existing: retained, submitted: [otherDraft])
        XCTAssertEqual(switched.map(\.id), [otherDraft.id])
        XCTAssertNil(recovery.confirmedConversationID)
        XCTAssertTrue(recovery.didFail(owner, message: "still unavailable", existing: switched, submitted: []).isEmpty)
    }

    func testSuccessfulRetryClearsFailureAndRetirementCannotResurrectConfirmedHistory() {
        let owner = UUID()
        let confirmed = [ChatMessage(role: .assistant, content: "History")]
        var recovery = ChatConversationLoadRecovery()
        recovery.didLoad(owner)
        _ = recovery.didFail(owner, message: "retry", existing: confirmed, submitted: [])
        recovery.didLoad(owner)
        XCTAssertNil(recovery.failure)
        recovery.retire()
        XCTAssertNil(recovery.confirmedConversationID)
        XCTAssertTrue(recovery.didFail(owner, message: "missing", existing: confirmed, submitted: []).isEmpty)
    }

    func testDelayedNewConversationSelectionPreservesSubmittedTurnAndResponseOwner() {
        let user = ChatMessage(role: .user, content: "Hello")
        let assistant = ChatMessage(role: .assistant, content: "")
        let submitted = [user, assistant]
        let loaded = ChatSubmissionPresentationPolicy.merging(persisted: [], submitted: submitted)
        XCTAssertEqual(loaded.map(\.id), [user.id, assistant.id])
        XCTAssertEqual(ChatSubmissionPresentationPolicy.activeAssistant(
            submitted: assistant.id, persisted: []), assistant.id)
    }

    func testSubmissionRetiresOnlyAfterAllMessageIDsArePersisted() {
        let user = ChatMessage(role: .user, content: "Hello")
        let assistant = ChatMessage(role: .assistant, content: "draft")
        let submitted = [user, assistant]
        XCTAssertEqual(ChatSubmissionPresentationPolicy.pendingSubmission(
            submitted, persisted: [user])?.map(\.id), submitted.map(\.id))
        XCTAssertNil(ChatSubmissionPresentationPolicy.pendingSubmission(submitted, persisted: submitted))
        XCTAssertNil(ChatSubmissionPresentationPolicy.pendingSubmission(nil, persisted: []))
    }

    func testExplicitConversationDeletionRetiresEvenUnpersistedSubmission() {
        let submitted = [ChatMessage(role: .user, content: "Hello"),
                         ChatMessage(role: .assistant, content: "draft")]
        XCTAssertNil(ChatSubmissionPresentationPolicy.pendingSubmission(
            submitted, persisted: [], conversationExists: false))
        XCTAssertEqual(ChatSubmissionPresentationPolicy.pendingSubmission(
            submitted, persisted: [])?.count, 2)
    }

    func testPartialPersistenceDoesNotDuplicateSubmissionAndCanonicalContentWins() {
        let user = ChatMessage(role: .user, content: "Hello")
        let draft = ChatMessage(role: .assistant, content: "stream")
        let partial = ChatSubmissionPresentationPolicy.merging(persisted: [user], submitted: [user, draft])
        XCTAssertEqual(partial.map(\.id), [user.id, draft.id])
        let committed = ChatMessage(id: draft.id, role: .assistant, content: "committed")
        let loaded = ChatSubmissionPresentationPolicy.merging(
            persisted: [user, committed], submitted: [user, draft])
        XCTAssertEqual(loaded.count, 2)
        XCTAssertEqual(loaded.last?.content, "committed")
    }

    func testConversationOwnedDraftSurvivesSwitchWithoutLeakingToOtherConversation() {
        let owner = UUID(), other = UUID()
        let user = ChatMessage(role: .user, content: "Hello")
        let assistant = ChatMessage(role: .assistant, content: "")
        var turns = [owner: [user, assistant]]
        XCTAssertTrue(ChatSubmissionPresentationPolicy.merging(
            persisted: [], submitted: turns[other] ?? []).isEmpty)
        ChatMessageDraftReducer.upsertAssistant(
            in: &turns[owner]!, assistantID: assistant.id, afterUserID: user.id,
            content: "completed while hidden")
        let restored = ChatSubmissionPresentationPolicy.merging(persisted: [], submitted: turns[owner] ?? [])
        XCTAssertEqual(restored.map(\.id), [user.id, assistant.id])
        XCTAssertEqual(restored.last?.content, "completed while hidden")
    }

    func testDismissAcknowledgesOccurrenceWithoutChangingRuntimeState() throws {
        let owner = UUID()
        let runtime = ChatProcessingState.error("connection lost")
        var presentation = ChatErrorPresentation()
        presentation.observe(runtime, conversationID: owner)
        let occurrence = try XCTUnwrap(presentation.occurrenceID)
        XCTAssertTrue(presentation.dismiss(occurrenceID: occurrence, conversationID: owner))
        XCTAssertNil(presentation.message)
        XCTAssertEqual(runtime, .error("connection lost"))
        presentation.observe(runtime, conversationID: owner)
        XCTAssertEqual(presentation.message, "connection lost")
        XCTAssertNotEqual(presentation.occurrenceID, occurrence)
        XCTAssertFalse(presentation.dismiss(occurrenceID: occurrence, conversationID: owner))
    }

    func testOldConversationAndOldErrorCannotDismissCurrentPresentation() throws {
        let first = UUID(), second = UUID()
        var presentation = ChatErrorPresentation()
        presentation.observe(.error("same text"), conversationID: first)
        let previous = try XCTUnwrap(presentation.occurrenceID)
        XCTAssertFalse(presentation.dismiss(occurrenceID: previous, conversationID: second))
        presentation.observe(.error("same text"), conversationID: second)
        XCTAssertFalse(presentation.dismiss(occurrenceID: previous, conversationID: second))
        XCTAssertEqual(presentation.message, "same text")
        presentation.observe(.processingWithCloud, conversationID: second)
        XCTAssertNil(presentation.message)
        XCTAssertNil(presentation.occurrenceID)
    }
}
