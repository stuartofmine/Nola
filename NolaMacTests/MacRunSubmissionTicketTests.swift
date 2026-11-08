import XCTest
@testable import Nola

@MainActor
final class MacRunSubmissionTicketTests: XCTestCase {
    private func ticket() -> MacRunSubmissionTicket {
        .init(identity: .init(conversationID: "conversation", submissionID: "run", generationID: 1))
    }

    func testAbandonmentPermanentlyPreventsDispatch() throws {
        let ticket = ticket()
        try ticket.checkPreparation()
        XCTAssertTrue(ticket.cancelPreparation())
        XCTAssertFalse(ticket.cancelPreparation())
        XCTAssertThrowsError(try ticket.claimDispatch())
        XCTAssertThrowsError(try ticket.checkPreparation())
        XCTAssertFalse(ticket.markAccepted())
        ticket.rejectBeforeTransport()
        XCTAssertEqual(ticket.phase, .abandoned)
    }

    func testClaimIsSingleUseAndIsNotAcceptance() throws {
        let ticket = ticket()
        XCTAssertFalse(ticket.markAccepted())
        try ticket.claimDispatch()
        XCTAssertEqual(ticket.phase, .dispatching)
        XCTAssertFalse(ticket.cancelPreparation())
        XCTAssertThrowsError(try ticket.claimDispatch())
        XCTAssertTrue(ticket.markAccepted())
        XCTAssertFalse(ticket.markAccepted())
        ticket.rejectBeforeTransport()
        XCTAssertEqual(ticket.phase, .accepted)
    }

    func testDefinitePreparationAndPreTransportFailuresCannotBeRetriedWithSameTicket() throws {
        for claimed in [false, true] {
            let ticket = ticket()
            if claimed { try ticket.claimDispatch() }
            ticket.rejectBeforeTransport()
            XCTAssertEqual(ticket.phase, .rejectedBeforeDispatch)
            XCTAssertThrowsError(try ticket.claimDispatch())
            XCTAssertFalse(ticket.markAccepted())
        }
    }

    func testCancellationAndDispatchHaveExactlyOneWinnerAcrossTasks() async {
        for _ in 0..<100 {
            let ticket = ticket()
            let winners = await withTaskGroup(of: Int.self, returning: Int.self) { group in
                group.addTask {
                    do { try ticket.claimDispatch(); return 1 } catch { return 0 }
                }
                group.addTask { ticket.cancelPreparation() ? 1 : 0 }
                var count = 0
                for await winner in group { count += winner }
                return count
            }
            XCTAssertEqual(winners, 1)
            XCTAssertTrue(ticket.phase == .dispatching || ticket.phase == .abandoned)
        }
    }

    func testOldTicketCannotInvalidateAnotherSubmission() throws {
        let old = ticket()
        let next = MacRunSubmissionTicket(identity: .init(conversationID: "conversation",
            submissionID: "next-run", generationID: 2))
        XCTAssertTrue(old.cancelPreparation())
        try next.claimDispatch()
        XCTAssertEqual(next.phase, .dispatching)
        XCTAssertEqual(old.identity.submissionID, "run")
        XCTAssertEqual(next.identity.generationID, 2)
    }

    func testTypedAdmissionDecisionCannotUndoAcceptedOrRejectedSubmission() throws {
        let rejected = ticket()
        XCTAssertFalse(rejected.rejectBeforeAdmission())
        try rejected.claimDispatch()
        XCTAssertTrue(rejected.rejectBeforeAdmission())
        XCTAssertEqual(rejected.phase, .rejectedBeforeAdmission)
        XCTAssertFalse(rejected.markAccepted())
        XCTAssertThrowsError(try rejected.claimDispatch())
        let accepted = ticket()
        try accepted.claimDispatch()
        XCTAssertTrue(accepted.markAccepted())
        XCTAssertFalse(accepted.rejectBeforeAdmission())
        XCTAssertEqual(accepted.phase, .accepted)
    }

    func testHostDecisionRequiresExactIdentityAndTypedReason() throws {
        let admitted = #"{"type":"submission_admitted","conversation_id":"c","run_id":"r","assistant_message_id":"a"}"#
        XCTAssertEqual(try MacHostSubmissionDecision.decode(admitted,
            conversationID: "c", runID: "r", assistantMessageID: "a")?.type, .admitted)
        let rejected = #"{"type":"submission_rejected","conversation_id":"c","run_id":"r","assistant_message_id":"a","code":"request_rejected_before_admission","message":"Rejected"}"#
        XCTAssertEqual(try MacHostSubmissionDecision.decode(rejected,
            conversationID: "c", runID: "r", assistantMessageID: "a")?.type, .rejected)
        for owner in [("wrong", "r", "a"), ("c", "wrong", "a"), ("c", "r", "wrong")] {
            XCTAssertThrowsError(try MacHostSubmissionDecision.decode(rejected,
                conversationID: owner.0, runID: owner.1, assistantMessageID: owner.2))
        }
        let malformed = #"{"type":"submission_rejected","conversation_id":"c","run_id":"r","assistant_message_id":"a"}"#
        XCTAssertThrowsError(try MacHostSubmissionDecision.decode(malformed,
            conversationID: "c", runID: "r", assistantMessageID: "a"))
        XCTAssertNil(try MacHostSubmissionDecision.decode("Host invocation unavailable: incompatible checkpoint",
            conversationID: "c", runID: "r", assistantMessageID: "a"))
    }
}
