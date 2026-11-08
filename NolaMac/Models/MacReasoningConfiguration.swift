// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated enum MacModelWireProtocol: String, Codable, CaseIterable, Identifiable, Sendable {
    case chatCompletions = "chat_completions"
    case responses
    case siwcResponses = "siwc_responses"
    case anthropic
    var id: String { rawValue }
    var usesResponsesTransport: Bool { self == .responses || self == .siwcResponses }
    var title: String {
        switch self {
        case .chatCompletions: return "Chat Completions"
        case .responses: return "Responses"
        case .siwcResponses: return "ChatGPT Authorization · Responses"
        case .anthropic: return "Anthropic Messages"
        }
    }
}


/// Stored intent is distinct from the resolved optional integer sent to a provider.
nonisolated enum MacSeedOverride: Hashable, Sendable, Codable {
    case clear
    case value(Int64)

    static func canonicalValue(_ text: String) -> Int64? {
        guard let value = Int64(text), String(value) == text else { return nil }
        return value
    }

    private struct Key: CodingKey {
        let stringValue: String
        var intValue: Int? { nil }
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
        static let kind = Key(stringValue: "kind")!
        static let value = Key(stringValue: "value")!
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Key.self)
        let kind = try container.decode(String.self, forKey: .kind)
        let keys = Set(container.allKeys.map(\.stringValue))
        switch kind {
        case "clear" where keys == ["kind"]: self = .clear
        case "value" where keys == ["kind", "value"]:
            let text = try container.decode(String.self, forKey: .value)
            guard let value = Self.canonicalValue(text) else {
                throw DecodingError.dataCorruptedError(forKey: .value, in: container,
                    debugDescription: "Seed must be a canonical decimal signed 64-bit integer")
            }
            self = .value(value)
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: container,
                debugDescription: "Invalid seed override shape")
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: Key.self)
        switch self {
        case .clear: try container.encode("clear", forKey: .kind)
        case .value(let value):
            try container.encode("value", forKey: .kind)
            try container.encode(String(value), forKey: .value)
        }
    }
}

/// Sparse, per-model conversation overrides. Nil inherits; zero is a value.
nonisolated struct MacConversationModelOverride: Codable, Hashable, Sendable {
    var wireProtocol: MacModelWireProtocol?
    var temperature: Double?
    var topP: Double?
    var maxOutputTokens: Int?
    var maxContextLength: Int?
    var topK: Int?
    var presencePenalty: Double?
    var frequencyPenalty: Double?
    var repetitionPenalty: Double?
    var seed: MacSeedOverride?
    var stopSequences: String?
    var reasoningPreference: String?

    func applying(to defaults: ModelParameters) -> ModelParameters {
        var value = defaults
        if let temperature { value.temperature = temperature }
        if let topP { value.topP = topP }
        if let maxOutputTokens { value.maxOutputTokens = maxOutputTokens }
        if let maxContextLength { value.maxContextLength = maxContextLength }
        if let topK { value.topK = topK }
        if let presencePenalty { value.presencePenalty = presencePenalty }
        if let frequencyPenalty { value.frequencyPenalty = frequencyPenalty }
        if let repetitionPenalty { value.repetitionPenalty = repetitionPenalty }
        if let seed {
            switch seed {
            case .clear: value.seed = nil
            case .value(let seed): value.seed = Int(seed)
            }
        }
        if let stopSequences { value.stopSequences = stopSequences }
        if let reasoningPreference { value.reasoningPreference = reasoningPreference; value.reasoningTier = nil }
        return value
    }
}

extension MacConversationModelOverride {
    nonisolated init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        // A stored null is not the patch operation that removes this field.
        if container.contains(.seed), try container.decodeNil(forKey: .seed) {
            throw DecodingError.dataCorruptedError(forKey: .seed, in: container,
                debugDescription: "Stored seed inheritance must omit the field")
        }
        wireProtocol = try container.decodeIfPresent(MacModelWireProtocol.self, forKey: .wireProtocol)
        temperature = try container.decodeIfPresent(Double.self, forKey: .temperature)
        topP = try container.decodeIfPresent(Double.self, forKey: .topP)
        maxOutputTokens = try container.decodeIfPresent(Int.self, forKey: .maxOutputTokens)
        maxContextLength = try container.decodeIfPresent(Int.self, forKey: .maxContextLength)
        topK = try container.decodeIfPresent(Int.self, forKey: .topK)
        presencePenalty = try container.decodeIfPresent(Double.self, forKey: .presencePenalty)
        frequencyPenalty = try container.decodeIfPresent(Double.self, forKey: .frequencyPenalty)
        repetitionPenalty = try container.decodeIfPresent(Double.self, forKey: .repetitionPenalty)
        seed = try container.decodeIfPresent(MacSeedOverride.self, forKey: .seed)
        stopSequences = try container.decodeIfPresent(String.self, forKey: .stopSequences)
        reasoningPreference = try container.decodeIfPresent(String.self, forKey: .reasoningPreference)
    }
}

