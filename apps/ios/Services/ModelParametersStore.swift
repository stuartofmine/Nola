// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelParametersStore logic in the iOS service layer.
// - Interfaces:
//   - PromptResponseStyle
//   - ModelParameters
//   - ModelParametersStore
// - APIs:
//   - none
//
import Foundation
import Combine

/// Product editor bounds, not a claim that every provider supports this size.
/// Local engine allocation can impose a tighter context ceiling.
nonisolated enum ModelParameterLimits {
    static let outputTokens = 64...1_000_000
    static let remoteContextTokens = 512...4_000_000
}

/// Strict editor parsing: reject incomplete input rather than saving an old
/// bound value. Decimal keyboards use the current locale's decimal separator;
/// grouping separators are deliberately not accepted in these ungrouped fields.
nonisolated enum ModelParameterNumberParser {
    static func decimal(_ text: String, locale: Locale = .current) -> Double? {
        var normalized = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if let separator = locale.decimalSeparator, separator != "." {
            normalized = normalized.replacingOccurrences(of: separator, with: ".")
        }
        guard let value = Double(normalized), value.isFinite else { return nil }
        return value
    }

    static func integer(_ text: String) -> Int? {
        Int(text.trimmingCharacters(in: .whitespacesAndNewlines))
    }
}

nonisolated public enum PromptResponseStyle: String, Codable, CaseIterable, Identifiable, Sendable {
    case professional = "professional"
    case casual = "casual"
    case playful = "playful"

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .professional:
            return NSLocalizedString(
                "prompt.response_style.professional",
                value: "Professional",
                comment: "Professional response style name"
            )
        case .casual:
            return NSLocalizedString(
                "prompt.response_style.casual",
                value: "Casual",
                comment: "Casual response style name"
            )
        case .playful:
            return NSLocalizedString(
                "prompt.response_style.playful",
                value: "Playful",
                comment: "Playful response style name"
            )
        }
    }

    public var promptInstruction: String {
        switch self {
        case .professional:
            return "Current response style: professional.\nBe restrained, accurate, polished, and direct. Preserve the user's language."
        case .casual:
            return "Current response style: casual.\nBe natural, calm, friendly, and reliable. Preserve the user's language."
        case .playful:
            return "Current response style: playful.\nBe lively and lightly humorous when appropriate without becoming exaggerated or distracting. Preserve the user's language."
        }
    }
}

