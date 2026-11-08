// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LocalProvider logic in the iOS LLM integration layer.
// - Interfaces:
//   - LocalProvider
// - APIs:
//   - none
//
//
//  LocalProvider.swift
//  Nola
//
//  Local on-device LLM provider for iOS.
//  Uses the MLX Swift implementation.
//

import Foundation
import MLXLLM
import MLXLMCommon

struct LocalProviderGenerationRequest {
    let prompt: String
    let systemPrompt: String?
    let model: LMModel
    let parameters: ModelParameters
    let enableThinking: Bool
}

public final class LocalProvider: LLMClient, LLMAgentCapabilityProviding {
    public var agentCapabilities: LLMAgentCapabilities {
        .init(maxOutputTokens: maxTokens, reasoning: enableThinking)
    }
    /// Actual catalog target selected once by the factory. This does not
    /// identify downloaded weight contents or promise cross-version replay.
    struct ModelIdentity: Equatable, Sendable {
        let id: String
        let name: String
    }

    var resolvedModelIdentity: ModelIdentity {
        .init(id: selectedModel.id, name: selectedModel.name)
    }

    // MARK: - Properties
    
    private let mlxService: MLXService
    private let selectedModel: LMModel
    public var auxiliaryModelIdentity: String? { selectedModel.name }
    public var auxiliaryConfigurationIdentity: Data {
        var fields: [String: Any] = ["model": selectedModel.name, "max_tokens": maxTokens,
            "temperature": temperature, "top_p": topP,
            "thinking": enableThinking]
        if let topK { fields["top_k"] = topK } else { fields["top_k"] = NSNull() }
        return (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data()
    }
    
    // Performance monitoring
    private var inferenceStartTime: Date?
    
    // Configuration
    private let maxTokens: Int
    private let temperature: Float
    private let topP: Float
    private let topK: Int?
    private let enableThinking: Bool
    private let runParameters: ModelParameters?
    private let generationOverride: ((LocalProviderGenerationRequest) async throws -> AsyncThrowingStream<String, Error>)?
    
    // MARK: - Initialization
    
    /// Initialize LocalProvider with MLX service
    /// - Parameters:
    ///   - model: Language model to use (default: qwen3:4b)
    ///   - maxTokens: Maximum tokens to generate (default: 512)
    ///   - temperature: Sampling temperature (default: 0.7)
    public init(
        model: LMModel? = nil,
        maxTokens: Int = 512,
        temperature: Float = Float(ModelParameters.localDefaults.temperature)
    ) {
        self.mlxService = MLXService.shared
        self.selectedModel = Self.resolveModel(model)
        self.maxTokens = maxTokens
        self.temperature = temperature
        self.topP = Float(ModelParameters.localDefaults.topP)
        self.topK = nil
        self.enableThinking = true
        self.runParameters = nil
        self.generationOverride = nil
    }

    /// Extended initializer with topP/topK support
    public init(
        model: LMModel? = nil,
        maxTokens: Int = 512,
        temperature: Float = Float(ModelParameters.localDefaults.temperature),
        topP: Float = Float(ModelParameters.localDefaults.topP),
        topK: Int? = nil
    ) {
        self.mlxService = MLXService.shared
        self.selectedModel = Self.resolveModel(model)
        self.maxTokens = maxTokens
        self.temperature = temperature
        self.topP = topP
        self.topK = topK
        self.enableThinking = true
        self.runParameters = nil
        self.generationOverride = nil
    }

    /// Extended initializer with topP/topK and thinking control
    public init(
        model: LMModel? = nil,
        maxTokens: Int = 512,
        temperature: Float = Float(ModelParameters.localDefaults.temperature),
        topP: Float = Float(ModelParameters.localDefaults.topP),
        topK: Int? = nil,
        enableThinking: Bool = true
    ) {
        self.mlxService = MLXService.shared
        self.selectedModel = Self.resolveModel(model)
        self.maxTokens = maxTokens
        self.temperature = temperature
        self.topP = topP
        self.topK = topK
        self.enableThinking = enableThinking
        self.runParameters = nil
        self.generationOverride = nil
    }

    /// A Run owns its generation values. Only loaded model weights are shared.
    init(model: LMModel, parameters: ModelParameters,
         generation: ((LocalProviderGenerationRequest) async throws -> AsyncThrowingStream<String, Error>)? = nil) throws {
        let sampling = LocalSamplingSettings(parameters: parameters)
        guard ModelParameterLimits.outputTokens.contains(parameters.maxOutputTokens),
              ModelParameterLimits.remoteContextTokens.contains(parameters.maxContextLength),
              parameters.temperature.isFinite, (0...2).contains(parameters.temperature),
              parameters.topP.isFinite, (0...1).contains(parameters.topP),
              (1...1_000_000).contains(sampling.topK),
              sampling.repetitionPenalty.isFinite, (0.8...1.4).contains(sampling.repetitionPenalty),
              parameters.presencePenalty.isFinite, (-2...2).contains(parameters.presencePenalty),
              parameters.frequencyPenalty.isFinite, (-2...2).contains(parameters.frequencyPenalty) else {
            throw NolaError.llm(.invalidInput(reason: "Invalid local Run generation parameters"))
        }
        guard parameters.stopSequences.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw NolaError.llm(.invalidInput(reason: "The local model runtime does not support stop sequences"))
        }
        self.mlxService = .shared
        self.selectedModel = model
        self.maxTokens = parameters.maxOutputTokens
        self.temperature = Float(parameters.temperature)
        self.topP = Float(parameters.topP)
        self.topK = parameters.topK
        self.enableThinking = parameters.showThinking
        self.runParameters = parameters
        self.generationOverride = generation
    }

