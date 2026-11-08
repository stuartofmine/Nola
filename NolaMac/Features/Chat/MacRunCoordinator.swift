import Foundation
import Observation

/// Owns execution attempts, not message content. Only committed Run evidence
/// can authorize terminal presentation; transport lifetime is not Run lifetime.
@MainActor @Observable
final class MacRunCoordinator {
    struct Identity: Equatable {
        let conversationID: String
        let runID: String?
        let assistantMessageID: UUID
    }

    typealias TerminalStatus = MacRunResult.Value.Kind
    enum FinishDisposition: Equatable { case completed, alreadyCompleted, inProgress, stale }
    enum State: Equatable {
        case idle, preparing, preparationAbandoned, running, awaitingInput, awaitingRecovery, cancelRequested, interrupted
        case confirmedTerminal(TerminalStatus)
    }

    private(set) var generationID: UInt64 = 0
    private(set) var identity: Identity?
    private(set) var state: State = .idle
    private(set) var cancellationRequestedGenerationID: UInt64?
    private(set) var submissionTicket: MacRunSubmissionTicket?
    private var cancellationDeliveryGenerationID: UInt64?
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var finishingGenerationID: UInt64?
    @ObservationIgnored private var finalizedGenerationID: UInt64?
    @ObservationIgnored private var finishWaiters: [UInt64: [CheckedContinuation<Bool, Never>]] = [:]
    private var stateBeforeCancellation: State = .running

    var canResume: Bool {
        identity?.runID != nil && (state == .awaitingInput || state == .awaitingRecovery || state == .interrupted)
    }

    @discardableResult
    func begin(identity: Identity) -> UInt64 {
        releaseFinishWaiters(generationID: generationID, completed: false)
        task?.cancel()
        task = nil
        generationID &+= 1
        self.identity = identity
        finishingGenerationID = nil
        finalizedGenerationID = nil
        state = .running
        cancellationRequestedGenerationID = nil
        submissionTicket = nil
        cancellationDeliveryGenerationID = nil
        return generationID
    }

