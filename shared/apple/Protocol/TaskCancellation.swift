// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated public struct AppleTaskCancellationIntent: Codable, Equatable, Sendable {
    public let hostId: String
    public let conversationId: String
    public let parentRunId: String
    public let taskId: String
    public init(hostId: String, conversationId: String, parentRunId: String, taskId: String) {
        self.hostId = hostId; self.conversationId = conversationId; self.parentRunId = parentRunId; self.taskId = taskId
    }
}
nonisolated public struct AppleTaskCancellationReceipt: Decodable, Equatable, Sendable {
    public let hostId: String
    public let conversationId: String
    public let parentRunId: String
    public let taskId: String
    public let cancelRequested: Bool
    public let terminal: Bool
    public let state: String
    public let applicationPending: Bool
    public func validate(_ intent: AppleTaskCancellationIntent) throws {
        guard hostId == intent.hostId, conversationId == intent.conversationId,
              parentRunId == intent.parentRunId, taskId == intent.taskId, !applicationPending else {
            throw AppleTaskCancellationTransport.Failure.unconfirmed
        }
        let terminalStates = ["sealed", "failed", "cancelled"]
        let activeStates = ["queued", "running", "retry_waiting", "waiting_permission", "suspended", "awaiting_reconciliation"]
        guard (terminal ? terminalStates : activeStates).contains(state) else {
            throw AppleTaskCancellationTransport.Failure.unconfirmed
        }
    }
}

/// A cancellation request is monotonic for an immutable Task owner. Retrying
/// reads that owner's durable fact first; it never targets today's parent Run.
/// Saved intent is not a stop ACK: only a terminal read skips reconciliation.
nonisolated public enum AppleTaskCancellationTransport {
    public enum Failure: LocalizedError {
        case invalidOwner, unconfirmed
        public var errorDescription: String? {
            switch self {
            case .invalidOwner: "The task cancellation owner is invalid."
            case .unconfirmed: "Task cancellation has not been confirmed. Retry to check the same task."
            }
        }
    }
    public static func request(_ intent: AppleTaskCancellationIntent,
        transport: @Sendable (String, Data) async throws -> Data) async throws -> AppleTaskCancellationReceipt {
        guard [intent.hostId, intent.conversationId, intent.parentRunId, intent.taskId].allSatisfy({
            !$0.isEmpty && $0 == $0.trimmingCharacters(in: .whitespacesAndNewlines) && $0.utf8.count <= 512
        }) else { throw Failure.invalidOwner }
        let body = try JSONEncoder().encode(intent)
        func read(_ path: String) async throws -> AppleTaskCancellationReceipt {
            let data = try await transport(path, body)
            let receipt = try JSONDecoder().decode(AppleTaskCancellationReceipt.self, from: data)
            try receipt.validate(intent)
            return receipt
        }
        let observed = try await read("chat/agent/v2/task/cancel/receipt")
        if observed.terminal { return observed }
        do {
            let receipt = try await read("chat/agent/v2/task/cancel")
            guard receipt.cancelRequested || receipt.terminal else { throw Failure.unconfirmed }
            return receipt
        } catch {
            let observed = try await read("chat/agent/v2/task/cancel/receipt")
            guard observed.terminal else { throw error }
            return observed
        }
    }
}

/// Parent admission identity for the narrow, user-only child recovery commands.
nonisolated struct AppleOwnedTaskRecoveryOwner: Codable, Equatable, Sendable {
    let parent_conversation_id: String
    let parent_run_id: String
    let task_id: String
}
