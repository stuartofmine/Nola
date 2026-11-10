// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MLXService logic in the iOS LLM integration layer.
// - Interfaces:
//   - MLXService
//   - HubApi
// - APIs:
//   - none
//
//
//  MLXService.swift
//  Nola
//
//  Created by Nola Team
//  Based on the MLXChatExample implementation
//

import Foundation
import MLX
import MLXNN
import MLXRandom
import MLXLLM
import MLXLMCommon
#if !os(macOS)
import MLXVLM
#endif
import Hub

/// A service class that manages machine learning models for text generation.
/// This class handles model loading, caching, and text generation using MLX models.
@Observable
class MLXService {
    /// Shared singleton instance for reuse across the app.
    public static let shared = MLXService()

    // Make init private to enforce singleton usage
    private init() {}
    /// List of available models that can be used for generation.
    static let availableModels: [LMModel] = [
        LMModel(name: "mlx-community/Qwen3.5-0.8B-MLX-4bit", configuration: ModelConfiguration(id: "mlx-community/Qwen3.5-0.8B-MLX-4bit"), type: .llm),
        LMModel(name: "mlx-community/Qwen3.5-4B-MLX-4bit", configuration: ModelConfiguration(id: "mlx-community/Qwen3.5-4B-MLX-4bit"), type: .llm),
        LMModel(name: "qwen3:1.7b", configuration: LLMRegistry.qwen3_1_7b_4bit, type: .llm),
        LMModel(name: "qwen2.5:1.5b", configuration: LLMRegistry.qwen2_5_1_5b, type: .llm),
        LMModel(name: "llama3.2:1b", configuration: LLMRegistry.llama3_2_1B_4bit, type: .llm),
        LMModel(name: "smolLM:135m", configuration: LLMRegistry.smolLM_135M_4bit, type: .llm),
    ]

    /// Cache to store loaded model containers to avoid reloading.
    private let modelCache = NSCache<NSString, ModelContainer>()

    /// Selection and runtime lifecycle are intentionally separate. A selected model
    /// does not consume memory until the user explicitly starts it.
    private(set) var activeModelName: String?
    private(set) var loadingModelName: String?
    private(set) var lastStartError: String?

    var isModelRunning: Bool {
        activeModelName != nil
    }

    func isRunning(_ model: LMModel) -> Bool {
        activeModelName == model.name
    }

    func start(model: LMModel) async throws {
        guard activeModelName != model.name else { return }
        loadingModelName = model.name
        lastStartError = nil
        defer { loadingModelName = nil }

        do {
            if activeModelName != nil {
                modelCache.removeAllObjects()
            }
            _ = try await load(model: model)
            activeModelName = model.name
        } catch {
            lastStartError = error.localizedDescription
            throw error
        }
    }

    func stop() {
        modelCache.removeAllObjects()
        activeModelName = nil
        loadingModelName = nil
        lastStartError = nil
        Logger.shared.log("Local model runtime stopped", level: .info)
    }

    /// Tracks the current model download progress.
    @MainActor
    private(set) var modelDownloadProgress: Progress?

    /// Loads a model from the hub or retrieves it from cache.
    /// - Parameter model: The model configuration to load
    /// - Returns: A ModelContainer instance containing the loaded model
    /// - Throws: Errors that might occur during model loading
    private func load(model: LMModel) async throws -> ModelContainer {
        // Set GPU memory limit to prevent out of memory issues
        MLX.Memory.cacheLimit = 20 * 1024 * 1024

        // Return cached model if available to avoid reloading
        if let container = modelCache.object(forKey: model.name as NSString) {
            Logger.shared.log("Using cached model: \(model.name)", level: .info)
            return container
        } else {
            Logger.shared.log("Loading model: \(model.name)", level: .info)
            
            // Select appropriate factory based on model type
            let factory: ModelFactory
            switch model.type {
                case .llm:
                    factory = LLMModelFactory.shared
                case .vlm:
#if os(macOS)
                    throw NSError(
                        domain: "Nola.LocalModel",
                        code: 2,
                        userInfo: [NSLocalizedDescriptionKey: "Vision local models are not hosted in the macOS app process."]
                    )
#else
                    factory = VLMModelFactory.shared
#endif
                }

            // Load model and track download progress
            let container = try await factory.loadContainer(
                hub: .defaultHub, configuration: model.configuration
            ) { progress in
                Task { @MainActor in
                    self.modelDownloadProgress = progress
                }
            }

            // Cache the loaded model for future use
            modelCache.setObject(container, forKey: model.name as NSString)
            Logger.shared.log("Model loaded successfully: \(model.name)", level: .info)

            return container
        }
    }

