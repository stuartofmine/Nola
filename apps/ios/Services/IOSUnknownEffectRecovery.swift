// SPDX-License-Identifier: Apache-2.0
// User-only A08 commands. These fences never enter model context or diagnostics.
import Foundation

nonisolated struct IOSUnknownEffectRecoveryTarget: Codable, Equatable, Sendable {
    let effect_id: String
    let request_id: String
    let attempt_id: String
    let worker_generation: String
    let lease_owner: String
    let lease_token: String
    let expected_attempt_revision: Int64
}

nonisolated struct IOSUnknownEffectRecoveryInspection: Codable, Equatable, Sendable {
    let conversation_id: String
    let run_id: String
    let assistant_message_id: String
    let invocation_config_digest: String
    let targets: [IOSUnknownEffectRecoveryTarget]
    let retry_allowed: Bool
    let independent_pending_effects: UInt64

    func confirmed(choice: IOSUnknownEffectRecoveryDecision.Choice.Kind, reason: String,
                   target: IOSUnknownEffectRecoveryTarget? = nil,
                   resolutionID: String = UUID().uuidString.lowercased(),
                   nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)) throws -> IOSUnknownEffectRecoveryDecision {
        let selected: [IOSUnknownEffectRecoveryTarget]
        switch choice {
        case .authorizeRiskRetry:
            guard retry_allowed, let target, targets.contains(target) else { throw IOSApplicationRuntimeError.invalidResponse }
            selected = [target]
        case .keepUnknownBlocked:
            guard independent_pending_effects == 0, target == nil else { throw IOSApplicationRuntimeError.invalidResponse }
            selected = targets
        }
        let decision = IOSUnknownEffectRecoveryDecision(conversation_id: conversation_id, run_id: run_id,
            invocation_config_digest: invocation_config_digest, resolution_id: resolutionID,
            targets: selected, choice: .init(kind: choice, reason: reason), resolved_at_ms: nowMs)
        try decision.validate()
        return decision
    }
}

nonisolated struct IOSUnknownEffectRecoveryDecision: Codable, Equatable, Sendable {
    struct Choice: Codable, Equatable, Sendable {
        enum Kind: String, Codable, Sendable {
            case authorizeRiskRetry = "authorize_risk_retry"
            case keepUnknownBlocked = "keep_unknown_blocked"
        }
        let kind: Kind
        let reason: String
    }
    let conversation_id: String
    let run_id: String
    let invocation_config_digest: String
    let resolution_id: String
    let targets: [IOSUnknownEffectRecoveryTarget]
    let choice: Choice
    let resolved_at_ms: Int64

    static func validID(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 512
            && value.trimmingCharacters(in: .whitespacesAndNewlines) == value
            && value.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
    }
    func validate() throws {
        guard [conversation_id, run_id, invocation_config_digest, resolution_id].allSatisfy(Self.validID),
              resolved_at_ms >= 0, (1...256).contains(targets.count),
              !choice.reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              choice.reason.utf8.count <= 4096 else { throw IOSApplicationRuntimeError.invalidResponse }
        for target in targets {
            guard [target.effect_id, target.request_id, target.attempt_id, target.worker_generation,
                   target.lease_owner, target.lease_token].allSatisfy(Self.validID),
                  target.expected_attempt_revision > 0, target.expected_attempt_revision < Int64.max else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
        }
    }
}

nonisolated struct IOSUnknownEffectRecoveryReply: Codable, Equatable, Sendable {
    enum Outcome: String, Codable, Sendable {
        case retryAuthorized = "retry_authorized", unknownPreservedBlocked = "unknown_preserved_blocked"
        case rejected, conflict
    }
    let outcome: Outcome
    let resolution_id: String?
    let run_id: String?
    let code: String?

    func validate(for decision: IOSUnknownEffectRecoveryDecision) throws {
        switch outcome {
        case .retryAuthorized, .unknownPreservedBlocked:
            guard resolution_id == decision.resolution_id, run_id == decision.run_id, code == nil,
                  (outcome == .retryAuthorized) == (decision.choice.kind == .authorizeRiskRetry) else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
        case .rejected, .conflict:
            guard let code, !code.isEmpty, resolution_id == nil, run_id == nil else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
        }
    }
}

/// An in-memory view of the existing Application pending envelope, not a journal.
nonisolated struct IOSPendingUnknownEffectRecovery: Equatable, Sendable {
    let requestID: String
    let decision: IOSUnknownEffectRecoveryDecision
}

/// UI-safe projection: no lease tokens, worker fences, or mutable command payload.
nonisolated struct IOSUnknownEffectRecoveryPresentation: Equatable, Sendable {
    enum Phase: Equatable, Sendable { case inspected, outcomePending, retryAuthorized, blocked, rejected }
    let conversationID: UUID
    let inspectionID: UUID
    let runID: String
    let assistantMessageID: String
    let attemptIDs: [String]
    let canRetry: Bool
    let canKeepBlocked: Bool
    let phase: Phase

    func acceptsConfirmation(_ captured: Self) -> Bool {
        phase == .inspected && self == captured
    }
}