    private func generationStream(prompt: String, systemPrompt: String?, enableThinking: Bool) async throws -> AsyncThrowingStream<String, Error> {
        var parameters = runParameters ?? ModelParameters.localDefaults
        if runParameters == nil {
            parameters.maxOutputTokens = maxTokens
            parameters.temperature = Double(temperature)
            parameters.topP = Double(topP)
            parameters.topK = topK ?? 0
            // Preserve the legacy service default for non-Run callers.
            parameters.repetitionPenalty = 1.1
        }
        let thinking = runParameters == nil ? enableThinking : (self.enableThinking && enableThinking)
        if let generationOverride {
            return try await generationOverride(.init(prompt: prompt, systemPrompt: systemPrompt,
                model: selectedModel, parameters: parameters, enableThinking: thinking))
        }
        return try await mlxService.generate(prompt: prompt, systemPrompt: systemPrompt, model: selectedModel,
            temperature: Float(parameters.temperature), maxTokens: parameters.maxOutputTokens,
            topP: Float(parameters.topP), topK: topK, enableThinking: thinking,
            repetitionPenalty: Float(LocalSamplingSettings(parameters: parameters).repetitionPenalty),
            maxContextTokens: runParameters?.maxContextLength, seed: runParameters?.seed,
            presencePenalty: Float(parameters.presencePenalty), frequencyPenalty: Float(parameters.frequencyPenalty))
    }

    private func generateComplete(prompt: String, systemPrompt: String?, enableThinking: Bool) async throws -> String {
        var result = ""
        for try await chunk in try await generationStream(prompt: prompt, systemPrompt: systemPrompt, enableThinking: enableThinking) {
            result += chunk
        }
        return result
    }

    private static func resolveModel(_ model: LMModel?) -> LMModel {
        if let model {
            return model
        }
        if let first = MLXService.availableModels.first {
            return first
        }

        Logger.shared.log(.error, "LocalProvider", "No MLX models configured; falling back to bundled Qwen 0.6B configuration")
        return LMModel(
            name: "mlx-community/Qwen3.5-0.8B-MLX-4bit",
            configuration: ModelConfiguration(id: "mlx-community/Qwen3.5-0.8B-MLX-4bit"),
            type: .vlm
        )
    }
    
    // MARK: - LLMClient Protocol
    
    public func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        inferenceStartTime = Date()
        
        // Build prompt
        let prompt = try buildPrompt(input: input)
        let systemPrompt = PromptTemplates.summarizationSystem
        
        Logger.shared.log("Generating summary with model: \(selectedModel.name)", level: .info)
        Logger.shared.log("Parameters: maxTokens=\(maxTokens), temperature=\(temperature), timeout=\(timeout?.description ?? "none")", level: .debug)
        
