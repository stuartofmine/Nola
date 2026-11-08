// SPDX-License-Identifier: Apache-2.0
// Core-authored conversation execution ownership, shared by Apple renderers.
import Foundation

nonisolated public enum AppleConversationExecutionOwnership: Decodable, Equatable, Sendable {
    case user
    case taskChild(AppleTaskChildDestination)
    case unavailable

    private enum CodingKeys: String, CodingKey {
        case kind
        case parentConversationID = "parent_conversation_id"
        case parentRunID = "parent_run_id"
        case taskID = "task_id"
        case childConversationID = "child_conversation_id"
    }

    private struct Field: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }

    public init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: Field.self)
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try values.decode(String.self, forKey: .kind)
        let expected: Set<String> = kind == "task_child"
            ? ["kind", "parent_conversation_id", "parent_run_id", "task_id", "child_conversation_id"] : ["kind"]
        guard Set(fields.allKeys.map(\.stringValue)) == expected else {
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values,
                debugDescription: "Unexpected conversation ownership fields")
        }
        switch kind {
        case "user": self = .user
        case "unavailable": self = .unavailable
        case "task_child":
            let destination = AppleTaskChildDestination(
                parentConversationID: try values.decode(String.self, forKey: .parentConversationID),
                parentRunID: try values.decode(String.self, forKey: .parentRunID),
                taskID: try values.decode(String.self, forKey: .taskID),
                childConversationID: try values.decode(String.self, forKey: .childConversationID))
            guard destination.hasValidIdentity else {
                throw DecodingError.dataCorruptedError(forKey: .kind, in: values,
                    debugDescription: "Invalid task-child conversation ownership")
            }
            self = .taskChild(destination)
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values,
                debugDescription: "Unsupported conversation execution ownership")
        }
    }
}

/// A navigation request, never permission to run a child. Core validates the
/// parent/Run/task relation; renderers match the returned ownership exactly.
nonisolated public struct AppleTaskChildDestination: Equatable, Hashable, Identifiable, Sendable {
    public let parentConversationID: String
    public let parentRunID: String
    public let taskID: String
    public let childConversationID: String
    public var id: Self { self }

    public init(parentConversationID: String, parentRunID: String, taskID: String,
                childConversationID: String) {
        self.parentConversationID = parentConversationID
        self.parentRunID = parentRunID
        self.taskID = taskID
        self.childConversationID = childConversationID
    }

    public var hasValidIdentity: Bool {
        [parentConversationID, parentRunID, taskID, childConversationID].allSatisfy {
            !$0.isEmpty && $0.count <= 512 && $0.trimmingCharacters(in: .whitespacesAndNewlines) == $0
                && $0.rangeOfCharacter(from: .controlCharacters) == nil
        } && parentConversationID != childConversationID
    }
}

/// Bounded exact-child query shared by local Core and paired Host transports.
nonisolated struct AppleTaskChildWindowRequest: Encodable, Sendable {
    let destination: AppleTaskChildDestination
    let beforeOrdinal: Int64?
    let limit: Int
    let contentByteLimit: Int

    private enum CodingKeys: String, CodingKey {
        case parentConversationID = "parent_conversation_id", parentRunID = "parent_run_id"
        case taskID = "task_id", childConversationID = "child_conversation_id"
        case beforeOrdinal = "before_ordinal", limit, contentByteLimit = "content_byte_limit"
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(destination.parentConversationID, forKey: .parentConversationID)
        try values.encode(destination.parentRunID, forKey: .parentRunID)
        try values.encode(destination.taskID, forKey: .taskID)
        try values.encode(destination.childConversationID, forKey: .childConversationID)
        try values.encode(beforeOrdinal, forKey: .beforeOrdinal)
        try values.encode(limit, forKey: .limit)
        try values.encode(contentByteLimit, forKey: .contentByteLimit)
    }
}

/// Durable source provenance, independent of whether a Run is still active.
nonisolated public enum AppleConversationSourceOwner: Decodable, Equatable, Sendable {
    case local
    case remote(hostID: String, storeEpoch: String)
    case unavailable

    private enum CodingKeys: String, CodingKey { case kind, source_host_id, source_store_epoch }
    private struct Field: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }

    public init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: Field.self)
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try values.decode(String.self, forKey: .kind)
        let expected: Set<String> = kind == "remote" ? ["kind", "source_host_id", "source_store_epoch"] : ["kind"]
        guard Set(fields.allKeys.map(\.stringValue)) == expected else {
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unexpected source ownership fields")
        }
        switch kind {
        case "local": self = .local
        case "unavailable": self = .unavailable
        case "remote":
            let host = try values.decode(String.self, forKey: .source_host_id)
            let epoch = try values.decode(String.self, forKey: .source_store_epoch)
            guard !host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  !epoch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Incomplete source ownership")
            }
            self = .remote(hostID: host, storeEpoch: epoch)
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unsupported source ownership")
        }
    }
}
