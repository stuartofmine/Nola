import Foundation
import CryptoKit

/// Non-secret model facts from the provider that will execute this Run.
nonisolated struct IOSPreparedHostInvocation: Sendable {
    let route: AgentCoreJSONValue
    let generation: AgentCoreJSONValue
    let outputAdmission: AgentCoreJSONValue

    static func skillIdentity(_ skills: [IOSAgentSkillSnapshot]) -> AgentCoreJSONValue {
        .array(skills.sorted { $0.id < $1.id }.map { skill in .object([
            "id": .string(skill.id), "name": .string(skill.name), "summary": .string(skill.summary),
            "instructions": .string(skill.instructions), "version": skill.version.map(AgentCoreJSONValue.string) ?? .null,
            "required_tools": .array(skill.requiredTools.map(AgentCoreJSONValue.string)),
            "validation_steps": .array(skill.validationSteps.map(AgentCoreJSONValue.string)),
        ]) })
    }

    static func decodeSkillIdentity(_ value: AgentCoreJSONValue) throws -> [IOSAgentSkillSnapshot] {
        guard case .array(let values) = value else { throw IOSApplicationRuntimeError.invalidResponse }
        var seen = Set<String>()
        return try values.map { entry in
            guard let fields = entry.objectFields,
                  Set(fields.keys) == Set(["id", "name", "summary", "instructions", "version", "required_tools", "validation_steps"]),
                  let id = fields["id"]?.stringScalar, !id.isEmpty, seen.insert(id).inserted,
                  let name = fields["name"]?.stringScalar, let summary = fields["summary"]?.stringScalar,
                  let instructions = fields["instructions"]?.stringScalar,
                  let version = fields["version"], version == .null || version.stringScalar != nil,
                  case .array(let required)? = fields["required_tools"],
                  case .array(let validation)? = fields["validation_steps"],
                  required.allSatisfy({ $0.stringScalar != nil }), validation.allSatisfy({ $0.stringScalar != nil }) else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            return .init(id: id, name: name, summary: summary, instructions: instructions, version: version.stringScalar,
                requiredTools: required.compactMap(\.stringScalar), validationSteps: validation.compactMap(\.stringScalar))
        }
    }

    func binding(executionPolicy: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let digest = SHA256.hash(data: try encoder.encode(executionPolicy))
            .map { String(format: "%02x", $0) }.joined()
        return .object([
            "version": .number(8), "primary": route, "fallbacks": .array([]),
            "generation": generation, "output_admission": outputAdmission,
            "execution_policy_sha256": .string(digest),
        ])
    }
}

/// In-memory owner for one new Run. This is not a durable recovery binding.
@MainActor
struct IOSPreparedRunModel {
    private struct RecoveryGeneration: Decodable {
        struct Options: Decodable {
            let topK: Int?
            let repetitionPenalty: Double?
            let presencePenalty: Double?
            let frequencyPenalty: Double?
            let seed: Int?
            let stop: [String]?
        }
        let temperature: Double
        let topP: Double
        let maxOutputTokens: Int
        let thinkingEnabled: Bool
        let reasoningSelection: AgentCoreJSONValue?
        let reasoningTier: String
        let reasoningSummarySupported: Bool?
        let generationOptions: Options
    }
    private struct RecoveryContext: Decodable {
        let contextWindowTokens: Int
        let reservedOutputTokens: Int
    }
    /// Process-local execution provenance, not a persistence DTO. HTTP config
    /// contains credentials and query contents: never serialize or log it.
    enum ExecutionSource {
        case http(configuration: OpenAIProvider.Config, profile: IOSResolvedCloudRunProfile)
        /// Catalog selection only; this is not a weights/content identity.
        case native(logicalModelID: String, catalogIdentity: LocalProvider.ModelIdentity)
    }

    let client: LLMClient
    let parameters: ModelParameters
    let modelName: String
    let supportsToolCalls: Bool
    // Retain the exact source revision/credential reference used to construct
    // the client. This in-memory source is not yet a durable Host binding.
    let executionSource: ExecutionSource
    let outputAdmission: AgentCoreJSONValue
    var frozenModelCapability: AgentCoreJSONValue? = nil

    var cloudProfile: IOSResolvedCloudRunProfile? {
        if case .http(_, let profile) = executionSource { return profile }
        return nil
    }

