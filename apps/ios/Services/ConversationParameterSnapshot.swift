// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated protocol ConversationParameterSnapshotStore: AnyObject {
    func conversationParameters(conversationId: UUID, modelKey: String) throws -> ModelParameters
    func hasConversationParameters(conversationId: UUID, modelKey: String) throws -> Bool
    func saveConversationParameters(conversationId: UUID, modelKey: String, parameters: ModelParameters) throws
}

/// Shared provider parameters have one authority: canonical modelOverrides.
/// Client-only display/prompt options are separate, never a second baseline.
nonisolated enum ConversationParameterSnapshot {
    /// The same already-resolved context presentation used by native capture.
    /// It is not model generation configuration or a second baseline.
    static func applyingPresentation(_ presentation: AgentCoreJSONValue, to parameters: ModelParameters) throws -> ModelParameters {
        if presentation == .null { return parameters }
        guard case .object(let values) = presentation else { throw IOSApplicationRuntimeError.invalidResponse }
        var result = parameters
        func boolean(_ key: String, fallback: Bool) throws -> Bool {
            guard let value = values[key] else { return fallback }
            guard case .bool(let flag) = value else { throw IOSApplicationRuntimeError.invalidResponse }
            return flag
        }
        result.showThinking = try boolean("showThinking", fallback: true)
        result.streamOutput = try boolean("streamOutput", fallback: true)
        if let value = values["responseStyle"] {
            guard case .string(let raw) = value, let style = PromptResponseStyle(rawValue: raw) else { throw IOSApplicationRuntimeError.invalidResponse }
            result.responseStyle = style
        } else { result.responseStyle = .casual }
        if let value = values["customSystemPrompt"] {
            guard case .string(let text) = value else { throw IOSApplicationRuntimeError.invalidResponse }
            result.customSystemPrompt = text
        } else { result.customSystemPrompt = "" }
        return result
    }
    static func read(_ json: String?, modelKey: String) throws -> ModelParameters {
        let root = try IOSConversationSettingsObject.decode(json)
        if modelKey == "execution-host" {
            guard let fields = root["clientHostParameters"] as? [String: Any] else {
                throw IOSApplicationRuntimeError.rejected("Conversation Host parameters are missing")
            }
            var projection = root
            projection["modelOverrides"] = ["host-parameters": fields]
            if let options = (root["clientModelPresentation"] as? [String: Any])?[modelKey] {
                projection["clientModelPresentation"] = ["host-parameters": options]
            }
            return try read(String(decoding: JSONSerialization.data(withJSONObject: projection), as: UTF8.self), modelKey: "host-parameters")
        }
        guard let models = root["modelOverrides"] as? [String: Any],
              let values = models[modelKey] as? [String: Any],
              let context = values["maxContextLength"] as? Int,
              let output = values["maxOutputTokens"] as? Int,
              let temperature = values["temperature"] as? Double,
              let topP = values["topP"] as? Double else {
            throw IOSApplicationRuntimeError.rejected("Conversation model parameters are missing; select and configure a model for this conversation.")
        }
        var value = ModelParameters(maxContextLength: context, maxOutputTokens: output,
            topK: values["topK"] as? Int, topP: topP, temperature: temperature,
            presencePenalty: values["presencePenalty"] as? Double ?? 0,
            frequencyPenalty: values["frequencyPenalty"] as? Double ?? 0,
            repetitionPenalty: values["repetitionPenalty"] as? Double)
        if let seed = values["seed"] as? [String: String], seed["kind"] == "value" { value.seed = seed["value"].flatMap(Int.init) }
        value.reasoningTier = values["reasoningTier"] as? String
        value.reasoningPreference = values["reasoningPreference"] as? String
        value.stopSequences = values["stopSequences"] as? String ?? ""
        if let clients = root["clientModelPresentation"] as? [String: Any], let options = clients[modelKey] as? [String: Any] {
            value = try applyingPresentation(JSONDecoder().decode(AgentCoreJSONValue.self,
                from: JSONSerialization.data(withJSONObject: options)), to: value)
        }
        return value
    }
    /// Apply the current conversation override contract to an already frozen
    /// profile projection. Null scalars inherit that profile, never a selection.
    static func applyingOverrides(_ json: String?, modelKey: String, to profile: ModelParameters) throws -> ModelParameters {
        let root = try IOSConversationSettingsObject.decode(json)
        guard let encodedModels = root["modelOverrides"], !(encodedModels is NSNull) else { return profile }
        guard let models = encodedModels as? [String: Any] else { throw IOSApplicationRuntimeError.invalidResponse }
        guard let encoded = models[modelKey] else { return profile }
        guard let raw = encoded as? [String: Any] else { throw IOSApplicationRuntimeError.invalidResponse }
        var fields = raw
        if let seed = fields["seed"], !(seed is NSNull) {
            guard let selection = seed as? [String: Any], let kind = selection["kind"] as? String else { throw IOSApplicationRuntimeError.rejected("Invalid conversation seed override") }
            switch kind {
            case "clear":
                guard selection.count == 1 else { throw IOSApplicationRuntimeError.invalidResponse }
                fields["seed"] = NSNull()
            case "value":
                guard selection.count == 2, let raw = selection["value"] as? String, let value = Int(raw), String(value) == raw else { throw IOSApplicationRuntimeError.rejected("Invalid conversation seed override") }
                fields["seed"] = value
            default: throw IOSApplicationRuntimeError.rejected("Invalid conversation seed override")
            }
        } else { fields.removeValue(forKey: "seed") }
        let data = try JSONSerialization.data(withJSONObject: fields)
        return try ModelParameters.decodeProfile(String(decoding: data, as: UTF8.self), inheriting: profile)
    }

    static func updating(_ json: String?, modelKey: String, parameters: ModelParameters) throws -> String {
        // Reject known-invalid editor input before it becomes a durable pending
        // command. An outer wire error is not a confirmed command outcome.
        let lines = parameters.stopSequences.components(separatedBy: "\n")
        let stops = lines.enumerated().map { index, line in
            index < lines.count - 1 && line.hasSuffix("\r") ? String(line.dropLast()) : line
        }.filter { !$0.isEmpty }
        guard ModelParameterLimits.remoteContextTokens.contains(parameters.maxContextLength),
              ModelParameterLimits.outputTokens.contains(parameters.maxOutputTokens),
              parameters.topK.map({ (1...1_000_000).contains($0) }) ?? true,
              parameters.temperature.isFinite, (0...2).contains(parameters.temperature),
              parameters.topP.isFinite, (0...1).contains(parameters.topP),
              parameters.presencePenalty.isFinite, (-2...2).contains(parameters.presencePenalty),
              parameters.frequencyPenalty.isFinite, (-2...2).contains(parameters.frequencyPenalty),
              parameters.repetitionPenalty.map({ $0.isFinite && (0.8...1.4).contains($0) }) ?? true,
              parameters.stopSequences.utf8.count <= 65_536,
              stops.count <= 16,
              stops.allSatisfy({ $0.utf8.count <= 2048 }) else {
            throw IOSApplicationRuntimeError.rejected("Invalid conversation model parameters")
        }
        var root = try IOSConversationSettingsObject.decode(json)
        if modelKey == "execution-host" {
            let projected = try IOSConversationSettingsObject.decode(updating(nil, modelKey: "host-parameters", parameters: parameters))
            root["clientHostParameters"] = (projected["modelOverrides"] as? [String: Any])?["host-parameters"]
            var options = root["clientModelPresentation"] as? [String: Any] ?? [:]
            options[modelKey] = (projected["clientModelPresentation"] as? [String: Any])?["host-parameters"]
            root["clientModelPresentation"] = options
            return String(decoding: try JSONSerialization.data(withJSONObject: root, options: [.sortedKeys]), as: UTF8.self)
        }
        var models = root["modelOverrides"] as? [String: Any] ?? [:]
        var fields = models[modelKey] as? [String: Any] ?? [:]
        fields["reasoningPreference"] = parameters.reasoningPreference.map { $0 as Any } ?? NSNull()
        fields.merge([
            "maxContextLength": parameters.maxContextLength, "maxOutputTokens": parameters.maxOutputTokens,
            "temperature": parameters.temperature, "topP": parameters.topP, "topK": parameters.topK.map { $0 as Any } ?? NSNull(),
            "presencePenalty": parameters.presencePenalty, "frequencyPenalty": parameters.frequencyPenalty,
            "repetitionPenalty": parameters.repetitionPenalty.map { $0 as Any } ?? NSNull(),
            "seed": parameters.seed.map { ["kind": "value", "value": String($0)] } ?? ["kind": "clear"],
            "stopSequences": parameters.stopSequences,
            "reasoningTier": parameters.reasoningTier.map { $0 as Any } ?? NSNull(),
        ]) { _, new in new }
        models[modelKey] = fields; root["modelOverrides"] = models
        var options = root["clientModelPresentation"] as? [String: Any] ?? [:]
        options[modelKey] = ["showThinking": parameters.showThinking, "streamOutput": parameters.streamOutput,
            "responseStyle": parameters.responseStyle.rawValue, "customSystemPrompt": parameters.customSystemPrompt]
        root["clientModelPresentation"] = options
        return String(decoding: try JSONSerialization.data(withJSONObject: root, options: [.sortedKeys]), as: UTF8.self)
    }
}