nonisolated public struct ModelParameters: Codable, Sendable, Equatable {
    public var maxContextLength: Int = 65_536
    public var maxOutputTokens: Int = 16_384
    public var topK: Int? = nil
    public var topP: Double = 0.9
    public var temperature: Double = 0.7
    public var presencePenalty: Double = 0.0
    public var frequencyPenalty: Double = 0.0
    public var repetitionPenalty: Double? = nil
    public var seed: Int? = nil
    public var stopSequences: String = ""
    public var reasoningTier: String? = nil
    public var reasoningPreference: String? = nil
    public var showThinking: Bool = true
    /// Whether the UI should progressively display streaming chunks as they arrive.
    /// If false, the UI will only show the final generated output after the stream completes.
    public var streamOutput: Bool = true
    public var responseStyle: PromptResponseStyle = .casual
    public var customSystemPrompt: String = ""

    private enum CodingKeys: String, CodingKey {
        case maxContextLength
        case maxOutputTokens
        case topK
        case topP
        case temperature
        case presencePenalty
        case frequencyPenalty
        case repetitionPenalty
        case seed
        case stopSequences
        case reasoningTier
        case reasoningPreference
        case showThinking
        case streamOutput
        case responseStyle
        case customSystemPrompt
    }

    public static let localDefaults = ModelParameters(
        maxContextLength: 65_536,
        maxOutputTokens: 4096,
        topK: 40,
        topP: 0.9,
        temperature: 0.7,
        presencePenalty: 0.0,
        frequencyPenalty: 0.0,
        repetitionPenalty: 1.0,
        seed: nil,
        stopSequences: "",
        showThinking: true,
        streamOutput: true,
        responseStyle: .casual,
        customSystemPrompt: ""
    )

    public static let webAlignedCloudDefaults = ModelParameters(
        maxContextLength: 128_000,
        maxOutputTokens: 16_384,
        topK: nil,
        topP: 0.9,
        temperature: 0.7,
        presencePenalty: 0.0,
        frequencyPenalty: 0.0,
        repetitionPenalty: nil,
        seed: nil,
        stopSequences: "",
        showThinking: true,
        streamOutput: true,
        responseStyle: .casual,
        customSystemPrompt: ""
    )

    public init(
        maxContextLength: Int = 65_536,
        maxOutputTokens: Int = 16_384,
        topK: Int? = nil,
        topP: Double = 0.9,
        temperature: Double = 0.7,
        presencePenalty: Double = 0.0,
        frequencyPenalty: Double = 0.0,
        repetitionPenalty: Double? = nil,
        seed: Int? = nil,
        stopSequences: String = "",
        showThinking: Bool = true,
        streamOutput: Bool = true,
        responseStyle: PromptResponseStyle = .casual,
        customSystemPrompt: String = ""
    ) {
        self.maxContextLength = maxContextLength
        self.maxOutputTokens = maxOutputTokens
        self.topK = topK
        self.topP = topP
        self.temperature = temperature
        self.presencePenalty = presencePenalty
        self.frequencyPenalty = frequencyPenalty
        self.repetitionPenalty = repetitionPenalty
        self.seed = seed
        self.stopSequences = stopSequences
        self.showThinking = showThinking
        self.streamOutput = streamOutput
        self.responseStyle = responseStyle
        self.customSystemPrompt = customSystemPrompt
    }

    public static func defaults(isLocal: Bool) -> ModelParameters {
        isLocal ? localDefaults : webAlignedCloudDefaults
    }

    /// Current canonical profiles permit sparse and nullable parameters. Missing
    /// required physical scalars inherit the supplied fixed Host policy; optional
    /// generation values explicitly cleared by null remain cleared.
    private struct CurrentProfileProjection: Decodable {
        let parameters: ModelParameters
        init(from decoder: Decoder) throws {
            let fields = try decoder.container(keyedBy: CodingKeys.self)
            var value = (decoder.userInfo[.nolaProfileHostDefaults] as? ModelParameters) ?? .webAlignedCloudDefaults
            if let v = try fields.decodeIfPresent(Int.self, forKey: .maxContextLength) { value.maxContextLength = v }
            if let v = try fields.decodeIfPresent(Int.self, forKey: .maxOutputTokens) { value.maxOutputTokens = v }
            if let v = try fields.decodeIfPresent(Double.self, forKey: .temperature) { value.temperature = v }
            if let v = try fields.decodeIfPresent(Double.self, forKey: .topP) { value.topP = v }
            if let v = try fields.decodeIfPresent(Double.self, forKey: .presencePenalty) { value.presencePenalty = v }
            if let v = try fields.decodeIfPresent(Double.self, forKey: .frequencyPenalty) { value.frequencyPenalty = v }
            if fields.contains(.topK) { value.topK = try fields.decodeIfPresent(Int.self, forKey: .topK) }
            if fields.contains(.repetitionPenalty) { value.repetitionPenalty = try fields.decodeIfPresent(Double.self, forKey: .repetitionPenalty) }
            if fields.contains(.seed) { value.seed = try fields.decodeIfPresent(Int.self, forKey: .seed) }
            if let v = try fields.decodeIfPresent(String.self, forKey: .stopSequences) { value.stopSequences = v }
            if let v = try fields.decodeIfPresent(String.self, forKey: .reasoningTier) { value.reasoningTier = v }
            if let v = try fields.decodeIfPresent(String.self, forKey: .reasoningPreference) { value.reasoningPreference = v }
            if let v = try fields.decodeIfPresent(Bool.self, forKey: .showThinking) { value.showThinking = v }
            if let v = try fields.decodeIfPresent(Bool.self, forKey: .streamOutput) { value.streamOutput = v }
            if let v = try fields.decodeIfPresent(PromptResponseStyle.self, forKey: .responseStyle) { value.responseStyle = v }
            if let v = try fields.decodeIfPresent(String.self, forKey: .customSystemPrompt) { value.customSystemPrompt = v }
            parameters = value
        }
    }

    public static func decodeProfile(_ json: String, inheriting hostDefaults: ModelParameters = .webAlignedCloudDefaults) throws -> ModelParameters {
        do {
            let decoder = JSONDecoder()
            decoder.userInfo[.nolaProfileHostDefaults] = hostDefaults
            return try decoder.decode(CurrentProfileProjection.self, from: Data(json.utf8)).parameters
        }
        catch let error as DecodingError {
            let path: [CodingKey]
            switch error {
            case .keyNotFound(let key, let context): path = context.codingPath + [key]
            case .typeMismatch(_, let context), .valueNotFound(_, let context), .dataCorrupted(let context):
                path = context.codingPath
            @unknown default: path = []
            }
            // Only field names are public. Do not include JSON, endpoint tokens
            // or decoder debug descriptions in the user-visible error.
            throw ModelParameterDecodingError(field: (["parameters"] + path.map(\.stringValue)).joined(separator: "."))
        }
    }
}