    /// Rebuild from admitted values, obtaining secrets only from currently
    /// authorized local profiles. Exact route/generation comparison below also
    /// verifies credential and query digests without publishing either secret.
    static func recover(_ frozen: IOSExactRunInvocation, using store: IOSApplicationRuntime,
        credentials: IOSCloudCredentialOwner = .shared) throws -> IOSPreparedRunModel {
        try recover(hostInvocation: frozen.hostInvocation, contextBudget: frozen.contextBudget,
            using: store, credentials: credentials)
    }

    /// Model-only reconstruction for a Core-prepared child, before Run admission.
    static func recover(hostInvocation: AgentCoreJSONValue, contextBudget: AgentCoreJSONValue,
        using store: IOSApplicationRuntime, credentials: IOSCloudCredentialOwner = .shared) throws -> IOSPreparedRunModel {
        guard case .object(let binding) = hostInvocation,
              binding["version"] == .number(8),
              case .array(let fallbacks)? = binding["fallbacks"], fallbacks.isEmpty,
              case .object(let route)? = binding["primary"],
              let generation = binding["generation"], let output = binding["output_admission"] else {
            throw IOSApplicationRuntimeError.rejected("Unsupported frozen native Host invocation")
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let values = try decoder.decode(RecoveryGeneration.self, from: JSONEncoder().encode(generation))
        let context = try decoder.decode(RecoveryContext.self, from: JSONEncoder().encode(contextBudget))
        guard context.reservedOutputTokens == values.maxOutputTokens else {
            throw IOSApplicationRuntimeError.rejected("Frozen output admission and context budget differ")
        }
        var parameters = ModelParameters()
        parameters.maxContextLength = context.contextWindowTokens
        parameters.maxOutputTokens = values.maxOutputTokens
        parameters.temperature = values.temperature
        parameters.topP = values.topP
        parameters.showThinking = values.thinkingEnabled
        parameters.topK = values.generationOptions.topK
        parameters.repetitionPenalty = values.generationOptions.repetitionPenalty
        parameters.presencePenalty = values.generationOptions.presencePenalty ?? 0
        parameters.frequencyPenalty = values.generationOptions.frequencyPenalty ?? 0
        parameters.seed = values.generationOptions.seed
        parameters.stopSequences = (values.generationOptions.stop ?? []).joined(separator: "\n")
        let candidates: [ModelInfo]
        if route["kind"] == .string("native"), case .string(let modelID)? = route["model_id"] {
            candidates = [ModelInfo(id: modelID, name: modelID, source: "local", isLocal: true)]
        } else if route["kind"] == .string("http") {
            candidates = try ModelSelectionManager.availableModels(using: store).filter { !$0.isLocal && !$0.isWebBackend }
        } else {
            throw IOSApplicationRuntimeError.rejected("Unsupported frozen model route")
        }
        for model in candidates {
            // Never construct a provider from a new selected model or fallback.
            // A candidate is usable only when all admitted facts match exactly.
            guard let prepared = try? prepare(model: model, parameters: parameters,
                resolveCloud: {
                    var profile = try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: credentials)
                    profile.reasoningSelection = values.reasoningSelection
                    profile.reasoningTier = values.reasoningTier
                    profile.reasoningSummarySupported = values.reasoningSummarySupported
                    return profile
                },
                admitOutput: { candidate, tokens in
                    try IOSPreparedHostOutputAdmission.requireExact(output, route: candidate, outputTokens: tokens)
                    return output
                }),
                let captured = try? prepared.prepareHostInvocation(),
                captured.route == .object(route), captured.generation == generation,
                captured.outputAdmission == output else { continue }
            return prepared
        }
        throw IOSApplicationRuntimeError.rejected("The admitted model or credential is no longer available with its frozen parameters")
    }