    func beginPreparation(identity: Identity, submissionID: String) -> MacRunSubmissionTicket {
        let generation = begin(identity: identity)
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: identity.conversationID,
            submissionID: submissionID, generationID: generation))
        submissionTicket = ticket
        state = .preparing
        return ticket
    }

    /// This is local intent cancellation, never a canonical cancelled Run.
    func abandonPreparation(generationID: UInt64) -> Bool {
        guard generationID == self.generationID,
              submissionTicket?.cancelPreparation() == true else { return false }
        state = .preparationAbandoned
        task?.cancel()
        task = nil
        return true
    }

    func preparationFailed(generationID: UInt64) {
        guard generationID == self.generationID else { return }
        state = .preparationAbandoned
        task = nil
    }

    func submissionAccepted(generationID: UInt64) {
        guard generationID == self.generationID, submissionTicket?.phase == .accepted else { return }
        if state == .preparing { state = .running }
    }

    func claimCancellationDelivery(generationID: UInt64) -> Bool {
        guard generationID == self.generationID,
              cancellationRequestedGenerationID == generationID,
              !canFinish(generationID: generationID),
              cancellationDeliveryGenerationID != generationID,
              submissionTicket == nil || submissionTicket?.phase == .accepted else { return false }
        cancellationDeliveryGenerationID = generationID
        return true
    }

    /// Delivery failure does not retract an already committed cancellation.
    /// Only the transport claim is released; ordinary execution stays disabled.
    func cancellationDeliveryInterrupted(generationID: UInt64) {
        guard generationID == self.generationID,
              cancellationRequestedGenerationID == generationID,
              !canFinish(generationID: generationID) else { return }
        cancellationDeliveryGenerationID = nil
    }

    @discardableResult
    func transportInterrupted(generationID: UInt64) -> Bool {
        guard generationID == self.generationID, identity != nil,
              !canFinish(generationID: generationID) else { return false }
        if cancellationRequestedGenerationID == generationID {
            stateBeforeCancellation = .awaitingRecovery
            state = .cancelRequested
        } else {
            state = .awaitingRecovery
        }
        return true
    }

    @discardableResult
    func waitSuspended(generationID: UInt64, conversationID: String, runID: String) -> Bool {
        guard generationID == self.generationID,
              identity?.conversationID == conversationID, identity?.runID == runID,
              cancellationRequestedGenerationID != generationID,
              !canFinish(generationID: generationID) else { return false }
        state = .awaitingInput
        return true
    }

    @discardableResult
    func requestCancellation(generationID: UInt64) -> Bool {
        guard generationID == self.generationID, identity != nil,
              cancellationRequestedGenerationID != generationID,
              !canFinish(generationID: generationID) else { return false }
        cancellationRequestedGenerationID = generationID
        stateBeforeCancellation = state
        state = .cancelRequested
        return true
    }

    func cancellationFailed(generationID: UInt64) {
        guard generationID == self.generationID,
              cancellationRequestedGenerationID == generationID,
              !canFinish(generationID: generationID) else { return }
        cancellationRequestedGenerationID = nil
        state = stateBeforeCancellation
        cancellationDeliveryGenerationID = nil
    }

    @discardableResult
    func confirmTerminal(generationID: UInt64, conversationID: String,
                         runID: String, status: TerminalStatus) -> Bool {
        guard generationID == self.generationID,
              identity?.conversationID == conversationID,
              identity?.runID == runID,
              !canFinish(generationID: generationID) else { return false }
        state = .confirmedTerminal(status)
        return true
    }

    func canFinish(generationID: UInt64) -> Bool {
        guard generationID == self.generationID else { return false }
        if case .confirmedTerminal = state { return true }
        return false
    }

    func claimFinish(generationID: UInt64) -> Bool {
        guard canFinish(generationID: generationID), finishingGenerationID != generationID else { return false }
        finishingGenerationID = generationID
        return true
    }

    func isFinalized(generationID: UInt64) -> Bool {
        generationID == self.generationID && finalizedGenerationID == generationID
    }

    func completeFinish(generationID: UInt64) {
        guard generationID == self.generationID, finishingGenerationID == generationID else { return }
        finalizedGenerationID = generationID
        releaseFinishWaiters(generationID: generationID, completed: true)
    }

    func waitForFinish(generationID: UInt64) async -> Bool {
        guard generationID == self.generationID else { return false }
        if isFinalized(generationID: generationID) { return true }
        guard finishingGenerationID == generationID else { return false }
        return await withCheckedContinuation { finishWaiters[generationID, default: []].append($0) }
    }

    private func releaseFinishWaiters(generationID: UInt64, completed: Bool) {
        let waiters = finishWaiters.removeValue(forKey: generationID) ?? []
        for waiter in waiters { waiter.resume(returning: completed) }
    }

    func resume(generationID: UInt64) -> UInt64? {
        guard generationID == self.generationID, canResume else { return nil }
        releaseFinishWaiters(generationID: generationID, completed: false)
        task?.cancel()
        task = nil
        self.generationID &+= 1
        finishingGenerationID = nil
        finalizedGenerationID = nil
        cancellationRequestedGenerationID = nil
        cancellationDeliveryGenerationID = nil
        submissionTicket = nil
        state = .running
        return self.generationID
    }

    func replaceTask(_ task: Task<Void, Never>?) { self.task = task }

    /// Reconstructs an observer for a canonical Run without inventing a trace
    /// identity or pretending that an unknown Run can be resumed.
    func observeInterruption(identity: Identity, generationID: UInt64) {
        guard generationID == self.generationID, self.identity == nil else { return }
        self.identity = identity
        state = .awaitingRecovery
    }

    func releaseTask(generationID: UInt64) {
        guard generationID == self.generationID else { return }
        task = nil
    }

    /// Relinquish only a disconnected presentation owner, not the canonical Run.
    /// Advancing the generation fences late callbacks from that owner.
    func detachRecovery(generationID: UInt64) -> Bool {
        guard generationID == self.generationID,
              state == .awaitingRecovery || state == .awaitingInput || state == .cancelRequested,
              task == nil else { return false }
        releaseFinishWaiters(generationID: generationID, completed: false)
        self.generationID &+= 1
        identity = nil
        state = .idle
        cancellationRequestedGenerationID = nil
        cancellationDeliveryGenerationID = nil
        submissionTicket = nil
        return true
    }
}
