// SPDX-License-Identifier: Apache-2.0
// Authenticated execution Host catalog reference, frozen before request hashing.
import Foundation

private nonisolated struct IOSHostToolPlanKey: CodingKey {
    let stringValue: String
    let intValue: Int? = nil
    init(_ value: String) { stringValue = value }
    init?(stringValue: String) { self.init(stringValue) }
    init?(intValue: Int) { return nil }
}

private nonisolated func iosHostToolPlanFields(_ decoder: Decoder, _ fields: Set<String>) throws -> KeyedDecodingContainer<IOSHostToolPlanKey> {
    let container = try decoder.container(keyedBy: IOSHostToolPlanKey.self)
    guard Set(container.allKeys.map(\.stringValue)) == fields else {
        throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Unexpected Host tool plan fields"))
    }
    return container
}

nonisolated struct IOSRemoteHostToolPlan: Decodable, Sendable {
    struct Reference: Codable, Sendable {
        let manifest_id: String
        let host_id: String
        let client_id: String
        let session_id: String
        let catalog_revision: String

        init(from decoder: Decoder) throws {
            let fields = try iosHostToolPlanFields(decoder, ["manifest_id", "host_id", "client_id", "session_id", "catalog_revision"])
            manifest_id = try fields.decode(String.self, forKey: .init("manifest_id"))
            host_id = try fields.decode(String.self, forKey: .init("host_id"))
            client_id = try fields.decode(String.self, forKey: .init("client_id"))
            session_id = try fields.decode(String.self, forKey: .init("session_id"))
            catalog_revision = try fields.decode(String.self, forKey: .init("catalog_revision"))
        }
    }
    struct Tool: Decodable, Sendable {
        let tool_id: String
        let route: String
        let schema: AgentCoreJSONValue

        init(from decoder: Decoder) throws {
            let fields = try iosHostToolPlanFields(decoder, ["tool_id", "route", "schema"])
            tool_id = try fields.decode(String.self, forKey: .init("tool_id"))
            route = try fields.decode(String.self, forKey: .init("route"))
            schema = try fields.decode(AgentCoreJSONValue.self, forKey: .init("schema"))
        }
    }
    let schema_version: Int
    let reference: Reference
    let issued_at_ms: Int64
    let expires_at_ms: Int64
    let tools: [Tool]

    init(from decoder: Decoder) throws {
        let fields = try iosHostToolPlanFields(decoder, ["schema_version", "reference", "issued_at_ms", "expires_at_ms", "tools"])
        schema_version = try fields.decode(Int.self, forKey: .init("schema_version"))
        reference = try fields.decode(Reference.self, forKey: .init("reference"))
        issued_at_ms = try fields.decode(Int64.self, forKey: .init("issued_at_ms"))
        expires_at_ms = try fields.decode(Int64.self, forKey: .init("expires_at_ms"))
        tools = try fields.decode([Tool].self, forKey: .init("tools"))
    }

    // The authenticated execution Host evaluates freshness on its own clock.
    // Clients validate the same structural bounds as Core validate(None).
    func validatedReference(hostID: String, clientID: String, sessionID: String) throws -> [String: Any] {
        let validIdentity: (String) -> Bool = {
            !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && $0.utf8.count <= 512
                && !$0.unicodeScalars.contains(where: { $0.value <= 0x1f || (0x7f...0x9f).contains($0.value) })
        }
        guard [hostID, clientID, sessionID, reference.manifest_id].allSatisfy(validIdentity),
              schema_version == 1, reference.host_id == hostID, reference.client_id == clientID,
              reference.session_id == sessionID,
              reference.catalog_revision.utf8.count == 64,
              reference.catalog_revision.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              issued_at_ms > 0, expires_at_ms > issued_at_ms,
              expires_at_ms <= 9_007_199_254_740_991, expires_at_ms - issued_at_ms <= 300_000,
              Set(tools.map(\.tool_id)).count == tools.count else {
            throw IOSApplicationRuntimeError.rejected("Invalid execution Host tool plan")
        }
        for tool in tools {
            let data = try JSONEncoder().encode(tool.schema)
            guard validIdentity(tool.tool_id), tool.route == "agent_host",
                  let schema = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  schema["type"] as? String == "function", let function = schema["function"] as? [String: Any],
                  function["name"] as? String == tool.tool_id, function["parameters"] is [String: Any] else {
                throw IOSApplicationRuntimeError.rejected("Invalid execution Host tool schema")
            }
        }
        return ["manifest_id": reference.manifest_id, "host_id": reference.host_id,
                "client_id": reference.client_id, "session_id": reference.session_id,
                "catalog_revision": reference.catalog_revision]
    }
}