private extension CodingUserInfoKey {
    nonisolated static let nolaProfileHostDefaults = CodingUserInfoKey(rawValue: "nola.current-profile.host-defaults")!
}

nonisolated public struct ModelParameterDecodingError: LocalizedError {
    public let field: String
    public var errorDescription: String? {
        String(format: NSLocalizedString("Invalid model configuration field: %@", comment: "Model configuration field decoding error"), field)
    }
}

/// A local engine requires concrete sampling settings. Resolving them here
/// must never materialize local defaults in a remote profile or conversation.
nonisolated public struct LocalSamplingSettings: Sendable {
    public let topK: Int
    public let repetitionPenalty: Double

    public init(parameters: ModelParameters) {
        topK = parameters.topK ?? 40
        repetitionPenalty = parameters.repetitionPenalty ?? 1.0
    }
}

/// Shared projection over canonical ModelProfile generation parameters.
public final class ModelParametersStore: ObservableObject {
    public static let shared = ModelParametersStore()
    @Published public private(set) var lastUpdated: Date? = nil

    #if os(macOS)
    private let applicationPort: MacApplicationPort

    private convenience init() {
        self.init(applicationPort: MacApplicationPort.shared)
    }

    init(applicationPort: MacApplicationPort) {
        self.applicationPort = applicationPort
    }
    #else
    private let applicationPort: IOSApplicationRuntime

    private convenience init() {
        guard let store = IOSApplicationRuntime.shared else {
            preconditionFailure("Nola Application runtime is unavailable")
        }
        self.init(applicationPort: store)
    }

    init(applicationPort: IOSApplicationRuntime) {
        self.applicationPort = applicationPort
    }
    #endif

    public func loadParameters(for modelID: String, isLocal: Bool? = nil) -> ModelParameters {
        let defaultParams = isLocal.map { ModelParameters.defaults(isLocal: $0) } ?? ModelParameters.localDefaults
        #if os(macOS)
        let profile = (try? applicationPort.modelProfile(modelID)) ??
            (try? applicationPort.listModelProfiles().first(where: {
                $0.modelName == modelID && $0.deletedAtMs == nil
            }))
        if let profile,
           let data = profile.parametersJSON.data(using: .utf8),
           let params = try? JSONDecoder().decode(ModelParameters.self, from: data) { return params }
        #else
        if let profile = try? applicationPort.modelProfile(id: modelID),
           let data = profile.parametersJSON.data(using: .utf8),
           let params = try? JSONDecoder().decode(ModelParameters.self, from: data) { return params }
        #endif
        return defaultParams
    }

    public func saveParameters(_ params: ModelParameters, for modelID: String, isLocal: Bool? = nil) throws {
        let parametersJSON = String(decoding: try JSONEncoder().encode(params), as: UTF8.self)
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        #if os(macOS)
        let canonicalID: String
        if let profile = try applicationPort.modelProfile(modelID) {
            canonicalID = profile.id
        } else {
            canonicalID = try applicationPort.listModelProfiles()
                .first(where: { $0.modelName == modelID && $0.deletedAtMs == nil })?.id ?? modelID
        }
        #else
        let canonicalID = modelID
        #endif
        // Parameter edits must not create profiles or rewrite source/selection
        // from a possibly stale client projection. Missing models are rejected.
        try applicationPort.resetModelParameters(
            profileIDs: [canonicalID], parametersJSON: parametersJSON, updatedAtMs: now
        )
        lastUpdated = Date()
    }

