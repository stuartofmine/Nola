// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Opt-in end-to-end validation of a downloaded MLX model through Nola's loopback API.

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacLocalModelRealInferenceEndToEndTests: XCTestCase {
    func testQwen35Point8BStartsServesAChatAndStops() async throws {
        let profile = MacLocalModelProfile.qwen35_08B
        let modelDirectory = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent(profile.id, isDirectory: true)
        let weights = modelDirectory.appendingPathComponent("model.safetensors")
        guard FileManager.default.fileExists(atPath: weights.path) else {
            throw XCTSkip("Download Qwen3.5 0.8B before running the real local-model test.")
        }
        let weightAttributes = try FileManager.default.attributesOfItem(atPath: weights.path)
        let weightSize = try XCTUnwrap(weightAttributes[.size] as? NSNumber).int64Value
        XCTAssertEqual(weightSize, 625_229_487, "The real inference test requires the complete 0.8B weight file.")

        let service = MacMLXOpenAIService.shared
        service.stop()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-mlx-native-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("native.sqlite"))
        let phaseCollector = Task { @MainActor in
            var phases: [MacLocalModelRuntimePhase] = []
            for await status in service.statusUpdates() {
                if phases.last != status.phase {
                    phases.append(status.phase)
                }
                if status.phase == .ready || status.phase == .failed {
                    return phases
                }
            }
            return phases
        }
        let startTask = Task { try await service.start(profile, applicationPort: port) }
        let concurrentStartTask = Task { try await service.start(profile, applicationPort: port) }
        let endpoint = try await startTask.value
        let concurrentEndpoint = try await concurrentStartTask.value
        let observedPhases = await phaseCollector.value
        defer { service.stop() }

        XCTAssertTrue(service.isRunning(profile))
        XCTAssertTrue(observedPhases.contains(.loadingWeights), "Observed phases: \(observedPhases)")
        XCTAssertTrue(observedPhases.contains(.ready), "Observed phases: \(observedPhases)")
        XCTAssertEqual(endpoint.modelID, profile.id)
        XCTAssertEqual(concurrentEndpoint.baseURL, endpoint.baseURL)
        XCTAssertEqual(concurrentEndpoint.apiKey, endpoint.apiKey)
        XCTAssertGreaterThanOrEqual(endpoint.contextWindowTokens, 65_536)

        var modelsRequest = URLRequest(url: try XCTUnwrap(URL(string: endpoint.baseURL + "/models")))
        modelsRequest.setValue("Bearer \(endpoint.apiKey)", forHTTPHeaderField: "Authorization")
        let (modelsData, modelsResponse) = try await URLSession.shared.data(for: modelsRequest)
        XCTAssertEqual((modelsResponse as? HTTPURLResponse)?.statusCode, 200)
        let modelsJSON = try XCTUnwrap(JSONSerialization.jsonObject(with: modelsData) as? [String: Any])
        let listedModels = try XCTUnwrap(modelsJSON["data"] as? [[String: Any]])
        XCTAssertTrue(listedModels.contains { $0["id"] as? String == profile.id })

        var chatRequest = URLRequest(url: try XCTUnwrap(URL(string: endpoint.baseURL + "/chat/completions")))
        chatRequest.httpMethod = "POST"
        chatRequest.setValue("Bearer \(endpoint.apiKey)", forHTTPHeaderField: "Authorization")
        chatRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        chatRequest.httpBody = try JSONSerialization.data(withJSONObject: [
            "model": profile.id,
            "stream": false,
            "messages": [
                ["role": "system", "content": "Answer briefly and directly."],
                ["role": "user", "content": "Reply with exactly the word OK."],
            ],
        ])

        let (chatData, chatResponse) = try await URLSession.shared.data(for: chatRequest)
        XCTAssertEqual(
            (chatResponse as? HTTPURLResponse)?.statusCode,
            200,
            String(decoding: chatData, as: UTF8.self)
        )
        let chatJSON = try XCTUnwrap(JSONSerialization.jsonObject(with: chatData) as? [String: Any])
        guard
            let choices = chatJSON["choices"] as? [[String: Any]],
            let firstChoice = choices.first,
            let message = firstChoice["message"] as? [String: Any],
            let content = message["content"] as? String
        else {
            XCTFail("Unexpected chat response: \(String(decoding: chatData, as: UTF8.self))")
            return
        }
        XCTAssertFalse(content.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines).isEmpty)

        // A fresh execution channel is allowed to rotate its token and port,
        // while the logical canonical profile/model identity stays exact.
        let originalRoute = try endpoint.nativeRoute()
        let originalProfile = try XCTUnwrap(port.modelProfile(profile.id))
        service.stop()
        let replacement = try await service.start(profile, expectedRevision: originalProfile.revision,
            applicationPort: port)
        XCTAssertNotEqual(replacement.apiKey, endpoint.apiKey)
        XCTAssertTrue(NSDictionary(dictionary: originalRoute).isEqual(to: try replacement.nativeRoute()))
        XCTAssertEqual(try port.modelProfile(profile.id), originalProfile, "A channel restart cannot rewrite model authority.")
        XCTAssertThrowsError(try service.validate(endpoint, applicationPort: port))
        let proof: [String: Any] = ["native_model_route": originalRoute,
            "endpoint": replacement.baseURL, "model": replacement.modelID,
            "credential_sha256": MacRunModelCredential.digest(replacement.apiKey)]
        try MacAgentHostRuntime.validateNativeModelChannel(proof, endpoint: replacement, applicationPort: port)
        var foreign = proof
        foreign["model"] = "foreign-model"
        do {
            try MacAgentHostRuntime.validateNativeModelChannel(foreign, endpoint: replacement, applicationPort: port)
            XCTFail("A proxy channel must never authorize another model.")
        } catch {}
        let settings = MacLocalModelConfigurationStore(applicationPort: port)
        var changed = settings.parameters(for: profile)
        changed.temperature = 0.123
        _ = try settings.saveParameters(changed, for: profile, expectedRevision: UInt64(originalProfile.revision))
        XCTAssertThrowsError(try service.validate(replacement, applicationPort: port))
        do {
            _ = try await service.start(profile, expectedRevision: originalProfile.revision, applicationPort: port)
            XCTFail("Changed profile authority must refuse the original frozen Run.")
        } catch {}
    }
}
