// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class MacSiwcProfileAdmissionTests: XCTestCase {
    nonisolated final class Reads: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0
        func read() -> String? { lock.lock(); defer { lock.unlock() }; count += 1; return nil }
        var total: Int { lock.lock(); defer { lock.unlock() }; return count }
    }
    func testSiwcProfileFreezesRevisionAndNeverReadsAPIKeyOrAddsFallback() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let service = MacModelConnectionService(application: port)
        let accountID = UUID().uuidString.lowercased()
        let account: MacModelAccountSnapshot = try service.command(["type": .string("save_account"), "input": .object([
            "id": .string(accountID), "display_name": .string("Fixture"), "source": .string("nola_chatgpt"),
            "credential_ref": .string("siwc:" + accountID), "state": .string("ready")]),
            "expected_revision": .null, "updated_at_ms": .signedInteger(1)])
        let connection: MacModelConnectionSnapshot = try service.command(["type": .string("save_connection"), "input": .object([
            "id": .string("siwc-" + accountID), "display_name": .string("Fixture"), "base_url": .string("https://api.openai.com/v1"),
            "protocols": .array([.string("siwc_responses")]), "account_id": .string(account.id),
            "source": .string("nola_chatgpt"), "source_id": .string(account.id), "enabled": .bool(true)]),
            "expected_revision": .null, "updated_at_ms": .signedInteger(1)])
        let catalog: MacModelConnectionSnapshot = try service.command(["type": .string("refresh_connection_catalog"),
            "connection_id": .string(connection.id), "expected_revision": .signedInteger(connection.revision),
            "models": .array([.object(["model_id": .string("fixture-model"), "display_name": .string("Fixture Model"),
                "protocol": .string("siwc_responses"), "context_window": .null, "output_limit": .null,
                "modalities": .array([]), "reasoning_levels": .array([.string("low"), .string("high"), .string("ultra")]), "execution_capabilities": .null])]),
            "observed_at_ms": .signedInteger(2)])
        try service.enable(catalog, modelIDs: ["siwc_responses:fixture-model"])
        let profile = try XCTUnwrap(try port.listModelProfiles().first)
        let reads = Reads()
        let store = MacAppleChatConfigurationStore(applicationPort: port, credentialReader: { _ in reads.read() })
        let configuration = try XCTUnwrap(store.loadThrowing(model: profile.id))
        XCTAssertEqual(try XCTUnwrap(configuration.reasoning).efforts, ["low", "high"])
        XCTAssertEqual(configuration.reasoning?.responses, true)
        let selected = try XCTUnwrap(configuration.siwcProfile)
        XCTAssertEqual(selected.revision, profile.revision)
        XCTAssertEqual(configuration.wireProtocol, .siwcResponses)
        let plan = try XCTUnwrap(store.routePlanThrowing(preferredModel: profile.id))
        XCTAssertTrue(plan.fallbacks.isEmpty)
        let request = try MacLocalAgentRunRequest(conversationID: "fixture-room", prompt: "fixture",
            projectURL: root, attachmentURLs: [], modelID: configuration.model, modelProfileID: profile.id,
            siwcProfile: selected, endpoint: configuration.baseURL.absoluteString,
            modelExecutionCapabilities: configuration.executionCapabilities, wireProtocol: .siwcResponses,
            memoryEnabled: false, databaseURLOverride: port.databaseURL, applicationPort: port)
        let captured = try MacLocalAgentCredentialGate.capture(request, store: store)
        XCTAssertTrue(captured.credentials.isEmpty)
        XCTAssertEqual(reads.total, 0)
        let runtime = MacAgentHostRuntime(credentialStore: store)
        let data = try await runtime.initialRequestData(for: request)
        let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(wire["model_profile_id"] as? String, profile.id)
        XCTAssertEqual(wire["expected_profile_revision"] as? Int64, profile.revision)
        XCTAssertNil(wire["api_key"])
        XCTAssertEqual((wire["mac_launch"] as? [String: Any])?["model_profile_revision"] as? Int64, profile.revision)
        let resumed = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: data,
            conversationID: "fixture-room", runID: request.submissionID, assistantMessageID: request.assistantMessageID)
        let frame = try XCTUnwrap(JSONSerialization.jsonObject(with: resumed) as? [String: Any])
        XCTAssertEqual((frame["configuration"] as? [String: Any])?["expected_profile_revision"] as? Int64, profile.revision)
        let arguments = await runtime.arguments(for: request, localEndpoint: nil)
        XCTAssertFalse(arguments.contains("--model")); XCTAssertFalse(arguments.contains("--endpoint"))
        // A changed profile must not silently recapture a newer selection.
        // Managed profiles can only change through their owning connection.
        let currentConnection = try XCTUnwrap(service.connections().first { $0.id == connection.id })
        _ = try service.disconnect(currentConnection)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.launch(selection: captured, isResume: true,
            store: store, operation: { XCTFail("Changed profile must not dispatch") }))
        XCTAssertEqual(reads.total, 0)
    }
}
