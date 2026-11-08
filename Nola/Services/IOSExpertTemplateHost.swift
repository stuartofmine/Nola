// SPDX-License-Identifier: Apache-2.0
// Native Host capture provider; custom bodies are resolved transactionally by Core.
import Foundation
import CryptoKit

/// One parameter producer for preview capture and actual native expert dispatch.
nonisolated enum IOSExpertInstructionProducer {
    static func model(declaration: AppleExpertModelDeclaration<AgentCoreJSONValue>, defaults: AgentCoreJSONValue,
                      presentation: AgentCoreJSONValue = .null, store: IOSApplicationRuntime) throws -> (ModelInfo, ModelParameters) {
        guard let profile = try store.modelProfile(id: declaration.profile_id), profile.enabled, profile.deletedAtMs == nil,
              profile.revision == declaration.profile_revision, profile.provider == declaration.provider,
              profile.modelName == declaration.model_name, profile.runtime == declaration.runtime,
              let model = ModelSelectionManager.modelInfo(from: profile), !model.isWebBackend,
              try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(profile.parametersJSON.utf8)) == declaration.parameters else {
            throw IOSApplicationRuntimeError.rejected("The previewed expert model configuration changed")
        }
        var parameters = try ModelParameters.decodeProfile(String(decoding: JSONEncoder().encode(declaration.parameters), as: UTF8.self),
            inheriting: .defaults(isLocal: model.isLocal))
        let data = try JSONEncoder().encode(defaults)
        guard let values = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw IOSApplicationRuntimeError.invalidResponse }
        let key = IOSConversationGenerationOverride.modelKey(for: model)
        if let overrides = values["modelOverrides"] as? [String: Any], overrides[key] != nil {
            parameters = try ConversationParameterSnapshot.applyingOverrides(String(decoding: data, as: UTF8.self), modelKey: key, to: parameters)
        }
        if let limit = values["contextLimitOverride"] as? Int { parameters.maxContextLength = limit }
        parameters = try ConversationParameterSnapshot.applyingPresentation(presentation, to: parameters)
        if let style = (values["promptResponseStyle"] as? String).flatMap(PromptResponseStyle.init(rawValue:)) { parameters.responseStyle = style }
        return (model, parameters)
    }

    static func definitions(parameters: ModelParameters) -> [AppleExpertPromptDefinition<AgentCoreJSONValue>] {
        var instructions = [("ios:user-response-style", PromptTemplates.agentResponseStyle(parameters.responseStyle))]
        let custom = parameters.customSystemPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if !custom.isEmpty { instructions.append(("ios:user-custom-instructions", custom)) }
        return instructions.map { id, content in .init(component_id: id, content: content,
            content_sha256: SHA256.hash(data: Data(content.utf8)).map { String(format: "%02x", $0) }.joined(),
            definition_revision: nil, platform: .string("ios")) }
    }
}

nonisolated extension IOSApplicationRuntime {
    func admitExpertMessage(_ input: AppleAdmitExpertConversationMessage) throws -> AppleExpertMessageManifest<AgentCoreJSONValue> {
        let authority = AppleExpertTemplateAuthority.local()
        let prior: AppleExpertMessageManifest<AgentCoreJSONValue>? = try expertHostOperation(
            .loadManifest(conversationID: input.conversation_id, messageID: input.user_message_id, authority: authority))
        if let prior {
            guard prior.input == input else { throw IOSApplicationRuntimeError.rejected("The original expert intent changed") }
            if let recovered = try reconcilePendingExpertAdmission(input) {
                guard recovered.input == prior.input else { throw IOSApplicationRuntimeError.invalidResponse }
                return recovered
            }
            return prior
        }
        let preview: AppleExpertCapturePreview<AgentCoreJSONValue> = try expertHostOperation(.preview(input: input, authority: authority))
        guard preview.participants.map(\.participant) == input.selected_participants else { throw IOSApplicationRuntimeError.invalidResponse }
        let builtinCatalog = BuiltInPromptTemplates.templates()
        let snapshot: PromptTemplateSnapshot?
        if let setting = try productSetting(key: "prompt.templates") {
            snapshot = try JSONDecoder().decode(PromptTemplateSnapshot.self, from: Data(setting.valueJSON.utf8))
        } else { snapshot = nil }
        let selections = try preview.participants.map { participant -> AppleExpertCaptureSelection<AgentCoreJSONValue> in
            let declaration = participant.model_declaration
            let (_, parameters) = try IOSExpertInstructionProducer.model(declaration: declaration, defaults: participant.defaults,
                presentation: participant.client_model_presentation, store: self)
            guard case .object(let defaults) = participant.defaults else { throw IOSApplicationRuntimeError.invalidResponse }
            let selected: String?
            let source: AppleExpertCaptureSelection<AgentCoreJSONValue>.Source
            if let value = defaults["selectedPromptTemplateID"], value != .null {
                guard case .string(let id) = value, UUID(uuidString: id) != nil else { throw IOSApplicationRuntimeError.invalidResponse }
                selected = id; source = .explicit
            } else if let value = defaults["language"], value != .null {
                guard case .string(let language) = value else { throw IOSApplicationRuntimeError.invalidResponse }
                if let global = snapshot?.selectedTemplateID { selected = global.uuidString }
                else {
                    selected = try PromptTemplateResolution.defaultTemplate(language: language,
                        catalog: builtinCatalog + (snapshot?.customTemplates ?? []), builtins: builtinCatalog)?.id.uuidString
                }
                source = selected == nil ? .noTemplate : .hostDefault
            } else { selected = nil; source = .noTemplate }
            return .init(context_conversation_id: participant.participant.context_conversation_id,
                context_configuration_revision: participant.context_configuration_revision,
                model_profile: .init(profile_id: declaration.profile_id, admitted_revision: declaration.profile_revision),
                source: source, template_id: selected,
                host_user_instructions: IOSExpertInstructionProducer.definitions(parameters: parameters))
        }
        return try expertHostOperation(.admit(input: input, context: .init(actor: authority.actor,
            request_host_id: authority.request_host_id, selections: selections,
            builtins: builtinCatalog.map { .init(provider_id: "apple.prompt-templates", template_id: $0.id.uuidString,
                content: $0.content, platform: .string("ios")) })))
    }
}
