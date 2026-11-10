import Foundation
import Testing
@testable import Nola

@MainActor
struct LLMAgentCapabilitiesTests {
    @Test
    func eventStreamDoesNotGrantUndeclaredToolsOrReasoning() throws {
        let provider = OpenAIProvider(config: .init(baseURL: try #require(URL(string: "https://fixture.invalid/v1")),
            apiKey: "fixture", model: "model", wireProtocol: .responses, maxCompletionTokens: 64))
        #expect(provider.supportsNativeEventStream)
        #expect(provider.agentCapabilities == .init(maxOutputTokens: 64, streamText: true, streamReasoning: true))
    }

    @Test
    func receivingProviderReasoningDoesNotRequestReasoningGeneration() {
        let transport = AgentModelInvocationPlan(reasoning: false, streamReasoning: true)
        #expect(!transport.reasoning)
        #expect(transport.streamReasoning)
        let declared = LLMAgentCapabilities(reasoning: false, streamReasoning: true)
        #expect(!declared.reasoning)
        #expect(declared.streamReasoning)
        #expect(!declared.reasoningContinuity)
    }

    @Test
    func capabilitiesComeFromFrozenConfigurationNotLaterCallerChanges() throws {
        var config = OpenAIProvider.Config(baseURL: try #require(URL(string: "https://fixture.invalid/v1")),
            apiKey: "fixture", model: "model", executionCapabilities: .init(toolCalls: true, imageInput: true, reasoning: true),
            maxCompletionTokens: 64,
            promptCacheEnabled: true)
        let provider = OpenAIProvider(config: config)
        config.executionCapabilities = .init()
        config.promptCacheEnabled = false
        config.maxCompletionTokens = 64000
        #expect(provider.agentCapabilities == .init(visionInput: true, maxOutputTokens: 64, nativeToolCalls: true, reasoning: true,
            promptCache: true, streamText: true, streamReasoning: true, streamToolCalls: true))
        #expect(!provider.agentCapabilities.parallelToolCalls)
        #expect(!provider.agentCapabilities.reasoningContinuity)
        #expect(!provider.agentCapabilities.requestContinuation)
    }

    @Test
    func cacheCapabilityMatchesExistingConfiguredAndDefaultExecutionPolicy() throws {
        for (host, explicit, expected) in [
            ("api.openai.com", Optional<Bool>.none, true),
            ("fixture.invalid", nil, false),
            ("api.openai.com", false, false),
            ("fixture.invalid", true, true),
        ] {
            let provider = OpenAIProvider(config: .init(baseURL: try #require(URL(string: "https://\(host)/v1")),
                apiKey: "fixture", model: "model", promptCacheEnabled: explicit))
            #expect(provider.agentCapabilities.promptCache == expected)
        }
    }

    @Test
    func unsupportedAdapterAndInconsistentSubCapabilitiesFailClosed() throws {
        let provider = OpenAIProvider(config: .init(baseURL: try #require(URL(string: "https://fixture.invalid")),
            apiKey: "fixture", model: "model", wireProtocol: .anthropic,
            executionCapabilities: .init(toolCalls: true, reasoning: true), promptCacheEnabled: true))
        #expect(provider.agentCapabilities == .init())
        #expect(LLMAgentCapabilities(parallelToolCalls: true, reasoningContinuity: true,
            streamReasoning: true, streamToolCalls: true, requestContinuation: true) == .init(streamReasoning: true))
    }
}
