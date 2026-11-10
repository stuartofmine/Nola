// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed iOS boundary for Core's managed-resource visibility and editing contract.
// - Boundary: Resource access never grants Skill, MCP, data-source, or tool execution.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

public nonisolated enum AgentCoreResourceKind: String, Codable, Hashable, Sendable {
    case skill
    case mcpServer = "mcp_server"
    case dataSource = "data_source"
    case promptTemplate = "prompt_template"
}

public nonisolated enum AgentCoreResourcePermission: String, Codable, Hashable, Sendable {
    case readMetadata = "read_metadata"
    case readContent = "read_content"
    case editContent = "edit_content"
    case delete
    case manageSharing = "manage_sharing"
}

nonisolated enum AgentCoreResourceStrictDecoding {
    private struct Field: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }
    static func fields(_ decoder: Decoder, allowed: Set<String>) throws -> Set<String> {
        let fields = Set(try decoder.container(keyedBy: Field.self).allKeys.map(\.stringValue))
        guard fields.isSubset(of: allowed) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Unexpected fields in the current resource schema"))
        }
        return fields
    }
    static func optional<T: Encodable, Key: CodingKey>(_ value: T?, _ key: Key,
        into container: inout KeyedEncodingContainer<Key>, present: Set<String>) throws {
        if let value { try container.encode(value, forKey: key) }
        else if present.contains(key.stringValue) { try container.encodeNil(forKey: key) }
    }
}

public nonisolated struct AgentCoreResourcePrincipal: Codable, Hashable, Sendable {
    public let kind: String
    public let id: String

    public init(kind: String, id: String) {
        self.kind = kind
        self.id = id
    }
    private enum CodingKeys: String, CodingKey { case kind, id }
    public init(from decoder: Decoder) throws {
        _ = try AgentCoreResourceStrictDecoding.fields(decoder, allowed: ["kind", "id"])
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try values.decode(String.self, forKey: .kind)
        guard ["local_user", "account", "device", "group", "anyone"].contains(kind) else {
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unknown current resource principal kind")
        }
        self.init(kind: kind, id: try values.decode(String.self, forKey: .id))
    }

}

public nonisolated struct AgentCoreResourceGrant: Codable, Hashable, Sendable {
    public let grantID: String
    public let resourceKind: AgentCoreResourceKind
    public let resourceID: String
    public let principal: AgentCoreResourcePrincipal
    public let permissions: Set<AgentCoreResourcePermission>
    public let hostID: String?
    public let resourceRevision: UInt64?
    public let contentDigest: String?
    public let expiresAtEpochMilliseconds: Int64?
    private var presentOptionalFields = Set<String>()

    public init(
        grantID: String,
        resourceKind: AgentCoreResourceKind,
        resourceID: String,
        principal: AgentCoreResourcePrincipal,
        permissions: Set<AgentCoreResourcePermission>,
        hostID: String? = nil,
        resourceRevision: UInt64? = nil,
        contentDigest: String? = nil,
        expiresAtEpochMilliseconds: Int64? = nil
    ) {
        self.grantID = grantID
        self.resourceKind = resourceKind
        self.resourceID = resourceID
        self.principal = principal
        self.permissions = permissions
        self.hostID = hostID
        self.resourceRevision = resourceRevision
        self.contentDigest = contentDigest
        self.expiresAtEpochMilliseconds = expiresAtEpochMilliseconds
    }

    enum CodingKeys: String, CodingKey, CaseIterable {
        case principal, permissions
        case grantID = "grant_id"
        case resourceKind = "resource_kind"
        case resourceID = "resource_id"
        case hostID = "host_id"
        case resourceRevision = "resource_revision"
        case contentDigest = "content_digest"
        case expiresAtEpochMilliseconds = "expires_at_epoch_ms"
    }
    public init(from decoder: Decoder) throws {
        let present = try AgentCoreResourceStrictDecoding.fields(decoder, allowed: Set(CodingKeys.allCases.map(\.rawValue)))
        let values = try decoder.container(keyedBy: CodingKeys.self)
        self.init(grantID: try values.decode(String.self, forKey: .grantID),
            resourceKind: try values.decode(AgentCoreResourceKind.self, forKey: .resourceKind),
            resourceID: try values.decode(String.self, forKey: .resourceID),
            principal: try values.decode(AgentCoreResourcePrincipal.self, forKey: .principal),
            permissions: try values.decode(Set<AgentCoreResourcePermission>.self, forKey: .permissions),
            hostID: try values.decodeIfPresent(String.self, forKey: .hostID),
            resourceRevision: try values.decodeIfPresent(UInt64.self, forKey: .resourceRevision),
            contentDigest: try values.decodeIfPresent(String.self, forKey: .contentDigest),
            expiresAtEpochMilliseconds: try values.decodeIfPresent(Int64.self, forKey: .expiresAtEpochMilliseconds))
        presentOptionalFields = present.intersection(["host_id", "resource_revision", "content_digest", "expires_at_epoch_ms"])
    }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(grantID, forKey: .grantID); try values.encode(resourceKind, forKey: .resourceKind)
        try values.encode(resourceID, forKey: .resourceID); try values.encode(principal, forKey: .principal)
        try values.encode(permissions, forKey: .permissions)
        try AgentCoreResourceStrictDecoding.optional(hostID, .hostID, into: &values, present: presentOptionalFields)
        try AgentCoreResourceStrictDecoding.optional(resourceRevision, .resourceRevision, into: &values, present: presentOptionalFields)
        try AgentCoreResourceStrictDecoding.optional(contentDigest, .contentDigest, into: &values, present: presentOptionalFields)
        try AgentCoreResourceStrictDecoding.optional(expiresAtEpochMilliseconds, .expiresAtEpochMilliseconds, into: &values, present: presentOptionalFields)
    }
    public static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.grantID == rhs.grantID && lhs.resourceKind == rhs.resourceKind && lhs.resourceID == rhs.resourceID &&
        lhs.principal == rhs.principal && lhs.permissions == rhs.permissions && lhs.hostID == rhs.hostID &&
        lhs.resourceRevision == rhs.resourceRevision && lhs.contentDigest == rhs.contentDigest &&
        lhs.expiresAtEpochMilliseconds == rhs.expiresAtEpochMilliseconds
    }
    public func hash(into hasher: inout Hasher) {
        hasher.combine(grantID); hasher.combine(resourceKind); hasher.combine(resourceID); hasher.combine(principal)
        hasher.combine(permissions); hasher.combine(hostID); hasher.combine(resourceRevision); hasher.combine(contentDigest)
        hasher.combine(expiresAtEpochMilliseconds)
    }

}