    /// Generates text based on the provided prompt using the specified model.
    /// - Parameters:
    ///   - prompt: The input prompt text
    ///   - systemPrompt: Optional system prompt to set context
    ///   - model: The language model to use for generation
    ///   - temperature: Sampling temperature (default: 0.7)
    ///   - maxTokens: Maximum tokens to generate (default: 512)
    /// - Returns: An AsyncThrowingStream of generated text chunks
    /// - Throws: Errors that might occur during generation
    func generate(
        prompt: String,
        systemPrompt: String? = nil,
        model: LMModel,
        temperature: Float = 0.7,
        maxTokens: Int = 512,
        topP: Float = 0.9,
        topK: Int? = nil,
        enableThinking: Bool = true,
        repetitionPenalty: Float = 1.1,
        maxContextTokens: Int? = nil,
        seed: Int? = nil,
        presencePenalty: Float = 0,
        frequencyPenalty: Float = 0
    ) async throws -> AsyncThrowingStream<String, Error> {
        guard isRunning(model) else {
            throw NSError(
                domain: "Nola.LocalModel",
                code: 1,
                userInfo: [
                    NSLocalizedDescriptionKey: NSLocalizedString(
                        "model.local.not_started.error",
                        value: "Start the selected local model in Model Center before chatting.",
                        comment: "Local model is selected but its runtime is stopped"
                    )
                ]
            )
        }

        // Explicit start has already populated the cache.
        let modelContainer = try await load(model: model)

        let effectivePrompt: String
        if !enableThinking, model.name.localizedCaseInsensitiveContains("qwen3") {
            effectivePrompt = prompt + "\n/no_think"
        } else {
            effectivePrompt = prompt
        }
        if let seed {
            MLXRandom.seed(UInt64(bitPattern: Int64(seed)))
        }

        // Generate response using the model
        return try await modelContainer.perform { (context: ModelContext) in
            var messages: [Chat.Message] = []
            if let systemPrompt {
                messages.append(Chat.Message(role: .system, content: systemPrompt))
            }
            messages.append(Chat.Message(role: .user, content: effectivePrompt))
            let lmInput = try await context.processor.prepare(input: UserInput(chat: messages))
            let parameters = GenerateParameters(
                maxTokens: maxTokens,
                maxKVSize: maxContextTokens,
                temperature: temperature,
                topP: topP,
                topK: topK ?? 0,
                repetitionPenalty: repetitionPenalty,
                presencePenalty: presencePenalty,
                frequencyPenalty: frequencyPenalty
            )

            let stream = try MLXLMCommon.generate(
                input: lmInput, parameters: parameters, context: context)
            
            // Transform Generation stream to String stream
            return AsyncThrowingStream<String, Error> { continuation in
                Task {
                    for await generation in stream {
                        switch generation {
                        case .chunk(let chunk):
                            continuation.yield(chunk)
                        case .info(let info):
                            await Logger.shared.log(
                                "Generation complete: \(info.tokensPerSecond) tokens/sec",
                                level: .debug
                            )
                        case .toolCall:
                            break
                        }
                    }
                    continuation.finish()
                }
            }
        }
    }

    /// Generates a complete response as a single string.
    /// - Parameters:
    ///   - prompt: The input prompt text
    ///   - systemPrompt: Optional system prompt to set context
    ///   - model: The language model to use for generation
    ///   - temperature: Sampling temperature (default: 0.7)
    ///   - maxTokens: Maximum tokens to generate (default: 512)
    /// - Returns: The complete generated text
    /// - Throws: Errors that might occur during generation
    func generateComplete(
        prompt: String,
        systemPrompt: String? = nil,
        model: LMModel,
        temperature: Float = 0.7,
        maxTokens: Int = 512,
        topP: Float = 0.9,
        topK: Int? = nil,
        enableThinking: Bool = true,
        repetitionPenalty: Float = 1.1,
        maxContextTokens: Int? = nil,
        seed: Int? = nil,
        presencePenalty: Float = 0,
        frequencyPenalty: Float = 0
    ) async throws -> String {
        var result = ""
        let stream = try await generate(
            prompt: prompt,
            systemPrompt: systemPrompt,
            model: model,
            temperature: temperature,
            maxTokens: maxTokens,
            topP: topP,
            topK: topK,
            enableThinking: enableThinking,
            repetitionPenalty: repetitionPenalty,
            maxContextTokens: maxContextTokens,
            seed: seed,
            presencePenalty: presencePenalty,
            frequencyPenalty: frequencyPenalty
        )
        
        for try await chunk in stream {
            result += chunk
        }
        
        return result
    }
    
    /// Clears the model cache to free memory
    func clearCache() {
        modelCache.removeAllObjects()
        activeModelName = nil
        loadingModelName = nil
        Logger.shared.log("Model cache cleared", level: .info)
    }
}

// MARK: - HubApi Extension

extension HubApi {
    /// Default HubApi instance configured to download models to the cache directory
    static let defaultHub: HubApi = {
        #if os(macOS)
        return HubApi(
            downloadBase: URL.downloadsDirectory.appending(path: "huggingface")
        )
        #else
        guard let cacheDirectory = FileManager.default.urls(
            for: .cachesDirectory,
            in: .userDomainMask
        ).first else {
            preconditionFailure("Unable to locate caches directory for MLX model downloads")
        }
        return HubApi(
            downloadBase: cacheDirectory.appending(path: "huggingface")
        )
        #endif
    }()
}
