// SPDX-License-Identifier: Apache-2.0
import Foundation
import CryptoKit

/// Immutable admission owner shared by Apple Host clients.
nonisolated struct AppleRemoteSubmissionIdentity: Codable, Equatable, Sendable {
    let source_host_id: String
    let client_id: String
    let conversation_id: String
    let submission_id: String
    let user_message_id: String
    let assistant_message_id: String
}

/// Transport acknowledgement is not a terminal Run receipt.
nonisolated struct AppleRemoteCancellationRequest: Encodable, Sendable {
    let identity: AppleRemoteSubmissionIdentity
    let request_digest: String

    enum ValidationError: Error { case missingAdmissionDigest }
    init(identity: AppleRemoteSubmissionIdentity, requestDigest: String) throws {
        guard !requestDigest.isEmpty else { throw ValidationError.missingAdmissionDigest }
        self.identity = identity
        request_digest = requestDigest
    }

    func object() throws -> [String: Any] {
        guard let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(self)) as? [String: Any] else {
            throw EncodingError.invalidValue(self, .init(codingPath: [], debugDescription: "Invalid cancellation envelope"))
        }
        return object
    }
}

/// Host transport intent, not a task projection. Its identity and complete input
/// survive an uncertain response so retry cannot enqueue a second instruction.
nonisolated public struct AppleTaskGuidanceIntent: Codable, Equatable, Sendable {
    public let hostId: String
    public let conversationId: String
    public let parentRunId: String
    public let taskId: String
    public let messageId: String
    public let submittedAtMs: Int64
    public let message: String
}

nonisolated public struct AppleTaskGuidanceReceipt: Codable, Equatable, Sendable {
    public let task_id: String
    public let message_id: String
    public let owner_session_id: String
    public let content: String
    public let submitted_at_ms: Int64
    public let status: String
    public let reason: String?
}

