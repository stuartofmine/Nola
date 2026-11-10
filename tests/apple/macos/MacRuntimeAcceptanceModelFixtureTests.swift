// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

#if DEBUG
final class MacRuntimeAcceptanceModelFixtureTests: XCTestCase {
    private func environment(
        execution: [String: Any] = ["toolCalls": true, "imageInput": false, "reasoning": false,
                                    "samplingParameters": false, "samplingRequiresNoReasoning": false],
        model: String = "fixture-model", wireProtocol: String = "chat_completions"
    ) throws -> [String: String] {
        let data = try JSONSerialization.data(withJSONObject: [
            "model": model, "wireProtocol": wireProtocol, "executionCapabilities": execution,
        ])
        return [
            "NOLA_MAC_RUNTIME_ACCEPTANCE_ENDPOINT": "http://127.0.0.1:1234/v1",
            "NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL": "fixture-model",
            "NOLA_MAC_RUNTIME_ACCEPTANCE_API_KEY": "fixture-secret",
            "NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON": String(decoding: data, as: UTF8.self),
            "NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_TOOL": "1",
        ]
    }

    func testExactDeclaredRouteAndDisabledCapabilitiesSurvive() throws {
        let provider = try MacRuntimeAcceptanceModelFixture.provider(environment: environment(wireProtocol: "responses"))
        XCTAssertEqual(provider.model, "fixture-model")
        XCTAssertEqual(provider.baseURL.absoluteString, "http://127.0.0.1:1234/v1")
        XCTAssertEqual(provider.wireProtocol, .responses)
        XCTAssertTrue(provider.executionCapabilities.toolCalls)
        XCTAssertFalse(provider.executionCapabilities.reasoning)
        XCTAssertFalse(provider.executionCapabilities.imageInput)
        XCTAssertFalse(provider.executionCapabilities.samplingParameters)
    }

    func testMissingDeclarationNeverCreatesAnAllFalseConfiguration() throws {
        var input = try environment()
        input.removeValue(forKey: "NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON")
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
    }

    func testDeclaredReasoningUsesProductionValidationAndProtocol() throws {
        var input = try environment(execution: ["toolCalls": true, "imageInput": false,
            "reasoning": true, "samplingParameters": false, "samplingRequiresNoReasoning": false],
            wireProtocol: "responses")
        var reasoning = MacReasoningConfiguration()
        reasoning.control = .effort
        reasoning.efforts = ["low", "medium", "high"]
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = String(decoding:
            try JSONEncoder().encode(reasoning), as: UTF8.self)
        let provider = try MacRuntimeAcceptanceModelFixture.provider(environment: input)
        XCTAssertEqual(provider.reasoning?.options.first, "low")
        XCTAssertEqual(provider.reasoning?.responses, true)
        reasoning.control = .budget
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = String(decoding:
            try JSONEncoder().encode(reasoning), as: UTF8.self)
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
    }

    func testReasoningOverrideCannotEnableDisabledCapability() throws {
        var input = try environment(wireProtocol: "responses")
        var reasoning = MacReasoningConfiguration()
        reasoning.control = .effort
        reasoning.efforts = ["low", "medium", "high"]
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = String(decoding:
            try JSONEncoder().encode(reasoning), as: UTF8.self)
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = "invalid-secret-fixture"
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input)) { error in
            XCTAssertFalse(error.localizedDescription.contains("invalid-secret-fixture"))
        }
    }

    func testFixtureCannotSwitchModelOrUnknownProtocol() throws {
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: environment(model: "other-model")))
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: environment(wireProtocol: "guess-from-url")))
    }

    func testFlagsRequireJSONBooleansAndKnownFields() throws {
        for flag: Any in [1, "true", NSNull()] {
            let input = try environment(execution: ["toolCalls": flag, "imageInput": false,
                "reasoning": false, "samplingParameters": false, "samplingRequiresNoReasoning": false])
            XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        }
        let alias = try environment(execution: ["toolCalls": true, "native_tool_calls": false,
            "imageInput": false, "reasoning": false, "samplingParameters": false, "samplingRequiresNoReasoning": false])
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: alias))
    }

    func testToolRequirementCannotEnableExplicitFalse() throws {
        var input = try environment(execution: ["toolCalls": false, "imageInput": false,
            "reasoning": false, "samplingParameters": false, "samplingRequiresNoReasoning": false])
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_TOOL"] = "0"
        let provider = try MacRuntimeAcceptanceModelFixture.provider(environment: input)
        XCTAssertFalse(provider.executionCapabilities.toolCalls)
    }

    func testLegacyVisionAndThinkingOptionsCannotEnableExplicitFalse() throws {
        for key in ["NOLA_MAC_RUNTIME_ACCEPTANCE_VISION_INPUT", "NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_THINKING"] {
            var input = try environment()
            input[key] = "1"
            XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        }
    }

    func testInvalidFixtureDiagnosticDoesNotLeakCredentials() throws {
        var input = try environment()
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON"] = "fixture-secret-invalid-json"
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input)) { error in
            XCTAssertFalse(error.localizedDescription.contains("fixture-secret"))
        }
    }
}

