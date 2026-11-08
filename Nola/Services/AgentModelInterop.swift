// SPDX-License-Identifier: Apache-2.0

import Foundation

#if canImport(NolaCore)
import NolaCore
#endif

public struct AgentLocalPrompt: Codable, Sendable {
    public let systemPrompt: String
    public let prompt: String
    public let allowedToolNames: [String]
    public let stablePrefixSHA256: String?

    enum CodingKeys: String, CodingKey {
        case systemPrompt = "system_prompt"
        case prompt
        case allowedToolNames = "allowed_tool_names"
        case stablePrefixSHA256 = "stable_prefix_sha256"
    }
}

public struct AgentParsedModelOutput: Codable, Sendable {
    public let content: String
    public let reasoning: String
    public let toolCalls: [AgentCoreToolCall]
    public let finishReason: String
    public let warnings: [String]

    enum CodingKeys: String, CodingKey {
        case content, reasoning, warnings
        case toolCalls = "tool_calls"
        case finishReason = "finish_reason"
    }
}

public enum AgentModelInterop {
    public static func stitchContinuation(accumulated: String, continuation: String) -> String? {
#if canImport(NolaCore)
        return callCore(
            value: ContinuationInput(accumulated: accumulated, continuation: continuation),
            function: nola_core_stitch_agent_model_continuation,
            result: ContinuationOutput.self
        )?.append
#else
        nil
#endif
    }

    public static func localPrompt(for request: AgentModelRequest) -> AgentLocalPrompt? {
        try? AgentRequestModelBuilder().build(request)
    }

    public static func parse(
        content: String,
        reasoning: String = "",
        nativeToolCalls: [AgentCoreToolCall] = [],
        allowedToolNames: [String]
    ) -> AgentParsedModelOutput? {
#if canImport(NolaCore)
        callCore(
            value: ParseInput(
                content: content,
                reasoning: reasoning,
                nativeToolCalls: nativeToolCalls,
                allowedToolNames: allowedToolNames
            ),
            function: nola_core_parse_agent_model_output,
            result: AgentParsedModelOutput.self
        )
#else
        nil
#endif
    }

    private struct ParseInput: Encodable {
        let content: String
        let reasoning: String
        let nativeToolCalls: [AgentCoreToolCall]
        let allowedToolNames: [String]

        enum CodingKeys: String, CodingKey {
            case content, reasoning
            case nativeToolCalls = "native_tool_calls"
            case allowedToolNames = "allowed_tool_names"
        }
    }

    private struct ContinuationInput: Encodable {
        let accumulated: String
        let continuation: String
    }

    private struct ContinuationOutput: Decodable {
        let append: String
    }

#if canImport(NolaCore)
    private static func callCore<Input: Encodable, Output: Decodable>(
        value: Input,
        function: (UnsafePointer<UInt8>?, Int) -> UnsafeMutablePointer<CChar>?,
        result: Output.Type
    ) -> Output? {
        guard let data = try? JSONEncoder().encode(value) else { return nil }
        let pointer = data.withUnsafeBytes { bytes in
            function(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        }
        guard let pointer else { return nil }
        defer { nola_core_free_c_string(pointer) }
        guard let json = String(validatingUTF8: pointer) else { return nil }
        return try? JSONDecoder().decode(Output.self, from: Data(json.utf8))
    }
#endif
}

nonisolated struct AgentRequestModelBuilder {
    typealias Composer = ([AgentCorePromptFragment]) throws -> AgentCorePromptComposition

    private let composer: Composer

    init(composer: @escaping Composer = AgentCorePromptComposer.compose) {
        self.composer = composer
    }

    func build(_ request: AgentModelRequest) throws -> AgentLocalPrompt {
        let toolNames = Self.toolNames(request.tools)
        let coreSystemMessages = request.messages
            .filter { $0.role == "system" }
            .map(\.content)
            .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        let auxiliaryPolicy = request.policyFragments.isEmpty
            ? nil
            : try composer(request.policyFragments).rendered
        let systemPrompt = ([auxiliaryPolicy].compactMap { $0 } + coreSystemMessages)
            .joined(separator: "\n\n")
        let conversation = request.messages.filter { $0.role != "system" }
        let data = try JSONEncoder().encode(conversation)
        guard let json = String(data: data, encoding: .utf8) else {
            throw AgentCorePromptComposerError.invalidResponse
        }
        return AgentLocalPrompt(
            systemPrompt: systemPrompt,
            prompt: """
            Continue the typed conversation below and answer the latest user message. Preserve role and tool-call boundaries exactly.
            <conversation_json>
            \(json)
            </conversation_json>
            """,
            allowedToolNames: toolNames,
            stablePrefixSHA256: request.stablePrefixSHA256
        )
    }

    private static func toolNames(_ tools: [AgentCoreJSONValue]) -> [String] {
        var names: [String] = []
        for tool in tools {
            guard case .object(let object) = tool else { continue }
            let direct = string(object["name"])
            let nested: String? = {
                guard case .object(let function)? = object["function"] else { return nil }
                return string(function["name"])
            }()
            guard let name = (nested ?? direct)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !name.isEmpty,
                  !names.contains(name) else { continue }
            names.append(name)
        }
        return names
    }

    private static func string(_ value: AgentCoreJSONValue?) -> String? {
        guard case .string(let string)? = value else { return nil }
        return string
    }

}
