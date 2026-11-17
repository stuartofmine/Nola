// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS run lifecycle, cancellation, completion side effects, coordinator state machine, recovery inspection, submission preparation/tickets, snapshot retirement, and unknown-effect recovery suite.

import Foundation
import XCTest
@testable import Nola

// Wire fixture only: production digest/source verification belongs to Store.
func macRunResultFixture(_ kind: String, conversation: String = "conversation", run: String = "run") -> [String: Any] {
    let value: [String: Any]
    switch kind {
    case "answered": value = ["kind": kind, "answer_part_refs": ["answer"]]
    case "action_completed": value = ["kind": kind, "requirement_acceptance_refs": ["assessment"]]
    case "incomplete": value = ["kind": kind, "reason": ["code": "unfinished", "detail": NSNull()], "remaining_requirement_refs": ["requirement"], "obligation_refs": []]
    case "blocked": value = ["kind": kind, "reason": ["code": "unavailable", "detail": NSNull()], "blocking_fact_refs": ["fact"], "resolution_condition": "Resource restored"]
    case "cancelled": value = ["kind": kind, "cancellation_id": "cancel", "observed_effect_refs": [], "unknown_effect_refs": []]
    case "failed": value = ["kind": kind, "error_ref": "error", "observed_effect_refs": [], "unknown_effect_refs": []]
    default: value = ["kind": kind]
    }
    return ["result_id": "result", "run_id": run, "conversation_id": conversation,
     "originating_submission_id": "submission", "input": ["watermark": "1", "applications_digest": String(repeating: "0", count: 64)],
     "decided_at_revision": "2", "decision_event_cursor": "3", "requirement_revision": "0", "evidence_frontier": "0",
     "value": value]
}

private actor CompletionDeliveryRecorder {
    private var values: [MacRunNotificationService.AgentCompletionNotification] = []
    func append(_ value: MacRunNotificationService.AgentCompletionNotification) { values.append(value) }
    func snapshot() -> [MacRunNotificationService.AgentCompletionNotification] { values }
}

final class MacRunCancellationAndOutcomeTests: XCTestCase {
    private func request() throws -> MacRunCancellationRequest {
        try .init(conversationID: "conversation", expectedRunID: "run",
            assistantMessageID: "assistant", cancellationID: "cancellation", requestedAtMs: 123)
    }

    private func receipt(_ disposition: String = "cancellation_committed", status: Any = NSNull()) -> [String: Any] {
        let result: Any
        let digest: Any
        if let kind = status as? String {
            result = macRunResultFixture(kind)
            digest = String(repeating: "0", count: 64)
        } else {
            result = NSNull()
            digest = NSNull()
        }
        return ["conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant",
         "cancellation_id": "cancellation", "disposition": disposition,
         "terminal_result": result, "result_digest": digest]
    }

    private func decode(_ object: [String: Any]) throws -> MacRunCancellationReceipt {
        try JSONDecoder().decode(MacRunCancellationReceipt.self,
            from: JSONSerialization.data(withJSONObject: object))
    }

    func testResultStructureAndCancellationRequestReceiptValidation() throws {
        let base: [String: Any] = ["result_id": "result", "run_id": "run", "conversation_id": "conversation",
            "originating_submission_id": "submission",
            "input": ["watermark": "1", "applications_digest": String(repeating: "a", count: 64)],
            "decided_at_revision": "2", "decision_event_cursor": "3", "requirement_revision": "0", "evidence_frontier": "0",
            "value": ["kind": "answered", "answer_part_refs": ["answer"]]]
        func decodeResult(_ object: [String: Any]) throws -> MacRunResult {
            try JSONDecoder().decode(MacRunResult.self, from: JSONSerialization.data(withJSONObject: object))
        }
        _ = try decodeResult(base)
        let badValues: [[String: Any]] = [
            ["kind": "blocked", "reason": ["code": "blocked", "detail": NSNull()], "blocking_fact_refs": [], "resolution_condition": "Restore"],
            ["kind": "incomplete", "reason": ["code": "unfinished", "detail": NSNull()], "remaining_requirement_refs": [], "obligation_refs": []],
            ["kind": "failed", "error_ref": "error", "observed_effect_refs": ["effect"], "unknown_effect_refs": ["effect"]],
            ["kind": "cancelled", "cancellation_id": "cancel", "observed_effect_refs": ["effect"], "unknown_effect_refs": ["effect"]],
            ["kind": "blocked", "reason": ["code": "Localized reason", "detail": NSNull()], "blocking_fact_refs": ["fact"], "resolution_condition": "Restore"],
            ["kind": "answered", "answer_part_refs": [" answer"]],
        ]
        for value in badValues {
            var broken = base; broken["value"] = value
            XCTAssertThrowsError(try decodeResult(broken))
        }
        for field in ["decided_at_revision", "decision_event_cursor", "requirement_revision", "evidence_frontier"] {
            var valid = base; valid[field] = String(Int64.max); _ = try decodeResult(valid)
            var broken = base; broken[field] = "9223372036854775808"
            XCTAssertThrowsError(try decodeResult(broken), field)
        }
        let empty = "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"
        for input in [["watermark": "0", "applications_digest": String(repeating: "a", count: 64)],
                      ["watermark": "1", "applications_digest": empty],
                      ["watermark": "9223372036854775808", "applications_digest": String(repeating: "a", count: 64)]] {
            var broken = base; broken["input"] = input
            XCTAssertThrowsError(try decodeResult(broken))
        }
        var validEmpty = base; validEmpty["input"] = ["watermark": "0", "applications_digest": empty]
        _ = try decodeResult(validEmpty)

        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request())) as? [String: Any])
        XCTAssertEqual(Set(object.keys), ["conversation_id", "expected_run_id", "assistant_message_id", "cancellation_id", "reason", "requested_at_ms"])
        XCTAssertTrue(object["reason"] is NSNull)
        XCTAssertEqual(object["requested_at_ms"] as? Int64, 123)
        XCTAssertEqual(object["expected_run_id"] as? String, "run")

        for identity in ["", " \n", String(repeating: "界", count: 342)] {
            XCTAssertThrowsError(try MacRunCancellationRequest(conversationID: identity,
                expectedRunID: "run", assistantMessageID: "assistant", cancellationID: "id", requestedAtMs: 1))
        }
        for time: Int64 in [0, -1] {
            XCTAssertThrowsError(try MacRunCancellationRequest(conversationID: "c",
                expectedRunID: "r", assistantMessageID: "a", cancellationID: "i", requestedAtMs: time))
        }
        XCTAssertThrowsError(try MacRunCancellationRequest(conversationID: "c", expectedRunID: "r",
            assistantMessageID: "a", cancellationID: "i", reason: String(repeating: "界", count: 1366), requestedAtMs: 1))
        let exact = try MacRunCancellationRequest(conversationID: " c ", expectedRunID: "r",
            assistantMessageID: "a", cancellationID: String(repeating: "x", count: 1024),
            reason: String(repeating: "x", count: 4096), requestedAtMs: Int64.max)
        XCTAssertEqual(exact.conversationID, " c ")

        XCTAssertNil(try decode(receipt()).terminalResult)
        XCTAssertNil(try decode(receipt("already_cancelling")).terminalResult)
        for status in ["answered", "action_completed", "incomplete", "blocked", "failed", "cancelled"] {
            XCTAssertEqual(try decode(receipt("already_terminal", status: status)).terminalResult?.value.kind.rawValue, status)
            XCTAssertEqual(try decode(receipt(status: status)).terminalResult?.value.kind.rawValue, status)
        }
        XCTAssertThrowsError(try decode(receipt("already_terminal")))
        XCTAssertThrowsError(try decode(receipt("already_cancelling", status: "cancelled")))

        var missing = receipt(); missing.removeValue(forKey: "terminal_result")
        XCTAssertThrowsError(try decode(missing))
        var extra = receipt(); extra["grant_workspace"] = true
        XCTAssertThrowsError(try decode(extra))
        var emptyID = receipt(); emptyID["run_id"] = " "
        XCTAssertThrowsError(try decode(emptyID))
        XCTAssertThrowsError(try decode(receipt("accepted")))
        XCTAssertThrowsError(try decode(receipt("already_terminal", status: "interrupted")))
        var wrongOwner = receipt("already_terminal", status: "answered")
        wrongOwner["terminal_result"] = macRunResultFixture("answered", run: "other")
        XCTAssertThrowsError(try decode(wrongOwner))
        var missingDigest = receipt("already_terminal", status: "answered")
        missingDigest["result_digest"] = NSNull()
        XCTAssertThrowsError(try decode(missingDigest))

        let original = try request()
        XCTAssertNoThrow(try decode(receipt()).validated(for: original))
        for key in ["conversation_id", "run_id", "assistant_message_id", "cancellation_id"] {
            var other = receipt(); other[key] = "replacement"
            XCTAssertThrowsError(try decode(other).validated(for: original)) { error in
                XCTAssertEqual(error as? MacRunCancellationContractError, .identityMismatch)
            }
        }
    }

    @MainActor
    func testCompletionSideEffectsActivityPersistenceNotificationsAndAttentionSemantics() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = MacActivityStore(baseDirectory: directory)
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            store.recordConversationRun(title: status.rawValue, conversationID: status.rawValue,
                execution: .thisMac, status: status, startedAt: Date(timeIntervalSince1970: 1), output: "retained partial")
        }
        XCTAssertNil(store.lastError)
        let reopened = MacActivityStore(baseDirectory: directory)
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            let entry = try XCTUnwrap(reopened.entries.first { $0.conversationID == status.rawValue })
            XCTAssertEqual(entry.status, .init(result: status))
            XCTAssertEqual(entry.outputPreview, "retained partial")
        }
        XCTAssertEqual(reopened.entries.count, 6)
        XCTAssertEqual(reopened.entries.first { $0.status == .cancelled }?.detail, String(localized: "Cancelled"))

        let recorder = CompletionDeliveryRecorder()
        let enabledService = MacRunNotificationService(completionNotificationsEnabled: { true },
            deliverAgentCompletion: { await recorder.append($0) })
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            await enabledService.notifyAgentCompletion(conversationTitle: status.rawValue, conversationID: status.rawValue, status: status)
        }
        let deliveries = await recorder.snapshot()
        XCTAssertEqual(deliveries.map(\.conversationID), ["answered", "action_completed", "incomplete", "blocked", "failed"])
        XCTAssertEqual(deliveries.map(\.title), [String(localized: "Nola answered"), String(localized: "Nola completed the action"), String(localized: "Nola has unfinished work"), String(localized: "Nola is blocked"), String(localized: "Nola encountered an error")])
        XCTAssertEqual(deliveries.map(\.body), ["answered", "action_completed", "incomplete", "blocked", "failed"])

        let disabledRecorder = CompletionDeliveryRecorder()
        let disabledService = MacRunNotificationService(completionNotificationsEnabled: { false },
            deliverAgentCompletion: { await disabledRecorder.append($0) })
        await disabledService.notifyAgentCompletion(conversationTitle: "Done", conversationID: "c", status: .answered)
        let disabledDeliveries = await disabledRecorder.snapshot()
        XCTAssertTrue(disabledDeliveries.isEmpty)

        for status in [MacActivityEntry.Status.answered, .actionCompleted, .delegationAccepted, .cancelled, .executionFinished] {
            XCTAssertFalse(status.needsAttention)
        }
        for status in [MacActivityEntry.Status.incomplete, .blocked, .failed] {
            XCTAssertTrue(status.needsAttention)
        }
        XCTAssertNotEqual(MacRunCompletionStatus.answered.title, MacRunCompletionStatus.actionCompleted.title)
        for status in [MacActivityEntry.Status.answered, .actionCompleted] {
            XCTAssertTrue(status.isSuccessful, status.rawValue)
        }
        for status in [MacActivityEntry.Status.incomplete, .blocked, .failed, .delegationAccepted, .cancelled, .executionFinished] {
            XCTAssertFalse(status.isSuccessful, status.rawValue)
        }
        let missingStatus: MacActivityEntry.Status? = nil
        XCTAssertFalse(missingStatus?.isSuccessful == true)
    }
}

