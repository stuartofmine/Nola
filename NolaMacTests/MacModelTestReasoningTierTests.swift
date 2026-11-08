// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

final class MacModelTestReasoningTierTests: XCTestCase {
    private func request(
        root: URL,
        port: MacApplicationPort,
        protocolValue: MacModelWireProtocol,
        reasoning: MacReasoningConfiguration,
        outputLimit: Int,
        preference: String = "automatic"
    ) throws -> MacLocalAgentRunRequest {
        var configuration = MacAppleChatConfiguration(
            profileID: "fixture-profile",
            baseURL: try XCTUnwrap(URL(string: "https://fixture.invalid/v1")),
            apiKey: "fixture-key", model: "fixture-model"
        )
        configuration.wireProtocol = protocolValue
        configuration.reasoning = reasoning
        configuration.parameters.maxOutputTokens = outputLimit
        configuration.parameters.reasoningPreference = preference
        configuration.parameters.temperature = 0.7
        configuration.parameters.topP = 0.9
        configuration.executionCapabilities = MacModelExecutionCapabilities(
            toolCalls: false, imageInput: false, reasoning: true, samplingParameters: false
        )
        return try MacModelTestRunRequestFactory.remote(
            prompt: "fixture", configuration: configuration,
            environment: MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root),
            applicationPort: port
        )
    }

    func testModelProbePreservesSelectedBudgetWithoutRejectingUnselectedHighBudget() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.db"))
        let request = try request(root: root, port: port, protocolValue: .anthropic,
                                  reasoning: .init(control: .budget), outputLimit: 6144, preference: "4096")
        XCTAssertNil(request.reasoningTier)
        XCTAssertEqual(request.reasoningPreference, "4096")
        let data = try await MacAgentHostRuntime().initialRequestData(for: request, localEndpoint: nil)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
        XCTAssertNil(payload["reasoning_tier"], "Unselected tier must be absent, not JSON null")
        XCTAssertEqual(selection["kind"] as? String, "budget")
        XCTAssertEqual(selection["tokens"] as? Int, 4096)
    }

    func testModelProbeRejectsSelectedBudgetAtOutputLimitInsteadOfSilentlyTestingAutomatic() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.db"))
        let request = try request(root: root, port: port, protocolValue: .anthropic,
                                  reasoning: .init(control: .budget), outputLimit: 4096, preference: "4096")
        do {
            _ = try await MacAgentHostRuntime().initialRequestData(for: request, localEndpoint: nil)
            XCTFail("The selected budget must fail shared admission, not become Automatic")
        } catch MacApplicationServiceError.rejected(let message) {
            XCTAssertTrue(message.contains("budget"), message)
            XCTAssertTrue(message.contains("output"), message)
        } catch {
            XCTFail("Expected shared budget rejection, not an unrelated service error: \(error)")
        }
    }

    func testModelProbePreservesSelectedEffortForBothHttpProtocols() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.db"))
        for (protocolValue, values, expected) in [
            (MacModelWireProtocol.responses, ["low", "high"], "low"),
            (MacModelWireProtocol.chatCompletions, ["low", "medium", "high"], "medium")
        ] {
            let request = try request(root: root, port: port, protocolValue: protocolValue,
                                      reasoning: .init(control: .effort, efforts: values), outputLimit: 4096, preference: expected)
            let data = try await MacAgentHostRuntime().initialRequestData(for: request, localEndpoint: nil)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
            XCTAssertNil(payload["reasoning_tier"], "Provider effort must not be paired with a null agent tier")
            XCTAssertEqual(selection["kind"] as? String, "effort")
            XCTAssertEqual(selection["value"] as? String, expected)
        }
    }
}