public nonisolated struct AgentCoreManagedResourceIdentity: Encodable, Sendable {
    public let kind: AgentCoreResourceKind
    public let resourceID: String
    public let owner: AgentCoreResourcePrincipal
    public let hostID: String?
    public let revision: UInt64
    public let contentDigest: String?

    public init(
        kind: AgentCoreResourceKind,
        resourceID: String,
        owner: AgentCoreResourcePrincipal,
        hostID: String? = nil,
        revision: UInt64,
        contentDigest: String? = nil
    ) {
        self.kind = kind
        self.resourceID = resourceID
        self.owner = owner
        self.hostID = hostID
        self.revision = revision
        self.contentDigest = contentDigest
    }

    enum CodingKeys: String, CodingKey {
        case kind, owner, revision
        case resourceID = "resource_id"
        case hostID = "host_id"
        case contentDigest = "content_digest"
    }
}

private nonisolated struct AgentCoreResourceAccessRequest: Encodable, Sendable {
    let schemaVersion: UInt32
    let resource: AgentCoreManagedResourceIdentity
    let actor: AgentCoreResourcePrincipal
    let permission: AgentCoreResourcePermission
    let grants: [AgentCoreResourceGrant]
    let nowEpochMilliseconds: Int64
    let requestHostID: String?

    enum CodingKeys: String, CodingKey {
        case resource, actor, permission, grants
        case schemaVersion = "schema_version"
        case nowEpochMilliseconds = "now_epoch_ms"
        case requestHostID = "request_host_id"
    }
}

public nonisolated struct AgentCoreResourceAccessDecision: Decodable, Equatable, Sendable {
    public let allowed: Bool
    public let reason: String
    public let toolExecutionAuthorized: Bool

    enum CodingKeys: String, CodingKey {
        case allowed, reason
        case toolExecutionAuthorized = "tool_execution_authorized"
    }
}

private nonisolated struct AgentCoreResourceAccessResponse: Decodable, Sendable {
    let schemaVersion: UInt32
    let decision: AgentCoreResourceAccessDecision?

    enum CodingKeys: String, CodingKey {
        case decision
        case schemaVersion = "schema_version"
    }
}

public nonisolated enum AgentCoreResourceAccess {
    private static let localPrincipalDefaultsKey = "ios.resourceAccess.localPrincipalID"

    public static var isAvailable: Bool {
#if canImport(NolaCore)
        nola_core_resource_access_abi_version() == 1
#else
        false
#endif
    }

    public static func localPrincipal(defaults: UserDefaults = .standard) -> AgentCoreResourcePrincipal {
        if let stored = defaults.string(forKey: localPrincipalDefaultsKey), !stored.isEmpty {
            return AgentCoreResourcePrincipal(kind: "local_user", id: stored)
        }
        let generated = UUID().uuidString.lowercased()
        defaults.set(generated, forKey: localPrincipalDefaultsKey)
        return AgentCoreResourcePrincipal(kind: "local_user", id: generated)
    }

    public static func evaluate(
        resource: AgentCoreManagedResourceIdentity,
        permission: AgentCoreResourcePermission,
        grants: [AgentCoreResourceGrant],
        actor: AgentCoreResourcePrincipal? = nil,
        requestHostID: String? = nil,
        defaults: UserDefaults = .standard
    ) -> AgentCoreResourceAccessDecision? {
#if canImport(NolaCore)
        guard isAvailable else { return nil }
        let request = AgentCoreResourceAccessRequest(
            schemaVersion: 1,
            resource: resource,
            actor: actor ?? localPrincipal(defaults: defaults),
            permission: permission,
            grants: grants,
            nowEpochMilliseconds: Int64(Date().timeIntervalSince1970 * 1_000),
            requestHostID: requestHostID
        )
        guard let data = try? JSONEncoder().encode(request) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_resource_access_evaluate_json(
                bytes.bindMemory(to: UInt8.self).baseAddress,
                bytes.count
            )
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer),
              let response = try? JSONDecoder().decode(
                AgentCoreResourceAccessResponse.self,
                from: Data(raw.utf8)
              ),
              response.schemaVersion == 1,
              let decision = response.decision,
              !decision.toolExecutionAuthorized else {
            return nil
        }
        return decision
#else
        return nil
#endif
    }
}
