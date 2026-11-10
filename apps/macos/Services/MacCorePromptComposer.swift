// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides the sole macOS Swift boundary to Core prompt-composer ABI v1.

import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated enum MacCorePromptLayer: String, Codable, Sendable {
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

nonisolated enum MacCorePromptTrust: String, Codable, Sendable {
    case hostFact = "host_fact"
    case hostInstruction = "host_instruction"
    case userInstruction = "user_instruction"
    case reference
}

nonisolated struct MacCorePromptFragment: Codable, Equatable, Sendable {
    let id: String
    let layer: MacCorePromptLayer
    let trust: MacCorePromptTrust
    let cacheable: Bool
    let content: String
}

nonisolated struct MacCorePromptComposition: Codable, Equatable, Sendable {
    let stablePrefix: String
    let dynamicSuffix: String
    let rendered: String
    let fragmentIDs: [String]
    let stablePrefixSHA256: String
    let stableFragmentSHA256: [String: String]

    private enum CodingKeys: String, CodingKey {
        case stablePrefix = "stable_prefix"
        case dynamicSuffix = "dynamic_suffix"
        case rendered
        case fragmentIDs = "fragment_ids"
        case stablePrefixSHA256 = "stable_prefix_sha256"
        case stableFragmentSHA256 = "stable_fragment_sha256"
    }
}

nonisolated enum MacCorePromptComposerError: Error, Equatable {
    case libraryUnavailable
    case incompatibleABI(UInt32)
    case invalidResponse
    case rejected(code: String, message: String)
}

nonisolated enum MacCorePromptComposer {
    static let schemaVersion = 1

    static var abiVersion: UInt32 {
        #if canImport(NolaCore)
        nola_core_prompt_composer_abi_version()
        #else
        0
        #endif
    }

    static func compose(_ fragments: [MacCorePromptFragment]) throws -> MacCorePromptComposition {
        #if canImport(NolaCore)
        let version = abiVersion
        guard version == schemaVersion else {
            throw MacCorePromptComposerError.incompatibleABI(version)
        }
        let data = try JSONEncoder().encode(Request(schemaVersion: schemaVersion, fragments: fragments))
        let pointer = data.withUnsafeBytes { bytes in
            nola_core_compose_agent_prompt_json(
                bytes.bindMemory(to: UInt8.self).baseAddress,
                bytes.count
            )
        }
        guard let pointer else { throw MacCorePromptComposerError.invalidResponse }
        defer { nola_core_free_c_string(pointer) }
        guard let raw = String(validatingUTF8: pointer),
              let response = try? JSONDecoder().decode(Response.self, from: Data(raw.utf8)),
              response.schemaVersion == schemaVersion else {
            throw MacCorePromptComposerError.invalidResponse
        }
        if let error = response.error {
            throw MacCorePromptComposerError.rejected(code: error.code, message: error.message)
        }
        guard let composition = response.composition else {
            throw MacCorePromptComposerError.invalidResponse
        }
        return composition
        #else
        throw MacCorePromptComposerError.libraryUnavailable
        #endif
    }

    private struct Request: Codable {
        let schemaVersion: Int
        let fragments: [MacCorePromptFragment]
        private enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case fragments
        }
    }

    private struct Response: Codable {
        let schemaVersion: Int
        let composition: MacCorePromptComposition?
        let error: ResponseError?
        private enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case composition, error
        }
    }

    private struct ResponseError: Codable {
        let code: String
        let message: String
    }
}

nonisolated struct MacCanonicalModelToolCall: Codable, Equatable, Sendable {
    struct Function: Codable, Equatable, Sendable {
        let name: String
        let arguments: String
    }

    let id: String
    let type: String
    let function: Function
}

