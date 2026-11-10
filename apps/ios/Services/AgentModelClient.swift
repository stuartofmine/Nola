// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provider-neutral model contract for the shared mobile agent runtime.
// - Contract: Mirrors the Core StreamedChoice and ToolCall response semantics.

import Foundation

nonisolated public struct AgentModelInvocationPlan: Sendable, Equatable {
    /// Core's admitted generation limit, distinct from context reservation.
    public let maxOutputTokens: Int?
    public let nativeToolCalls: Bool
    public let parallelToolCalls: Bool
    public let reasoning: Bool
    public let promptCache: Bool
    public let streamText: Bool
    public let streamReasoning: Bool
    public let streamToolCalls: Bool

    public init(
        maxOutputTokens: Int? = nil,
        nativeToolCalls: Bool = false,
        parallelToolCalls: Bool = false,
        reasoning: Bool = false,
        promptCache: Bool = false,
        streamText: Bool = false,
        streamReasoning: Bool = false,
        streamToolCalls: Bool = false
    ) {
        self.maxOutputTokens = maxOutputTokens
        self.nativeToolCalls = nativeToolCalls
        self.parallelToolCalls = nativeToolCalls && parallelToolCalls
        self.reasoning = reasoning
        self.promptCache = promptCache
        self.streamText = streamText
        self.streamReasoning = streamReasoning
        self.streamToolCalls = nativeToolCalls && streamToolCalls
    }
}

nonisolated public struct AgentModelRequest: Sendable {
    public let sessionId: String
    public let turnId: String
    public let iteration: Int
    public let messages: [AgentCoreMessage]
    public let tools: [AgentCoreJSONValue]
    public let reservedOutputTokens: Int
    public let policyFragments: [AgentCorePromptFragment]
    public let stablePrefixSHA256: String?
    public let invocationPlan: AgentModelInvocationPlan
    public let requiredToolName: String?
    public let providerContinuation: AgentCoreJSONValue?

    public init(
        sessionId: String,
        turnId: String,
        iteration: Int,
        messages: [AgentCoreMessage],
        tools: [AgentCoreJSONValue],
        reservedOutputTokens: Int,
        policyFragments: [AgentCorePromptFragment] = [],
        stablePrefixSHA256: String? = nil,
        invocationPlan: AgentModelInvocationPlan = .init(),
        requiredToolName: String? = nil,
        providerContinuation: AgentCoreJSONValue? = nil
    ) {
        self.sessionId = sessionId
        self.turnId = turnId
        self.iteration = iteration
        self.messages = messages
        self.tools = tools
        self.reservedOutputTokens = reservedOutputTokens
        self.policyFragments = policyFragments
        self.stablePrefixSHA256 = stablePrefixSHA256
        self.invocationPlan = invocationPlan
        self.requiredToolName = requiredToolName
        self.providerContinuation = providerContinuation
    }
}

nonisolated public struct AgentModelCompletion: Sendable {
    public let content: String
    public let reasoning: String
    public let toolCalls: [AgentCoreToolCall]
    public let finishReason: String
    public let usage: AgentCoreTokenUsage
    public let providerContinuation: AgentCoreJSONValue?

    public init(
        content: String,
        reasoning: String = "",
        toolCalls: [AgentCoreToolCall] = [],
        finishReason: String = "stop",
        usage: AgentCoreTokenUsage = .init(),
        providerContinuation: AgentCoreJSONValue? = nil
    ) {
        self.content = content
        self.reasoning = reasoning
        self.toolCalls = toolCalls
        self.finishReason = finishReason
        self.usage = usage
        self.providerContinuation = providerContinuation
    }
}

nonisolated func isModelOutputLimitFinishReason(_ finishReason: String) -> Bool {
    switch finishReason.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
    case "length", "max_tokens", "max_output_tokens", "token_limit":
        return true
    default:
        return false
    }
}

nonisolated public enum AgentModelEvent: Sendable {
    case textDelta(String)
    case reasoningDelta(String)
    case toolCallDelta(partId: String, callId: String, capabilityId: String, argumentsDelta: String)
    case completed(AgentModelCompletion)
}

@MainActor
public protocol AgentModelClient: Sendable {
    /// Provider/transport facts frozen into Core before a Turn starts.
    /// A client without a declaration cannot be admitted to Core execution;
    /// Core never infers native features from its model name.
    nonisolated var coreModelCapabilityProfile: AgentCoreJSONValue? { get }

    func generateAgentTurn(
        request: AgentModelRequest
    ) -> AsyncThrowingStream<AgentModelEvent, Error>
}

public extension AgentModelClient {
    nonisolated var coreModelCapabilityProfile: AgentCoreJSONValue? { nil }
}

/// Only adapters that can join the exact physical producer may confirm a stop.
@MainActor
public protocol AgentModelCancellationClient: AgentModelClient {
    func cancelAgentTurn(request: AgentModelRequest) async throws
}

