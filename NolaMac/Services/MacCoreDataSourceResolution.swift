// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed macOS boundary for Core's data-source resolution ABI.
// - Boundary: macOS owns I/O and persistence; Core owns policy, contract, and freshness decisions.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated struct MacCoreDataSourceResolutionInput: Encodable, Sendable {
    let schemaVersion: UInt32
    let sourceID: String
    let policy: String
    let conversationOverride: Bool?
    let contract: [String: MacCoreDataSourceJSON]?
    let now: String?

    init(
        sourceID: String,
        policy: String,
        conversationOverride: Bool?,
        contract: [String: MacCoreDataSourceJSON]?,
        now: String? = nil
    ) {
        self.schemaVersion = 2
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

nonisolated indirect enum MacCoreDataSourceJSON: Codable, Equatable, Sendable {
    case object([String: Self])
    case array([Self])
    case string(String)
    case number(Double)
    case bool(Bool)
    case null

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let decoded = try? value.decode(Bool.self) { self = .bool(decoded) }
        else if let decoded = try? value.decode(Double.self) { self = .number(decoded) }
        else if let decoded = try? value.decode(String.self) { self = .string(decoded) }
        else if let decoded = try? value.decode([Self].self) { self = .array(decoded) }
        else { self = .object(try value.decode([String: Self].self)) }
    }

    func encode(to encoder: Encoder) throws {
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

nonisolated struct MacCoreDataSourceResolution: Decodable, Equatable, Sendable {
    let schemaVersion: UInt32
    let enabled: Bool
    let usable: Bool
    let shouldRequestConsent: Bool
    let shouldRefresh: Bool
    let reason: String
}

nonisolated enum MacCoreDataSourceResolver {
    static var isAvailable: Bool {
#if canImport(NolaCore)
        nola_core_data_source_abi_version() == 3
#else
        false
#endif
    }

    static func resolve(_ input: MacCoreDataSourceResolutionInput) -> MacCoreDataSourceResolution? {
#if canImport(NolaCore)
        guard isAvailable, let data = try? JSONEncoder().encode(input) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_data_source_resolve(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(MacCoreDataSourceResolution.self, from: Data(raw.utf8))
#else
        return nil
#endif
    }

    static func sealContract(
        _ contract: [String: MacCoreDataSourceJSON]
    ) -> [String: MacCoreDataSourceJSON]? {
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
            [String: MacCoreDataSourceJSON].self,
            from: Data(raw.utf8)
        )
#else
        return nil
#endif
    }
}
