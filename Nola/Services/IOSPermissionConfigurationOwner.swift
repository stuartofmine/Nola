// SPDX-License-Identifier: Apache-2.0
// Permission ownership is canonical Run/admission data, never UI activity.
import Foundation

nonisolated enum IOSPermissionChangeFence {
    static func matches(owner: UUID?, changeID: UUID, selectedOwner: UUID?, currentChangeID: UUID?) -> Bool {
        owner == selectedOwner && changeID == currentChangeID
    }
}

nonisolated struct IOSPermissionHydrationState {
    private var completed: IOSRemoteSubmissionIdentity?
    private var inFlight: IOSRemoteSubmissionIdentity?
    private var requestID: UUID?

    mutating func begin(_ identity: IOSRemoteSubmissionIdentity) -> UUID? {
        guard completed != identity, inFlight != identity else { return nil }
        let token = UUID()
        inFlight = identity
        requestID = token
        return token
    }

    @discardableResult
    mutating func finish(_ identity: IOSRemoteSubmissionIdentity, request: UUID, succeeded: Bool) -> Bool {
        guard inFlight == identity, requestID == request else { return false }
        inFlight = nil
        requestID = nil
        if succeeded { completed = identity }
        return true
    }

    mutating func invalidate(forgetSuccess: Bool = false) {
        inFlight = nil
        requestID = nil
        if forgetSuccess { completed = nil }
    }

    mutating func confirm(_ identity: IOSRemoteSubmissionIdentity) {
        invalidate()
        completed = identity
    }
}

nonisolated enum IOSPermissionConfigurationOwner: Equatable, Sendable {
    case unknown
    case inactive
    case local(runID: String)
    case remote(IOSRemoteSubmissionIdentity)

    var runID: String? {
        switch self {
        case .unknown, .inactive: nil
        case .local(let runID): runID
        case .remote(let identity): identity.submission_id
        }
    }

    static func read(conversationID: String, hostID: String?, clientID: String?,
                     unresolvedRunID: String?, sourceUnconfirmed: Bool,
                     execute: (AgentCoreJSONValue) throws -> AgentCoreJSONValue) throws -> Self {
        let response = try execute(.object([
            "type": .string("read_permission_configuration"),
            "conversation_id": .string(conversationID),
            "source_host_id": hostID.map(AgentCoreJSONValue.string) ?? .null,
            "client_id": clientID.map(AgentCoreJSONValue.string) ?? .null,
            "unresolved_run_ids": .array(unresolvedRunID.map { [.string($0)] } ?? []),
            "source_unconfirmed": .bool(sourceUnconfirmed),
        ]))
        guard let value = response.permissionOwnerObject,
              value.permissionOwnerString("conversation_id") == conversationID,
              let owner = value["owner"]?.permissionOwnerObject else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        switch owner.permissionOwnerString("kind") {
        case "inactive": return .inactive
        case "local":
            guard let runID = owner.permissionOwnerString("run_id"), !runID.isEmpty else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            return .local(runID: runID)
        case "remote":
            guard let identity = owner["identity"] else { throw IOSApplicationRuntimeError.invalidResponse }
            return .remote(try JSONDecoder().decode(IOSRemoteSubmissionIdentity.self,
                from: JSONEncoder().encode(identity)))
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
    }
}

private extension AgentCoreJSONValue {
    nonisolated var permissionOwnerObject: [String: AgentCoreJSONValue]? {
        guard case .object(let value) = self else { return nil }; return value
    }
}
private extension Dictionary where Key == String, Value == AgentCoreJSONValue {
    nonisolated func permissionOwnerString(_ key: String) -> String? {
        guard case .string(let value)? = self[key] else { return nil }; return value
    }
}