/// Independent stream/cancellation bookkeeping over the same frozen provider.
/// This is not permission to select a different model or current default.
public protocol AgentIndependentModelClientFactory: AgentModelClient {
    func makeIndependentAgentModelClient() -> any AgentModelClient
}

public enum AgentStructuredOutputExecutor {
    @MainActor
    public static func execute(
        sessionId: String,
        turnId: String,
        systemInstructions: String,
        modelInput: AgentCoreJSONValue,
        responseSchema: AgentCoreJSONValue,
        submissionToolName: String,
        submissionToolDescription: String,
        client: any AgentModelClient,
        reservedOutputTokens: Int = 16_384,
        maxAttempts: Int = 3
    ) async throws -> AgentCoreJSONValue {
        let normalizedToolName = submissionToolName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedToolName.isEmpty else {
            throw AgentCoreRuntimeError.core("Structured output requires a submission tool name.")
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let modelInputJSON = String(decoding: try encoder.encode(modelInput), as: UTF8.self)
        let tool: AgentCoreJSONValue = .object([
            "type": .string("function"),
            "function": .object([
                "name": .string(normalizedToolName),
                "description": .string(submissionToolDescription),
                "parameters": responseSchema,
            ]),
        ])
        let attempts = max(1, min(maxAttempts, 3))
        var messages = [
            AgentCoreMessage(role: "system", content: systemInstructions),
            AgentCoreMessage(role: "user", content: modelInputJSON),
        ]
        var lastValidationError = "Model did not submit the required structured output."
        for attempt in 1...attempts {
            let request = AgentModelRequest(
                sessionId: sessionId,
                turnId: turnId,
                iteration: attempt,
                messages: messages,
                tools: [tool],
                reservedOutputTokens: max(256, min(reservedOutputTokens, 8_192)),
                invocationPlan: AgentModelInvocationPlan(
                    nativeToolCalls: true,
                    parallelToolCalls: false,
                    reasoning: false,
                    promptCache: false,
                    streamText: false,
                    streamReasoning: false,
                    streamToolCalls: true
                ),
                requiredToolName: normalizedToolName
            )
            var completion: AgentModelCompletion?
            for try await event in client.generateAgentTurn(request: request) {
                if case .completed(let value) = event {
                    completion = value
                }
            }
            let matchingCalls = completion?.toolCalls.filter { $0.name == normalizedToolName } ?? []
            if matchingCalls.count == 1, let call = matchingCalls.first {
                do {
                    try AgentStructuredOutputSchemaValidator.validate(
                        value: call.arguments,
                        schema: responseSchema
                    )
                    return call.arguments
                } catch {
                    lastValidationError = String(describing: error)
                }
            } else {
                lastValidationError = "Expected exactly one \(normalizedToolName) call; received \(matchingCalls.count)."
            }
            if attempt < attempts {
                messages.append(AgentCoreMessage(
                    role: "user",
                    content: "The previous structured submission was rejected: \(lastValidationError) Correct the arguments and call \(normalizedToolName) exactly once. Do not return plain text."
                ))
            }
        }
        throw AgentCoreRuntimeError.core(
            "Failed to obtain valid structured output after \(attempts) attempt(s): \(lastValidationError)"
        )
    }
}

private enum AgentStructuredOutputSchemaValidator {
    static func validate(
        value: AgentCoreJSONValue,
        schema: AgentCoreJSONValue,
        path: String = "$"
    ) throws {
        guard case .object(let definition) = schema else {
            throw AgentCoreRuntimeError.core("Structured output schema at \(path) must be an object.")
        }
        if let constant = definition["const"], constant != value {
            throw AgentCoreRuntimeError.core("Structured output at \(path) does not match its constant value.")
        }
        if let allowed = definition["type"] {
            let types: [String]
            switch allowed {
            case .string(let type):
                types = [type]
            case .array(let values):
                types = values.compactMap { item in
                    if case .string(let type) = item { return type }
                    return nil
                }
            default:
                types = []
            }
            if !types.isEmpty && !types.contains(where: { matches(value, type: $0) }) {
                throw AgentCoreRuntimeError.core(
                    "Structured output at \(path) does not match type \(types.joined(separator: "|" ))."
                )
            }
        }
        if let enumDefinition = definition["enum"],
           case .array(let allowedValues) = enumDefinition,
           !allowedValues.contains(value) {
                throw AgentCoreRuntimeError.core("Structured output at \(path) is outside the allowed enum.")
        }

        switch value {
        case .object(let object):
            let required: [String]
            if let requiredDefinition = definition["required"],
               case .array(let values) = requiredDefinition {
                required = values.compactMap { value in
                    guard case .string(let requiredName) = value else { return nil }
                    return requiredName
                }
            } else {
                required = []
            }
            if let missing = required.first(where: { object[$0] == nil }) {
                throw AgentCoreRuntimeError.core(
                    "Structured output at \(path) is missing required property \(missing)."
                )
            }
            let properties: [String: AgentCoreJSONValue]
            if let propertyDefinition = definition["properties"],
               case .object(let propertyValues) = propertyDefinition {
                properties = propertyValues
            } else {
                properties = [:]
            }
            if definition["additionalProperties"] == .bool(false),
               let unexpected = object.keys.first(where: { properties[$0] == nil }) {
                throw AgentCoreRuntimeError.core(
                    "Structured output at \(path) contains unexpected property \(unexpected)."
                )
            }
            for (key, childSchema) in properties {
                guard let child = object[key] else { continue }
                try validate(value: child, schema: childSchema, path: "\(path).\(key)")
            }
        case .array(let values):
            if let minimum = numericValue(definition["minItems"]),
               values.count < Int(minimum) {
                throw AgentCoreRuntimeError.core("Structured output at \(path) has too few items.")
            }
            if let maximum = numericValue(definition["maxItems"]),
               values.count > Int(maximum) {
                throw AgentCoreRuntimeError.core("Structured output at \(path) has too many items.")
            }
            if let itemSchema = definition["items"] {
                for (index, item) in values.enumerated() {
                    try validate(value: item, schema: itemSchema, path: "\(path)[\(index)]")
                }
            }
        case .string(let string):
            if let minimum = numericValue(definition["minLength"]),
               string.count < Int(minimum) {
                throw AgentCoreRuntimeError.core("Structured output at \(path) is too short.")
            }
            if let maximum = numericValue(definition["maxLength"]),
               string.count > Int(maximum) {
                throw AgentCoreRuntimeError.core("Structured output at \(path) is too long.")
            }
        case .number(let number):
            if definition["type"] == .string("integer"), number.rounded() != number {
                throw AgentCoreRuntimeError.core("Structured output at \(path) must be an integer.")
            }
            if let minimum = numericValue(definition["minimum"]), number < minimum {
                throw AgentCoreRuntimeError.core("Structured output at \(path) is below its minimum.")
            }
            if let maximum = numericValue(definition["maximum"]), number > maximum {
                throw AgentCoreRuntimeError.core("Structured output at \(path) exceeds its maximum.")
            }
        case .signedInteger(let number):
            try validateInteger(Double(number), definition: definition, path: path)
        case .unsignedInteger(let number):
            try validateInteger(Double(number), definition: definition, path: path)
        case .bool, .null:
            break
        }
    }

