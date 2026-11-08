import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacLocalAgentCredentialGateTests: XCTestCase {
    nonisolated final class Credentials: @unchecked Sendable {
        private let lock = NSLock()
        private var values: [String: String] = ["endpoint:endpoint:original": "fixture-key"]
        func read(_ account: String) -> String? { lock.lock(); defer { lock.unlock() }; return values[account] }
        func set(_ account: String, _ value: String?) { lock.lock(); defer { lock.unlock() }; values[account] = value }
    }

    @MainActor struct Fixture {
        let root: URL
        let port: MacApplicationPort
        let credentials: Credentials
        let store: MacAppleChatConfigurationStore

        init() throws {
            root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-credential-gate-\(UUID())")
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
            credentials = Credentials()
            let reader = credentials
            store = MacAppleChatConfigurationStore(applicationPort: port, credentialReader: { reader.read($0) })
            try save()
        }

        func save(account: String = "endpoint:endpoint:original", enabled: Bool = true,
                  endpointEnabled: Bool = true, deleted: Bool = false,
                  endpoint: String = "https://fixture.invalid/v1", model: String = "upstream-model",
                  wireProtocol: String = "chat_completions") throws {
            let source: [String: Any] = ["schemaVersion": 2, "endpointID": "endpoint",
                "endpointName": "Fixture", "baseURL": endpoint,
                "endpointEnabled": endpointEnabled, "endpointOrder": 0, "modelEnabled": true,
                "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
                "wireProtocol": wireProtocol, "credentialAccount": account]
            let current = try port.listModelProfiles().first { $0.id == "profile" }
            try port.upsertModelProfile(MacCanonicalModelProfile(id: "profile", displayName: "Fixture",
                provider: "openai-compatible", modelName: model, runtime: "remote",
                sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: "{}", selected: false, enabled: enabled, revision: current?.revision ?? 0,
                createdAtMs: 1, updatedAtMs: 2, deletedAtMs: deleted ? 2 : nil))
        }

        func request(profileID: String? = "profile", key: String = "fixture-key",
                     fallbacks: [MacRemoteModelFallback] = []) throws -> MacLocalAgentRunRequest {
            try MacLocalAgentRunRequest(conversationID: "conversation", prompt: "fixture",
                projectURL: root, attachmentURLs: [], modelID: "upstream-model", modelProfileID: profileID,
                endpoint: "https://fixture.invalid/v1", apiKey: key, modelTestMode: true, modelFallbacks: fallbacks,
                memoryEnabled: false, databaseURLOverride: root.appendingPathComponent("fixture.sqlite"),
                applicationPort: port)
        }

        func close() { try? FileManager.default.removeItem(at: root) }
    }

    func testRevokedChangedDisabledAndReboundCredentialsNeverDispatch() throws {
        for change in ["deleted-key", "changed-key", "disabled-profile", "disabled-endpoint", "changed-account", "deleted-profile", "changed-endpoint", "changed-model", "changed-protocol"] {
            let fixture = try Fixture()
            defer { fixture.close() }
            let selection = try MacLocalAgentCredentialGate.capture(fixture.request(), store: fixture.store)
            switch change {
            case "deleted-key": fixture.credentials.set("endpoint:endpoint:original", nil)
            case "changed-key": fixture.credentials.set("endpoint:endpoint:original", "different-key")
            case "disabled-profile": try fixture.save(enabled: false)
            case "disabled-endpoint": try fixture.save(endpointEnabled: false)
            case "changed-endpoint": try fixture.save(endpoint: "https://different.invalid/v1")
            case "changed-model": try fixture.save(model: "different-model")
            case "changed-protocol": try fixture.save(wireProtocol: "responses")
            case "changed-account":
                fixture.credentials.set("endpoint:endpoint:replacement", "fixture-key")
                try fixture.save(account: "endpoint:endpoint:replacement")
            default: try fixture.save(deleted: true)
            }
            var dispatches = 0
            XCTAssertThrowsError(try MacLocalAgentCredentialGate.launch(selection: selection, isResume: true,
                store: fixture.store, operation: { dispatches += 1 }), change)
            XCTAssertEqual(dispatches, 0, change)
        }
    }

    func testExactCredentialAllowsDispatchAndKnownProfileFailureCannotBecomeTemporary() throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let selection = try MacLocalAgentCredentialGate.capture(fixture.request(), store: fixture.store)
        var dispatches = 0
        try MacLocalAgentCredentialGate.launch(selection: selection, isResume: true,
            store: fixture.store, operation: { dispatches += 1 })
        XCTAssertEqual(dispatches, 1)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(profileID: "missing"), store: fixture.store))
        fixture.credentials.set("endpoint:endpoint:original", nil)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(), store: fixture.store))
    }

    func testProtocolChangedDuringPreparationCannotBeCapturedAsTheOriginalRoute() throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let request = try fixture.request()
        try fixture.save(wireProtocol: "responses")
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(request, store: fixture.store))
    }

    func testAnonymousIsExplicitAndTemporaryCredentialsCannotResume() throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        try fixture.save(account: "")
        fixture.credentials.set("endpoint:endpoint:original", nil)
        let anonymous = try MacLocalAgentCredentialGate.capture(fixture.request(key: ""), store: fixture.store)
        var dispatches = 0
        try MacLocalAgentCredentialGate.launch(selection: anonymous, isResume: true,
            store: fixture.store, operation: { dispatches += 1 })
        XCTAssertEqual(dispatches, 1)
        let temporary = try MacLocalAgentCredentialGate.capture(fixture.request(profileID: nil), store: fixture.store)
        try MacLocalAgentCredentialGate.launch(selection: temporary, isResume: false,
            store: fixture.store, operation: { dispatches += 1 })
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.launch(selection: temporary, isResume: true,
            store: fixture.store, operation: { dispatches += 1 }))
        XCTAssertEqual(dispatches, 2)
    }

    func testNativeEndpointDoesNotResolveCloudProfileCredentials() throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let unowned = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345/v1",
            apiKey: "unowned", modelID: "native-model", contextWindowTokens: 4096, profileRevision: 1)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(profileID: "not-a-cloud-profile"),
            localEndpoint: unowned, store: fixture.store))
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(fixture.request(profileID: "not-a-cloud-profile"),
            store: fixture.store))
    }

    func testAllConfiguredFallbackCredentialsAreConservativelyRequired() throws {
        let fixture = try Fixture()
        defer { fixture.close() }
        let fallback = MacRemoteModelFallback(profileID: "missing-fallback", endpointName: "Fallback",
            endpoint: "https://fallback.invalid/v1", apiKey: "fixture-fallback", model: "fallback")
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.capture(
            fixture.request(fallbacks: [fallback]), store: fixture.store),
            "Until shared executable-route admission is wired, even an unused frozen fallback is checked.")
    }
}
