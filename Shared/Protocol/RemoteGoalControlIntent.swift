// SPDX-License-Identifier: Apache-2.0
import Foundation
import CryptoKit

nonisolated public struct AppleGoalControl: Codable, Equatable, Sendable {
    public enum Action: String, Codable, Equatable, Sendable { case start, pause, resume, cancel, schedule }
    public let operationId: String
    public let action: Action
    public let expectedGoalId: String?
    public let expectedControlToken: String?
    public let objective: String?
    public let constraints: [String]?
    public let wakeAtEpochMs: Int64?

    public init(action: Action, expectedGoalId: String? = nil, expectedControlToken: String? = nil,
        objective: String? = nil, constraints: [String]? = nil, wakeAtEpochMs: Int64? = nil,
        operationId: String = UUID().uuidString) {
        self.operationId = operationId; self.action = action; self.expectedGoalId = expectedGoalId
        self.expectedControlToken = expectedControlToken; self.objective = objective
        self.constraints = constraints; self.wakeAtEpochMs = wakeAtEpochMs
    }
    public func validate() throws {
        guard UUID(uuidString: operationId) != nil else { throw AppleGoalControlOutbox.Failure.invalidIntent }
        if action == .start {
            guard expectedGoalId == nil, expectedControlToken == nil, wakeAtEpochMs == nil,
                  let objective, !objective.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  constraints != nil else { throw AppleGoalControlOutbox.Failure.invalidIntent }
        } else {
            guard let expectedGoalId, !expectedGoalId.isEmpty, let expectedControlToken, !expectedControlToken.isEmpty,
                  objective == nil, constraints == nil else { throw AppleGoalControlOutbox.Failure.invalidIntent }
            if action == .schedule {
                guard let wakeAtEpochMs, wakeAtEpochMs > 0, wakeAtEpochMs <= 9_007_199_254_740_991 else { throw AppleGoalControlOutbox.Failure.invalidIntent }
            } else if wakeAtEpochMs != nil { throw AppleGoalControlOutbox.Failure.invalidIntent }
        }
    }
}
nonisolated public struct AppleGoalControlIntent: Codable, Equatable, Sendable {
    public let expected_host_id: String
    public let conversation_id: String
    public let control: AppleGoalControl
    public init(hostID: String, conversationID: String, control: AppleGoalControl) {
        expected_host_id = hostID; conversation_id = conversationID; self.control = control
    }
}
nonisolated public struct AppleGoalDraft: Codable, Equatable, Sendable {
    public let objective: String
    public let constraints: [String]
}
nonisolated public struct AppleGoalOperation: Decodable, Equatable, Sendable {
    public let command_sha256: String
    public let goal_revision: UInt64
    public let rejection: String?
    public var isValid: Bool {
        goal_revision > 0 && command_sha256.utf8.count == 64
            && command_sha256.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
}
nonisolated public struct AppleGoalControlResponse: Decodable, Sendable {
    public struct Projection: Decodable, Sendable {
        public let applied_operations: [String: AppleGoalOperation]
    }
    public let source_host_id: String
    public let conversationId: String?
    public let operationId: String?
    public let controlPending: Bool?
    public let authoritative: Bool?
    public let goalProjection: Projection?
    public let error: String?
}

/// Durable transport intent only. Goal transitions and acceptance belong to Core.
public actor AppleGoalControlOutbox {
    public static let shared = AppleGoalControlOutbox(directory: FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("NolaGoalControls", isDirectory: true))
    private let directory: URL
    private var inFlight: Set<String> = []
    public init(directory: URL) { self.directory = directory }
    public enum Failure: LocalizedError {
        case invalidIntent, invalidResponse, unresolvedPrevious, inFlight, unconfirmed(String), rejected(String)
        public var errorDescription: String? {
            switch self {
            case .invalidIntent: "Invalid Goal control identity."
            case .invalidResponse: "Goal confirmation does not match this Host and conversation."
            case .unresolvedPrevious: "An earlier Goal action is unconfirmed. Retry the saved action first."
            case .inFlight: "This Goal action is already being confirmed."
            case .unconfirmed(let reason): "Goal action is unconfirmed: \(reason)"
            case .rejected(let reason): reason
            }
        }
    }
    private func file(hostID: String, conversationID: String) throws -> URL {
        guard !hostID.isEmpty, !conversationID.isEmpty else { throw Failure.invalidIntent }
        let key = SHA256.hash(data: try JSONEncoder().encode([hostID, conversationID])).map { String(format: "%02x", $0) }.joined()
        return directory.appendingPathComponent(key + ".json")
    }
    public func pending(hostID: String, conversationID: String) throws -> AppleGoalControlIntent? {
        let url = try file(hostID: hostID, conversationID: conversationID)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let saved = try JSONDecoder().decode(AppleGoalControlIntent.self, from: Data(contentsOf: url))
        guard saved.expected_host_id == hostID, saved.conversation_id == conversationID else { throw Failure.invalidResponse }
        try saved.control.validate()
        return saved
    }
    public func draft(hostID: String, conversationID: String) throws -> AppleGoalDraft? {
        let url = try file(hostID: hostID, conversationID: conversationID).appendingPathExtension("draft")
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try JSONDecoder().decode(AppleGoalDraft.self, from: Data(contentsOf: url))
    }
    private func clearAppliedDraft(_ intent: AppleGoalControlIntent) throws {
        guard intent.control.action == .start else { return }
        let url = try file(hostID: intent.expected_host_id, conversationID: intent.conversation_id).appendingPathExtension("draft")
        if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
    }
    public func submit(_ intent: AppleGoalControlIntent,
        transport: @Sendable (AppleGoalControlIntent) async throws -> AppleGoalControlResponse
    ) async throws -> Bool {
        try intent.control.validate()
        let url = try file(hostID: intent.expected_host_id, conversationID: intent.conversation_id)
        guard inFlight.insert(url.path).inserted else { throw Failure.inFlight }
        defer { inFlight.remove(url.path) }
        if let saved = try pending(hostID: intent.expected_host_id, conversationID: intent.conversation_id) {
            guard saved == intent else { throw Failure.unresolvedPrevious }
        } else {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            if intent.control.action == .start, let objective = intent.control.objective, let constraints = intent.control.constraints {
                try JSONEncoder().encode(AppleGoalDraft(objective: objective, constraints: constraints))
                    .write(to: url.appendingPathExtension("draft"), options: .atomic)
            }
            try JSONEncoder().encode(intent).write(to: url, options: .atomic)
        }
        let response = try await transport(intent)
        guard response.source_host_id == intent.expected_host_id else { throw Failure.invalidResponse }
        guard response.conversationId == intent.conversation_id, response.operationId == intent.control.operationId else {
            if let error = response.error { throw Failure.unconfirmed(error) }
            throw Failure.invalidResponse
        }
        // A 202/provisional projection cannot settle the durable intent.
        guard response.controlPending != true, response.authoritative == true,
              let operation = response.goalProjection?.applied_operations[intent.control.operationId] else { return false }
        guard operation.isValid else { throw Failure.invalidResponse }
        if let reason = operation.rejection {
            try FileManager.default.removeItem(at: url)
            throw Failure.rejected(reason)
        }
        try clearAppliedDraft(intent)
        try FileManager.default.removeItem(at: url)
        return true
    }
    public func reconcile(hostID: String, conversationID: String,
        appliedOperations: [String: AppleGoalOperation]) throws -> Bool {
        guard let saved = try pending(hostID: hostID, conversationID: conversationID),
              let operation = appliedOperations[saved.control.operationId] else { return false }
        guard operation.isValid else { throw Failure.invalidResponse }
        if let reason = operation.rejection {
            try FileManager.default.removeItem(at: file(hostID: hostID, conversationID: conversationID))
            throw Failure.rejected(reason)
        }
        try clearAppliedDraft(saved)
        try FileManager.default.removeItem(at: file(hostID: hostID, conversationID: conversationID))
        return true
    }
}
