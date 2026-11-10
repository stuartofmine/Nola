// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed iOS wrapper for the Core-owned Agent prompt composer ABI.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated public enum AgentCorePromptLayer: String, Codable, Sendable {
    case identity
    case safety
    case toolPolicy = "tool_policy"
    case executionPolicy = "execution_policy"
    case memoryPolicy = "memory_policy"
    case hostCapabilities = "host_capabilities"
    case runtimeState = "runtime_state"
    case skillContext = "skill_context"
    case conversationContext = "conversation_context"
    case responseStyle = "response_style"
}

nonisolated public enum AgentCorePromptTrust: String, Codable, Sendable {
    case hostFact = "host_fact"
    case hostInstruction = "host_instruction"
    case userInstruction = "user_instruction"
    case reference
}

nonisolated public struct AgentCorePromptFragment: Codable, Equatable, Sendable {
    public let id: String
    public let layer: AgentCorePromptLayer
    public let trust: AgentCorePromptTrust
    public let cacheable: Bool
    public let content: String

    public init(id: String, layer: AgentCorePromptLayer, trust: AgentCorePromptTrust, cacheable: Bool, content: String) {
        self.id = id
        self.layer = layer
        self.trust = trust
        self.cacheable = cacheable
        self.content = content
    }
}

nonisolated public struct AgentCorePromptComposition: Codable, Equatable, Sendable {
    public let stablePrefix: String
    public let dynamicSuffix: String
    public let rendered: String
    public let fragmentIds: [String]
    public let stablePrefixSha256: String
    public let stableFragmentSha256: [String: String]?

    enum CodingKeys: String, CodingKey {
        case stablePrefix = "stable_prefix"
        case dynamicSuffix = "dynamic_suffix"
        case rendered
        case fragmentIds = "fragment_ids"
        case stablePrefixSha256 = "stable_prefix_sha256"
        case stableFragmentSha256 = "stable_fragment_sha256"
    }
}

nonisolated public enum AgentCorePromptComposerError: Error, Equatable {
    case libraryUnavailable
    case incompatibleABI(UInt32)
    case invalidResponse
    case rejected(code: String, message: String)
}

nonisolated public enum AgentCorePromptComposer {
    public static let schemaVersion = 1

    public static var abiVersion: UInt32 {
#if canImport(NolaCore)
        nola_core_prompt_composer_abi_version()
#else
        0
#endif
    }

    public static func compose(_ fragments: [AgentCorePromptFragment]) throws -> AgentCorePromptComposition {
#if canImport(NolaCore)
        let version = abiVersion
        guard version == schemaVersion else { throw AgentCorePromptComposerError.incompatibleABI(version) }
        let input = Request(schemaVersion: schemaVersion, fragments: fragments)
        let data = try JSONEncoder().encode(input)
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_compose_agent_prompt_json(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { throw AgentCorePromptComposerError.invalidResponse }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer),
              let response = try? JSONDecoder().decode(Response.self, from: Data(raw.utf8)),
              response.schemaVersion == schemaVersion else {
            throw AgentCorePromptComposerError.invalidResponse
        }
        if let error = response.error {
            throw AgentCorePromptComposerError.rejected(code: error.code, message: error.message)
        }
        guard let composition = response.composition else { throw AgentCorePromptComposerError.invalidResponse }
        return composition
#else
        throw AgentCorePromptComposerError.libraryUnavailable
#endif
    }

    private struct Request: Codable {
        let schemaVersion: Int
        let fragments: [AgentCorePromptFragment]
        enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case fragments
        }
    }

    private struct Response: Codable {
        let schemaVersion: Int
        let composition: AgentCorePromptComposition?
        let error: ResponseError?
        enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case composition, error
        }
    }

    private struct ResponseError: Codable {
        let code: String
        let message: String
    }
}