nonisolated struct MacCanonicalModelMessage: Codable, Equatable, Sendable {
    let role: String
    let content: String
    let name: String?
    let toolCallID: String?
    let toolCalls: [MacCanonicalModelToolCall]
    let reasoningContent: String?

    init(
        role: String,
        content: String,
        name: String? = nil,
        toolCallID: String? = nil,
        toolCalls: [MacCanonicalModelToolCall] = [],
        reasoningContent: String? = nil
    ) {
        self.role = role
        self.content = content
        self.name = name
        self.toolCallID = toolCallID
        self.toolCalls = toolCalls
        self.reasoningContent = reasoningContent
    }

    private enum CodingKeys: String, CodingKey {
        case role, content, name
        case toolCallID = "tool_call_id"
        case toolCalls = "tool_calls"
        case reasoningContent = "reasoning_content"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        role = try container.decode(String.self, forKey: .role)
        content = try container.decodeIfPresent(String.self, forKey: .content) ?? ""
        name = try container.decodeIfPresent(String.self, forKey: .name)
        toolCallID = try container.decodeIfPresent(String.self, forKey: .toolCallID)
        toolCalls = try container.decodeIfPresent([MacCanonicalModelToolCall].self, forKey: .toolCalls) ?? []
        reasoningContent = try container.decodeIfPresent(String.self, forKey: .reasoningContent)
    }

    var localInferenceContent: String {
        guard role == "assistant", !toolCalls.isEmpty else { return content }
        let renderedCalls = toolCalls.map { call in
            let arguments: Any = call.function.arguments.data(using: .utf8)
                .flatMap { try? JSONSerialization.jsonObject(with: $0) }
                ?? call.function.arguments
            let payload: [String: Any] = [
                "name": call.function.name,
                "arguments": arguments,
            ]
            let data = try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
            let json = data.flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
            return "<tool_call>\n\(json)\n</tool_call>"
        }
        return ([content] + renderedCalls)
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }
}

nonisolated struct MacCanonicalModelRequest: Equatable, Sendable {
    let systemPrompt: String
    let messages: [MacCanonicalModelMessage]
    let stablePrefixSHA256: String
    let fragmentIDs: [String]
    let stableFragmentSHA256: [String: String]
}

/// Builds the same typed prompt boundary for API and local model transports.
/// User input is a model message and can never become a prompt fragment.
nonisolated enum MacCanonicalModelRequestBuilder {
    static func compose(
        fragments: [MacCorePromptFragment],
        history: [MacCanonicalModelMessage] = [],
        userQuery: String
    ) throws -> MacCanonicalModelRequest {
        guard history.allSatisfy({ $0.role != "system" }) else {
            throw MacCanonicalModelRequestBuilderError.systemMessageInConversation
        }
        let composition = try MacCorePromptComposer.compose(fragments)
        return MacCanonicalModelRequest(
            systemPrompt: composition.rendered,
            messages: history + [.init(role: "user", content: userQuery)],
            stablePrefixSHA256: composition.stablePrefixSHA256,
            fragmentIDs: composition.fragmentIDs,
            stableFragmentSHA256: composition.stableFragmentSHA256
        )
    }

    /// Agent Host already owns composition. This path preserves its typed
    /// role boundary and refuses to invent or append another system prompt.
    static func providerMessages(
        _ messages: [MacCanonicalModelMessage]
    ) throws -> [MacCanonicalModelMessage] {
        guard !messages.isEmpty,
              messages.contains(where: { $0.role != "system" }) else {
            throw MacCanonicalModelRequestBuilderError.missingConversationMessage
        }
        let supported = Set(["system", "user", "assistant", "tool"])
        guard messages.allSatisfy({ supported.contains($0.role) }) else {
            throw MacCanonicalModelRequestBuilderError.invalidRole
        }
        let systemMessageIndices = messages.indices.filter { messages[$0].role == "system" }
        guard systemMessageIndices.count <= 1,
              systemMessageIndices.first.map({ $0 == messages.startIndex }) ?? true else {
            throw MacCanonicalModelRequestBuilderError.invalidSystemMessagePlacement
        }
        return messages
    }
}

nonisolated enum MacCanonicalModelRequestBuilderError: Error, Equatable {
    case systemMessageInConversation
    case missingConversationMessage
    case invalidRole
    case invalidSystemMessagePlacement
}
