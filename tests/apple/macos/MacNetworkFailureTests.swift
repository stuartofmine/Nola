// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

final class MacNetworkFailureTests: XCTestCase {
    func testOfflineCodeAloneDoesNotMeanPermissionDenied() {
        XCTAssertEqual(MacNetworkFailure.classify(URLError(.notConnectedToInternet)), .offline)
        let proseOnly = NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [
            NSLocalizedDescriptionKey: "unsatisfied (Local network prohibited)",
            NSURLErrorFailingURLErrorKey: URL(string: "http://192.168.1.2/v1/models")!,
        ])
        XCTAssertEqual(MacNetworkFailure.classify(proseOnly), .offline)
        XCTAssertNil(MacNetworkFailure.classify(NSError(domain: "Provider", code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (Local network prohibited)",
        ])))
        XCTAssertNil(MacNetworkFailure.classify(URLError(.timedOut)))
    }

    func testExplicitUnderlyingOSPathPolicyIsDistinguishedFromOffline() {
        let denied = NSError(domain: "kCFErrorDomainCFNetwork", code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (Local network prohibited), interface: en0[802.11]",
        ])
        let wrapped = NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [NSUnderlyingErrorKey: denied])
        XCTAssertEqual(MacNetworkFailure.classify(wrapped), .localNetworkDenied)
        XCTAssertNotEqual(MacNetworkFailure.message(for: wrapped), MacNetworkFailure.message(for: URLError(.notConnectedToInternet)))
        let ordinary = NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (No network route)",
        ])
        XCTAssertEqual(MacNetworkFailure.classify(ordinary), .offline)
    }

    func testActualDiscoveryPreservesPolicyEvidenceForTheUIWithoutNetworkAccess() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [DeniedModelCatalogProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        do {
            _ = try await MacModelCatalogClient.fetch(baseURL: URL(string: "http://catalog.invalid/v1/responses")!, apiKey: "fixture-key", session: session)
            XCTFail("Expected the injected URLSession policy denial")
        } catch {
            XCTAssertEqual(MacNetworkFailure.classify(error), .localNetworkDenied)
        }
        XCTAssertEqual(MacModelCatalogClient.catalogURL(URL(string: "http://catalog.invalid/v1/chat/completions")!).path, "/v1/models")
    }

    func testBuiltMacAppDeclaresLocalNetworkUsage() {
        XCTAssertFalse((Bundle.main.object(forInfoDictionaryKey: "NSLocalNetworkUsageDescription") as? String)?.isEmpty ?? true)
    }

    func testDiscoveryUsesDraftCredentialAndReturnsSortedUniqueModelIDs() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SuccessfulModelCatalogProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let models = try await MacModelCatalogClient.fetch(
            baseURL: URL(string: "http://catalog.invalid/v1/responses")!,
            apiKey: "current-draft-key", session: session
        )
        XCTAssertEqual(models, ["chat-fast", "reasoning-large"])
    }

    func testDiscoveryNormalizesBaseAndInferenceRoutesWithoutDuplicatingVersion() {
        for route in ["/v1", "/v1/", "/v1/responses", "/v1/chat/completions"] {
            XCTAssertEqual(
                MacModelCatalogClient.catalogURL(URL(string: "http://catalog.invalid" + route)!).path,
                "/v1/models", route
            )
        }
    }

    func testDiscoveryPreservesHTTPFailureWithoutLeakingResponseBody() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [RejectedModelCatalogProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        for status in [401, 403, 404, 405, 429, 500] {
            do {
                _ = try await MacModelCatalogClient.fetch(
                    baseURL: URL(string: "http://catalog.invalid/status/\(status)")!,
                    apiKey: "fixture-key", session: session
                )
                XCTFail("Expected HTTP rejection")
            } catch {
                XCTAssertEqual(error as? MacModelCatalogHTTPError, MacModelCatalogHTTPError(statusCode: status))
                XCTAssertFalse(MacNetworkFailure.message(for: error).contains("secret-response-marker"))
                XCTAssertFalse(MacNetworkFailure.message(for: error).isEmpty)
            }
        }
    }
}

private final class SuccessfulModelCatalogProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        XCTAssertEqual(request.url?.path, "/v1/models")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer current-draft-key")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "application/json")
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(#"{"data":[{"id":"reasoning-large"},{"id":"chat-fast"},{"id":"reasoning-large"}]}"#.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

private final class RejectedModelCatalogProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let url = request.url!
        let status = Int(url.deletingLastPathComponent().lastPathComponent)!
        let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("secret-response-marker".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

private final class DeniedModelCatalogProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        XCTAssertEqual(request.url?.path, "/v1/models")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer fixture-key")
        client?.urlProtocol(self, didFailWithError: NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (Local network prohibited)",
        ]))
    }
    override func stopLoading() {}
}
