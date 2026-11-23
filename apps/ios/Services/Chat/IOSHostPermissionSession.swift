// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns one Host permission interaction and its replacement, retry, and continuation lifecycle.

import Foundation

@MainActor
final class IOSHostPermissionSession {
    struct State {
        let request: HostAgentPermissionRequest?
        let conversationID: UUID?
        let submissionError: String?
        let isSubmitting: Bool
        let pendingCount: Int
    }

    private struct QueuedInteraction {
        let request: HostAgentPermissionRequest
        let conversationID: UUID?
        let isCurrent: () -> Bool
        let send: ([String: Any]) async throws -> Void
        let onSubmitted: (HostAgentPermissionDecision) -> Void
        let onSubmissionFailure: (HostAgentPermissionRequest, Error) -> Void
    }

    var onStateChange: (@MainActor (State) -> Void)?

    private var request: HostAgentPermissionRequest?
    private var conversationID: UUID?
    private var submissionError: String?
    private var isActiveSubmitting = false
    private var continuation: CheckedContinuation<HostAgentPermissionDecision, Error>?
    private var interactionTask: Task<Void, Never>?
    private var interactionToken: UUID?
    private var pendingQueue: [QueuedInteraction] = []

    func resolve(requestID: String, decision: HostAgentPermissionDecision) {
        guard let request, request.id == requestID, let continuation else { return }
        if case .provideInstruction(let instruction) = decision {
            guard request.deviceBridge != true,
                  HostAgentPermissionDecision.validatedInstruction(
                    instruction,
                    requestID: request.id,
                    existing: request.permissionInstructions ?? []
                  ) != nil else { return }
        } else if case .returnToComposer = decision {
            guard request.deviceBridge != true else { return }
        }
        self.continuation = nil
        self.isActiveSubmitting = true
        publish(request: request, conversationID: conversationID, error: nil, isSubmitting: true)
        continuation.resume(returning: decision)
    }

    static func decisionPayload(
        for request: HostAgentPermissionRequest,
        decision: HostAgentPermissionDecision
    ) -> [String: Any] {
        var payload: [String: Any] = [
            "id": request.id,
            "request_id": request.id,
            "tool_name": request.toolName,
            "match": request.match,
            "resolution": decision.resolutionPayload,
        ]
        if let value = request.nonce { payload["nonce"] = value }
        if let value = request.hostId { payload["host_id"] = value }
        if let value = request.clientId { payload["client_id"] = value }
        if let value = request.sessionId { payload["session_id"] = value }
        if let value = request.protocolVersion { payload["protocol_version"] = value }
        return payload
    }

    @discardableResult
    func start(
        _ request: HostAgentPermissionRequest,
        conversationID: UUID?,
        isCurrent: @escaping () -> Bool,
        send: @escaping ([String: Any]) async throws -> Void,
        onSubmitted: @escaping (HostAgentPermissionDecision) -> Void,
        onSubmissionFailure: @escaping (HostAgentPermissionRequest, Error) -> Void
    ) -> Task<Void, Never> {
        guard isCurrent() else { return Task {} }
        // A renewed challenge can retain the logical request ID while changing
        // its nonce, expiry or review content. Only an exact replay is inert.
        if self.request == request, self.conversationID == conversationID,
           let interactionTask { return interactionTask }
        if pendingQueue.contains(where: { $0.request == request && $0.conversationID == conversationID }) {
            return Task {}
        }
        // Renewed challenge for an already-queued request updates in place.
        if let queuedIndex = pendingQueue.firstIndex(where: { $0.request.id == request.id }) {
            pendingQueue[queuedIndex] = QueuedInteraction(
                request: request,
                conversationID: conversationID,
                isCurrent: isCurrent,
                send: send,
                onSubmitted: onSubmitted,
                onSubmissionFailure: onSubmissionFailure
            )
            return Task {}
        }
        // Queue concurrent distinct requests (e.g. multiple subagents) in FIFO order
        // when the active card is still waiting for the user's decision.
        // If the active request has the same ID (renewed nonce) or is already submitting
        // (e.g. successor arrived while an earlier HTTP reply is in flight), replace active.
        if let active = self.request, active.id != request.id, !isActiveSubmitting {
            pendingQueue.append(QueuedInteraction(
                request: request,
                conversationID: conversationID,
                isCurrent: isCurrent,
                send: send,
                onSubmitted: onSubmitted,
                onSubmissionFailure: onSubmissionFailure
            ))
            publish(request: active, conversationID: self.conversationID, error: submissionError, isSubmitting: false)
            return Task {}
        }
        return activate(
            request,
            conversationID: conversationID,
            isCurrent: isCurrent,
            send: send,
            onSubmitted: onSubmitted,
            onSubmissionFailure: onSubmissionFailure
        )
    }