    /// Fresh execution reconstructs only model facts, never an old Run or its budget/policy.
    static func reconstitute(_ frozen: AppleFrozenResendModel<AgentCoreJSONValue>, using store: IOSApplicationRuntime,
        credentials: IOSCloudCredentialOwner = .shared) throws -> IOSPreparedRunModel {
        guard frozen.version == 8, case .array(let fallbacks) = frozen.fallbacks, fallbacks.isEmpty,
              case .object(let route) = frozen.primary else {
            throw IOSApplicationRuntimeError.rejected("Unsupported original model route for edited resend")
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let values = try decoder.decode(RecoveryGeneration.self, from: JSONEncoder().encode(frozen.generation))
        let facts = try decoder.decode(AppleFrozenModelCapabilityFacts.self, from: JSONEncoder().encode(frozen.model_capability))
        guard !facts.profileId.isEmpty, facts.revision > 0, facts.directToolCapabilityIds.isEmpty,
              facts.contextWindowTokens.map({ $0 > 0 }) ?? true,
              facts.maxOutputTokens.map({ $0 >= values.maxOutputTokens }) ?? true else {
            throw IOSApplicationRuntimeError.rejected("The original intrinsic model facts are invalid")
        }
        let candidates: [ModelInfo]
        if route["kind"] == .string("native"), case .string(let modelID)? = route["model_id"] {
            candidates = [ModelInfo(id: modelID, name: modelID, source: "local", isLocal: true)]
        } else if route["kind"] == .string("http") {
            candidates = try ModelSelectionManager.availableModels(using: store).filter { !$0.isLocal && !$0.isWebBackend }
        } else { throw IOSApplicationRuntimeError.rejected("The original model is unavailable for native resend") }
        for model in candidates {
            do {
                let cloud: IOSResolvedCloudRunProfile?
                if model.isLocal { cloud = nil }
                else { cloud = try ModelSelectionManager.resolvedCloudProfileForRun(model, using: store, credentials: credentials) }
                // Current authorized profile supplies a new context budget; sampling/output stay frozen.
                var parameters = try cloud.map { try ModelParameters.decodeProfile($0.parametersJSON) }
                    ?? ModelParameters.defaults(isLocal: true)
                if let capacity = facts.contextWindowTokens { parameters.maxContextLength = min(parameters.maxContextLength, capacity) }
                parameters.maxOutputTokens = values.maxOutputTokens
                parameters.temperature = values.temperature; parameters.topP = values.topP
                parameters.showThinking = values.thinkingEnabled
                parameters.topK = values.generationOptions.topK
                parameters.repetitionPenalty = values.generationOptions.repetitionPenalty
                parameters.presencePenalty = values.generationOptions.presencePenalty ?? 0
                parameters.frequencyPenalty = values.generationOptions.frequencyPenalty ?? 0
                parameters.seed = values.generationOptions.seed
                parameters.stopSequences = (values.generationOptions.stop ?? []).joined(separator: "\n")
                let candidate = try prepare(model: model, parameters: parameters,
                    resolveCloud: { candidate in
                        guard let cloud, cloud.model.id == candidate.id else { throw IOSApplicationRuntimeError.invalidResponse }
                        var frozenProfile = cloud
                        frozenProfile.reasoningSelection = values.reasoningSelection
                        frozenProfile.reasoningTier = values.reasoningTier
                        frozenProfile.reasoningSummarySupported = values.reasoningSummarySupported
                        return frozenProfile
                    }, admitOutput: { candidate, tokens in
                        try IOSPreparedHostOutputAdmission.requireExact(frozen.output_admission, route: candidate, outputTokens: tokens)
                        return frozen.output_admission
                    })
                let prepared: IOSPreparedRunModel
                if case .http(var configuration, let profile) = candidate.executionSource {
                    configuration.executionCapabilities.toolCalls = facts.nativeToolCalls
                    configuration.executionCapabilities.imageInput = facts.visionInput
                    configuration.executionCapabilities.reasoning = facts.reasoning
                    configuration.reasoningSelection = values.reasoningSelection
                    configuration.reasoningTier = values.reasoningTier
                    configuration.reasoningSummarySupported = values.reasoningSummarySupported
                    if case .object(let generation) = frozen.generation,
                       case .bool(let sampling)? = generation["sampling_override"] {
                        configuration.executionCapabilities.samplingParameters = sampling
                        configuration.executionCapabilities.samplingRequiresNoReasoning = false
                    }
                    configuration.promptCacheEnabled = facts.promptCache
                    prepared = IOSPreparedRunModel(client: OpenAIProvider(config: configuration), parameters:parameters,
                        modelName:candidate.modelName, supportsToolCalls:facts.nativeToolCalls,
                        executionSource:.http(configuration:configuration,profile:profile),outputAdmission:candidate.outputAdmission,
                        frozenModelCapability:frozen.model_capability)
                } else {
                    prepared = IOSPreparedRunModel(client:candidate.client,parameters:parameters,modelName:candidate.modelName,
                        supportsToolCalls:facts.nativeToolCalls,executionSource:candidate.executionSource,
                        outputAdmission:candidate.outputAdmission,frozenModelCapability:frozen.model_capability)
                }
                let captured = try prepared.prepareHostInvocation()
                guard captured.route == frozen.primary, captured.generation == frozen.generation,
                      captured.outputAdmission == frozen.output_admission else { continue }
                return prepared
            } catch { continue }
        }
        throw IOSApplicationRuntimeError.rejected("The original model, credential, or frozen parameters have changed")
    }
    var displayModel: ModelInfo {
        switch executionSource {
        case .http(_, let profile): profile.model
        case .native(let id, _): ModelInfo(id: id, name: id, source: "local", isLocal: true)
        }
    }

    /// Concrete embedded delegation capability; unsupported routes remain
    /// ordinary chat models and are never advertised as local child executors.
    func mobileDelegationSettings(projectRoot: String, conversationID: String,
        using store: IOSApplicationRuntime) throws -> AgentCoreJSONValue? {
        guard supportsToolCalls, case .http(let configuration, let profile) = executionSource,
              configuration.baseURL.query == nil else { return nil }
        let stored = try store.executeAgentCommand(.object(["type": .string("load_conversation_settings"),
            "conversation_id": .string(conversationID)]))
        let agents: AgentCoreJSONValue
        if stored == .null { agents = .object([:]) }
        else {
            guard let text = stored.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }
            let settings = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(text.utf8))
            agents = settings.objectFields?["executionPolicy"]?.objectFields?["agents"] ?? .object([:])
        }
        return .object(["project_root": .string(projectRoot), "independent_model_profiles": .bool(true),
            "selected_profile": .object(["profile_id": .string(profile.profileID),
                "admitted_revision": .signedInteger(profile.revision)]), "agents": agents])
    }

    func prepareHostInvocation() throws -> IOSPreparedHostInvocation {
        guard case .object(let admission) = outputAdmission,
              case .array(let routes)? = admission["routes"], routes.count == 1,
              case .object(let limits) = routes[0], let route = limits["route"] else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let capabilities = (client as? LLMAgentCapabilityProviding)?.agentCapabilities
        var options: [String: AgentCoreJSONValue] = [:]
        let temperature: Double
        let topP: Double
        let sampling: Bool
        let thinking: Bool
        let reasoningSummarySupported: Bool?
        let reasoningSelection: AgentCoreJSONValue
        let reasoningTier: String
        let cache: Bool
        var cacheKey: String?
        var cachePrefix: String?
        let outputTokens: Int
        switch executionSource {
        case .http(let configuration, _):
            temperature = configuration.temperature
            topP = configuration.topP
            sampling = configuration.executionCapabilities.samplingParameters
            thinking = configuration.reasoningSelection?.objectFields?["value"] != .string("none")
            reasoningSelection = configuration.reasoningSelection ?? .null
            reasoningTier = configuration.reasoningTier
            reasoningSummarySupported = configuration.reasoningSummarySupported
            outputTokens = configuration.maxCompletionTokens
            cache = capabilities?.promptCache ?? false
            cacheKey = configuration.promptCacheKey?.trimmingCharacters(in: .whitespacesAndNewlines)
            cachePrefix = configuration.promptCacheKeyPrefix?.trimmingCharacters(in: .whitespacesAndNewlines)
            if configuration.presencePenalty != 0 { options["presence_penalty"] = .number(configuration.presencePenalty) }
            if configuration.frequencyPenalty != 0 { options["frequency_penalty"] = .number(configuration.frequencyPenalty) }
            if let seed = configuration.seed { options["seed"] = .signedInteger(Int64(seed)) }
            if !configuration.stopSequences.isEmpty { options["stop"] = .array(configuration.stopSequences.map(AgentCoreJSONValue.string)) }
        case .native:
            temperature = parameters.temperature
            topP = parameters.topP
            sampling = true
            thinking = parameters.showThinking
            reasoningSelection = .null
            reasoningTier = "balanced"
            reasoningSummarySupported = nil
            cache = false
            outputTokens = parameters.maxOutputTokens
            if let topK = parameters.topK { options["top_k"] = .signedInteger(Int64(topK)) }
            if let penalty = parameters.repetitionPenalty { options["repetition_penalty"] = .number(penalty) }
            options["presence_penalty"] = .number(parameters.presencePenalty)
            options["frequency_penalty"] = .number(parameters.frequencyPenalty)
            if let seed = parameters.seed { options["seed"] = .signedInteger(Int64(seed)) }
        }
        try IOSPreparedHostOutputAdmission.requireExact(outputAdmission, route: route, outputTokens: outputTokens)
        func digest(_ value: String?) -> AgentCoreJSONValue {
            guard let value, !value.isEmpty else { return .null }
            return .string(SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined())
        }
        let generation: AgentCoreJSONValue = .object([
            "max_retries": .number(0),
            "parallel_tool_calls": .bool(capabilities?.parallelToolCalls ?? false),
            "reasoning_selection": reasoningSelection, "sampling_override": .bool(sampling),
            "reasoning_tier": .string(reasoningTier),
            "generation_options": .object(options), "temperature": .number(temperature), "top_p": .number(topP),
            "max_output_tokens": .signedInteger(Int64(outputTokens)), "thinking_enabled": .bool(thinking),
            "reasoning_summary_supported": reasoningSummarySupported.map(AgentCoreJSONValue.bool) ?? .null,
            "prompt_cache": .bool(cache), "prompt_cache_key_sha256": digest(cacheKey),
            "prompt_cache_key_prefix_sha256": digest(cachePrefix), "prompt_cache_retention": .null,
        ])
        // Match the wire decoder's integer representation without rounding
        // seeds outside Double's exact range.
        let wireGeneration = try JSONDecoder().decode(AgentCoreJSONValue.self,
            from: JSONEncoder().encode(generation))
        return IOSPreparedHostInvocation(route: route, generation: wireGeneration, outputAdmission: outputAdmission)
    }

    /// Normal expert dispatch resolves the Core-admitted declaration, never the
    /// current inherited selection. A profile edit is rejected explicitly.
    static func prepareExpert(_ entry: AppleExpertMessageEntry<AgentCoreJSONValue>, using store: IOSApplicationRuntime) throws -> IOSPreparedRunModel {
        let declared = entry.model_declaration
        let (model, parameters) = try IOSExpertInstructionProducer.model(declaration: declared, defaults: entry.defaults, store: store)
        return try prepare(model: model, parameters: parameters, resolveCloud: { selected in
            let resolved = try ModelSelectionManager.resolvedCloudProfileForRun(selected, using: store, parameters: parameters)
            guard resolved.profileID == declared.profile_id, resolved.revision == declared.profile_revision else {
                throw IOSApplicationRuntimeError.rejected("The original expert credential owner changed")
            }
            return resolved
        })
    }

    static func prepare(
        model: ModelInfo,
        parameters: ModelParameters,
        resolveCloud: (@MainActor (ModelInfo) throws -> IOSResolvedCloudRunProfile)? = nil,
        admitOutput: @MainActor (AgentCoreJSONValue, Int) throws -> AgentCoreJSONValue = {
            try IOSPreparedHostOutputAdmission.admit(route: $0, outputTokens: $1)
        }
    ) throws -> IOSPreparedRunModel {
        guard ModelParameterLimits.remoteContextTokens.contains(parameters.maxContextLength),
              ModelParameterLimits.outputTokens.contains(parameters.maxOutputTokens),
              !model.isWebBackend else {
            throw NolaError.llm(.invalidInput(reason: "Invalid local-host Run model configuration"))
        }
        if model.isLocal {
            let client = try IOSConversationRunConfiguration.localProvider(model: model, parameters: parameters)
            let route = IOSPreparedHostOutputAdmission.nativeRoute(modelID: client.resolvedModelIdentity.id)
            let admission = try admitOutput(route, parameters.maxOutputTokens)
            try IOSPreparedHostOutputAdmission.requireExact(admission, route: route, outputTokens: parameters.maxOutputTokens)
            return IOSPreparedRunModel(
                client: client,
                parameters: parameters, modelName: model.id, supportsToolCalls: true,
                executionSource: .native(logicalModelID: model.id, catalogIdentity: client.resolvedModelIdentity),
                outputAdmission: admission)
        }
        let resolved = try resolveCloud?(model) ?? ModelSelectionManager.resolvedCloudProfileForRun(model, parameters: parameters)
        guard resolved.profileID == model.id, resolved.model.id == resolved.profileID,
              resolved.revision > 0 else {
            throw NolaError.llm(.invalidInput(reason: "Invalid cloud model profile admission identity"))
        }
        var configuration = try IOSConversationRunConfiguration.directCloudProviderConfig(model: resolved.model, parameters: parameters,
            reasoningSelection: resolved.reasoningSelection, reasoningTier: resolved.reasoningTier)
        configuration.reasoningSummarySupported = resolved.reasoningSummarySupported
        let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: configuration, provider: resolved.model.source)
        let admission = try admitOutput(route, configuration.maxCompletionTokens)
        try IOSPreparedHostOutputAdmission.requireExact(admission, route: route, outputTokens: configuration.maxCompletionTokens)
        return IOSPreparedRunModel(client: OpenAIProvider(config: configuration), parameters: parameters,
            modelName: configuration.model, supportsToolCalls: configuration.executionCapabilities.toolCalls,
            executionSource: .http(configuration: configuration, profile: resolved), outputAdmission: admission)
    }
}