@MainActor
final class MacRunCoordinatorAndTicketTests: XCTestCase {
    private func identity(conversationID: String = "conversation", runID: String? = "canonical-run") -> MacRunCoordinator.Identity {
        .init(conversationID: conversationID, runID: runID, assistantMessageID: UUID())
    }

    private func ticket() -> MacRunSubmissionTicket {
        .init(identity: .init(conversationID: "conversation", submissionID: "run", generationID: 1))
    }

    private enum SnapshotFailure: Error { case unavailable }

    private func snapshotTerminal(run: String = "run") -> MacCanonicalRunTerminal {
        let result = try! JSONDecoder().decode(MacRunResult.self,
            from: JSONSerialization.data(withJSONObject: macRunResultFixture("answered", run: run)))
        return .init(conversationID: "conversation", runID: run, assistantMessageID: "\(run):assistant",
              status: .sealed, result: result, resultDigest: String(repeating: "0", count: 64), completedAtMs: 42)
    }

    func testCoordinatorQuestionSuspensionTransportInterruptionAndGenerationIsolation() {
        let qCoordinator = MacRunCoordinator()
        let qGen = qCoordinator.begin(identity: identity())
        XCTAssertFalse(qCoordinator.waitSuspended(generationID: qGen, conversationID: "other", runID: "canonical-run"))
        XCTAssertFalse(qCoordinator.waitSuspended(generationID: qGen, conversationID: "conversation", runID: "other"))
        XCTAssertFalse(qCoordinator.waitSuspended(generationID: qGen &+ 1, conversationID: "conversation", runID: "canonical-run"))
        XCTAssertEqual(qCoordinator.state, .running)
        XCTAssertTrue(qCoordinator.waitSuspended(generationID: qGen, conversationID: "conversation", runID: "canonical-run"))
        XCTAssertEqual(qCoordinator.state, .awaitingInput)
        XCTAssertTrue(qCoordinator.canResume)
        XCTAssertFalse(qCoordinator.canFinish(generationID: qGen))
        XCTAssertTrue(qCoordinator.requestCancellation(generationID: qGen))
        XCTAssertFalse(qCoordinator.waitSuspended(generationID: qGen, conversationID: "conversation", runID: "canonical-run"))
        XCTAssertEqual(qCoordinator.state, .cancelRequested)
        qCoordinator.cancellationFailed(generationID: qGen)
        XCTAssertEqual(qCoordinator.state, .awaitingInput)
        XCTAssertTrue(qCoordinator.confirmTerminal(generationID: qGen, conversationID: "conversation", runID: "canonical-run", status: .cancelled))
        XCTAssertFalse(qCoordinator.waitSuspended(generationID: qGen, conversationID: "conversation", runID: "canonical-run"))

        XCTAssertNil(MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: "run", openWaits: [], observedWaitIDs: []))
        XCTAssertNil(MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: nil, openWaits: [], observedWaitIDs: ["wait"]))
        XCTAssertNil(MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: "successor", openWaits: [], observedWaitIDs: ["wait"]))
        let suspension = MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: "run", openWaits: [], observedWaitIDs: ["wait"])
        XCTAssertEqual(suspension?.runID, "run")
        XCTAssertEqual(suspension?.readyToResume, true)
        XCTAssertEqual(suspension?.waits.count, 0)

        let eofCoordinator = MacRunCoordinator()
        let eofRun = identity()
        let eofGen = eofCoordinator.begin(identity: eofRun)
        XCTAssertEqual(eofCoordinator.state, .running)
        XCTAssertFalse(eofCoordinator.canFinish(generationID: eofGen))
        XCTAssertTrue(eofCoordinator.transportInterrupted(generationID: eofGen))
        XCTAssertEqual(eofCoordinator.state, .awaitingRecovery)
        XCTAssertEqual(eofCoordinator.identity, eofRun)
        XCTAssertTrue(eofCoordinator.canResume)
        XCTAssertFalse(eofCoordinator.canFinish(generationID: eofGen))

        let genCoordinator = MacRunCoordinator()
        let prev = identity(runID: "previous-run")
        let oldGen = genCoordinator.begin(identity: prev)
        let curr = identity(conversationID: "other-conversation", runID: "new-run")
        let currGen = genCoordinator.begin(identity: curr)
        XCTAssertNotEqual(currGen, oldGen)
        XCTAssertFalse(genCoordinator.confirmTerminal(generationID: oldGen, conversationID: prev.conversationID, runID: "previous-run", status: .answered))
        XCTAssertFalse(genCoordinator.transportInterrupted(generationID: oldGen))
        XCTAssertFalse(genCoordinator.requestCancellation(generationID: oldGen))
        XCTAssertFalse(genCoordinator.confirmTerminal(generationID: currGen, conversationID: "wrong-conversation", runID: "new-run", status: .answered))
        XCTAssertFalse(genCoordinator.confirmTerminal(generationID: currGen, conversationID: curr.conversationID, runID: "wrong-run", status: .failed))
        XCTAssertEqual(genCoordinator.identity, curr)
        XCTAssertEqual(genCoordinator.state, .running)
        XCTAssertFalse(genCoordinator.canFinish(generationID: currGen))
    }

    func testCoordinatorCancellationDeliveryRacesAndAuthoritativeCompletion() {
        let c1 = MacRunCoordinator()
        let r1 = identity()
        let g1 = c1.begin(identity: r1)
        XCTAssertTrue(c1.requestCancellation(generationID: g1))
        XCTAssertEqual(c1.state, .cancelRequested)
        XCTAssertFalse(c1.canFinish(generationID: g1))
        XCTAssertFalse(c1.requestCancellation(generationID: g1))
        c1.cancellationFailed(generationID: g1)
        XCTAssertEqual(c1.state, .running)
        XCTAssertTrue(c1.requestCancellation(generationID: g1))
        XCTAssertTrue(c1.confirmTerminal(generationID: g1, conversationID: r1.conversationID, runID: "canonical-run", status: .cancelled))
        XCTAssertEqual(c1.state, .confirmedTerminal(.cancelled))
        XCTAssertTrue(c1.canFinish(generationID: g1))
        XCTAssertFalse(c1.canResume)

        let c2 = MacRunCoordinator()
        let r2 = identity()
        let g2 = c2.begin(identity: r2)
        XCTAssertTrue(c2.requestCancellation(generationID: g2))
        XCTAssertTrue(c2.claimCancellationDelivery(generationID: g2))
        XCTAssertFalse(c2.claimCancellationDelivery(generationID: g2))
        c2.cancellationDeliveryInterrupted(generationID: g2 &+ 1)
        XCTAssertFalse(c2.claimCancellationDelivery(generationID: g2))
        c2.cancellationDeliveryInterrupted(generationID: g2)
        XCTAssertEqual(c2.state, .cancelRequested)
        XCTAssertEqual(c2.cancellationRequestedGenerationID, g2)
        XCTAssertFalse(c2.canResume)
        XCTAssertFalse(c2.canFinish(generationID: g2))
        XCTAssertTrue(c2.claimCancellationDelivery(generationID: g2))
        XCTAssertTrue(c2.confirmTerminal(generationID: g2, conversationID: r2.conversationID, runID: "canonical-run", status: .answered))
        c2.cancellationDeliveryInterrupted(generationID: g2)
        XCTAssertFalse(c2.claimCancellationDelivery(generationID: g2))
        XCTAssertEqual(c2.state, .confirmedTerminal(.answered))

        let c3 = MacRunCoordinator()
        let g3 = c3.begin(identity: identity())
        XCTAssertTrue(c3.requestCancellation(generationID: g3))
        XCTAssertTrue(c3.transportInterrupted(generationID: g3))
        XCTAssertEqual(c3.state, .cancelRequested)
        c3.cancellationFailed(generationID: g3)
        XCTAssertEqual(c3.state, .awaitingRecovery)
        XCTAssertTrue(c3.canResume)
        XCTAssertFalse(c3.canFinish(generationID: g3))

        let c4 = MacRunCoordinator()
        let r4 = identity()
        let g4 = c4.begin(identity: r4)
        XCTAssertTrue(c4.requestCancellation(generationID: g4))
        XCTAssertTrue(c4.confirmTerminal(generationID: g4, conversationID: r4.conversationID, runID: "canonical-run", status: .answered))
        c4.cancellationFailed(generationID: g4)
        XCTAssertEqual(c4.state, .confirmedTerminal(.answered))
        XCTAssertTrue(c4.canFinish(generationID: g4))
    }

    func testCoordinatorResumeTerminalIdempotenceFinishOwnerAndTaskCancellation() async throws {
        let coordinator = MacRunCoordinator()
        let run = identity()
        let initialGeneration = coordinator.begin(identity: run)
        XCTAssertTrue(coordinator.transportInterrupted(generationID: initialGeneration))
        let resumedGeneration = try XCTUnwrap(coordinator.resume(generationID: initialGeneration))
        XCTAssertNotEqual(resumedGeneration, initialGeneration)
        XCTAssertEqual(coordinator.generationID, resumedGeneration)
        XCTAssertEqual(coordinator.identity, run)
        XCTAssertEqual(coordinator.state, .running)
        XCTAssertNil(coordinator.resume(generationID: initialGeneration))
        XCTAssertFalse(coordinator.confirmTerminal(generationID: initialGeneration, conversationID: run.conversationID, runID: "canonical-run", status: .answered))
        XCTAssertFalse(coordinator.canFinish(generationID: resumedGeneration))
        XCTAssertTrue(coordinator.requestCancellation(generationID: resumedGeneration))
        coordinator.cancellationFailed(generationID: initialGeneration)
        XCTAssertEqual(coordinator.state, .cancelRequested)
        XCTAssertTrue(coordinator.confirmTerminal(generationID: resumedGeneration, conversationID: run.conversationID, runID: "canonical-run", status: .answered))
        XCTAssertTrue(coordinator.canFinish(generationID: resumedGeneration))
        XCTAssertFalse(coordinator.canFinish(generationID: initialGeneration))

        let noRunCoordinator = MacRunCoordinator()
        let noRun = identity(runID: nil)
        let noRunGen = noRunCoordinator.begin(identity: noRun)
        XCTAssertTrue(noRunCoordinator.transportInterrupted(generationID: noRunGen))
        XCTAssertFalse(noRunCoordinator.canResume)
        XCTAssertNil(noRunCoordinator.resume(generationID: noRunGen))
        XCTAssertFalse(noRunCoordinator.canFinish(generationID: noRunGen))
        XCTAssertEqual(noRunCoordinator.identity, noRun)

        for terminal in [MacRunCoordinator.TerminalStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            let termCoord = MacRunCoordinator()
            let termRun = identity()
            let termGen = termCoord.begin(identity: termRun)
            XCTAssertTrue(termCoord.confirmTerminal(generationID: termGen, conversationID: termRun.conversationID, runID: "canonical-run", status: terminal))
            XCTAssertFalse(termCoord.confirmTerminal(generationID: termGen, conversationID: termRun.conversationID, runID: "canonical-run", status: .answered))
            XCTAssertFalse(termCoord.transportInterrupted(generationID: termGen))
            XCTAssertFalse(termCoord.requestCancellation(generationID: termGen))
            XCTAssertEqual(termCoord.state, .confirmedTerminal(terminal))
            XCTAssertNil(termCoord.resume(generationID: termGen))
        }

        let finishCoord = MacRunCoordinator()
        let finishRun = identity()
        let finishGen = finishCoord.begin(identity: finishRun)
        XCTAssertFalse(finishCoord.claimFinish(generationID: finishGen))
        finishCoord.completeFinish(generationID: finishGen)
        XCTAssertFalse(finishCoord.isFinalized(generationID: finishGen))
        XCTAssertTrue(finishCoord.confirmTerminal(generationID: finishGen, conversationID: finishRun.conversationID, runID: "canonical-run", status: .answered))
        XCTAssertTrue(finishCoord.canFinish(generationID: finishGen))
        XCTAssertFalse(finishCoord.isFinalized(generationID: finishGen))
        XCTAssertTrue(finishCoord.claimFinish(generationID: finishGen))
        XCTAssertFalse(finishCoord.claimFinish(generationID: finishGen))
        XCTAssertFalse(finishCoord.isFinalized(generationID: finishGen))
        finishCoord.completeFinish(generationID: finishGen)
        XCTAssertTrue(finishCoord.isFinalized(generationID: finishGen))
        XCTAssertFalse(finishCoord.claimFinish(generationID: finishGen))

        let staleCoord = MacRunCoordinator()
        let old = staleCoord.begin(identity: identity(runID: "old"))
        XCTAssertTrue(staleCoord.confirmTerminal(generationID: old, conversationID: "conversation", runID: "old", status: .answered))
        XCTAssertTrue(staleCoord.claimFinish(generationID: old))
        let current = staleCoord.begin(identity: identity(runID: "new"))
        staleCoord.completeFinish(generationID: old)
        XCTAssertFalse(staleCoord.isFinalized(generationID: old))
        XCTAssertFalse(staleCoord.isFinalized(generationID: current))
        XCTAssertFalse(staleCoord.claimFinish(generationID: old))
        XCTAssertTrue(staleCoord.confirmTerminal(generationID: current, conversationID: "conversation", runID: "new", status: .answered))
        XCTAssertTrue(staleCoord.claimFinish(generationID: current))
        staleCoord.completeFinish(generationID: old)
        XCTAssertFalse(staleCoord.isFinalized(generationID: current))
        staleCoord.completeFinish(generationID: current)
        XCTAssertTrue(staleCoord.isFinalized(generationID: current))

        let taskCoord = MacRunCoordinator()
        let taskGen = taskCoord.begin(identity: identity())
        let oldTask = Task<Void, Never> { try? await Task.sleep(for: .seconds(60)) }
        let newTask = Task<Void, Never> { try? await Task.sleep(for: .seconds(60)) }
        defer { oldTask.cancel(); newTask.cancel() }
        taskCoord.replaceTask(oldTask)
        XCTAssertTrue(taskCoord.transportInterrupted(generationID: taskGen))
        let taskResumed = try XCTUnwrap(taskCoord.resume(generationID: taskGen))
        XCTAssertTrue(oldTask.isCancelled)
        taskCoord.replaceTask(newTask)
        taskCoord.releaseTask(generationID: taskGen)
        XCTAssertTrue(taskCoord.transportInterrupted(generationID: taskResumed))
        _ = try XCTUnwrap(taskCoord.resume(generationID: taskResumed))
        XCTAssertTrue(newTask.isCancelled)
        oldTask.cancel(); newTask.cancel()
        await oldTask.value; await newTask.value
    }

    func testSubmissionTicketLifecycleConcurrencyAndSnapshotRetirement() async throws {
        let abandonedTicket = ticket()
        try abandonedTicket.checkPreparation()
        XCTAssertTrue(abandonedTicket.cancelPreparation())
        XCTAssertFalse(abandonedTicket.cancelPreparation())
        XCTAssertThrowsError(try abandonedTicket.claimDispatch())
        XCTAssertThrowsError(try abandonedTicket.checkPreparation())
        XCTAssertFalse(abandonedTicket.markAccepted())
        abandonedTicket.rejectBeforeTransport()
        XCTAssertEqual(abandonedTicket.phase, .abandoned)

        let claimTicket = ticket()
        XCTAssertFalse(claimTicket.markAccepted())
        try claimTicket.claimDispatch()
        XCTAssertEqual(claimTicket.phase, .dispatching)
        XCTAssertFalse(claimTicket.cancelPreparation())
        XCTAssertThrowsError(try claimTicket.claimDispatch())
        XCTAssertTrue(claimTicket.markAccepted())
        XCTAssertFalse(claimTicket.markAccepted())
        claimTicket.rejectBeforeTransport()
        XCTAssertEqual(claimTicket.phase, .accepted)

        for claimed in [false, true] {
            let rejTicket = ticket()
            if claimed { try rejTicket.claimDispatch() }
            rejTicket.rejectBeforeTransport()
            XCTAssertEqual(rejTicket.phase, .rejectedBeforeDispatch)
            XCTAssertThrowsError(try rejTicket.claimDispatch())
            XCTAssertFalse(rejTicket.markAccepted())
        }

        let oldTicket = ticket()
        let nextTicket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: "next-run", generationID: 2))
        XCTAssertTrue(oldTicket.cancelPreparation())
        try nextTicket.claimDispatch()
        XCTAssertEqual(nextTicket.phase, .dispatching)
        XCTAssertEqual(oldTicket.identity.submissionID, "run")
        XCTAssertEqual(nextTicket.identity.generationID, 2)

        let rejectedAdm = ticket()
        XCTAssertFalse(rejectedAdm.rejectBeforeAdmission())
        try rejectedAdm.claimDispatch()
        XCTAssertTrue(rejectedAdm.rejectBeforeAdmission())
        XCTAssertEqual(rejectedAdm.phase, .rejectedBeforeAdmission)
        XCTAssertFalse(rejectedAdm.markAccepted())
        XCTAssertThrowsError(try rejectedAdm.claimDispatch())
        let acceptedAdm = ticket()
        try acceptedAdm.claimDispatch()
        XCTAssertTrue(acceptedAdm.markAccepted())
        XCTAssertFalse(acceptedAdm.rejectBeforeAdmission())
        XCTAssertEqual(acceptedAdm.phase, .accepted)

        let admittedJSON = #"{"type":"submission_admitted","conversation_id":"c","run_id":"r","assistant_message_id":"a"}"#
        XCTAssertEqual(try MacHostSubmissionDecision.decode(admittedJSON, conversationID: "c", runID: "r", assistantMessageID: "a")?.type, .admitted)
        let rejectedJSON = #"{"type":"submission_rejected","conversation_id":"c","run_id":"r","assistant_message_id":"a","code":"request_rejected_before_admission","message":"Rejected"}"#
        XCTAssertEqual(try MacHostSubmissionDecision.decode(rejectedJSON, conversationID: "c", runID: "r", assistantMessageID: "a")?.type, .rejected)
        for owner in [("wrong", "r", "a"), ("c", "wrong", "a"), ("c", "r", "wrong")] {
            XCTAssertThrowsError(try MacHostSubmissionDecision.decode(rejectedJSON, conversationID: owner.0, runID: owner.1, assistantMessageID: owner.2))
        }
        let malformedJSON = #"{"type":"submission_rejected","conversation_id":"c","run_id":"r","assistant_message_id":"a"}"#
        XCTAssertThrowsError(try MacHostSubmissionDecision.decode(malformedJSON, conversationID: "c", runID: "r", assistantMessageID: "a"))
        XCTAssertNil(try MacHostSubmissionDecision.decode("Host invocation unavailable: incompatible checkpoint", conversationID: "c", runID: "r", assistantMessageID: "a"))

        for _ in 0..<100 {
            let raceTicket = ticket()
            let winners = await withTaskGroup(of: Int.self, returning: Int.self) { group in
                group.addTask {
                    do { try raceTicket.claimDispatch(); return 1 } catch { return 0 }
                }
                group.addTask { raceTicket.cancelPreparation() ? 1 : 0 }
                var count = 0
                for await winner in group { count += winner }
                return count
            }
            XCTAssertEqual(winners, 1)
            XCTAssertTrue(raceTicket.phase == .dispatching || raceTicket.phase == .abandoned)
        }

        // Completed run snapshot retirement retry, bounds, owner verification, and cancellation
        var reads = 0, releases = 0, waits = 0
        let retired = try await MacCompletedRunSnapshotRetirement.retire(
            conversationID: "conversation", runID: "run",
            assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("run:assistant")),
            readTerminal: {
                reads += 1
                if reads == 1 { throw SnapshotFailure.unavailable }
                return self.snapshotTerminal()
            }, release: { _ in releases += 1 }, wait: { waits += 1 })
        XCTAssertTrue(retired)
        XCTAssertEqual(reads, 2)
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(waits, 1)

        reads = 0; releases = 0
        do {
            _ = try await MacCompletedRunSnapshotRetirement.retire(
                conversationID: "conversation", runID: "run",
                assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("run:assistant")),
                readTerminal: { reads += 1; return self.snapshotTerminal() },
                release: { _ in releases += 1; throw SnapshotFailure.unavailable }, wait: {})
            XCTFail("Expected bounded failure")
        } catch {}
        XCTAssertEqual(reads, 3)
        XCTAssertEqual(releases, 3)

        releases = 0
        for value in [nil, snapshotTerminal(run: "other"), snapshotTerminal()] as [MacCanonicalRunTerminal?] {
            let res = try await MacCompletedRunSnapshotRetirement.retire(
                conversationID: "conversation", runID: "run",
                assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("wrong:assistant")),
                readTerminal: { value }, release: { _ in releases += 1 },
                wait: { XCTFail("Missing evidence is not a transient exception") })
            XCTAssertFalse(res)
        }
        XCTAssertEqual(releases, 0)

        reads = 0; releases = 0
        do {
            _ = try await MacCompletedRunSnapshotRetirement.retire(
                conversationID: "conversation", runID: "run",
                assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("run:assistant")),
                readTerminal: { reads += 1; throw SnapshotFailure.unavailable },
                release: { _ in releases += 1 }, wait: { throw CancellationError() })
            XCTFail("Expected cancellation")
        } catch { XCTAssertTrue(error is CancellationError) }
        XCTAssertEqual(reads, 1)
        XCTAssertEqual(releases, 0)
    }
}

