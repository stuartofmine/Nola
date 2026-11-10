// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class MacModelConnectionServiceTests: XCTestCase {
    func testImportedProviderFalseSurvivesOmittedOptionalCapabilityFields() throws {
        let data = Data(#"{"toolCalls":false,"imageInput":false,"reasoning":false,"samplingParameters":false}"#.utf8)
        let declaration = try JSONDecoder().decode(MacModelExecutionCapabilities.self, from: data)
        XCTAssertFalse(declaration.toolCalls)
        XCTAssertFalse(declaration.samplingRequiresNoReasoning)
        XCTAssertNil(declaration.chatOutputTokenField)
    }

    func testSourceManagementDistinguishesLiteralLoopbackFromRemoteOwner() throws {
        for address in ["http://127.0.0.1:5001", "https://[::1]:5001"] {
            XCTAssertTrue(MacModelConnectionsView.isLocalManagementHost(try XCTUnwrap(URL(string: address))))
        }
        for address in ["https://host.example", "http://192.0.2.1:5001", "http://localhost:5001",
                        "http://127.0.0.1.example:5001", "ftp://127.0.0.1", "http://user@127.0.0.1"] {
            XCTAssertFalse(MacModelConnectionsView.isLocalManagementHost(try XCTUnwrap(URL(string: address))))
        }
    }

    func testCapabilityOverrideProjectsRuntimeAndClearPreservesSelection() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at:root) }
        let application = try MacApplicationPort(databaseURL:root.appendingPathComponent("override.sqlite"))
        let service = MacModelConnectionService(application:application)
        let initial = try service.create(name:"Fixture", endpoint:"https://fixture.invalid/v1", wireProtocol:"responses", secret:"")
        let modelData = Data(#"{"model_id":"fixture","display_name":"Fixture","protocol":"responses","context_window":null,"output_limit":null,"modalities":[],"reasoning_levels":[],"execution_capabilities":null}"#.utf8)
        let model = try JSONDecoder().decode(MacModelConnectionSnapshot.Model.self, from:modelData)
        let modelJSON = try JSONDecoder().decode(MacCanonicalJSONValue.self, from:modelData)
        let catalog: MacModelConnectionSnapshot = try service.command(["type":.string("refresh_connection_catalog"),
            "connection_id":.string(initial.id), "expected_revision":.signedInteger(initial.revision),
            "models":.array([modelJSON]), "observed_at_ms":.signedInteger(1)])
        let selected: MacModelConnectionSnapshot = try service.command(["type":.string("configure_connection_models"),
            "connection_id":.string(initial.id), "expected_revision":.signedInteger(catalog.revision),
            "enabled_model_ids":.array([.string(model.id)]), "selected_model_id":.string(model.id),
            "updated_at_ms":.signedInteger(2)])
        let capabilities = MacCanonicalJSONValue.object(["toolCalls":.bool(true), "imageInput":.bool(false),
            "reasoning":.bool(false), "samplingParameters":.bool(false)])
        let overridden = try service.setCapabilityOverride(selected, model:model, capabilities:capabilities)
        XCTAssertNil(overridden.document.catalog?.models.first?.execution_capabilities)
        XCTAssertNotNil(overridden.document.capability_assessments?.first?.user_override)
        struct Profile: Decodable { let selected: Bool; let source_json: String }
        let profiles: [Profile] = try service.command(["type":.string("list_profiles")])
        let profile = try XCTUnwrap(profiles.first)
        XCTAssertTrue(profile.selected)
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with:Data(profile.source_json.utf8)) as? [String:Any])
        XCTAssertEqual(source["executionCapabilityBasis"] as? String, "user_override")
        XCTAssertEqual((source["executionCapabilities"] as? [String:Any])?["toolCalls"] as? Bool, true)
        let cleared = try service.setCapabilityOverride(overridden, model:model, capabilities:nil)
        XCTAssertNil(cleared.document.capability_assessments?.first?.user_override)
        let after: [Profile] = try service.command(["type":.string("list_profiles")])
        XCTAssertTrue(try XCTUnwrap(after.first).selected)
        XCTAssertThrowsError(try service.setCapabilityOverride(selected, model:model, capabilities:capabilities))
    }

    func testCatalogTransportBoundsStreamingBodyAndRejectsRedirect() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CatalogFixtureProtocol.self]
        for path in ["oversized", "declared", "redirect"] {
            let started = ContinuousClock.now
            do {
                _ = try await AppleModelCatalogTransport.fetch(
                    URLRequest(url: URL(string: "https://catalog.invalid/" + path)!),
                    configuration: configuration)
                XCTFail("Expected rejection: " + path)
            } catch let error as URLError {
                XCTAssertEqual(error.code, path == "redirect" ? .badServerResponse : .dataLengthExceedsMaximum)
            }
            XCTAssertLessThan(ContinuousClock.now - started, .seconds(5), "A buffered catalog response must not spend seconds iterating individual bytes")
        }
        let (data, _) = try await AppleModelCatalogTransport.fetch(
            URLRequest(url: URL(string: "https://catalog.invalid/valid")!), configuration: configuration)
        XCTAssertEqual(String(data: data, encoding: .utf8), "{\"data\":[]}")
    }

    func testCancellingActiveCatalogReadSettlesAndReleasesCapacity() async throws {
        let started = expectation(description: "Catalog request reached its transport")
        CatalogFixtureProtocol.observer.set { started.fulfill() }
        defer { CatalogFixtureProtocol.observer.set(nil) }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CatalogFixtureProtocol.self]
        let request = Task {
            try await AppleModelCatalogTransport.fetch(
                URLRequest(url: URL(string: "https://catalog.invalid/waiting")!), configuration: configuration)
        }
        defer { request.cancel() }
        await fulfillment(of: [started], timeout: 2)
        request.cancel()
        do {
            _ = try await request.value
            XCTFail("A cancelled catalog read cannot report a completed catalog")
        } catch is CancellationError {} catch { XCTFail("Unexpected cancellation result: \(error)") }
        let (data, _) = try await AppleModelCatalogTransport.fetch(
            URLRequest(url: URL(string: "https://catalog.invalid/valid")!), configuration: configuration)
        XCTAssertEqual(String(data: data, encoding: .utf8), "{\"data\":[]}")
    }

    func testCatalogRedirectDelegateNeverForwardsCredential() {
        let transport = AppleModelCatalogTransport()
        let url = URL(string: "https://catalog.invalid/models")!
        let task = URLSession.shared.dataTask(with: url)
        transport.urlSession(URLSession.shared, task: task,
            willPerformHTTPRedirection: HTTPURLResponse(url: url, statusCode: 302, httpVersion: nil,
                headerFields: ["Location": "https://untrusted.invalid"])!,
            newRequest: URLRequest(url: URL(string: "https://untrusted.invalid")!)) { request in
                XCTAssertNil(request)
            }
        task.cancel()
    }

    func testAlreadyCancelledCatalogDoesNotStartNetwork() async {
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await AppleModelCatalogTransport.fetch(
                URLRequest(url: URL(string: "https://catalog.invalid/must-not-start")!))
        }
        do {
            _ = try await task.value
            XCTFail("Cancelled task must not issue a directory request")
        } catch is CancellationError {
        } catch {
            XCTFail("Expected cancellation, got \(error)")
        }
    }

    func testAnonymousConnectionUsesCurrentCoreCommandsAndDisconnectDoesNotDelete() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("connection.sqlite"))
        let service = MacModelConnectionService(application: application)
        let created = try service.create(name: "Fixture", endpoint: "http://127.0.0.1:9999/v1",
            wireProtocol: "responses", secret: "")
        XCTAssertTrue(created.document.connection.enabled)
        XCTAssertNil(created.document.connection.account_id)
        XCTAssertEqual(try service.credential(connectionID: created.id, endpoint: "http://127.0.0.1:9999/v1",
            wireProtocol: "responses").secret, "")
        _ = try service.disconnect(created)
        let retained = try XCTUnwrap(service.connections().first(where: { $0.id == created.id }))
        XCTAssertFalse(retained.document.connection.enabled)
        XCTAssertThrowsError(try service.credential(connectionID: created.id, endpoint: "http://127.0.0.1:9999/v1",
            wireProtocol: "responses"))
    }
}

private nonisolated final class CatalogFixtureObserver: @unchecked Sendable {
    private let lock = NSLock()
    private var started: (() -> Void)?
    func set(_ callback: (() -> Void)?) { lock.withLock { started = callback } }
    func notify() { let callback = lock.withLock { started }; callback?() }
}

private final class CatalogFixtureProtocol: URLProtocol, @unchecked Sendable {
    static let observer = CatalogFixtureObserver()
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "catalog.invalid" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let path = request.url!.lastPathComponent
        if path == "waiting" { Self.observer.notify(); return }
        let response = HTTPURLResponse(url: request.url!, statusCode: path == "redirect" ? 302 : 200,
            httpVersion: "HTTP/1.1", headerFields: path == "declared" ? ["Content-Length": "2097153"] : [:])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: path == "oversized" ? Data(repeating: 32, count: 2097153) : Data("{\"data\":[]}".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
