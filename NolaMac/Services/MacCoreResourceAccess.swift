// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed macOS boundary for Core's managed-resource access ABI.
// - Boundary: This contract governs metadata/content visibility and editing only; it never authorizes execution.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated enum MacCoreResourceKind: String, Encodable, Sendable {
    case skill
    case mcpServer = "mcp_server"
    case dataSource = "data_source"
}

nonisolated enum MacCoreResourcePermission: String, Encodable, Sendable {
    case readMetadata = "read_metadata"
    case readContent = "read_content"
    case editContent = "edit_content"
    case delete
    case manageSharing = "manage_sharing"
}

private nonisolated struct MacCoreResourcePrincipal: Encodable, Sendable {
    let kind: String
    let id: String
}

private nonisolated struct MacCoreResourceIdentity: Encodable, Sendable {
    let kind: MacCoreResourceKind
    let resourceID: String
    let owner: MacCoreResourcePrincipal
    let hostID: String?
    let revision: UInt64
    let contentDigest: String?

    enum CodingKeys: String, CodingKey {
        case kind, owner, revision
        case resourceID = "resource_id"
        case hostID = "host_id"
        case contentDigest = "content_digest"
    }
}

private nonisolated struct MacCoreResourceAccessRequest: Encodable, Sendable {
    let schemaVersion: UInt32
    let resource: MacCoreResourceIdentity
    let actor: MacCoreResourcePrincipal
    let permission: MacCoreResourcePermission
    let grants: [MacCoreResourceAccessGrant]
    let nowEpochMilliseconds: Int64
    let requestHostID: String?

    enum CodingKeys: String, CodingKey {
        case resource, actor, permission, grants
        case schemaVersion = "schema_version"
        case nowEpochMilliseconds = "now_epoch_ms"
        case requestHostID = "request_host_id"
    }
}

private nonisolated struct MacCoreResourceAccessGrant: Encodable, Sendable {}

nonisolated struct MacCoreResourceAccessDecision: Decodable, Equatable, Sendable {
    let allowed: Bool
    let reason: String
    let toolExecutionAuthorized: Bool

    enum CodingKeys: String, CodingKey {
        case allowed, reason
        case toolExecutionAuthorized = "tool_execution_authorized"
    }
}

private nonisolated struct MacCoreResourceAccessResponse: Decodable, Sendable {
    let schemaVersion: UInt32
    let decision: MacCoreResourceAccessDecision?

    enum CodingKeys: String, CodingKey {
        case decision
        case schemaVersion = "schema_version"
    }
}

nonisolated enum MacCoreResourceAccess {
    private static let principalDefaultsKey = "mac.resourceAccess.localPrincipalID"

    static var isAvailable: Bool {
#if canImport(NolaCore)
        nola_core_resource_access_abi_version() == 1
#else
        false
#endif
    }

    static func evaluate(
        item: MacExtensionItem,
        kind: MacCoreResourceKind,
        permission: MacCoreResourcePermission,
        defaults: UserDefaults = .standard
    ) -> MacCoreResourceAccessDecision? {
#if canImport(NolaCore)
        return evaluate(
            resourceID: item.id,
            kind: kind,
            ownerID: nil,
            revision: 1,
            permission: permission,
            defaults: defaults
        )
#else
        return nil
#endif
    }

    static func evaluate(
        resourceID: String,
        kind: MacCoreResourceKind,
        ownerID: String?,
        revision: UInt64,
        permission: MacCoreResourcePermission,
        defaults: UserDefaults = .standard
    ) -> MacCoreResourceAccessDecision? {
#if canImport(NolaCore)
        guard isAvailable else { return nil }
        let localID = localPrincipalID(defaults: defaults)
        let principal = MacCoreResourcePrincipal(kind: "local_user", id: localID)
        let request = MacCoreResourceAccessRequest(
            schemaVersion: 1,
            resource: MacCoreResourceIdentity(
                kind: kind,
                resourceID: resourceID,
                owner: MacCoreResourcePrincipal(kind: "local_user", id: ownerID ?? localID),
                hostID: nil,
                revision: max(1, revision),
                contentDigest: contentDigest(from: resourceID)
            ),
            actor: principal,
            permission: permission,
            grants: [],
            nowEpochMilliseconds: Int64(Date().timeIntervalSince1970 * 1_000),
            requestHostID: nil
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
                MacCoreResourceAccessResponse.self,
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

    static func localPrincipal(defaults: UserDefaults = .standard) -> String {
        localPrincipalID(defaults: defaults)
    }

    private static func localPrincipalID(defaults: UserDefaults) -> String {
        if let stored = defaults.string(forKey: principalDefaultsKey), !stored.isEmpty {
            return stored
        }
        let generated = UUID().uuidString.lowercased()
        defaults.set(generated, forKey: principalDefaultsKey)
        return generated
    }

    private static func contentDigest(from itemID: String) -> String? {
        guard let marker = itemID.range(of: "#sha256=", options: .backwards) else { return nil }
        let digest = String(itemID[marker.upperBound...]).lowercased()
        guard digest.count == 64, digest.allSatisfy(\.isHexDigit) else { return nil }
        return "sha256:\(digest)"
    }
}
