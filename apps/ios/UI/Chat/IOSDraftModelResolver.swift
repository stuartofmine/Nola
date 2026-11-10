// SPDX-License-Identifier: Apache-2.0
import Foundation

/// Drafts persist directory identity and parameter values, never credentials.
nonisolated enum IOSDraftModelResolver {
    static func resolve(_ configuration: ConversationComposerDraftStore.ModelConfiguration,
                        catalog: [ModelInfo]) throws -> ModelInfo {
        guard let model = catalog.first(where: {
            let source: ConversationComposerDraftStore.ModelConfiguration.Source = $0.isWebBackend ? .host : $0.isLocal ? .local : .configured
            return $0.id == configuration.modelID && source == configuration.source
        }),
              ModelParameterLimits.remoteContextTokens.contains(configuration.parameters.maxContextLength),
              ModelParameterLimits.outputTokens.contains(configuration.parameters.maxOutputTokens) else {
            throw IOSApplicationRuntimeError.rejected("The saved draft model is unavailable or its parameters are invalid. Select a model explicitly.")
        }
        return model
    }
}