    #if os(macOS)
    struct ParameterSnapshot {
        let parameters: ModelParameters
        fileprivate let profile: MacCanonicalModelProfile
        let revision: UInt64
    }

    func parameterSnapshot(for modelID: String) throws -> ParameterSnapshot {
        let profile: MacCanonicalModelProfile?
        if let exact = try applicationPort.modelProfile(modelID) {
            profile = exact
        } else {
            let matches = try applicationPort.listModelProfiles().filter { $0.modelName == modelID && $0.deletedAtMs == nil }
            guard matches.count == 1 else { throw MacApplicationServiceError.invalidResponse }
            profile = matches.first
        }
        guard let profile, profile.deletedAtMs == nil, profile.revision >= 0,
              let data = profile.parametersJSON.data(using: .utf8) else { throw MacApplicationServiceError.invalidResponse }
        return ParameterSnapshot(parameters: try JSONDecoder().decode(ModelParameters.self, from: data),
                                 profile: profile, revision: UInt64(profile.revision))
    }

    @discardableResult
    func saveParameters(_ params: ModelParameters, snapshot: ParameterSnapshot) throws -> ParameterSnapshot {
        let profile = snapshot.profile
        let input = MacCanonicalModelProfile(
            id: profile.id, displayName: profile.displayName, provider: profile.provider,
            modelName: profile.modelName, runtime: profile.runtime, sourceJSON: profile.sourceJSON,
            parametersJSON: String(decoding: try JSONEncoder().encode(params), as: UTF8.self),
            selected: false, enabled: true, revision: 0,
            createdAtMs: profile.createdAtMs, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000), deletedAtMs: nil
        )
        let revision = try applicationPort.configureModelParameters(input, expectedRevision: snapshot.revision)
        lastUpdated = Date()
        return ParameterSnapshot(parameters: params, profile: input, revision: revision)
    }
    #else
    struct ParameterSnapshot {
        let parameters: ModelParameters
        fileprivate let profile: IOSCanonicalModelProfile
        var offersReasoningTiers: Bool {
            guard let source = try? JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any],
                  let reasoning = source["reasoning"] as? [String: Any] else { return false }
            let capabilities = source["executionCapabilities"] as? [String: Any]
            return profile.runtime == "remote" && capabilities?["reasoning"] as? Bool == true
                && reasoning["control"] as? String == "effort"
        }
    }

    func reasoningEfforts(for snapshot: ParameterSnapshot) throws -> [String] {
        guard snapshot.offersReasoningTiers else { return [] }
        let source = try JSONSerialization.jsonObject(with: Data(snapshot.profile.sourceJSON.utf8)) as? [String: Any]
        return (source?["reasoning"] as? [String: Any])?["efforts"] as? [String] ?? []
    }

    func parameterSnapshot(for modelID: String) throws -> ParameterSnapshot {
        guard let profile = try applicationPort.modelProfile(id: modelID),
              profile.revision >= 0,
              let data = profile.parametersJSON.data(using: .utf8) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        return ParameterSnapshot(parameters: try JSONDecoder().decode(ModelParameters.self, from: data), profile: profile)
    }

    @discardableResult
    func saveParameters(_ params: ModelParameters, snapshot: ParameterSnapshot) throws -> UInt64 {
        let profile = snapshot.profile
        let input = IOSCanonicalModelProfile(
            id: profile.id, displayName: profile.displayName, provider: profile.provider,
            modelName: profile.modelName, runtime: profile.runtime, sourceJSON: profile.sourceJSON,
            parametersJSON: String(decoding: try JSONEncoder().encode(params), as: UTF8.self),
            selected: false, enabled: true, revision: 0, createdAtMs: profile.createdAtMs,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000), deletedAtMs: nil
        )
        let revision = try applicationPort.configureModelParameters(input, expectedRevision: UInt64(profile.revision))
        lastUpdated = Date()
        return revision
    }
    #endif
}
