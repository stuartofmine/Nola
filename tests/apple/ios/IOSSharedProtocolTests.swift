// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSSharedProtocolTests: XCTestCase {
    func testHostSetupUsesStrictVersionAndSafeOrigin() throws {
        let valid = #"{"type":"nola.server.connection","protocol_version":1,"origin":"http://192.168.1.8:5001","host_id":"host-fixture","access_token":"fixture-token"}"#
        XCTAssertEqual(AppleHostConnectionSetup.decode(valid)?.hostID, "host-fixture")
        for invalid in [valid.replacingOccurrences(of: #""protocol_version":1"#, with: #""protocol_version":true"#),
                        valid.replacingOccurrences(of: #""protocol_version":1"#, with: #""protocol_version":2"#),
                        valid.replacingOccurrences(of: "192.168.1.8:5001", with: "user:pass@192.168.1.8:5001"),
                        valid.replacingOccurrences(of: "192.168.1.8:5001", with: "8.8.8.8:5001"),
                        valid.replacingOccurrences(of: "192.168.1.8:5001", with: "192.168.1.8:5001?token=secret")] {
            XCTAssertNil(AppleHostConnectionSetup.decode(invalid))
        }
        XCTAssertTrue(AppleHostConnectionSetup.sameOrigin(URL(string: "https://Studio.local")!, URL(string: "https://studio.local:443/api/mobile")!))
        XCTAssertFalse(AppleHostConnectionSetup.sameOrigin(URL(string: "http://one.local")!, URL(string: "http://two.local")!))
    }

    func testCancellationCarriesExactAdmissionOwnerWithoutClaimingTerminal() throws {
        let identity = AppleRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        let object = try AppleRemoteCancellationRequest(identity: identity, requestDigest: "digest").object()
        XCTAssertEqual(Set(object.keys), ["identity", "request_digest"])
        XCTAssertEqual((object["identity"] as? [String: Any])?["submission_id"] as? String, "run")
        XCTAssertThrowsError(try AppleRemoteCancellationRequest(identity: identity, requestDigest: ""))
    }

    func testPermissionWriteCarriesExplicitInactiveOwnerAndRevision() throws {
        let command = SetConversationPermissionCommand(conversationID: "conversation", expectedRevision: 7,
            expectedRunID: nil, expectedSourceHostID: nil, profile: "cautious", updatedAtMs: 100)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(command)) as? [String: Any])
        XCTAssertEqual(object["type"] as? String, "set_conversation_permission")
        XCTAssertEqual(object["expected_revision"] as? Int, 7)
        XCTAssertTrue(object["expected_run_id"] is NSNull)
        XCTAssertTrue(object["expected_source_host_id"] is NSNull)
    }

    func testExecutionIntentSeparatesInheritanceFromExecutionLocation() throws {
        for selection in [SelectConversationExecutionCommand<AgentCoreJSONValue>.Selection.inherit, .location(executionHost: "remote")] {
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(selection)) as? [String: Any])
            XCTAssertNil(object["modelSelectionID"])
            XCTAssertNil(object["executionHostBinding"])
            if object["kind"] as? String == "location" {
                XCTAssertEqual(object["execution_host"] as? String, "remote")
            } else { XCTAssertEqual(object["kind"] as? String, "inherit") }
        }
    }

    func testSharedResultEvidenceRejectsOverlappingCancelledEffects() throws {
        let wire = Data(#"{"kind":"cancelled","cancellation_id":"cancel-1","observed_effect_refs":["effect-1"],"unknown_effect_refs":["effect-1"]}"#.utf8)
        let value = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: wire)
        XCTAssertFalse(value.hasRequiredEvidenceFields)
    }

    func testSharedResultEvidencePreservesAnsweredVariant() throws {
        let wire = Data(#"{"kind":"answered","answer_part_refs":["part-1"]}"#.utf8)
        let value = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: wire)
        XCTAssertTrue(value.hasRequiredEvidenceFields)
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value)),
                       try JSONDecoder().decode(AgentCoreJSONValue.self, from: wire))
    }

    func testSharedJournalReadUsesDurableCursorAndBoundedPage() throws {
        let identity = AppleAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        let cursor = AppleAssistantJournalCursor(identity: identity, sequence: "9007199254740993", record_digest: "digest")
        let request = AppleAssistantJournalReadRequest<AgentCoreJSONValue>.records(after: cursor, through: cursor)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertEqual(object["operation"] as? String, "read")
        XCTAssertEqual(object["max_records"] as? Int, 32)
        XCTAssertEqual(object["max_encoded_bytes"] as? Int, 256 * 1024)
        XCTAssertEqual((object["after"] as? [String: Any])?["sequence"] as? String, "9007199254740993")
    }

    func testSharedDiscoveryUsesProviderAuthenticationAndCatalogOnly() throws {
        let request = try AppleModelCatalogDiscovery.request(endpoint: "https://example.test/v1/", wireProtocol: "anthropic", secret: "test-secret")
        XCTAssertEqual(request.url?.path, "/v1/models")
        XCTAssertEqual(request.httpMethod, "GET")
        XCTAssertEqual(request.value(forHTTPHeaderField: "x-api-key"), "test-secret")
        XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
        let url = try XCTUnwrap(request.url)
        let response = try XCTUnwrap(HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
        let models = try AppleModelCatalogDiscovery.parse(data: Data(#"{"data":[{"id":"a","display_name":"A"},{"id":"b"},{"id":""}]}"#.utf8), response: response)
        XCTAssertEqual(models.map(\.id), ["a", "b"])
        XCTAssertEqual(models.map(\.displayName), ["A", "b"])
        XCTAssertThrowsError(try AppleModelCatalogDiscovery.parse(data: Data(#"{"models":[]}"#.utf8), response: response))
    }

    @MainActor func testCatalogReadConsumesBoundedChunksWithoutBlockingThePicker() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [IOSCatalogFixtureProtocol.self]
        for path in ["oversized", "declared", "redirect"] {
            let started = ContinuousClock.now
            do {
                _ = try await AppleModelCatalogTransport.fetch(
                    URLRequest(url: URL(string: "https://catalog.invalid/" + path)!), configuration: configuration)
                XCTFail("Expected catalog rejection: " + path)
            } catch let error as URLError {
                XCTAssertEqual(error.code, path == "redirect" ? .badServerResponse : .dataLengthExceedsMaximum)
            }
            XCTAssertLessThan(ContinuousClock.now - started, .seconds(5))
        }
        let (data, _) = try await AppleModelCatalogTransport.fetch(
            URLRequest(url: URL(string: "https://catalog.invalid/valid")!), configuration: configuration)
        XCTAssertEqual(String(data: data, encoding: .utf8), "{\"data\":[]}")
    }

    func testDiscoveredReasoningMetadataSurvivesCoreCatalogSerialization() throws {
        let url = try XCTUnwrap(URL(string: "https://models.example/v1/models"))
        let response = try XCTUnwrap(HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
        for levels in [#""supported_reasoning_levels":[{"effort":"none"},{"effort":"low"},{"effort":"high"},{"effort":"max"},{"effort":"high"}]"#,
                       #""reasoning_levels":["none","low","high","max"]"#] {
            let body = "{\"data\":[{\"id\":\"fixture\",\"display_name\":\"Readable\",\"context_window\":128000,\"max_output_tokens\":32000," + levels + "}]}"
            let model = try XCTUnwrap(AppleModelCatalogDiscovery.parse(data: Data(body.utf8), response: response).first)
            XCTAssertEqual(model.reasoningLevels, ["none", "low", "high", "max"])
            let entry = try XCTUnwrap(JSONSerialization.jsonObject(with: model.catalogData(wireProtocol: "chat_completions")) as? [String: Any])
            XCTAssertEqual(entry["reasoning_levels"] as? [String], model.reasoningLevels)
            XCTAssertEqual(entry["context_window"] as? Int, 128000)
            XCTAssertEqual((entry["execution_capabilities"] as? [String: Any])?["reasoning"] as? Bool, true)
        }
        let body = #"{"data":[{"id":"gpt-looking-name"},{"id":"empty","supported_reasoning_levels":[],"reasoning_levels":["high"]}]}"#
        let models = try AppleModelCatalogDiscovery.parse(data: Data(body.utf8), response: response)
        XCTAssertTrue(models.allSatisfy { $0.reasoningLevels.isEmpty })
        XCTAssertTrue(models.allSatisfy { $0.reasoningSupported == nil })
    }

    @MainActor func testDiscoveredModelCanUpdateAnExistingManualConnectionWithoutLosingEfforts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("catalog-efforts-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("isolated.sqlite")
        let application = try IOSApplicationRuntime(databaseURL: database)
        let service = IOSModelConnectionService(application: application)
        let initial = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1", wireProtocol: "chat_completions", secret: "", modelID: "fixture-model")
        let discovered = AppleModelCatalogDiscovery.Model(id: "fixture-model", displayName: "Discovered", reasoningLevels: ["low", "high"])
        let refreshed = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1", wireProtocol: "chat_completions", secret: "", modelID: "fixture-model", discoveredModel: discovered)
        XCTAssertEqual(refreshed.id, initial.id)
        XCTAssertGreaterThan(refreshed.revision, initial.revision)
        XCTAssertEqual(refreshed.document.catalog?.models.first?.reasoning_levels, ["low", "high"])
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let profile = try XCTUnwrap(reopened.listModelProfiles().first { $0.modelName == "fixture-model" })
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any])
        XCTAssertEqual((source["reasoning"] as? [String: Any])?["efforts"] as? [String], ["low", "high"])
    }

    @MainActor func testObservedAnthropicEffortsKeepExactDeclaredLevels() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("anthropic-catalog-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("isolated.sqlite"))
        let service = IOSModelConnectionService(application: application)
        let model = AppleModelCatalogDiscovery.Model(id: "adaptive-observed", displayName: "Fixture", reasoningLevels: ["low", "high"])
        let saved = try service.create(name: "Fixture", endpoint: "https://fixture.invalid/v1", wireProtocol: "anthropic", secret: "", modelID: model.id, discoveredModel: model)
        XCTAssertEqual(saved.document.catalog?.models.first?.reasoning_levels, ["low", "high"])
        let profile = try XCTUnwrap(application.listModelProfiles().first { $0.modelName == model.id })
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any])
        XCTAssertEqual((source["reasoning"] as? [String: Any])?["efforts"] as? [String], ["low", "high"])
    }

}


private nonisolated final class IOSCatalogFixtureProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "catalog.invalid" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let path = request.url!.lastPathComponent
        let response = HTTPURLResponse(url: request.url!, statusCode: path == "redirect" ? 302 : 200,
            httpVersion: "HTTP/1.1", headerFields: path == "declared" ? ["Content-Length": "2097153"] : [:])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: path == "oversized" ? Data(repeating: 32, count: 2097153) : Data("{\"data\":[]}".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}

}
