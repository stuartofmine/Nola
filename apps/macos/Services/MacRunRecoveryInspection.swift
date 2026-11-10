import Foundation

/// Committed, read-only recovery facts. This is not permission to take over a
/// process, retry an effect, or conclude that an absent Run was never submitted.
nonisolated struct MacRunRecoveryInspection: Decodable, Equatable, Sendable {
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let observation: Observation

    /// Validate the remote source before contacting a selected Host. Recovery
    /// facts do not authorize execution; the Host still admits the exact Run.
    func requireActiveRemoteSource(_ hostID: String) throws {
        guard !hostID.isEmpty, case .active(let active) = observation,
              active.sourceHostID == hostID else {
            throw MacApplicationPortError.rejected("Recovery target does not match the active Run's recorded Host.")
        }
    }

    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", runID = "run_id"
        case assistantMessageID = "assistant_message_id", observation
    }

    struct DeliveryCount: Decodable, Equatable, Sendable {
        let state: String
        let count: UInt64
    }

    struct Active: Decodable, Equatable, Sendable {
        let runStatus: String
        // Existence of a conversation checkpoint is not proof that its content
        // is a valid recovery checkpoint for this Run; Application must verify.
        let checkpointPresent: Bool
        let pendingEffectCount: UInt64
        let sourceHostID: String?
        // Persisted lease existence is not evidence of OS process liveness.
        let hasSessionLease: Bool
        let deliveries: [DeliveryCount]
        enum CodingKeys: String, CodingKey {
            case runStatus = "run_status", checkpointPresent = "checkpoint_present"
            case pendingEffectCount = "pending_effect_count", sourceHostID = "source_host_id"
            case hasSessionLease = "has_session_lease", deliveries
        }
    }

    enum Observation: Decodable, Equatable, Sendable {
        case missing
        case identityMismatch
        case inconsistent(reason: String)
        case terminal(result: MacRunResult, resultDigest: String, completedAtMs: Int64)
        case active(Active)

        private enum CodingKeys: String, CodingKey {
            case state, reason, result, resultDigest = "result_digest", completedAtMs = "completed_at_ms"
        }
        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            switch try container.decode(String.self, forKey: .state) {
            case "missing": self = .missing
            case "identity_mismatch": self = .identityMismatch
            case "inconsistent": self = .inconsistent(reason: try container.decode(String.self, forKey: .reason))
            case "terminal": self = .terminal(
                result: try container.decode(MacRunResult.self, forKey: .result),
                resultDigest: try container.decode(String.self, forKey: .resultDigest),
                completedAtMs: try container.decode(Int64.self, forKey: .completedAtMs))
            case "active": self = .active(try Active(from: decoder))
            default: throw DecodingError.dataCorruptedError(forKey: .state, in: container,
                debugDescription: "Unknown recovery observation is not takeover authority")
            }
        }
    }
}