/// Both Apple clients use the same durable retry contract; authentication and
/// encryption stay in their existing HTTP adapters. Core owns task settlement.
public actor AppleTaskGuidanceOutbox {
    public static let shared = AppleTaskGuidanceOutbox(directory: FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("NolaTaskGuidance", isDirectory: true))
    private let directory: URL
    private var inFlight: Set<String> = []
    public init(directory: URL) { self.directory = directory }

    public enum Failure: LocalizedError {
        case invalidOwner, invalidReceipt, unresolvedPrevious, inFlight, rejected(String), recoveryIncomplete(Int)
        public var errorDescription: String? {
            switch self {
            case .invalidOwner: "Task guidance requires the observed Host and parent Run."
            case .invalidReceipt: "The Host returned a task guidance receipt for a different instruction."
            case .unresolvedPrevious: "An earlier task instruction is still unconfirmed. Retry that instruction first."
            case .inFlight: "This task instruction is already being confirmed."
            case .rejected(let reason): reason
            case .recoveryIncomplete(let count): "\(count) saved task instructions remain unconfirmed."
            }
        }
    }

    public func submit(hostId: String, conversationId: String, parentRunId: String,
        taskId: String, message: String,
        transport: @Sendable (String, Data) async throws -> Data
    ) async throws -> AppleTaskGuidanceReceipt {
        let content = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard [hostId, conversationId, parentRunId, taskId, content].allSatisfy({ !$0.isEmpty }) else {
            throw Failure.invalidOwner
        }
        let owner = SHA256.hash(data: try JSONEncoder().encode([hostId, conversationId, parentRunId, taskId]))
            .map { String(format: "%02x", $0) }.joined()
        guard inFlight.insert(owner).inserted else { throw Failure.inFlight }
        defer { inFlight.remove(owner) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent(owner + ".json")
        let existing = FileManager.default.fileExists(atPath: file.path)
        let intent: AppleTaskGuidanceIntent
        if existing {
            intent = try JSONDecoder().decode(AppleTaskGuidanceIntent.self, from: Data(contentsOf: file))
            guard intent.hostId == hostId, intent.conversationId == conversationId,
                  intent.parentRunId == parentRunId, intent.taskId == taskId,
                  intent.message == content else { throw Failure.unresolvedPrevious }
        } else {
            intent = AppleTaskGuidanceIntent(hostId: hostId, conversationId: conversationId,
                parentRunId: parentRunId, taskId: taskId, messageId: UUID().uuidString,
                submittedAtMs: Int64(Date().timeIntervalSince1970 * 1_000), message: content)
            try JSONEncoder().encode(intent).write(to: file, options: .atomic)
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let payload = try encoder.encode(intent)
        let path = "chat/agent/v2/task/message"
        var receipt: AppleTaskGuidanceReceipt?
        try Task.checkCancellation()
        if existing {
            receipt = try decode(await transport(path + "/receipt", payload), intent: intent, allowsMissing: true)
        }
        if receipt == nil {
            do {
                receipt = try decode(await transport(path, payload), intent: intent, allowsMissing: false)
            } catch {
                try Task.checkCancellation()
                // An HTTP error/timeout is not evidence that admission failed.
                // Only a matching authoritative null permits resubmission.
                receipt = try decode(await transport(path + "/receipt", payload), intent: intent, allowsMissing: true)
                if receipt == nil {
                    receipt = try decode(await transport(path, payload), intent: intent, allowsMissing: false)
                }
            }
        }
        guard let receipt else { throw Failure.invalidReceipt }
        try FileManager.default.removeItem(at: file)
        if receipt.status == "rejected" { throw Failure.rejected(receipt.reason ?? "Task guidance was rejected.") }
        return receipt
    }

    /// Reconnect resumes only already-authorized immutable intents belonging
    /// to this exact observed owner. No transcript text creates a new intent.
    public func recover(hostId: String, conversationId: String, parentRunId: String,
        transport: @Sendable (String, Data) async throws -> Data
    ) async throws {
        guard FileManager.default.fileExists(atPath: directory.path) else { return }
        var unresolved = 0
        for file in try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) where file.pathExtension == "json" {
            try Task.checkCancellation()
            do {
                let intent = try JSONDecoder().decode(AppleTaskGuidanceIntent.self, from: Data(contentsOf: file))
                guard intent.hostId == hostId, intent.conversationId == conversationId,
                      intent.parentRunId == parentRunId else { continue }
                _ = try await submit(hostId: hostId, conversationId: conversationId, parentRunId: parentRunId,
                    taskId: intent.taskId, message: intent.message, transport: transport)
            } catch Failure.inFlight {
                // The same intent already has an owner awaiting its receipt.
                continue
            } catch {
                // Preserve corrupt/unknown data and continue independent tasks.
                // Never rebuild or discard another instruction to unblock one.
                unresolved += 1
            }
        }
        if unresolved > 0 { throw Failure.recoveryIncomplete(unresolved) }
    }

    private func decode(_ data: Data, intent: AppleTaskGuidanceIntent, allowsMissing: Bool) throws -> AppleTaskGuidanceReceipt? {
        struct Envelope: Decodable {
            let hostId: String; let conversationId: String; let taskId: String; let messageId: String
            let guidance: AppleTaskGuidanceReceipt?
            enum CodingKeys: String, CodingKey { case hostId, conversationId, taskId, messageId, guidance }
            init(from decoder: Decoder) throws {
                let values = try decoder.container(keyedBy: CodingKeys.self)
                hostId = try values.decode(String.self, forKey: .hostId)
                conversationId = try values.decode(String.self, forKey: .conversationId)
                taskId = try values.decode(String.self, forKey: .taskId)
                messageId = try values.decode(String.self, forKey: .messageId)
                guidance = try values.decode(AppleTaskGuidanceReceipt?.self, forKey: .guidance)
            }
        }
        let envelope = try JSONDecoder().decode(Envelope.self, from: data)
        guard envelope.hostId == intent.hostId, envelope.conversationId == intent.conversationId,
              envelope.taskId == intent.taskId, envelope.messageId == intent.messageId else { throw Failure.invalidReceipt }
        guard let receipt = envelope.guidance else {
            guard allowsMissing else { throw Failure.invalidReceipt }
            return nil
        }
        guard receipt.task_id == intent.taskId, receipt.message_id == intent.messageId,
              receipt.owner_session_id == intent.conversationId, receipt.content == intent.message,
              receipt.submitted_at_ms == intent.submittedAtMs,
              ["submitted", "accepted", "delivered", "rejected"].contains(receipt.status) else { throw Failure.invalidReceipt }
        return receipt
    }
}

/// Embedded Apple hosts share the same Core command mapping as their durable
/// outbox. The caller supplies its already-observed exact Run/Turn owner and
/// its existing Application transport; no Host identity is inferred from text.
nonisolated public enum AppleTaskGuidanceCoreTransport {
    public static func exchange(path: String, payload: Data,
        conversationId: String, parentRunId: String, parentTurnId: String,
        execute: (Data) throws -> Data
    ) throws -> Data {
        let intent = try JSONDecoder().decode(AppleTaskGuidanceIntent.self, from: payload)
        guard intent.conversationId == conversationId, intent.parentRunId == parentRunId,
              !parentTurnId.isEmpty else { throw AppleTaskGuidanceOutbox.Failure.invalidOwner }
        guard path == "chat/agent/v2/task/message/receipt" || path == "chat/agent/v2/task/message" else {
            throw AppleTaskGuidanceOutbox.Failure.invalidOwner
        }
        // Core validates immutable parent Run authority even for a fresh intent.
        // A current view's task/Turn IDs alone cannot authorize another Run.
        let lookup: [String: Any] = ["type": "load_task_guidance_receipt", "task_id": intent.taskId,
            "message_id": intent.messageId, "parent_conversation_id": conversationId,
            "parent_run_id": parentRunId]
        var guidance = try JSONSerialization.jsonObject(with: execute(
            JSONSerialization.data(withJSONObject: lookup, options: [.sortedKeys])), options: [.fragmentsAllowed])
        if path == "chat/agent/v2/task/message", guidance is NSNull {
            let command: [String: Any] = ["type": "submit_task_guidance", "task_id": intent.taskId,
                "message_id": intent.messageId, "parent_turn_id": parentTurnId,
                "content": intent.message, "submitted_at_ms": intent.submittedAtMs]
            guidance = try JSONSerialization.jsonObject(with: execute(
                JSONSerialization.data(withJSONObject: command, options: [.sortedKeys])), options: [.fragmentsAllowed])
        }
        return try JSONSerialization.data(withJSONObject: [
            "hostId": intent.hostId, "conversationId": conversationId, "taskId": intent.taskId,
            "messageId": intent.messageId, "guidance": guidance,
        ], options: [.sortedKeys])
    }
}

// A source observation freezes admission preconditions, never rebases a bound request.
nonisolated struct AppleSubmissionFrontier: Decodable, Sendable {
    enum Failure: LocalizedError {
        case invalidOwner, parentChanged
        var errorDescription: String? {
            switch self {
            case .invalidOwner: "Host submission frontier has an invalid owner or revision"
            case .parentChanged: "The Host conversation changed. Reload its history before sending; your draft is retained."
            }
        }
    }
    let protocol_version: Int
    let source_host_id: String
    let client_id: String
    let conversation_id: String
    let configuration_revision: Int64?
    let parent_message_id: String?

    func validate(host: String, client: String, conversation: String, observedParent: String?) throws {
        guard protocol_version == 1, source_host_id == host, client_id == client,
              conversation_id == conversation,
              configuration_revision.map({ $0 > 0 }) ?? true,
              configuration_revision != nil || parent_message_id == nil else {
            throw Failure.invalidOwner
        }
        guard parent_message_id == observedParent else {
            throw Failure.parentChanged
        }
    }
}