    private func activate(
        _ request: HostAgentPermissionRequest,
        conversationID: UUID?,
        isCurrent: @escaping () -> Bool,
        send: @escaping ([String: Any]) async throws -> Void,
        onSubmitted: @escaping (HostAgentPermissionDecision) -> Void,
        onSubmissionFailure: @escaping (HostAgentPermissionRequest, Error) -> Void
    ) -> Task<Void, Never> {
        cancelActiveOnly()
        let token = UUID()
        interactionToken = token
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                let decision = try await self.submit(
                    request,
                    conversationID: conversationID,
                    isCurrent: { isCurrent() && self.interactionToken == token },
                    send: send,
                    onSubmissionFailure: onSubmissionFailure
                )
                guard self.interactionToken == token, isCurrent(), !Task.isCancelled else { return }
                onSubmitted(decision)
            } catch is CancellationError {
                // Replacement and explicit cancellation do not produce UI failures.
            } catch {
                onSubmissionFailure(request, error)
            }
            self.finish(token: token)
        }
        interactionTask = task
        return task
    }

    func cancel(requestID: String? = nil) {
        if let requestID {
            let removedQueued = pendingQueue.contains { $0.request.id == requestID }
            pendingQueue.removeAll { $0.request.id == requestID }
            if request?.id != requestID {
                if removedQueued, let active = request {
                    publish(request: active, conversationID: conversationID, error: submissionError, isSubmitting: isActiveSubmitting)
                }
                return
            }
            cancelActiveOnly()
            promoteNextQueuedIfNeeded()
            return
        }
        pendingQueue.removeAll()
        cancelActiveOnly()
    }

    private func cancelActiveOnly() {
        interactionToken = nil
        interactionTask?.cancel()
        interactionTask = nil
        isActiveSubmitting = false
        continuation?.resume(throwing: CancellationError())
        continuation = nil
        clear(requestID: request?.id)
    }

    func submit(
        _ request: HostAgentPermissionRequest,
        conversationID: UUID?,
        isCurrent: () -> Bool,
        send: ([String: Any]) async throws -> Void,
        onSubmissionFailure: (HostAgentPermissionRequest, Error) -> Void
    ) async throws -> HostAgentPermissionDecision {
        var pendingFailure: Error?
        while true {
            guard isCurrent(), !Task.isCancelled else { throw CancellationError() }
            isActiveSubmitting = false
            let decision = try await waitForDecision(request, conversationID: conversationID, onReady: {
                if let pendingFailure { onSubmissionFailure(request, pendingFailure) }
            })
            pendingFailure = nil
            guard isCurrent(), !Task.isCancelled else {
                clear(requestID: request.id)
                throw CancellationError()
            }
            do {
                try await send(Self.decisionPayload(for: request, decision: decision))
                guard isCurrent(), !Task.isCancelled else { throw CancellationError() }
                isActiveSubmitting = false
                clear(requestID: request.id)
                return decision
            } catch {
                isActiveSubmitting = false
                guard isCurrent(), !Task.isCancelled else {
                    clear(requestID: request.id)
                    throw CancellationError()
                }
                // Do not expose a retryable failure until the next decision
                // receiver exists. Either callback may synchronously retry.
                pendingFailure = error
                submissionError = error.localizedDescription
            }
        }
    }

    func waitForDecision(
        _ request: HostAgentPermissionRequest,
        conversationID: UUID?,
        onReady: () -> Void = {}
    ) async throws -> HostAgentPermissionDecision {
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation?.resume(throwing: CancellationError())
            self.continuation = continuation
            self.isActiveSubmitting = false
            // Rendering can synchronously deliver a decision. Expose the card
            // only after its receiver exists; an early tap must not be lost.
            publish(request: request, conversationID: conversationID, error: submissionError)
            onReady()
        }
    }

    func clear(requestID: String?) {
        guard let requestID, request?.id == requestID else { return }
        isActiveSubmitting = false
        publish(request: nil, conversationID: nil, error: nil)
    }

    private func finish(token: UUID) {
        guard interactionToken == token else { return }
        interactionTask = nil
        interactionToken = nil
        isActiveSubmitting = false
        promoteNextQueuedIfNeeded()
    }

    private func promoteNextQueuedIfNeeded() {
        guard request == nil else { return }
        while !pendingQueue.isEmpty {
            let next = pendingQueue.removeFirst()
            guard next.isCurrent() else { continue }
            _ = activate(
                next.request,
                conversationID: next.conversationID,
                isCurrent: next.isCurrent,
                send: next.send,
                onSubmitted: next.onSubmitted,
                onSubmissionFailure: next.onSubmissionFailure
            )
            return
        }
    }

    private func publish(
        request: HostAgentPermissionRequest?,
        conversationID: UUID?,
        error: String?,
        isSubmitting: Bool = false
    ) {
        self.request = request
        self.conversationID = conversationID
        self.submissionError = error
        onStateChange?(State(
            request: request,
            conversationID: conversationID,
            submissionError: error,
            isSubmitting: isSubmitting,
            pendingCount: request == nil ? 0 : 1 + pendingQueue.count
        ))
    }
}
