// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements PlaceholderLocalProvider logic in the iOS LLM integration layer.
// - Interfaces:
//   - PlaceholderLocalProvider
// - APIs:
//   - none
//
import Foundation

/// A placeholder LLM provider used on platforms/environments where local MLX
/// inference is unavailable (e.g. iOS Simulator). This avoids initializing
/// heavy MLX services that crash in unsupported environments.
public final class PlaceholderLocalProvider: LLMClient {
    public static let shared = PlaceholderLocalProvider()

    private init() {}

    public func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        throw NolaError.modelNotLoaded
    }

    public func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        return AsyncThrowingStream { continuation in
            continuation.finish(throwing: NolaError.modelNotLoaded)
        }
    }

    public func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        throw NolaError.modelNotLoaded
    }
}