/// A model/endpoint declaration, not a guess from a provider or model name.
nonisolated struct MacReasoningConfiguration: Codable, Hashable, Sendable {
    // Supported efforts are an endpoint/model declaration, never inferred from
    // a model-name allowlist. Providers can add models or change capabilities
    // independently of a client release.
    enum Control: String, Codable, CaseIterable, Sendable {
        case automatic, effort, budget
    }
    static func availableControls(for wireProtocol: MacModelWireProtocol) -> [Control] {
        wireProtocol == .anthropic ? [.automatic, .budget] : [.automatic, .effort]
    }

    func supports(_ wireProtocol: MacModelWireProtocol) -> Bool {
        Self.availableControls(for: wireProtocol).contains(control)
    }

    /// Validate against the effective protocol, including inherited contracts.
    /// A protocol switch must not silently turn an incompatible budget into Automatic.
    func isValid(for wireProtocol: MacModelWireProtocol) -> Bool {
        // Persisted fields obey the shared schema even while their control is
        // inactive. In particular, switching to Automatic must not make an
        // unfinished budget draft (-1) saveable or transport unknown efforts.
        guard budgets.allSatisfy({ $0 >= 0 }),
              efforts.allSatisfy({ ["none", "minimal", "low", "medium", "high", "xhigh", "max"].contains($0) }) else {
            return false
        }
        var resolved = self
        resolved.responses = wireProtocol.usesResponsesTransport
        return supports(wireProtocol) && (control == .automatic || !resolved.options.isEmpty)
    }
    var control: Control = .automatic
    var responses: Bool = false
    var efforts: [String] = []
    var budgets: [Int] = [1024, 4096, 8192]
    var outputLimit: Int = 16384

    /// Validate the derived transport selection against the declared capability.
    func isValid(for wireProtocol: MacModelWireProtocol, preferenceOverride: String?) -> Bool {
        guard isValid(for: wireProtocol) else { return false }
        guard let preferenceOverride, preferenceOverride != "automatic" else { return true }
        var effective = self
        effective.responses = wireProtocol.usesResponsesTransport
        return effective.options.contains(preferenceOverride)
    }

    var options: [String] {
        switch control {
        case .automatic: return []
        case .effort:
            return efforts
        case .budget:
            guard !responses, budgets.count == 3, budgets == budgets.sorted(), Set(budgets).count == 3,
                  budgets.allSatisfy({ $0 >= 1024 && $0 < outputLimit }) else { return [] }
            return budgets.map(String.init)
        }
    }

    func selection(_ preference: String?) -> [String: Any]? {
        guard let preference else { return nil }
        let declared = control == .budget ? budgets.map(String.init) : options
        guard let index = declared.firstIndex(of: preference) else { return nil }
        switch control {
        case .automatic: return nil
        case .effort: return ["kind": "effort", "value": preference]
        case .budget: return ["kind": "budget", "tokens": budgets[index]]
        }
    }

}

extension MacReasoningConfiguration {
    // Core's capability declaration is sparse. Editor-only defaults are not
    // required fields in a connection-managed execution profile.
    nonisolated init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        control = try values.decode(Control.self, forKey: .control)
        responses = try values.decodeIfPresent(Bool.self, forKey: .responses) ?? false
        efforts = try values.decodeIfPresent([String].self, forKey: .efforts) ?? []
        budgets = try values.decodeIfPresent([Int].self, forKey: .budgets) ?? [1024, 4096, 8192]
        outputLimit = try values.decodeIfPresent(Int.self, forKey: .outputLimit) ?? 16384
    }
}
