// SPDX-License-Identifier: MIT
// Request-level generation admission for the local OpenAI-compatible adapter.
import Foundation

struct MLXGenerationParameters: Equatable, Sendable {
    let maxTokens: Int
    let temperature: Double
    let topP: Double
    let topK: Int
    let repetitionPenalty: Double
    let presencePenalty: Double
    let frequencyPenalty: Double
}

enum MLXGenerationRequestError: LocalizedError, Equatable {
    case unsupported(String)
    case invalid(String)
    case conflictingOutputLimits

    var errorDescription: String? {
        switch self {
        case .unsupported(let field): "The local MLX runtime does not support request parameter \(field)."
        case .invalid(let field): "Invalid local MLX generation parameter: \(field)."
        case .conflictingOutputLimits: "Specify only one of max_tokens and max_completion_tokens."
        }
    }
}

/// Decodes the actual HTTP request, not the desktop IPC configuration. Null is
/// an explicit invalid value; only a missing field inherits the runtime default.
struct MLXGenerationRequest: Decodable, Sendable {
    private let maxTokens: Int?
    private let temperature: Double?
    private let topP: Double?
    private let topK: Int?
    private let repetitionPenalty: Double?
    private let presencePenalty: Double?
    private let frequencyPenalty: Double?

    private struct Key: CodingKey {
        let stringValue: String
        var intValue: Int? { nil }
        init(_ value: String) { stringValue = value }
        init?(stringValue: String) { self.init(stringValue) }
        init?(intValue: Int) { return nil }
    }

    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: Key.self)
        let supported: Set<String> = [
            "max_tokens", "max_completion_tokens", "temperature", "top_p", "top_k",
            "repetition_penalty", "presence_penalty", "frequency_penalty",
            // Envelope/tool controls retain their separate ChatRequest owner.
            "model", "messages", "tools", "stream", "tool_choice", "parallel_tool_calls", "stream_options"
        ]
        if let unknown = values.allKeys.map(\.stringValue).sorted().first(where: { !supported.contains($0) }) {
            throw MLXGenerationRequestError.unsupported(unknown)
        }
        func optional<T: Decodable>(_ field: String, _: T.Type) throws -> T? {
            let key = Key(field)
            guard values.contains(key) else { return nil }
            return try values.decode(T.self, forKey: key)
        }
        if values.contains(Key("max_tokens")), values.contains(Key("max_completion_tokens")) {
            throw MLXGenerationRequestError.conflictingOutputLimits
        }
        maxTokens = try optional("max_tokens", Int.self) ?? optional("max_completion_tokens", Int.self)
        temperature = try optional("temperature", Double.self)
        topP = try optional("top_p", Double.self)
        topK = try optional("top_k", Int.self)
        repetitionPenalty = try optional("repetition_penalty", Double.self)
        presencePenalty = try optional("presence_penalty", Double.self)
        frequencyPenalty = try optional("frequency_penalty", Double.self)
    }

    func resolve(defaults: MLXGenerationParameters) throws -> MLXGenerationParameters {
        let result = MLXGenerationParameters(maxTokens: maxTokens ?? defaults.maxTokens,
            temperature: temperature ?? defaults.temperature, topP: topP ?? defaults.topP,
            topK: topK ?? defaults.topK, repetitionPenalty: repetitionPenalty ?? defaults.repetitionPenalty,
            presencePenalty: presencePenalty ?? defaults.presencePenalty,
            frequencyPenalty: frequencyPenalty ?? defaults.frequencyPenalty)
        guard (64...1_000_000).contains(result.maxTokens) else { throw MLXGenerationRequestError.invalid("max_tokens") }
        guard result.temperature.isFinite, (0...2).contains(result.temperature) else { throw MLXGenerationRequestError.invalid("temperature") }
        guard result.topP.isFinite, (0...1).contains(result.topP) else { throw MLXGenerationRequestError.invalid("top_p") }
        guard (1...1_000_000).contains(result.topK) else { throw MLXGenerationRequestError.invalid("top_k") }
        guard result.repetitionPenalty.isFinite, (0.8...1.4).contains(result.repetitionPenalty) else {
            throw MLXGenerationRequestError.invalid("repetition_penalty")
        }
        guard result.presencePenalty.isFinite, (-2...2).contains(result.presencePenalty) else {
            throw MLXGenerationRequestError.invalid("presence_penalty")
        }
        guard result.frequencyPenalty.isFinite, (-2...2).contains(result.frequencyPenalty) else {
            throw MLXGenerationRequestError.invalid("frequency_penalty")
        }
        return result
    }
}
