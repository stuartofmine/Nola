import Foundation
import MLXLMCommon
import Testing
@testable import Nola

@MainActor
struct IOSLocalRunConfigurationTests {
    private let model = LMModel(name: "fixture/local", configuration: ModelConfiguration(id: "fixture/local"), type: .llm)

    @Test
    func runOwnsSessionValuesAcrossLaterDefaultChangesAndCalls() async throws {
        var defaults = ModelParameters.localDefaults
        var session = IOSConversationGenerationOverride(maxContextLength: 4096, maxOutputTokens: 64)
            .applying(to: defaults)
        session.temperature = 0
        session.topP = 0
        session.topK = 1
        session.presencePenalty = -0.5
        session.frequencyPenalty = 0.75
        session.repetitionPenalty = 1.2
        session.seed = 0
        session.showThinking = false
        var requests: [LocalProviderGenerationRequest] = []
        let provider = try IOSConversationRunConfiguration.localProvider(
            model: identity, parameters: session, availableModels: [model], generation: { request in
                requests.append(request)
                return AsyncThrowingStream { $0.yield("answer"); $0.finish() }
            })
        _ = try await provider.chat(message: "first", systemPrompt: "system", enableThinking: true, timeout: nil)
        #expect(provider.agentCapabilities == .init(maxOutputTokens: 64, reasoning: false))
        defaults.temperature = 1.8
        defaults.maxOutputTokens = 64000
        session = defaults
        let laterProvider = try IOSConversationRunConfiguration.localProvider(
            model: identity, parameters: defaults, availableModels: [model], generation: { request in
                requests.append(request)
                return AsyncThrowingStream { $0.yield("later"); $0.finish() }
            })
        _ = try await laterProvider.chat(message: "later", systemPrompt: "system", enableThinking: true, timeout: nil)
        _ = try await provider.chat(message: "second", systemPrompt: "system", enableThinking: true, timeout: nil)
        #expect(provider.agentCapabilities.maxOutputTokens == 64)
        #expect(!provider.agentCapabilities.nativeToolCalls)
        #expect(requests.count == 3)
        for request in [requests[0], requests[2]] {
            #expect(request.model.name == model.name)
            #expect(request.parameters.maxOutputTokens == 64)
            #expect(request.parameters.maxContextLength == 4096)
            #expect(request.parameters.temperature == 0)
            #expect(request.parameters.topP == 0)
            #expect(request.parameters.topK == 1)
            #expect(request.parameters.presencePenalty == -0.5)
            #expect(request.parameters.frequencyPenalty == 0.75)
            #expect(request.parameters.repetitionPenalty == 1.2)
            #expect(request.parameters.seed == 0)
            #expect(!request.enableThinking)
        }
        #expect(requests[1].parameters.temperature == 1.8)
        #expect(requests[1].parameters.maxOutputTokens == 64000)
    }

    @Test
    func invalidOrUnsupportedValuesRejectBeforeGeneration() throws {
        let mutations: [(inout ModelParameters) -> Void] = [
            { $0.maxOutputTokens = 0 }, { $0.maxContextLength = 0 },
            { $0.temperature = .nan }, { $0.topP = 1.1 }, { $0.topK = 0 },
            { $0.presencePenalty = 3 }, { $0.frequencyPenalty = -.infinity },
            { $0.repetitionPenalty = 0 }, { $0.stopSequences = "STOP" }
        ]
        for mutate in mutations {
            var parameters = ModelParameters.localDefaults
            mutate(&parameters)
            #expect(throws: Error.self) {
                try IOSConversationRunConfiguration.localProvider(model: identity, parameters: parameters,
                    availableModels: [model], generation: { _ in
                        Issue.record("Invalid configuration reached generation")
                        return AsyncThrowingStream { $0.finish() }
                    })
            }
        }
    }

    @Test
    func unavailableModelDoesNotFallBackToAnotherLoadedModel() throws {
        #expect(throws: Error.self) {
            try IOSConversationRunConfiguration.localProvider(
                model: ModelInfo(id: "absent", name: "Absent", source: "Local", isLocal: true),
                parameters: .localDefaults, availableModels: [model])
        }
    }

    @Test
    func catalogNameAliasKeepsActualTargetAndFrozenGenerationTogether() async throws {
        let alias = ModelDownloader.ModelType.qwen3_5_0_8b
        let target = LMModel(name: try #require(alias.mlxModelName),
            configuration: ModelConfiguration(id: "fixture/actual-catalog-target"), type: .llm)
        var parameters = ModelParameters.localDefaults
        parameters.maxOutputTokens = 64
        parameters.temperature = 0
        var observed: LocalProviderGenerationRequest?
        let provider = try IOSConversationRunConfiguration.localProvider(
            model: ModelInfo(id: alias.rawValue, name: "UI alias", source: "Local", isLocal: true),
            parameters: parameters, availableModels: [model, target], generation: { request in
                observed = request
                return AsyncThrowingStream { $0.yield("fixture"); $0.finish() }
            })
        parameters.maxOutputTokens = 64000
        _ = try await provider.chat(message: "fixture", systemPrompt: "fixture", enableThinking: true, timeout: nil)
        let request = try #require(observed)
        #expect(provider.resolvedModelIdentity.id == target.id)
        #expect(provider.resolvedModelIdentity.name == target.name)
        #expect(provider.resolvedModelIdentity.id == request.model.id)
        #expect(provider.resolvedModelIdentity.name == request.model.name)
        #expect(request.parameters.maxOutputTokens == 64)
        #expect(request.parameters.temperature == 0)
    }

    private var identity: ModelInfo {
        ModelInfo(id: model.name, name: "Fixture", source: "Local", isLocal: true)
    }
}