@MainActor
final class MacRunRecoveryInspectionAndPresentationTests: XCTestCase {
    private func unknownPresentation(independent: UInt64 = 0) -> MacUnknownEffectRecoveryPresentation {
        let owner = MacLocalRunRecovery(conversationID: "conversation", expectedRunID: "run",
            canonicalAssistantMessageID: "assistant", assistantMessageID: UUID(), generationID: 3)
        let targets = ["first", "second"].map { id in
            MacUnknownEffectRecoveryTarget(effectID: id, requestID: "request-\(id)", attemptID: "attempt-\(id)",
                workerGeneration: "generation", leaseOwner: "owner", leaseToken: "private", expectedAttemptRevision: 2)
        }
        return .init(owner: owner, inspection: .init(conversationID: "conversation", runID: "run",
            assistantMessageID: "assistant", invocationConfigDigest: "digest", targets: targets,
            retryAllowed: true, independentPendingEffects: independent), pendingDecision: nil)
    }

    func testRecoveryPresentationTitlesDecoderValidationAndUnknownEffectPresentation() throws {
        let live = MacLocalRunRecovery(conversationID: "c", expectedRunID: "r",
            canonicalAssistantMessageID: "r:assistant", assistantMessageID: UUID(), generationID: 1)
        var restored = live
        restored.restoredFromStorage = true
        XCTAssertEqual(restored.title, String(localized: "An earlier task is unfinished. It has not been restarted.", table: "Chat"))
        XCTAssertNotEqual(restored.title, live.title)
        XCTAssertEqual(restored.expectedRunID, live.expectedRunID)

        func decode(_ observation: String) throws -> MacRunRecoveryInspection {
            try JSONDecoder().decode(MacRunRecoveryInspection.self, from: Data(
                "{\"conversation_id\":\"c\",\"run_id\":\"r\",\"assistant_message_id\":\"a\",\"observation\":\(observation)}".utf8))
        }
        XCTAssertThrowsError(try decode(#"{"state":"safe_to_retry"}"#))
        XCTAssertThrowsError(try decode(#"{"state":"terminal","status":"unknown","completed_at_ms":1}"#))
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            let object: [String: Any] = ["state": "terminal", "result": macRunResultFixture(status.rawValue, conversation: "c", run: "r"),
                "result_digest": String(repeating: "0", count: 64), "completed_at_ms": 42]
            let value = try decode(String(decoding: JSONSerialization.data(withJSONObject: object), as: UTF8.self))
            guard case .terminal(let result, _, let timestamp) = value.observation else { return XCTFail("Missing result") }
            XCTAssertEqual(result.value.kind, status)
            XCTAssertEqual(timestamp, 42)
            XCTAssertThrowsError(try value.requireActiveRemoteSource("fixture-host"))
        }

        XCTAssertEqual(MacRunRecoveryPresentation.detail(.missing),
            String(localized: "No committed Run record was found. Submission remains unconfirmed.", table: "Chat"))
        let facts = MacRunRecoveryInspection.Active(runStatus: "running", checkpointPresent: true,
            pendingEffectCount: 1, sourceHostID: nil, hasSessionLease: false,
            deliveries: [.init(state: "outcome_unknown", count: 1)])
        let local = MacRunRecoveryInspection(conversationID: "c", runID: "r",
            assistantMessageID: "a", observation: .active(facts))
        XCTAssertThrowsError(try local.requireActiveRemoteSource("fixture-host"))
        XCTAssertEqual(MacRunRecoveryPresentation.detail(.active(facts)),
            String(localized: "An execution result is unknown. No automatic retry was attempted.", table: "Chat"))

        let snapshot = unknownPresentation()
        let retry = try snapshot.decision(targetIndex: 1, reason: "I accept the risk")
        XCTAssertEqual(retry.targets, [snapshot.inspection.targets[1]])
        XCTAssertEqual(retry.choice.kind, .authorizeRiskRetry)
        XCTAssertEqual(try snapshot.decision(targetIndex: nil, reason: "Do not repeat").targets, snapshot.inspection.targets)
        XCTAssertThrowsError(try unknownPresentation(independent: 1).decision(targetIndex: nil, reason: "Do not repeat"))
        XCTAssertThrowsError(try snapshot.decision(targetIndex: 2, reason: "Out of range"))
        XCTAssertThrowsError(try snapshot.decision(targetIndex: 0, reason: " "))
        XCTAssertTrue(snapshot.matches(snapshot.owner, selectedConversationID: "conversation", generationID: 3))
        XCTAssertFalse(snapshot.matches(snapshot.owner, selectedConversationID: "another", generationID: 3))
        XCTAssertFalse(snapshot.matches(snapshot.owner, selectedConversationID: "conversation", generationID: 4))
        let pending = try snapshot.decision(targetIndex: 0, reason: "Already confirmed")
        let frozen = MacUnknownEffectRecoveryPresentation(owner: snapshot.owner, inspection: snapshot.inspection, pendingDecision: pending)
        XCTAssertThrowsError(try frozen.decision(targetIndex: 1, reason: "Must not replace pending intent"))
    }

    func testRealApplicationQueryPreservesMissingAndBoundRemoteIdentityWithoutMutation() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("store.db"))
        let missing = try port.inspectRunRecovery(conversationID: "conversation", expectedRunID: "run",
                                                  assistantMessageID: "run:assistant")
        XCTAssertEqual(missing.observation, .missing)
        XCTAssertThrowsError(try missing.requireActiveRemoteSource("fixture-host"))
        _ = try port.prepareFixtureRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Inspection",
            submissionID: "run", sourceHostID: "fixture-host", clientID: "fixture-client", userMessageID: "user", userPartID: "user:text",
            userText: "Original", userContentJSON: nil, assistantMessageID: "run:assistant",
            assistantPartID: "run:assistant:text", contextCheckpointID: nil, createdAtMs: 1))
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: "conversation",
            runID: "run", assistantMessageID: "run:assistant")
        withExtendedLifetime(source) {}
        let before = try port.loadConversationWindow("conversation").projection.revision
        let inspection = try port.inspectRunRecovery(conversationID: "conversation", expectedRunID: "run",
                                                     assistantMessageID: "run:assistant")
        guard case .active(let facts) = inspection.observation else { return XCTFail("Expected persisted active facts") }
        XCTAssertEqual(facts.sourceHostID, "fixture-host")
        XCTAssertNoThrow(try inspection.requireActiveRemoteSource("fixture-host"))
        XCTAssertThrowsError(try inspection.requireActiveRemoteSource("different-host"))
        XCTAssertThrowsError(try inspection.requireActiveRemoteSource(""))
        XCTAssertEqual(facts.pendingEffectCount, 0)
        XCTAssertTrue(facts.deliveries.isEmpty)
        XCTAssertEqual(try port.inspectRunRecovery(conversationID: "conversation", expectedRunID: "run",
            assistantMessageID: "other-assistant").observation, .identityMismatch)
        XCTAssertEqual(try port.inspectRunRecovery(conversationID: "other-conversation", expectedRunID: "run",
            assistantMessageID: "run:assistant").observation, .identityMismatch)
        XCTAssertEqual(try port.loadConversationWindow("conversation").projection.revision, before)
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
    }
}

