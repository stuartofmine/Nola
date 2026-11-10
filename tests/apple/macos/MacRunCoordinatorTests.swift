// SPDX-License-Identifier: Apache-2.0
//! Run identity and terminal evidence must outlive a transport attempt.

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacRunCoordinatorTests: XCTestCase {
    func testQuestionSuspensionIsExactRunNonterminalAndResumable() {
        let coordinator = MacRunCoordinator()
        let generation = coordinator.begin(identity: identity())
        XCTAssertFalse(coordinator.waitSuspended(generationID: generation, conversationID: "other", runID: "canonical-run"))
        XCTAssertFalse(coordinator.waitSuspended(generationID: generation, conversationID: "conversation", runID: "other"))
        XCTAssertFalse(coordinator.waitSuspended(generationID: generation &+ 1, conversationID: "conversation", runID: "canonical-run"))
        XCTAssertEqual(coordinator.state, .running)
        XCTAssertTrue(coordinator.waitSuspended(generationID: generation, conversationID: "conversation", runID: "canonical-run"))
        XCTAssertEqual(coordinator.state, .awaitingInput)
        XCTAssertTrue(coordinator.canResume)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
        XCTAssertTrue(coordinator.requestCancellation(generationID: generation))
        XCTAssertFalse(coordinator.waitSuspended(generationID: generation, conversationID: "conversation", runID: "canonical-run"))
        XCTAssertEqual(coordinator.state, .cancelRequested)
        coordinator.cancellationFailed(generationID: generation)
        XCTAssertEqual(coordinator.state, .awaitingInput)
        XCTAssertTrue(coordinator.confirmTerminal(generationID: generation, conversationID: "conversation", runID: "canonical-run", status: .cancelled))
        XCTAssertFalse(coordinator.waitSuspended(generationID: generation, conversationID: "conversation", runID: "canonical-run"))
    }

    func testQuestionExitRaceRequiresDurableActiveRunAndObservedQuestion() {
        XCTAssertNil(MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: "run", openWaits: [], observedWaitIDs: []))
        XCTAssertNil(MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: nil, openWaits: [], observedWaitIDs: ["wait"]))
        XCTAssertNil(MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: "successor", openWaits: [], observedWaitIDs: ["wait"]))
        let suspension = MacLocalAgentWaitSuspension.validated(conversationID: "conversation", runID: "run", activeRunID: "run", openWaits: [], observedWaitIDs: ["wait"])
        XCTAssertEqual(suspension?.runID, "run")
        XCTAssertEqual(suspension?.readyToResume, true)
        XCTAssertEqual(suspension?.waits.count, 0)
    }

    private func identity(
        conversationID: String = "conversation",
        runID: String? = "canonical-run"
    ) -> MacRunCoordinator.Identity {
        .init(conversationID: conversationID, runID: runID, assistantMessageID: UUID())
    }

    func testTransportEndingWithoutTerminalEvidenceAwaitsRecoveryNotCompletion() {
        let coordinator = MacRunCoordinator()
        let run = identity()
        let generation = coordinator.begin(identity: run)
        XCTAssertEqual(coordinator.state, .running)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))

        XCTAssertTrue(coordinator.transportInterrupted(generationID: generation))
        XCTAssertEqual(coordinator.state, .awaitingRecovery)
        XCTAssertEqual(coordinator.identity, run)
        XCTAssertTrue(coordinator.canResume)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
    }

    func testLateTerminalFromPreviousGenerationCannotCompleteNewRun() {
        let coordinator = MacRunCoordinator()
        let previous = identity(runID: "previous-run")
        let oldGeneration = coordinator.begin(identity: previous)
        let current = identity(conversationID: "other-conversation", runID: "new-run")
        let generation = coordinator.begin(identity: current)
        XCTAssertNotEqual(generation, oldGeneration)

        XCTAssertFalse(coordinator.confirmTerminal(generationID: oldGeneration,
            conversationID: previous.conversationID, runID: "previous-run", status: .answered))
        XCTAssertFalse(coordinator.transportInterrupted(generationID: oldGeneration))
        XCTAssertFalse(coordinator.requestCancellation(generationID: oldGeneration))
        XCTAssertEqual(coordinator.identity, current)
        XCTAssertEqual(coordinator.state, .running)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
    }

    func testTerminalRequiresMatchingConversationAndCanonicalRunIdentity() {
        let coordinator = MacRunCoordinator()
        let run = identity()
        let generation = coordinator.begin(identity: run)
        XCTAssertFalse(coordinator.confirmTerminal(generationID: generation,
            conversationID: "wrong-conversation", runID: "canonical-run", status: .answered))
        XCTAssertFalse(coordinator.confirmTerminal(generationID: generation,
            conversationID: run.conversationID, runID: "wrong-run", status: .failed))
        XCTAssertEqual(coordinator.state, .running)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
    }

    func testCancellationRequestIsNotConfirmedCancellation() {
        let coordinator = MacRunCoordinator()
        let run = identity()
        let generation = coordinator.begin(identity: run)
        XCTAssertTrue(coordinator.requestCancellation(generationID: generation))
        XCTAssertEqual(coordinator.state, .cancelRequested)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
        XCTAssertFalse(coordinator.requestCancellation(generationID: generation))

        XCTAssertTrue(coordinator.confirmTerminal(generationID: generation,
            conversationID: run.conversationID, runID: "canonical-run", status: .cancelled))
        XCTAssertEqual(coordinator.state, .confirmedTerminal(.cancelled))
        XCTAssertTrue(coordinator.canFinish(generationID: generation))
        XCTAssertFalse(coordinator.canResume)
    }

    func testFailedCancellationDoesNotInventTerminalOutcome() {
        let coordinator = MacRunCoordinator()
        let generation = coordinator.begin(identity: identity())
        XCTAssertTrue(coordinator.requestCancellation(generationID: generation))
        coordinator.cancellationFailed(generationID: generation)
        XCTAssertEqual(coordinator.state, .running)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
        XCTAssertTrue(coordinator.requestCancellation(generationID: generation))
    }

    func testCancellationDeliveryRetryPreservesIntentAndCannotOutliveTerminal() {
        let coordinator = MacRunCoordinator()
        let run = identity()
        let generation = coordinator.begin(identity: run)
        XCTAssertTrue(coordinator.requestCancellation(generationID: generation))
        XCTAssertTrue(coordinator.claimCancellationDelivery(generationID: generation))
        XCTAssertFalse(coordinator.claimCancellationDelivery(generationID: generation))
        coordinator.cancellationDeliveryInterrupted(generationID: generation &+ 1)
        XCTAssertFalse(coordinator.claimCancellationDelivery(generationID: generation))
        coordinator.cancellationDeliveryInterrupted(generationID: generation)
        XCTAssertEqual(coordinator.state, .cancelRequested)
        XCTAssertEqual(coordinator.cancellationRequestedGenerationID, generation)
        XCTAssertFalse(coordinator.canResume)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
        XCTAssertTrue(coordinator.claimCancellationDelivery(generationID: generation))
        XCTAssertTrue(coordinator.confirmTerminal(generationID: generation,
            conversationID: run.conversationID, runID: "canonical-run", status: .answered))
        coordinator.cancellationDeliveryInterrupted(generationID: generation)
        XCTAssertFalse(coordinator.claimCancellationDelivery(generationID: generation))
        XCTAssertEqual(coordinator.state, .confirmedTerminal(.answered))
    }

    func testTransportLossDuringCancellationStillRequiresRecoveryIfCancellationFails() {
        let coordinator = MacRunCoordinator()
        let generation = coordinator.begin(identity: identity())
        XCTAssertTrue(coordinator.requestCancellation(generationID: generation))
        XCTAssertTrue(coordinator.transportInterrupted(generationID: generation))
        XCTAssertEqual(coordinator.state, .cancelRequested)
        coordinator.cancellationFailed(generationID: generation)
        XCTAssertEqual(coordinator.state, .awaitingRecovery,
            "failed cancellation must not restore a running observer after its transport was lost")
        XCTAssertTrue(coordinator.canResume)
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
    }

    func testAuthoritativeCompletionCanWinRaceWithCancellationRequest() {
        let coordinator = MacRunCoordinator()
        let run = identity()
        let generation = coordinator.begin(identity: run)
        XCTAssertTrue(coordinator.requestCancellation(generationID: generation))
        XCTAssertTrue(coordinator.confirmTerminal(generationID: generation,
            conversationID: run.conversationID, runID: "canonical-run", status: .answered))
        coordinator.cancellationFailed(generationID: generation)
        XCTAssertEqual(coordinator.state, .confirmedTerminal(.answered))
        XCTAssertTrue(coordinator.canFinish(generationID: generation))
    }

    func testResumePreservesRunAndMessageIdentityButInvalidatesPreviousAttempt() throws {
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

        XCTAssertFalse(coordinator.confirmTerminal(generationID: initialGeneration,
            conversationID: run.conversationID, runID: "canonical-run", status: .answered))
        XCTAssertFalse(coordinator.canFinish(generationID: resumedGeneration))
        XCTAssertTrue(coordinator.requestCancellation(generationID: resumedGeneration))
        coordinator.cancellationFailed(generationID: initialGeneration)
        XCTAssertEqual(coordinator.state, .cancelRequested)
        XCTAssertTrue(coordinator.confirmTerminal(generationID: resumedGeneration,
            conversationID: run.conversationID, runID: "canonical-run", status: .answered))
        XCTAssertTrue(coordinator.canFinish(generationID: resumedGeneration))
        XCTAssertFalse(coordinator.canFinish(generationID: initialGeneration))
    }

    func testUnknownCanonicalRunCannotResumeUsingOnlyLocalIdentity() {
        let coordinator = MacRunCoordinator()
        let run = identity(runID: nil)
        let generation = coordinator.begin(identity: run)
        XCTAssertTrue(coordinator.transportInterrupted(generationID: generation))
        XCTAssertFalse(coordinator.canResume)
        XCTAssertNil(coordinator.resume(generationID: generation))
        XCTAssertFalse(coordinator.canFinish(generationID: generation))
        XCTAssertEqual(coordinator.identity, run)
    }

    func testTerminalEvidenceIsIdempotentAndCannotBeReplacedByLateTransportFailure() {
        for terminal in [MacRunCoordinator.TerminalStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            let coordinator = MacRunCoordinator()
            let run = identity()
            let generation = coordinator.begin(identity: run)
            XCTAssertTrue(coordinator.confirmTerminal(generationID: generation,
                conversationID: run.conversationID, runID: "canonical-run", status: terminal))
            XCTAssertFalse(coordinator.confirmTerminal(generationID: generation,
                conversationID: run.conversationID, runID: "canonical-run", status: .answered))
            XCTAssertFalse(coordinator.transportInterrupted(generationID: generation))
            XCTAssertFalse(coordinator.requestCancellation(generationID: generation))
            XCTAssertEqual(coordinator.state, .confirmedTerminal(terminal))
            XCTAssertNil(coordinator.resume(generationID: generation))
        }
    }

    func testTerminalEvidenceDoesNotFinalizeUntilTheSingleFinishOwnerCompletes() {
        let coordinator = MacRunCoordinator()
        let run = identity()
        let generation = coordinator.begin(identity: run)
        XCTAssertFalse(coordinator.claimFinish(generationID: generation))
        coordinator.completeFinish(generationID: generation)
        XCTAssertFalse(coordinator.isFinalized(generationID: generation))
        XCTAssertTrue(coordinator.confirmTerminal(generationID: generation,
            conversationID: run.conversationID, runID: "canonical-run", status: .answered))
        XCTAssertTrue(coordinator.canFinish(generationID: generation))
        XCTAssertFalse(coordinator.isFinalized(generationID: generation))
        XCTAssertTrue(coordinator.claimFinish(generationID: generation))
        XCTAssertFalse(coordinator.claimFinish(generationID: generation), "The losing observer must not own cleanup")
        XCTAssertFalse(coordinator.isFinalized(generationID: generation), "A claim is not completed presentation")
        coordinator.completeFinish(generationID: generation)
        XCTAssertTrue(coordinator.isFinalized(generationID: generation))
        XCTAssertFalse(coordinator.claimFinish(generationID: generation))
        coordinator.completeFinish(generationID: generation)
        XCTAssertTrue(coordinator.isFinalized(generationID: generation))
    }

    func testStaleFinishCannotFinalizeOrClaimTheNextGeneration() {
        let coordinator = MacRunCoordinator()
        let old = coordinator.begin(identity: identity(runID: "old"))
        XCTAssertTrue(coordinator.confirmTerminal(generationID: old,
            conversationID: "conversation", runID: "old", status: .answered))
        XCTAssertTrue(coordinator.claimFinish(generationID: old))
        let current = coordinator.begin(identity: identity(runID: "new"))
        coordinator.completeFinish(generationID: old)
        XCTAssertFalse(coordinator.isFinalized(generationID: old))
        XCTAssertFalse(coordinator.isFinalized(generationID: current))
        XCTAssertFalse(coordinator.claimFinish(generationID: old))
        XCTAssertTrue(coordinator.confirmTerminal(generationID: current,
            conversationID: "conversation", runID: "new", status: .answered))
        XCTAssertTrue(coordinator.claimFinish(generationID: current))
        coordinator.completeFinish(generationID: old)
        XCTAssertFalse(coordinator.isFinalized(generationID: current))
        coordinator.completeFinish(generationID: current)
        XCTAssertTrue(coordinator.isFinalized(generationID: current))
    }

    func testResumeCancelsOldObservationAndStaleReleaseCannotDetachNewTask() async throws {
        let coordinator = MacRunCoordinator()
        let generation = coordinator.begin(identity: identity())
        let oldTask = Task<Void, Never> { try? await Task.sleep(for: .seconds(60)) }
        let newTask = Task<Void, Never> { try? await Task.sleep(for: .seconds(60)) }
        defer {
            oldTask.cancel()
            newTask.cancel()
        }
        coordinator.replaceTask(oldTask)
        XCTAssertTrue(coordinator.transportInterrupted(generationID: generation))
        let resumed = try XCTUnwrap(coordinator.resume(generationID: generation))
        XCTAssertTrue(oldTask.isCancelled)

        coordinator.replaceTask(newTask)
        coordinator.releaseTask(generationID: generation)
        XCTAssertTrue(coordinator.transportInterrupted(generationID: resumed))
        _ = try XCTUnwrap(coordinator.resume(generationID: resumed))
        XCTAssertTrue(newTask.isCancelled, "old completion must not detach the new observation task")

        // Explicit cleanup also bounds this test if the ownership assertion fails.
        oldTask.cancel()
        newTask.cancel()
        await oldTask.value
        await newTask.value
    }
}
