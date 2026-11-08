import Foundation

/// One request's preparation/dispatch boundary, shared across actor and Task
/// hops. It authorizes dispatch once; it is not evidence of a Host Run terminal.
nonisolated final class MacRunSubmissionTicket: @unchecked Sendable {
    struct Identity: Equatable, Sendable {
        let conversationID: String
        let submissionID: String
        let generationID: UInt64
    }

    enum Phase: Equatable, Sendable {
        case preparing, dispatching, accepted, abandoned, rejectedBeforeDispatch, rejectedBeforeAdmission
    }

    enum SubmissionError: Error, Equatable {
        case preparationAbandoned, dispatchAlreadyClaimed, rejectedBeforeDispatch, identityMismatch
    }

    let identity: Identity
    private let lock = NSLock()
    private var storedPhase: Phase = .preparing

    init(identity: Identity) { self.identity = identity }

    var phase: Phase {
        lock.lock()
        defer { lock.unlock() }
        return storedPhase
    }

    func checkPreparation() throws {
        lock.lock()
        defer { lock.unlock() }
        try validatePreparation()
    }

    func claimDispatch() throws {
        lock.lock()
        defer { lock.unlock() }
        try validatePreparation()
        storedPhase = .dispatching
    }

    @discardableResult
    func cancelPreparation() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard storedPhase == .preparing else { return false }
        storedPhase = .abandoned
        return true
    }

    /// Only the dispatch adapter may call this on a definite pre-transport
    /// failure. A timeout or disconnect after dispatch never proves rejection.
    func rejectBeforeTransport() {
        lock.lock()
        defer { lock.unlock() }
        guard storedPhase == .preparing || storedPhase == .dispatching else { return }
        storedPhase = .rejectedBeforeDispatch
    }

    /// A matching, structured Host response proves the dispatched submission
    /// was not admitted. EOF, stderr and process exit never authorize this.
    @discardableResult
    func rejectBeforeAdmission() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard storedPhase == .dispatching else { return false }
        storedPhase = .rejectedBeforeAdmission
        return true
    }

    @discardableResult
    func markAccepted() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard storedPhase == .dispatching else { return false }
        storedPhase = .accepted
        return true
    }

    private func validatePreparation() throws {
        switch storedPhase {
        case .preparing: return
        case .dispatching, .accepted: throw SubmissionError.dispatchAlreadyClaimed
        case .abandoned: throw SubmissionError.preparationAbandoned
        case .rejectedBeforeDispatch, .rejectedBeforeAdmission: throw SubmissionError.rejectedBeforeDispatch
        }
    }
}