    private static func validateInteger(
        _ number: Double,
        definition: [String: AgentCoreJSONValue],
        path: String
    ) throws {
        if let minimum = numericValue(definition["minimum"]), number < minimum {
            throw AgentCoreRuntimeError.core("Structured output at \(path) is below its minimum.")
        }
        if let maximum = numericValue(definition["maximum"]), number > maximum {
            throw AgentCoreRuntimeError.core("Structured output at \(path) exceeds its maximum.")
        }
    }

    private static func matches(_ value: AgentCoreJSONValue, type: String) -> Bool {
        switch (value, type) {
        case (.object, "object"), (.array, "array"), (.string, "string"),
             (.number, "number"), (.signedInteger, "number"), (.unsignedInteger, "number"),
             (.signedInteger, "integer"), (.unsignedInteger, "integer"),
             (.bool, "boolean"), (.null, "null"):
            return true
        case (.number(let value), "integer"):
            return value.rounded() == value
        default:
            return false
        }
    }

    private static func numericValue(_ value: AgentCoreJSONValue?) -> Double? {
        switch value {
        case .number(let number): return number
        case .signedInteger(let number): return Double(number)
        case .unsignedInteger(let number): return Double(number)
        default: return nil
        }
    }
}

public enum AgentMemoryWorkerModelExecutor {
    public static let submissionToolName = "submit_memory_worker_output"

    @MainActor
    public static func execute(
        plan: AgentCoreJSONValue,
        client: any AgentModelClient,
        reservedOutputTokens: Int = 6_144
    ) async throws -> AgentCoreJSONValue {
        guard case .object(let object) = plan,
              case .string(let jobId) = object["job_id"],
              case .string(let systemInstructions) = object["system_instructions"],
              let modelInput = object["model_input"],
              let responseSchema = object["response_schema"] else {
            throw AgentCoreRuntimeError.core("Core returned an invalid Memory Worker plan.")
        }
        return try await AgentStructuredOutputExecutor.execute(
            sessionId: "memory-worker:\(jobId)",
            turnId: "memory-worker:\(jobId)",
            systemInstructions: systemInstructions,
            modelInput: modelInput,
            responseSchema: responseSchema,
            submissionToolName: submissionToolName,
            submissionToolDescription: "Submit semantic memory candidates for deterministic Core materialization and validation.",
            client: client,
            reservedOutputTokens: max(256, min(reservedOutputTokens, 6_144))
        )
    }
}
