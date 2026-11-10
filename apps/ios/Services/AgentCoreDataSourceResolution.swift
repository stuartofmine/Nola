// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed iOS boundary for Core's data-source resolution ABI.
// - Boundary: iOS owns protected-data access and I/O; Core owns deterministic resolution.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

public nonisolated struct AgentCoreDataSourceResolutionInput: Encodable, Sendable {
    public let schemaVersion: UInt32
    public let sourceID: String
    public let policy: String
    public let conversationOverride: Bool?
    public let contract: [String: AgentCoreDataSourceJSON]?
    public let now: String?

    public init(
        sourceID: String,
        policy: String,
        conversationOverride: Bool?,
        contract: [String: AgentCoreDataSourceJSON]?,
        now: String? = nil
    ) {
        self.schemaVersion = 1
        self.sourceID = sourceID
        self.policy = policy
        self.conversationOverride = conversationOverride
        self.contract = contract
        self.now = now
    }

    enum CodingKeys: String, CodingKey {
        case schemaVersion, policy, contract, now
        case sourceID = "sourceId"
        case conversationOverride
    }
}

public nonisolated indirect enum AgentCoreDataSourceJSON: Codable, Equatable, Sendable {
    case object([String: Self])
    case array([Self])
    case string(String)
    case number(Double)
    case bool(Bool)
    case null

    public init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let decoded = try? value.decode(Bool.self) { self = .bool(decoded) }
        else if let decoded = try? value.decode(Double.self) { self = .number(decoded) }
        else if let decoded = try? value.decode(String.self) { self = .string(decoded) }
        else if let decoded = try? value.decode([Self].self) { self = .array(decoded) }
        else { self = .object(try value.decode([String: Self].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .object(let decoded): try value.encode(decoded)
        case .array(let decoded): try value.encode(decoded)
        case .string(let decoded): try value.encode(decoded)
        case .number(let decoded): try value.encode(decoded)
        case .bool(let decoded): try value.encode(decoded)
        case .null: try value.encodeNil()
        }
    }
}

public nonisolated struct AgentCoreDataSourceResolution: Decodable, Equatable, Sendable {
    public let schemaVersion: UInt32
    public let enabled: Bool
    public let usable: Bool
    public let shouldRequestConsent: Bool
    public let shouldRefresh: Bool
    public let reason: String
}

public nonisolated enum AgentCoreDataSourceResolver {
    public static var isAvailable: Bool {
#if canImport(NolaCore)
        nola_core_data_source_abi_version() == 1
#else
        false
#endif
    }

    public static func resolve(
        _ input: AgentCoreDataSourceResolutionInput
    ) -> AgentCoreDataSourceResolution? {
#if canImport(NolaCore)
        guard isAvailable, let data = try? JSONEncoder().encode(input) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_data_source_resolve(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(AgentCoreDataSourceResolution.self, from: Data(raw.utf8))
#else
        return nil
#endif
    }

    public static func sealContract(
        _ contract: [String: AgentCoreDataSourceJSON]
    ) -> [String: AgentCoreDataSourceJSON]? {
#if canImport(NolaCore)
        guard isAvailable, let data = try? JSONEncoder().encode(contract) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_data_source_seal_contract(
                bytes.bindMemory(to: UInt8.self).baseAddress,
                bytes.count
            )
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(
            [String: AgentCoreDataSourceJSON].self,
            from: Data(raw.utf8)
        )
#else
        return nil
#endif
    }
}