final class MacRuntimeAcceptancePermissionFixtureTests: XCTestCase {
    private func root() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("nola-mac-permission-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
        return url
    }

    private func pending(_ fixture: MacRuntimeAcceptancePermissionFixture, access: String = "write",
                         target: URL? = nil) throws -> MacCorePendingPermissionIdentity {
        let resource = try JSONSerialization.data(withJSONObject: [
            "execution_target": "fixture-host", "capability": "fixture.overwrite", "operation": "filesystem",
            "resources": [["kind": "file", "filesystem_id": "fixture-filesystem", "access": access,
                           "components": (target ?? fixture.url).pathComponents.filter { $0 != "/" }]],
        ])
        return .init(allowConversation: true, requestID: "canonical-request", turnID: "canonical-run", callID: "canonical-call",
                     capabilityID: "fixture.overwrite", resource: String(decoding: resource, as: UTF8.self),
                     permissionDomain: "filesystem", rationale: "fixture overwrite")
    }

    private func challenge(callID: String = "canonical-call") -> MacRemotePermissionRequest {
        .init(id: "canonical-request", allowConversation: true, protocolVersion: 1, nonce: "fixture-nonce", hostID: "fixture-host",
              clientID: "fixture-client", sessionID: "fixture-session", toolName: "fixture.overwrite",
              arguments: "{}", callID: callID, reason: "fixture", message: "fixture", expiresAt: nil)
    }

    func testAuditRequiresExactCanonicalIdentityAndUnchangedOriginalBytes() throws {
        let root = try root()
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("original\n".utf8).write(to: root.appendingPathComponent("existing.txt"))
        let fixture = try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "existing.txt")
        let identity = try pending(fixture)
        let audit = try fixture.audit(pending: identity, challenge: challenge(), runID: "canonical-run", currentBytes: fixture.originalBytes)
        XCTAssertEqual(audit["canonical_pending_verified"] as? Bool, true)
        XCTAssertEqual(audit["fixture_bytes_base64"] as? String, fixture.originalBytes.base64EncodedString())
        XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(callID: "other-call"),
                                              runID: "canonical-run", currentBytes: fixture.originalBytes))
        XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(),
                                              runID: "other-run", currentBytes: fixture.originalBytes))
        XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(),
                                              runID: "canonical-run", currentBytes: Data("changed\n".utf8)))
    }

    func testReadOrDifferentFileResourceCannotProvePendingOverwrite() throws {
        let root = try root()
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("original\n".utf8).write(to: root.appendingPathComponent("existing.txt"))
        let fixture = try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "existing.txt")
        for identity in [try pending(fixture, access: "read"), try pending(fixture, target: root.appendingPathComponent("different.txt"))] {
            XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(),
                                                  runID: "canonical-run", currentBytes: fixture.originalBytes))
        }
    }

    func testFixtureMustAlreadyExistWithoutTraversalOrSymlink() throws {
        let root = try root()
        defer { try? FileManager.default.removeItem(at: root) }
        XCTAssertThrowsError(try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "missing.txt"))
        XCTAssertThrowsError(try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "../outside.txt"))
        let original = root.appendingPathComponent("existing.txt")
        try Data("original\n".utf8).write(to: original)
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("linked.txt"), withDestinationURL: original)
        XCTAssertThrowsError(try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "linked.txt"))
        XCTAssertEqual(try Data(contentsOf: original), Data("original\n".utf8))
    }
}
#endif