        // Generate response with timeout handling
        let response: String
        let inferenceStart = Date()
        do {
            if let timeout = timeout {
                response = try await withTimeout(timeout) {
                    try await self.generateComplete(
                        prompt: prompt,
                        systemPrompt: systemPrompt,
                        enableThinking: self.enableThinking
                    )
                }
            } else {
                response = try await generateComplete(
                    prompt: prompt,
                    systemPrompt: systemPrompt,
                    enableThinking: self.enableThinking
                )
            }
        } catch {
            Logger.shared.log("Generation failed: \(error.localizedDescription)", level: .error)
            throw error
        }
        
        let inferenceDuration = Date().timeIntervalSince(inferenceStart)
        Logger.shared.log("Generated response preview: \(response.prefix(200))...", level: .debug)
        // Also log the full response at debug level to aid debugging of parsing issues
        Logger.shared.log("Generated response (full): \(response)", level: .debug)
        
        // Parse response
        let output = try parseResponse(response)
        
        if let startTime = inferenceStartTime {
            let elapsed = Date().timeIntervalSince(startTime)
            Logger.shared.log("LocalProvider inference completed in \(String(format: "%.2f", elapsed))s (inference: \(String(format: "%.2f", inferenceDuration))s)", level: .info)
        }
        
        return output
    }

    // New chat API: allow caller to provide a systemPrompt and control thinking output.
    public func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        let system: String
        if let systemPrompt {
            system = systemPrompt
        } else {
            system = try PromptTemplates.chatSystem
        }

        Logger.shared.log("LocalProvider.chat using model: \(selectedModel.name)", level: .debug)

        // Delegate to MLXService to generate a complete response honoring systemPrompt and thinking flag
        do {
            let resp = try await generateComplete(
                prompt: message,
                systemPrompt: system,
                enableThinking: enableThinking
            )
            return enableThinking ? resp : ChatThinkingParser.splitThinkingBlocks(from: resp).response
        } catch {
            Logger.shared.log("LocalProvider.chat failed: \(error)", level: .error)
            throw error
        }
    }

    // Streamed generation - exposes the underlying MLXService stream directly.
    public func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        // Detect chat mode by checking if candidates is empty and message is in userPrefs
        let isChat = input.candidates.isEmpty && input.userPrefs["message"] != nil
        var corePreparedPrompt: (system: String, prompt: String)?
        
        // Build prompt based on mode
        let promptResult: Result<String, Error>
        do {
            corePreparedPrompt = try decodePreparedCorePrompt(input: input)
            if isChat {
                promptResult = .success(corePreparedPrompt?.prompt ?? input.userPrefs["message"] ?? "")
            } else {
                // Summarization mode: structured JSON
                let p = try buildPrompt(input: input)
                promptResult = .success(p)
            }
        } catch {
            corePreparedPrompt = nil
            promptResult = .failure(error)
        }

        let style = PromptResponseStyle(rawValue: input.userPrefs["response_style"] ?? "") ?? .casual
        let customSystemPrompt = input.userPrefs["custom_system_prompt"]

        return AsyncThrowingStream { continuation in
            Task {
                switch promptResult {
                case .failure(let err):
                    continuation.finish(throwing: err)
                    return
                case .success(let prompt):
                    do {
                        let systemPrompt: String
                        if !isChat {
                            systemPrompt = PromptTemplates.summarizationSystem
                        } else if let corePreparedPrompt {
                            systemPrompt = corePreparedPrompt.system
                        } else {
                            systemPrompt = try PromptTemplates.chatSystem(
                                responseStyle: style,
                                customSystemPrompt: customSystemPrompt
                            )
                        }
                        let stream = try await self.generationStream(prompt: prompt, systemPrompt: systemPrompt, enableThinking: self.enableThinking)
                        for try await chunk in stream {
                            continuation.yield(chunk)
                        }
                        continuation.finish()
                    } catch {
                        continuation.finish(throwing: error)
                    }
                }
            }
        }
    }
    
    // MARK: - Prompt Building

    private func decodePreparedCorePrompt(input: ModelInput) throws -> (system: String, prompt: String)? {
        guard input.userPrefs["prepared_core_payload"] == "true" else { return nil }
        guard let rawPayload = input.userPrefs["prepared_core_payload_json"],
              let data = rawPayload.data(using: .utf8),
              let envelope = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let systemPrompt = envelope["system_prompt"] as? String,
              !systemPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let rawMessages = envelope["messages"] as? [[String: Any]] else {
            throw NolaError.invalidInput(reason: "Invalid prepared Core prompt envelope")
        }

        let conversation = rawMessages.filter { message in
            (message["role"] as? String) != "system"
        }
        let conversationData = try JSONSerialization.data(
            withJSONObject: conversation,
            options: [.sortedKeys, .withoutEscapingSlashes]
        )
        guard let conversationJSON = String(data: conversationData, encoding: .utf8) else {
            throw NolaError.invalidInput(reason: "Failed to encode prepared Core conversation")
        }
        return (
            system: systemPrompt,
            prompt: """
            Continue the typed conversation below and answer the latest user message. Preserve role and tool-call boundaries exactly.
            <conversation_json>
            \(conversationJSON)
            </conversation_json>
            """
        )
    }
    
    private func buildPrompt(input: ModelInput) throws -> String {
        // Convert input to JSON
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let inputData = try encoder.encode(input)
        guard let inputJSON = String(data: inputData, encoding: .utf8) else {
            throw NolaError.invalidInput(reason: "Failed to encode input as JSON")
        }
        
        // Build prompt using template
        let userPrompt = "\(PromptTemplates.summarizationUserPrefix)\(inputJSON)"
        
        return userPrompt
    }
    
    // MARK: - Response Parsing
    
    private func parseResponse(_ response: String) throws -> ModelOutput {
        // Extract JSON from response, removing common model artifacts
        var cleanedResponse = response
            .replacingOccurrences(of: "<|im_end|>", with: "")
            .replacingOccurrences(of: "</s>", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        
        // Remove all <think>...</think> blocks (reasoning models)
                while let thinkStart = cleanedResponse.range(of: "<think>"),
                            let thinkEnd = cleanedResponse.range(of: "</think>", range: thinkStart.upperBound..<cleanedResponse.endIndex) {
                        cleanedResponse.removeSubrange(thinkStart.lowerBound..<thinkEnd.upperBound)
                        cleanedResponse = cleanedResponse.trimmingCharacters(in: .whitespacesAndNewlines)
                }
        
        // Remove markdown code block markers if present
        cleanedResponse = cleanedResponse
            .replacingOccurrences(of: "```json", with: "")
            .replacingOccurrences(of: "```", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        
        // Try to find JSON object in response
        guard let jsonStart = cleanedResponse.firstIndex(of: "{"),
              let jsonEnd = cleanedResponse.lastIndex(of: "}") else {
            Logger.shared.log("No JSON found in response", level: .warn)
            Logger.shared.log("Response was: \(response)", level: .debug)
            return ModelOutput.empty
        }
        
        let jsonSubstring = String(cleanedResponse[jsonStart...jsonEnd])
        
        // Decode JSON
        guard let jsonData = jsonSubstring.data(using: .utf8) else {
            throw NolaError.invalidResponse(reason: "Failed to convert JSON string to data")
        }
        
        do {
            let decoder = JSONDecoder()
            let output = try decoder.decode(ModelOutput.self, from: jsonData)
            Logger.shared.log("Successfully parsed \(output.items.count) items", level: .info)
            return output
        } catch {
            Logger.shared.log("Failed to parse JSON: \(error.localizedDescription)", level: .error)
            Logger.shared.log("Extracted JSON was: \(jsonSubstring)", level: .debug)
            throw NolaError.invalidResponse(reason: "Failed to decode ModelOutput: \(error.localizedDescription)")
        }
    }
    
    // MARK: - Timeout Helper
    
    private func withTimeout<T>(_ timeout: TimeInterval, operation: @escaping () async throws -> T) async throws -> T {
        try await withThrowingTaskGroup(of: T.self) { group in
            // Start the actual operation
            group.addTask {
                try await operation()
            }
            
            // Start the timeout task
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                await Logger.shared.log("Operation timed out after \(timeout)s", level: .warn)
                throw NolaError.timeout
            }
            
            // Wait for the first task to complete (either success or timeout)
            guard let result = try await group.next() else {
                throw NolaError.timeout
            }
            
            // Cancel any remaining tasks
            group.cancelAll()
            return result
        }
    }
    
    // MARK: - Cleanup

}
