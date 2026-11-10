// SPDX-License-Identifier: MIT
// Compile with NolaMLXRuntime/MLXGenerationRequest.swift; no model or network.
import Foundation

@main
struct GenerationRequestTests {
    static let defaults = MLXGenerationParameters(maxTokens: 16384, temperature: 0.7,
        topP: 0.9, topK: 40, repetitionPenalty: 1.1, presencePenalty: 0.5, frequencyPenalty: -0.5)

    static func decode(_ json: String) throws -> MLXGenerationRequest {
        try JSONDecoder().decode(MLXGenerationRequest.self, from: Data(json.utf8))
    }

    static func reject(_ json: String) {
        do {
            _ = try decode(json).resolve(defaults: defaults)
            fatalError("Unexpectedly admitted: \(json)")
        } catch { }
    }

    static func main() throws {
        let inherited = try decode("{}").resolve(defaults: defaults)
        precondition(inherited == defaults)
        let zeros = try decode(#"{"model":"fixture","messages":[],"stream":true,"max_tokens":64,"temperature":0,"top_p":0,"top_k":1,"repetition_penalty":0.8,"presence_penalty":0,"frequency_penalty":0}"#)
            .resolve(defaults: defaults)
        precondition(zeros == MLXGenerationParameters(maxTokens: 64, temperature: 0, topP: 0,
            topK: 1, repetitionPenalty: 0.8, presencePenalty: 0, frequencyPenalty: 0))
        let upper = try decode(#"{"max_completion_tokens":1000000,"temperature":2,"top_p":1,"top_k":1000000,"repetition_penalty":1.4,"presence_penalty":2,"frequency_penalty":-2}"#)
            .resolve(defaults: defaults)
        precondition(upper.maxTokens == 1_000_000 && upper.topK == 1_000_000)
        precondition(upper.temperature == 2 && upper.topP == 1 && upper.repetitionPenalty == 1.4)
        precondition(upper.presencePenalty == 2 && upper.frequencyPenalty == -2)
        let sparse = try decode(#"{"temperature":0.2}"#).resolve(defaults: defaults)
        precondition(sparse.temperature == 0.2 && sparse.maxTokens == defaults.maxTokens
            && sparse.topP == defaults.topP && sparse.topK == defaults.topK)
        for json in [
            #"{"max_tokens":0}"#, #"{"max_tokens":63}"#, #"{"max_tokens":1000001}"#,
            #"{"max_tokens":64.5}"#, #"{"max_tokens":"64"}"#, #"{"max_tokens":true}"#,
            #"{"max_tokens":null}"#, #"{"temperature":null}"#, #"{"temperature":-0.01}"#,
            #"{"temperature":2.01}"#, #"{"top_p":-0.01}"#, #"{"top_p":1.01}"#,
            #"{"top_k":0}"#, #"{"top_k":-1}"#, #"{"top_k":1000001}"#,
            #"{"repetition_penalty":0}"#, #"{"repetition_penalty":1.41}"#,
            #"{"presence_penalty":-2.01}"#, #"{"frequency_penalty":2.01}"#,
            #"{"max_tokens":64,"max_completion_tokens":64}"#,
            #"{"max_tokens":null,"max_completion_tokens":64}"#
        ] { reject(json) }
        for json in [
            #"{"seed":0}"#, #"{"stop":[]}"#, #"{"stop":null}"#,
            #"{"reasoning_effort":"none"}"#, #"{"min_p":0}"#,
            #"{"generation_options":{"seed":1}}"#, #"{"n":2}"#, #"{"unexpected":0}"#
        ] { reject(json) }
        let configured = try decode(#"{"max_tokens":64000,"temperature":0,"top_p":0,"top_k":5,"repetition_penalty":1,"presence_penalty":0,"frequency_penalty":0}"#)
        let resolved = try configured.resolve(defaults: defaults)
        let changed = MLXGenerationParameters(maxTokens: 100, temperature: 1, topP: 1,
            topK: 400, repetitionPenalty: 1.2, presencePenalty: 1, frequencyPenalty: 1)
        let resolvedAfterDefaultChange = try configured.resolve(defaults: changed)
        precondition(resolvedAfterDefaultChange == resolved)
        precondition(resolved.maxTokens == 64000, "Never clamp an admitted request back to the startup output limit")
        print("MLX generation request contracts passed: defaults, explicit zero, bounds, aliases, unsupported fields, exact overrides")
    }
}
