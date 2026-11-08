// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Shared typed journal display facts and independent Core lifecycle control.

import Foundation

nonisolated public struct AgentCoreTurnTerminalEnvelope: Decodable, Equatable, Sendable {
    public enum Status: String, Decodable, Equatable, Sendable {
        case completed
        case failed
        case cancelled
    }

    public let sessionId: String
    public let turnId: String
    public let cursor: UInt64?
    public let revision: UInt64?
    public let status: Status
    public let reason: String?

    private init(
        sessionId: String,
        turnId: String,
        cursor: UInt64?,
        revision: UInt64?,
        status: Status,
        reason: String?
    ) {
        self.sessionId = sessionId
        self.turnId = turnId
        self.cursor = cursor
        self.revision = revision
        self.status = status
        self.reason = reason
    }

    private struct CoreWire: Decodable {
        let sessionId: String
        let turnId: String?
        let cursor: CoreCursor?
        let revision: UInt64?
        let event: Event

        enum CodingKeys: String, CodingKey {
            case cursor, revision, event
            case sessionId = "session_id"
            case turnId = "turn_id"
        }
    }

    private struct HostWire: Decodable {
        let protocolVersion: Int
        let type: String
        let sessionId: String
        let turnId: String
        let cursor: UInt64
        let revision: UInt64
        let reason: String?

        enum CodingKeys: String, CodingKey {
            case type, cursor, revision, reason
            case protocolVersion = "protocol_version"
            case sessionId = "session_id"
            case turnId = "turn_id"
        }
    }

    private struct Event: Decodable {
        let type: String
        let turnId: String?
        let reason: String?

        enum CodingKeys: String, CodingKey {
            case type, reason
            case turnId = "turn_id"
        }
    }

    private struct CoreCursor: Decodable {
        let sequence: UInt64
    }

    public init(from decoder: Decoder) throws {
        let wire = try CoreWire(from: decoder)
        let status: Status
        switch wire.event.type {
        case "turn_completed": status = .completed
        case "turn_failed": status = .failed
        case "turn_cancelled": status = .cancelled
        default:
            throw DecodingError.dataCorrupted(.init(
                codingPath: decoder.codingPath,
                debugDescription: "Invalid Agent Core Turn terminal envelope"
            ))
        }
        guard let turnId = wire.event.turnId ?? wire.turnId else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: decoder.codingPath,
                debugDescription: "Agent Core Turn terminal identity is missing"
            ))
        }
        self.sessionId = wire.sessionId
        self.turnId = turnId
        self.cursor = wire.cursor?.sequence
        self.revision = wire.revision
        self.status = status
        self.reason = wire.event.reason
    }

    public static func decodeCore(from value: AgentCoreJSONValue) -> Self? {
        guard let data = try? JSONEncoder().encode(value) else { return nil }
        return try? JSONDecoder().decode(Self.self, from: data)
    }

    public static func decodeHost(from object: [String: Any]) -> Self? {
        guard JSONSerialization.isValidJSONObject(object),
              object["event"] == nil,
              let data = try? JSONSerialization.data(withJSONObject: object),
              let wire = try? JSONDecoder().decode(HostWire.self, from: data),
              wire.protocolVersion == 1 else { return nil }
        let status: Status
        switch wire.type {
        case "turn_completed": status = .completed
        case "turn_failed": status = .failed
        case "turn_cancelled": status = .cancelled
        default: return nil
        }
        return Self(
            sessionId: wire.sessionId,
            turnId: wire.turnId,
            cursor: wire.cursor,
            revision: wire.revision,
            status: status,
            reason: wire.reason
        )
    }
}

nonisolated public struct AgentCoreCapabilityOrigin: Decodable, Equatable, Sendable {
    public let kind: String
    public let providerId: String?
    public let providerTitle: String?
    public let namespace: String?

    enum CodingKeys: String, CodingKey {
        case kind, namespace
        case providerId = "provider_id"
        case providerTitle = "provider_title"
    }
}

nonisolated public struct AgentCoreSourceEvidence: Decodable, Equatable, Sendable {
    public let kind: String
    public let usage: ChatMemoryUsageEvidence?
    private enum CodingKeys: String, CodingKey { case kind, usage }
    private struct AnyKey: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }
    public init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: AnyKey.self)
        guard Set(keys.allKeys.map(\.stringValue)) == Set(["kind", "usage"]) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Evidence contains only typed redacted usage"))
        }
        let v = try decoder.container(keyedBy: CodingKeys.self)
        kind = try v.decode(String.self, forKey: .kind)
        guard kind == "memory" else {
            throw DecodingError.dataCorruptedError(forKey: .kind, in: v, debugDescription: "Unknown evidence kind")
        }
        usage = try v.decode(ChatMemoryUsageEvidence.self, forKey: .usage)
    }
}

nonisolated public enum AgentCoreTextReview: String, Codable, Equatable, Sendable { case pending, approved, rejected, interrupted }

nonisolated public struct AgentCoreToolPermission: Decodable, Equatable, Sendable {
    public let requestId: String
    public let permissionDomain: String
    public let rationale: String
    public let decision: String?

    enum CodingKeys: String, CodingKey {
        case rationale, decision
        case requestId = "request_id"
        case permissionDomain = "permission_domain"
    }
}

nonisolated public struct AgentCorePartArtifact: Decodable, Equatable, Sendable {
    public let uri: String
    public let name: String
    public let mimeType: String?
    public let operation: String?

    enum CodingKeys: String, CodingKey {
        case uri, name, operation
        case mimeType = "mime_type"
    }
}
