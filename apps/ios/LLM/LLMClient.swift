// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LLMClient logic in the iOS LLM integration layer.
// - Interfaces:
//   - LLMClient
// - APIs:
//   - none
//
//
//  LLMClient.swift
//  Nola
//
//  Protocol for LLM providers (cloud/local).
//

//
//  LLMClient.swift
//  Nola
//
//  Protocol for LLM providers (cloud/local).
//

import Foundation
import CryptoKit

nonisolated public indirect enum AgentCoreJSONValue: Codable, Sendable, Equatable {
    private static let largestExactlyRepresentableIntegerInDouble: UInt64 = 9_007_199_254_740_991

    case string(String)
    case number(Double)
    case signedInteger(Int64)
    case unsignedInteger(UInt64)
    case bool(Bool)
    case object([String: AgentCoreJSONValue])
    case array([AgentCoreJSONValue])
    case null

    public var objectFields: [String: AgentCoreJSONValue]? {
        guard case .object(let fields) = self else { return nil }
        return fields
    }

    public var stringScalar: String? {
        guard case .string(let value) = self else { return nil }
        return value
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Int64.self),
                  value > Int64(Self.largestExactlyRepresentableIntegerInDouble)
                    || value < -Int64(Self.largestExactlyRepresentableIntegerInDouble) {
            self = .signedInteger(value)
        } else if let value = try? container.decode(UInt64.self),
                  value > Self.largestExactlyRepresentableIntegerInDouble {
            self = .unsignedInteger(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([String: AgentCoreJSONValue].self) {
            self = .object(value)
        } else {
            self = .array(try container.decode([AgentCoreJSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .signedInteger(let value): try container.encode(value)
        case .unsignedInteger(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

public protocol LLMClient {
    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput
    func generateSummaryReceipt(input: ModelInput, timeout: TimeInterval?) async throws -> LLMSummaryPhysicalReceipt
    var auxiliaryConfigurationIdentity: Data { get }
    var auxiliaryModelIdentity: String? { get }
    var supportsAuxiliaryPhysicalInvocation: Bool { get }
    // Streamed generation: yields raw string chunks (JSON fragments or text pieces).
    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error>
    
    // Chat interface: direct message processing without structured input.
    // New variant allows callers to provide a custom `systemPrompt` and control
    // whether internal reasoning (<think>...</think>) is enabled via `enableThinking`.
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String
}

public struct LLMSummaryPhysicalReceipt {
    public let output: ModelOutput
    public let usage: AgentCoreJSONValue
    public let finishReason: String?
    public let hasToolCalls: Bool?
    public init(output: ModelOutput, finishReason: String? = nil, hasToolCalls: Bool? = nil, usage: AgentCoreJSONValue = .object([
        "coverage": .string("unknown"), "provider_raw": .null, "normalized": .null
    ])) { self.output = output; self.usage = usage; self.finishReason = finishReason; self.hasToolCalls = hasToolCalls }
}

public struct LLMSummaryPhysicalFailure: Error {
    public let underlying: Error
    public let usage: AgentCoreJSONValue
    public init(underlying: Error, usage: AgentCoreJSONValue) { self.underlying = underlying; self.usage = usage }
}

/// Remote Host alone owns physical Attempts. Local code persists only exact
/// source receipts, including the pre-dispatch admission needed after a crash.
public protocol LLMRemoteAuxiliaryInvocationProviding: LLMClient {
    var remoteAuxiliarySourceHostID: String? { get }
    func inspectRemoteAuxiliary(_ observation: AgentCoreJSONValue,
        observe: @escaping @Sendable (AgentCoreJSONValue) throws -> Void) async throws -> Bool
    func generateRemoteAuxiliary(input: ModelInput, operationID: UUID,
        scenario: LLMAuxiliaryInvocationScenario, timeout: TimeInterval?,
        existingReceipt: AgentCoreJSONValue?,
        observe: @escaping @Sendable (AgentCoreJSONValue) throws -> Void) async throws -> LLMSummaryPhysicalReceipt
}

public struct LLMRemoteAuxiliaryOutcomeUnknown: Error, LocalizedError, Sendable {
    public let sourceHostID: String
    public let operationID: String
    public let attemptID: String
    public var errorDescription: String? {
        "Auxiliary outcome is unconfirmed; inspect source \(sourceHostID), operation \(operationID), attempt \(attemptID). Do not automatically resubmit."
    }
}

public struct LLMRemoteAuxiliaryScopeBlocked: Error, LocalizedError, Sendable {
    public let sourceHostID: String
    public let operationID: String
    public let attemptID: String
    public var errorDescription: String? {
        "The auxiliary worker scope has an unresolved source operation \(sourceHostID)/\(operationID)/\(attemptID). Resolve it before starting new work."
    }
}

public extension LLMClient {
    var auxiliaryConfigurationIdentity: Data { Data(String(reflecting: type(of: self)).utf8) }
    var auxiliaryModelIdentity: String? { nil }
    var supportsAuxiliaryPhysicalInvocation: Bool { true }
    func generateSummaryReceipt(input: ModelInput, timeout: TimeInterval?) async throws -> LLMSummaryPhysicalReceipt {
        LLMSummaryPhysicalReceipt(output: try await generateSummary(input: input, timeout: timeout))
    }
}

public enum LLMAuxiliaryInvocationScope: Sendable {
    case conversation(UUID)
    case account(String)
    case task(String)
    #if os(iOS)
    var wire: AgentCoreJSONValue {
        switch self {
        case .conversation(let id): .object(["kind": .string("conversation"), "conversation_id": .string(id.uuidString)])
        case .account(let id): .object(["kind": .string("account"), "account_id": .string(id)])
        case .task(let id): .object(["kind": .string("task"), "task_id": .string(id)])
        }
    }
    #endif
}

public enum LLMAuxiliaryInvocationScenario: String, Sendable {
    case roomRouting = "room_routing"
    case contextSummary = "context_summary"
    case eventSummary = "event_summary"
    case eventProcessing = "event_processing"
}

/// A bounded background invocation, distinct from conversation state and UI
/// model selection. One invocation owns one provider call; its caller may
/// schedule a new operation but cannot silently multiply provider retries.
#if os(iOS)
public struct LLMSummaryInvocation {
    public let ownerID: String
    public let operationID: UUID
    public let configurationRevision: UInt64
    public let scenario: LLMAuxiliaryInvocationScenario
    private let provider: LLMClient
    private let scope: LLMAuxiliaryInvocationScope
    private var ledgerOverride: IOSApplicationRuntime?

    public init(ownerID: String, operationID: UUID = UUID(), configurationRevision: UInt64 = 0, provider: LLMClient,
                scope: LLMAuxiliaryInvocationScope? = nil, scenario: LLMAuxiliaryInvocationScenario = .eventSummary) {
        self.ownerID = ownerID
        self.operationID = operationID
        let captured = (provider as? ModelService)?.captureAuxiliaryProvider()
        self.configurationRevision = captured?.revision ?? configurationRevision
        self.provider = captured?.provider ?? provider
        self.scope = scope ?? .account(ownerID)
        self.scenario = scenario
    }

    func usingLedger(_ runtime: IOSApplicationRuntime) -> Self {
        var copy = self
        copy.ledgerOverride = runtime
        return copy
    }

    public func generate(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        try await generateReceipt(input: input, timeout: timeout).output
    }

    public func generateReceipt(input: ModelInput, timeout: TimeInterval?) async throws -> LLMSummaryPhysicalReceipt {
        guard !ownerID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw NolaError.llm(.unknown)
        }
        try Task.checkCancellation()
        guard let ledger = ledgerOverride ?? IOSApplicationRuntime.shared else {
            throw NolaError.invalidInput(reason: "Canonical auxiliary invocation ledger is unavailable")
        }
        if let remote = provider as? LLMRemoteAuxiliaryInvocationProviding {
            guard let sourceHost = remote.remoteAuxiliarySourceHostID else {
                throw NolaError.invalidInput(reason: "Remote auxiliary invocation requires a paired source Host")
            }
            let operation = operationID.uuidString.lowercased()
            let localOwner = scope.wire
            let observe: @Sendable (AgentCoreJSONValue) throws -> Void = { observation in
                _ = try ledger.executeAuxiliaryInvocation([
                    "type": .string("observe_source"), "local_owner": localOwner,
                    "observation": observation
                ])
            }
            let slot = try ledger.executeAgentCommand(.object([
                "type": .string("load_auxiliary_source_slot"), "local_owner": localOwner,
                "scenario": .string(scenario.rawValue)
            ]))
            if case .object(let pending) = slot,
               case .string(let pendingHost)? = pending["source_host_id"],
               case .string(let pendingOperation)? = pending["operation_id"],
               case .string(let pendingAttempt)? = pending["attempt_id"], pendingOperation != operation || pendingHost != sourceHost {
                let blocked = LLMRemoteAuxiliaryScopeBlocked(sourceHostID: pendingHost,
                    operationID: pendingOperation, attemptID: pendingAttempt)
                guard pendingHost == sourceHost else { throw blocked }
                // Resource admission, not content matching: inspect the old
                // identity and persist its terminal receipt, but never return
                // its old output as the result of this new logical request.
                do {
                    guard try await remote.inspectRemoteAuxiliary(slot, observe: observe) else { throw blocked }
                } catch { throw blocked }
            }
            let existing = try? ledger.executeAgentCommand(.object([
                "type": .string("auxiliary_invocation"), "command": .object([
                    "type": .string("inspect_source"), "local_owner": localOwner,
                    "source_host_id": .string(sourceHost), "operation_id": .string(operation)
                ])
            ]))
            return try await remote.generateRemoteAuxiliary(input: input, operationID: operationID,
                scenario: scenario, timeout: timeout, existingReceipt: existing, observe: observe)
        }
        guard provider.supportsAuxiliaryPhysicalInvocation else {
            throw NolaError.invalidInput(reason: "This adapter has no physical auxiliary invocation port")
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let inputData = try encoder.encode(input)
        let operation = operationID.uuidString.lowercased()
        let attemptID = UUID().uuidString.lowercased()
        let atMS: @Sendable () -> AgentCoreJSONValue = { .signedInteger(Int64(Date().timeIntervalSince1970 * 1000)) }
        let outputCap = (provider as? LLMAgentCapabilityProviding)?.agentCapabilities.maxOutputTokens
            .flatMap { $0 > 0 ? AgentCoreJSONValue.unsignedInteger(UInt64($0)) : nil } ?? .null
        let configuration: AgentCoreJSONValue = .object([
            "provider": .string(String(reflecting: type(of: provider))),
            "model": provider.auxiliaryModelIdentity.map { .string($0) } ?? .null,
            "configuration_revision": .unsignedInteger(configurationRevision),
            "configuration_digest": .string(SHA256.hash(data: provider.auxiliaryConfigurationIdentity).map { String(format: "%02x", $0) }.joined()),
            "input_digest": .string(SHA256.hash(data: inputData).map { String(format: "%02x", $0) }.joined()),
            "max_output_tokens_per_attempt": outputCap, "total_output_token_budget": outputCap,
            "max_attempts": .number(1), "maximum_duration_ms": .unsignedInteger(UInt64(max(1, min(timeout ?? 300, 3600)) * 1000))
        ])
        let admitted = try ledger.executeAuxiliaryInvocation([
            "type": .string("admit"), "operation_id": .string(operation), "owner": scope.wire,
            "scenario": .string(scenario.rawValue), "configuration": configuration, "at_ms": atMS()
        ])
        let started = try ledger.executeAuxiliaryInvocation([
            "type": .string("start_attempt"), "operation_id": .string(operation), "attempt_id": .string(attemptID),
            "expected_revision": .signedInteger(admitted.operation.revision), "at_ms": atMS()
        ])
        guard started.dispatchPermitted, let attempt = started.attempt else {
            throw NolaError.invalidInput(reason: "Auxiliary attempt is already dispatched or no longer admissible")
        }
        return try await withTaskCancellationHandler {
            let physical: LLMSummaryPhysicalReceipt
            do {
                try Task.checkCancellation()
                physical = try await provider.generateSummaryReceipt(input: input, timeout: timeout)
            } catch {
                let physicalFailure = error as? LLMSummaryPhysicalFailure
                let cancelled = error is CancellationError || physicalFailure?.underlying is CancellationError || Task.isCancelled
                _ = try ledger.executeAuxiliaryInvocation([
                    "type": .string("record_receipt"), "operation_id": .string(operation), "attempt_id": .string(attemptID),
                    "expected_attempt_revision": .signedInteger(attempt.revision),
                    "state": .string(cancelled ? "cancelled" : "failed"),
                    "usage": physicalFailure?.usage
                        ?? .object(["coverage": .string("unknown"), "provider_raw": .null, "normalized": .null]),
                    "failure": .object(["code": .string(cancelled ? "cancelled" : "provider_failed"),
                        "message": .string(cancelled ? "Auxiliary model request cancelled" : "Auxiliary model request failed"), "retryable": .bool(false)]),
                    "at_ms": atMS()
                ])
                throw error
            }
            _ = try ledger.executeAuxiliaryInvocation([
                "type": .string("record_receipt"), "operation_id": .string(operation), "attempt_id": .string(attemptID),
                "expected_attempt_revision": .signedInteger(attempt.revision), "state": .string("succeeded"),
                "usage": physical.usage, "failure": .null, "at_ms": atMS()
            ])
            try Task.checkCancellation()
            return physical
        } onCancel: {
            // Cancellation intent is durable independently of provider stop.
            _ = try? ledger.executeAuxiliaryInvocation([
                "type": .string("cancel"), "operation_id": .string(operation), "at_ms": atMS()
            ])
        }
    }
}

#endif

public struct LLMStreamUsage: Sendable, Equatable {
    public let promptTokens: Int?
    public let cachedInputTokens: Int?
    public let cacheWriteInputTokens: Int?
    public let completionTokens: Int?
    public let totalTokens: Int?
    public let reasoningTokens: Int?
    public let providerRawUsage: AgentCoreJSONValue?

    public init(
        promptTokens: Int? = nil,
        cachedInputTokens: Int? = nil,
        cacheWriteInputTokens: Int? = nil,
        completionTokens: Int? = nil,
        totalTokens: Int? = nil,
        reasoningTokens: Int? = nil,
        providerRawUsage: AgentCoreJSONValue? = nil
    ) {
        self.promptTokens = promptTokens
        self.cachedInputTokens = cachedInputTokens
        self.cacheWriteInputTokens = cacheWriteInputTokens
        self.completionTokens = completionTokens
        self.totalTokens = totalTokens
        self.reasoningTokens = reasoningTokens
        self.providerRawUsage = providerRawUsage
    }
}

public struct LLMToolCallDelta: Sendable, Equatable {
    public let index: Int
    public let callId: String
    public let capabilityId: String
    public let argumentsDelta: String

    public init(index: Int, callId: String, capabilityId: String, argumentsDelta: String) {
        self.index = index
        self.callId = callId
        self.capabilityId = capabilityId
        self.argumentsDelta = argumentsDelta
    }
}

public enum LLMStreamEvent: Sendable, Equatable {
    case textDelta(String)
    case reasoningDelta(String)
    case toolCallDelta(LLMToolCallDelta)
    case usage(LLMStreamUsage)
    case continuation(String)
    case completed(finishReason: String)
}

/// Joins the provider's actual producer, not merely its stream consumer.
nonisolated public final class LLMStreamProducerControl: @unchecked Sendable {
    private let lock = NSLock()
    private var producer: Task<Void, Never>?

    func install(_ task: Task<Void, Never>) {
        lock.lock()
        producer = task
        lock.unlock()
    }

    private func capturedProducer() -> Task<Void, Never>? {
        lock.lock()
        defer { lock.unlock() }
        return producer
    }

    public func cancelAndWait() async {
        let task = capturedProducer()
        task?.cancel()
        await task?.value
    }
}

nonisolated public struct LLMControlledEventStream: Sendable {
    public let events: AsyncThrowingStream<LLMStreamEvent, Error>
    public let producer: LLMStreamProducerControl
}

public protocol LLMControlledEventStreamingClient: LLMEventStreamingClient {
    func generateControlledEventStream(input: ModelInput) -> LLMControlledEventStream
}

/// Generic String streaming cannot preserve native tool-call and usage boundaries.
public protocol LLMEventStreamingClient: LLMClient {
    var supportsNativeEventStream: Bool { get }
    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error>
}

/// Frozen adapter/model execution facts. A typed event stream alone grants no
/// tool or reasoning capability. This LLM-layer value has no Core wire schema.
nonisolated public struct LLMAgentCapabilities: Sendable, Equatable, Encodable {
    public let visionInput: Bool
    /// Actual frozen request limit, not a guessed model capacity.
    public let maxOutputTokens: Int?
    public let nativeToolCalls: Bool
    public let parallelToolCalls: Bool
    public let reasoning: Bool
    public let reasoningContinuity: Bool
    public let promptCache: Bool
    public let streamText: Bool
    public let streamReasoning: Bool
    public let streamToolCalls: Bool
    public let requestContinuation: Bool
    public let maxRequestContinuations: Int

    public init(visionInput: Bool = false, maxOutputTokens: Int? = nil,
        nativeToolCalls: Bool = false, parallelToolCalls: Bool = false,
        reasoning: Bool = false, reasoningContinuity: Bool = false, promptCache: Bool = false,
        streamText: Bool = false, streamReasoning: Bool = false, streamToolCalls: Bool = false,
        requestContinuation: Bool = false, maxRequestContinuations: Int = 0) {
        self.visionInput = visionInput
        self.maxOutputTokens = maxOutputTokens
        self.nativeToolCalls = nativeToolCalls
        self.parallelToolCalls = nativeToolCalls && parallelToolCalls
        self.reasoning = reasoning
        self.reasoningContinuity = reasoning && reasoningContinuity
        self.promptCache = promptCache
        self.streamText = streamText
        self.streamReasoning = streamReasoning
        self.streamToolCalls = nativeToolCalls && streamToolCalls
        self.requestContinuation = requestContinuation && maxRequestContinuations > 0
        self.maxRequestContinuations = self.requestContinuation ? maxRequestContinuations : 0
    }
}

public protocol LLMAgentCapabilityProviding: LLMClient {
    var agentCapabilities: LLMAgentCapabilities { get }
}

/// Optional provider capability for preserving the stable Core-composed prompt
/// prefix across Agent turns. This belongs to the provider contract so every
/// platform can use the same cache identity without depending on an iOS Host adapter.
public protocol AgentPromptCacheIdentityClient: LLMClient {
    func chatAgentTurn(
        message: String,
        systemPrompt: String,
        enableThinking: Bool,
        stablePrefixSHA256: String
    ) async throws -> String
}

// Default implementation for chat method
public extension LLMClient {
    // Provide a default implementation of the richer chat variant that falls back to
    // the streaming `generateSummaryStream` when a provider doesn't override it.
    func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        // Default behavior: build minimal ModelInput and stream
        var userPrefs = ["message": message]
        if !enableThinking {
            userPrefs["enable_thinking"] = "false"
        }
        let input = ModelInput(
            dateRange: ModelInput.DateRange(
                start: ISO8601DateFormatter().string(from: Date().addingTimeInterval(-86400)),
                end: ISO8601DateFormatter().string(from: Date())
            ),
            candidates: [],
            userPrefs: userPrefs
        )

        var fullResponse = ""
        let stream = generateSummaryStream(input: input)
        for try await chunk in stream {
            fullResponse += chunk
        }

        if !enableThinking {
            fullResponse = ChatThinkingParser.splitThinkingBlocks(from: fullResponse).response
        }
        return fullResponse.isEmpty ? NSLocalizedString("autogen.LLMClient.d01e6a95", comment: "AUTOGEN") : fullResponse
    }
}
