import Foundation
import Testing
@testable import Nola

@MainActor
struct OpenAIProviderConfigurationValidationTests {
    private func configuration(_ wireProtocol: ModelWireProtocol = .chatCompletions) -> OpenAIProvider.Config {
        .init(baseURL: URL(string: "https://fixture.invalid/v1")!, apiKey: "fixture", model: "fixture",
            wireProtocol: wireProtocol, executionCapabilities: .init(samplingParameters: true))
    }

    @Test
    func sharedValidationRejectsInvalidNumericAndStopSchemaForBothAdapters() {
        let mutations: [(inout OpenAIProvider.Config) -> Void] = [
            { $0.temperature = .nan }, { $0.temperature = 2.01 },
            { $0.topP = -.infinity }, { $0.topP = 1.01 },
            { $0.maxCompletionTokens = 0 }, { $0.maxCompletionTokens = 1_000_001 },
            { $0.presencePenalty = 2.01 }, { $0.frequencyPenalty = .infinity },
            { $0.stopSequences = [""] }, { $0.stopSequences = Array(repeating: "s", count: 17) },
            { $0.stopSequences = [String(repeating: "界", count: 683)] },
        ]
        for wireProtocol in [ModelWireProtocol.chatCompletions, .responses] {
            for mutate in mutations {
                var config = configuration(wireProtocol)
                mutate(&config)
                #expect(throws: Error.self) { try OpenAIProvider.validateGenerationConfiguration(config) }
            }
        }
    }

    @Test
    func validBoundaryValuesAreNotRewritten() throws {
        var config = configuration()
        config.temperature = 0
        config.topP = 0
        config.maxCompletionTokens = 64
        config.presencePenalty = -2
        config.frequencyPenalty = 2
        config.seed = Int.max
        config.stopSequences = ["  exact  "]
        let frozen = config
        try OpenAIProvider.validateGenerationConfiguration(config)
        #expect(config == frozen)
    }

    @Test
    func declaredSamplingAndConditionalContractRemainRequired() throws {
        var config = configuration()
        config.executionCapabilities = .init()
        config.temperature = 0.7
        config.topP = 0.9
        try OpenAIProvider.validateGenerationConfiguration(config)
        config.temperature = 0
        #expect(throws: Error.self) { try OpenAIProvider.validateGenerationConfiguration(config) }
        config.executionCapabilities = .init(samplingParameters: true, samplingRequiresNoReasoning: true)
        #expect(throws: Error.self) { try OpenAIProvider.validateGenerationConfiguration(config) }
    }

    @Test
    func responsesRejectsHostUnsupportedParametersAndUnsupportedWire() throws {
        try OpenAIProvider.validateGenerationConfiguration(configuration(.responses))
        let mutations: [(inout OpenAIProvider.Config) -> Void] = [
            { $0.presencePenalty = 0.1 }, { $0.frequencyPenalty = -0.1 },
            { $0.seed = 0 }, { $0.stopSequences = ["stop"] },
            { $0.executionCapabilities.samplingRequiresNoReasoning = true },
        ]
        for mutate in mutations {
            var config = configuration(.responses)
            mutate(&config)
            #expect(throws: Error.self) { try OpenAIProvider.validateGenerationConfiguration(config) }
        }
        #expect(throws: Error.self) { try OpenAIProvider.validateGenerationConfiguration(configuration(.anthropic)) }
    }
}