@MainActor
final class MacRunSubmissionPreparationTests: XCTestCase {
    func testKnowledgeDebounceBlocksSendWithoutConsumingMessageDraft() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        model.draft = "Keep this message"
        model.selectedKnowledgePointsText = "New knowledge before debounce"
        XCTAssertTrue(model.selectedKnowledgeIsSaving)
        XCTAssertFalse(model.selectedComposerState.canSend)
        XCTAssertNotNil(model.selectedComposerState.submissionBlockReason)
        model.send()
        XCTAssertEqual(model.draft, "Keep this message")
        XCTAssertNil(gate.captured)
        XCTAssertFalse(model.isRunning)
        try await waitUntil { !model.selectedKnowledgeIsSaving }
        XCTAssertTrue(model.selectedComposerState.canSend)
        XCTAssertEqual(model.draft, "Keep this message")
    }

    func testArchiveAcknowledgementCannotSurviveForgetAndRestore() {
        let scheduler = MacConversationArchiveScheduler()
        let oldSave = scheduler.invalidate(conversationID: "restored")
        scheduler.forget(conversationID: "restored")
        let restoredSave = scheduler.invalidate(conversationID: "restored")
        XCTAssertNotEqual(oldSave, restoredSave)
        XCTAssertFalse(scheduler.isCurrent(conversationID: "restored", revision: oldSave))
        XCTAssertTrue(scheduler.isCurrent(conversationID: "restored", revision: restoredSave))
        let otherSave = scheduler.invalidate(conversationID: "another")
        XCTAssertTrue(scheduler.isCurrent(conversationID: "restored", revision: restoredSave))
        XCTAssertFalse(scheduler.isCurrent(conversationID: "restored", revision: otherSave))
    }

    func testNewArchiveEditInvalidatesInFlightAcknowledgementBeforeDebounce() async throws {
        let scheduler = MacConversationArchiveScheduler()
        let oldSave = scheduler.invalidate(conversationID: "editing")
        let saved = expectation(description: "Only the newest debounced edit runs")
        scheduler.schedule(conversationID: "editing", delay: .milliseconds(10)) { _ in
            XCTFail("Superseded edit must not save")
        }
        scheduler.schedule(conversationID: "editing", delay: .milliseconds(10)) { revision in
            XCTAssertTrue(scheduler.isCurrent(conversationID: "editing", revision: revision))
            XCTAssertFalse(scheduler.isCurrent(conversationID: "editing", revision: oldSave))
            saved.fulfill()
        }
        XCTAssertFalse(scheduler.isCurrent(conversationID: "editing", revision: oldSave))
        await fulfillment(of: [saved], timeout: 3)
    }

    @MainActor
    private final class PreparationGate {
        var captured: MacRunSubmissionSnapshot?
        private var continuation: CheckedContinuation<Void, Never>?
        private(set) var returned = false

        func prepare(_ snapshot: MacRunSubmissionSnapshot) async throws -> [MacCoreContextComponent] {
            captured = snapshot
            // Deliberately finish even if the caller cancels while awaiting.
            // A late service result must not resurrect an abandoned submission.
            await withCheckedContinuation { continuation = $0 }
            returned = true
            return snapshot.contextComponents
        }

        func release() {
            continuation?.resume()
            continuation = nil
        }
    }

    private func waitUntil(_ predicate: () -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while !predicate() && Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(predicate(), "Expected submission boundary", file: file, line: line)
    }

    private func configureRemote(_ model: MacWorkspaceModel, host: PermissionEnvelopeRemoteHost) async throws {
        model.executionHost = .remote
        if let id = model.selectedConversationID { try await model.waitForModelConfigurationCommit(conversationID: id) }
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        if let id = model.selectedConversationID { try await model.waitForModelConfigurationCommit(conversationID: id) }
    }

    func testComposerExecutionAndStopStayWithTheirConversation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        let first = try XCTUnwrap(model.selectedConversationID)
        model.draft = "First conversation"
        model.send()
        try await waitUntil { gate.captured != nil }
        XCTAssertTrue(model.selectedComposerState.isVisiblyRunning)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .running)
        model.newConversation()
        let second = try XCTUnwrap(model.selectedConversationID)
        XCTAssertNotEqual(first, second)
        let secondStyle = model.style
        let secondPermission = model.permissionMode
        XCTAssertFalse(model.selectComposerReasoningPreference("automatic", conversationID: first),
            "A menu opened in another conversation cannot mutate the selected conversation")
        XCTAssertFalse(model.selectComposerPermissionMode(.cautious, conversationID: first))
        XCTAssertEqual(model.style, secondStyle)
        XCTAssertEqual(model.permissionMode, secondPermission)
        XCTAssertFalse(model.selectComposerReasoningPreference("automatic", conversationID: second),
            "Connected Host reasoning is owned by its model profile")
        try await model.waitForModelConfigurationCommit(conversationID: second)
        XCTAssertTrue(model.selectComposerPermissionMode(.allowAll, conversationID: second))
        model.draft = "Second conversation draft"
        XCTAssertFalse(model.selectedComposerState.canSend, "The owner's in-flight configuration receipt fences submission")
        try await model.waitForModelConfigurationCommit(conversationID: second)
        let idleComposer = model.selectedComposerState
        XCTAssertFalse(idleComposer.isRunning)
        XCTAssertFalse(idleComposer.isVisiblyRunning)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .ready,
            "Another conversation's active execution must not become this draft's status")
        XCTAssertFalse(model.isSelectedConversationLoading, "A new draft has no canonical history to load")
        XCTAssertTrue(idleComposer.canSend)
        XCTAssertNil(idleComposer.submissionBlockReason)
        XCTAssertFalse(model.stopComposerRun(conversationID: second))
        XCTAssertFalse(model.stopComposerRun(conversationID: first), "Stale action from a deselected composer is rejected")
        model.send()
        XCTAssertEqual(model.selectedConversationDisplayStatus, .waitingResource)
        XCTAssertEqual(model.draft, "Second conversation draft")
        XCTAssertEqual(host.runRequests, 0)
        model.selectedConversationID = first
        XCTAssertTrue(model.selectedComposerState.isVisiblyRunning)
        XCTAssertTrue(model.stopComposerRun(conversationID: first))
        try await waitUntil { !model.isRunning }
        model.selectedConversationID = second
        XCTAssertEqual(model.draft, "Second conversation draft")
        XCTAssertTrue(model.selectedComposerState.canSend)
        XCTAssertNil(model.selectedComposerState.submissionBlockReason)
    }

    func testStopDuringPreparationRetainsInputAndNeverDispatchesOrCancelsHostRun() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        let input = "  Keep my unsent request intact.  "
        let attachment = MacAttachment(url: root.appendingPathComponent("notes.txt"), status: .ready,
            extractedText: "Immutable attachment body", extractionMethod: "fixture")
        model.draft = input
        model.attachments = [attachment]
        let messages = model.selectedMessages.map(\.id)
        let terminalRevision = model.selectedTerminalTranscriptRevision
        model.send()
        try await waitUntil { gate.captured != nil }
        XCTAssertTrue(model.isRunning)
        XCTAssertEqual(host.runRequests, 0)
        model.stop()
        try await waitUntil { !model.isRunning }
        XCTAssertEqual(model.draft, input)
        XCTAssertEqual(model.attachments, [attachment])
        XCTAssertEqual(model.selectedMessages.map(\.id), messages)
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertNil(try port.activeRunIdentity(conversationID: conversation))
        XCTAssertNil(model.remoteRunRecovery)
        gate.release()
        try await waitUntil { gate.returned }
        await Task.yield()
        XCTAssertEqual(host.runRequests, 0, "Late preparation must not submit an abandoned Run")
        XCTAssertTrue(host.cancellationRequests.isEmpty, "There is no Host Run to cancel before dispatch")
        XCTAssertEqual(model.draft, input)
        XCTAssertEqual(model.attachments, [attachment])
        XCTAssertFalse(model.isRunning)
    }

    func testSubmissionFreezesTemplateWorkspaceAndExternalSessionBeforePreparationYields() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        model.selectExecutionBackend(.fallback(id: "codex"), available: true)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        let index = try XCTUnwrap(model.conversations.firstIndex(where: { $0.id == conversation }))
        let template = try XCTUnwrap(model.promptTemplateService.templates.first(where: {
            model.promptTemplateService.readableTemplate(id: $0.id) != nil
        }))
        model.selectPromptTemplate(template.id)
        let selectedWorkspace = root.appendingPathComponent("original-workspace")
        model.workspaceURL = selectedWorkspace
        model.conversations[index].externalExecutionSessionIDs = ["codex": "original-session"]
        model.draft = "Original frozen request"
        model.memoryEnabledOverride = false
        model.send()
        try await waitUntil { gate.captured != nil }
        let snapshot = try XCTUnwrap(gate.captured)
        XCTAssertEqual(snapshot.workspaceSelection.selectedURL, selectedWorkspace)
        XCTAssertEqual(snapshot.externalSessionID, "original-session")
        XCTAssertTrue(snapshot.contextComponents.contains { $0.kind == "prompt_template" && $0.id == "prompt_template:\(template.id)" })
        model.selectPromptTemplate(nil)
        model.memoryEnabledOverride = true
        model.workspaceURL = root.appendingPathComponent("changed-workspace")
        model.conversations[index].externalExecutionSessionIDs = ["codex": "changed-session"]
        model.draft = "Next request typed while preparing"
        gate.release()
        try await waitUntil { host.runContinuation != nil }
        XCTAssertEqual(host.lastRunPrompt, "Original frozen request")
        XCTAssertEqual(host.lastRunMemoryEnabled, false, "Preparation must not reread the new UI memory preference")
        XCTAssertEqual(host.lastRunExternalSessionID, "original-session")
        XCTAssertEqual(host.lastRunContextComponents, snapshot.contextComponents)
        XCTAssertEqual(snapshot.workspaceSelection.selectedURL, selectedWorkspace)
        // Once dispatch has happened, stop is a real cancellation request; it
        // cannot pretend the submission never existed and restore the old draft.
        let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        model.stop()
        try await waitUntil { host.cancellationRequests.count == 1 }
        XCTAssertEqual(model.draft, "Next request typed while preparing")
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, runID)
        host.runContinuation?.finish()
        try await waitUntil { !model.isRunning }
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
            runID: runID, assistantMessageID: "\(runID):assistant")
        _ = try source.finish("The dispatched request completed before cancellation took effect.")
        try source.commitTerminal(into: port)
        try await waitUntil { model.remoteRunRecovery == nil && model.selectedTerminalTranscriptRevision > 0 }
    }
}

@MainActor
final class MacUnknownEffectRecoveryControlTests: XCTestCase {
    private func fixture(_ run: String = "run") -> MacUnknownEffectRecoveryDecision {
        MacUnknownEffectRecoveryDecision(conversationID: "conversation-\(run)", runID: run,
            invocationConfigDigest: "digest", resolutionID: "resolution-\(run)",
            targets: [MacUnknownEffectRecoveryTarget(effectID: "effect", requestID: "request", attemptID: "attempt",
                workerGeneration: "generation", leaseOwner: "owner", leaseToken: "fence", expectedAttemptRevision: 4)],
            choice: .init(kind: .authorizeRiskRetry, reason: "Explicitly accept duplicate-effect risk"), resolvedAtMs: 1234)
    }
    private func database() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try FileManager.default.removeItem(at: directory) }
        return directory.appendingPathComponent("fixture.db")
    }
    private func reply(_ decision: MacUnknownEffectRecoveryDecision) -> [String: Any] {
        ["outcome": "retry_authorized", "resolution_id": decision.resolutionID, "run_id": decision.runID]
    }

    func testOwnedTaskKeepsUnknownAndRecoversExactReceiptAfterReopen() throws {
        let url = try database()
        let owner = AppleOwnedTaskRecoveryOwner(parent_conversation_id: "parent", parent_run_id: "parent-run", task_id: "task")
        let risky = fixture()
        let decision = MacUnknownEffectRecoveryDecision(conversationID: risky.conversationID, runID: risky.runID,
            invocationConfigDigest: risky.invocationConfigDigest, resolutionID: risky.resolutionID,
            targets: risky.targets, choice: .init(kind: .keepUnknownBlocked, reason: "Keep the physical outcome unknown"),
            resolvedAtMs: risky.resolvedAtMs)
        var submitted = 0
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, id in
            submitted += 1
            let command = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(command["type"] as? String, "resolve_owned_task_recovery")
            let sentOwner = try XCTUnwrap(command["owner"] as? [String: String])
            XCTAssertEqual(sentOwner["parent_run_id"], owner.parent_run_id)
            XCTAssertEqual(id, "fixed-owned")
            throw MacApplicationServiceError.connectionFailed("lost reply")
        }
        XCTAssertThrowsError(try first.submitConfirmed(risky, assistantMessageID: "assistant", taskOwner: owner))
        XCTAssertEqual(submitted, 0, "Parent recovery must not authorize a retry")
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "fixed-owned", taskOwner: owner))
        XCTAssertEqual(try first.pendingOwnedDecision(owner), decision)
        XCTAssertNil(try first.pendingOwnedDecision(.init(parent_conversation_id: "parent", parent_run_id: "another-run", task_id: "task")))
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["type"] as? String, "command_receipt")
            XCTAssertEqual(query["request_id"] as? String, "fixed-owned")
            return try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "fixed-owned", "status": "committed",
                "result": ["outcome": "unknown_preserved_blocked", "resolution_id": decision.resolutionID, "run_id": decision.runID]])
        }
        XCTAssertEqual(try reopened.recoverPending(runID: decision.runID, taskOwner: owner)?.outcome, .unknownPreservedBlocked)
        XCTAssertNil(try reopened.pendingOwnedDecision(owner))
    }

    func testLostReplyReopensAndQueriesOriginalReceiptWithoutResending() throws {
        let url = try database(), decision = fixture()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, id in
            XCTAssertEqual(id, "fixed-request")
            throw MacApplicationServiceError.connectionFailed("lost reply")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "fixed-request"))
        XCTAssertEqual(try first.pendingDecision(runID: "run"), decision)
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["type"] as? String, "command_receipt")
            XCTAssertEqual(query["request_id"] as? String, "fixed-request")
            return try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "fixed-request", "status": "committed", "result": self.reply(decision)])
        }
        XCTAssertEqual(try reopened.recoverPending(runID: "run")?.outcome, .retryAuthorized)
        XCTAssertEqual(calls, 1)
        XCTAssertNil(try reopened.pendingDecision(runID: "run"))
    }

    func testLocalRejectedErrorIsNotAuthorityToRetireAndOtherRunsRemainIndependent() throws {
        let url = try database()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.rejected("local protocol mismatch, no receipt")
        }
        XCTAssertThrowsError(try first.submitConfirmed(fixture(), assistantMessageID: "assistant", requestID: "original"))
        XCTAssertThrowsError(try first.submitConfirmed(fixture("other"), assistantMessageID: "other-assistant", requestID: "other-request"))
        XCTAssertEqual(try first.pendingDecision(runID: "other"), fixture("other"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["request_id"] as? String, "original")
            return try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "original", "status": "rejected", "error": ["message": "canonical rejection"]])
        }
        XCTAssertThrowsError(try reopened.submitConfirmed(fixture(), assistantMessageID: "assistant", requestID: "replacement"))
        XCTAssertEqual(calls, 0)
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
        XCTAssertEqual(calls, 1)
        XCTAssertNil(try reopened.pendingDecision(runID: "run"))
        XCTAssertNotNil(try reopened.pendingDecision(runID: "other"))
    }

    func testMissingReceiptReplaysIdenticalPayloadAndRequestNotNewAuthorization() throws {
        let url = try database(), decision = fixture()
        var original: Data?
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            original = data
            throw MacApplicationServiceError.outcomePending(requestID: "original")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, id in
            calls += 1
            if calls == 1 { return Data("null".utf8) }
            XCTAssertEqual(id, "original")
            XCTAssertEqual(data, original)
            return try JSONSerialization.data(withJSONObject: self.reply(decision))
        }
        XCTAssertEqual(try reopened.recoverPending(runID: "run")?.outcome, .retryAuthorized)
        XCTAssertEqual(calls, 2)
    }

    func testTerminalReconciliationOnlyQueriesReceiptAndPreservesMissingIntent() throws {
        let url = try database(), decision = fixture()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("reply lost")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let command = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(command["type"] as? String, "command_receipt")
            XCTAssertEqual(command["request_id"] as? String, "original")
            return Data("null".utf8)
        }
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run", replayIfMissing: false))
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(try reopened.pendingDecision(runID: "run"), decision)
    }

    func testTerminalBlockedReceiptRetiresOriginalDecisionWithoutReplaying() throws {
        let url = try database(), base = fixture()
        let decision = MacUnknownEffectRecoveryDecision(conversationID: base.conversationID,
            runID: base.runID, invocationConfigDigest: base.invocationConfigDigest,
            resolutionID: base.resolutionID, targets: base.targets,
            choice: .init(kind: .keepUnknownBlocked, reason: "Preserve the unknown result"), resolvedAtMs: base.resolvedAtMs)
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("reply lost after commit")
        }
        XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
        var calls = 0
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { data, _ in
            calls += 1
            let query = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(query["type"] as? String, "command_receipt")
            return try JSONSerialization.data(withJSONObject: [
                "protocol_version": MacApplicationServiceClient.protocolVersion,
                "client_id": "client", "request_id": "original", "status": "committed",
                "result": ["outcome": "unknown_preserved_blocked", "resolution_id": decision.resolutionID,
                           "run_id": decision.runID],
            ])
        }
        XCTAssertEqual(try reopened.recoverPending(runID: "run", replayIfMissing: false)?.outcome, .unknownPreservedBlocked)
        XCTAssertEqual(calls, 1)
        XCTAssertNil(try reopened.pendingDecision(runID: "run"))
    }

    func testWrongClientAndPersistenceFailureNeverTransmit() throws {
        let url = try database()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "owner") { _, _ in
            throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try first.submitConfirmed(fixture(), assistantMessageID: "assistant"))
        let wrong = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "foreign") { _, _ in
            XCTFail("wrong client must not transmit"); return Data()
        }
        XCTAssertThrowsError(try wrong.recoverPending(runID: "run"))
        XCTAssertNotNil(try first.pendingDecision(runID: "run"))
        let brokenURL = url.deletingLastPathComponent().appendingPathComponent("broken.db")
        try Data("not a directory".utf8).write(to: brokenURL.appendingPathExtension("pending-recovery-decisions"))
        let broken = MacUnknownEffectRecoveryControl(databaseURL: brokenURL, clientID: "client") { _, _ in
            XCTFail("persistence must precede transmission"); return Data()
        }
        XCTAssertThrowsError(try broken.submitConfirmed(fixture(), assistantMessageID: "assistant"))
    }

    func testProtocolMaximumTargetsFitTheBoundedJournal() throws {
        let url = try database(), base = fixture()
        let identity = String(repeating: "\\", count: 512)
        let target = MacUnknownEffectRecoveryTarget(effectID: identity, requestID: identity, attemptID: identity,
            workerGeneration: identity, leaseOwner: identity, leaseToken: identity, expectedAttemptRevision: 4)
        let decision = MacUnknownEffectRecoveryDecision(conversationID: base.conversationID, runID: base.runID,
            invocationConfigDigest: base.invocationConfigDigest, resolutionID: base.resolutionID,
            targets: Array(repeating: target, count: 256), choice: .init(kind: .keepUnknownBlocked, reason: String(repeating: "r", count: 4096)), resolvedAtMs: 1234)
        var transmitted = false
        let service = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            transmitted = true; throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try service.submitConfirmed(decision, assistantMessageID: "assistant"))
        XCTAssertTrue(transmitted)
        XCTAssertEqual(try service.pendingDecision(runID: "run"), decision)
    }

    func testAcceptedOrWrongOwnerReceiptKeepsTheFrozenIntent() throws {
        for wrongOwner in [false, true] {
            let url = try database(), decision = fixture()
            let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
                throw MacApplicationServiceError.connectionFailed("offline")
            }
            XCTAssertThrowsError(try first.submitConfirmed(decision, assistantMessageID: "assistant", requestID: "original"))
            let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
                try JSONSerialization.data(withJSONObject: ["protocol_version": MacApplicationServiceClient.protocolVersion,
                    "client_id": wrongOwner ? "foreign" : "client", "request_id": "original", "status": "accepted"])
            }
            XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
            XCTAssertEqual(try reopened.pendingDecision(runID: "run"), decision)
        }
    }

    func testCorruptIntentFailsClosedWithoutDeletingOrSending() throws {
        let url = try database()
        let service = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try service.submitConfirmed(fixture(), assistantMessageID: "assistant"))
        let files = try FileManager.default.contentsOfDirectory(at: url.appendingPathExtension("pending-recovery-decisions"), includingPropertiesForKeys: nil)
        let journal = try XCTUnwrap(files.first { $0.pathExtension == "json" })
        let corrupt = Data("{broken".utf8)
        try corrupt.write(to: journal)
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            XCTFail("corrupt intent cannot authorize a transmission"); return Data()
        }
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
        XCTAssertEqual(try Data(contentsOf: journal), corrupt)
    }

    func testOversizedIntentBlocksRecoveryAndReplacementWithoutTransmission() throws {
        let url = try database()
        let first = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            throw MacApplicationServiceError.connectionFailed("offline")
        }
        XCTAssertThrowsError(try first.submitConfirmed(fixture(), assistantMessageID: "assistant"))
        let files = try FileManager.default.contentsOfDirectory(
            at: url.appendingPathExtension("pending-recovery-decisions"), includingPropertiesForKeys: nil)
        let journal = try XCTUnwrap(files.first { $0.pathExtension == "json" })
        let size = 12 * 1024 * 1024 + 1
        try Data(repeating: 0x20, count: size).write(to: journal)
        let reopened = MacUnknownEffectRecoveryControl(databaseURL: url, clientID: "client") { _, _ in
            XCTFail("oversized intent must not permit any transmission"); return Data()
        }
        XCTAssertThrowsError(try reopened.recoverPending(runID: "run"))
        XCTAssertThrowsError(try reopened.submitConfirmed(fixture(), assistantMessageID: "assistant", requestID: "replacement"))
        let attributes = try FileManager.default.attributesOfItem(atPath: journal.path)
        XCTAssertEqual((attributes[.size] as? NSNumber)?.intValue, size)
    }
}
