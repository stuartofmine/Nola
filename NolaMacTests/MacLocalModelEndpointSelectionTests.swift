// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacLocalModelEndpointSelectionTests: XCTestCase {
    func testNativeIdentitySurvivesChannelRotationButNotModelOrProfileRevisionChanges() throws {
        let model = MacLocalModelProfile.qwen35_08B.id
        let original = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345/v1",
            apiKey: "old-token", modelID: model, contextWindowTokens: 65536, profileRevision: 7)
        let fresh = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:54321/v1",
            apiKey: "fresh-token", modelID: model, contextWindowTokens: 65536, profileRevision: 7)
        let route = try original.nativeRoute()
        XCTAssertTrue(NSDictionary(dictionary: route).isEqual(to: try fresh.nativeRoute()))
        let encoded = try JSONSerialization.data(withJSONObject: route, options: [.sortedKeys])
        let text = String(decoding: encoded, as: UTF8.self)
        XCTAssertFalse(text.contains("token"))
        XCTAssertFalse(text.contains("127.0.0.1"))
        for replacement in [
            MacMLXOpenAIService.Endpoint(baseURL: fresh.baseURL, apiKey: fresh.apiKey,
                modelID: "different-model", contextWindowTokens: 65536, profileRevision: 7),
            MacMLXOpenAIService.Endpoint(baseURL: fresh.baseURL, apiKey: fresh.apiKey,
                modelID: model, contextWindowTokens: 65536, profileRevision: 8)
        ] {
            XCTAssertFalse(NSDictionary(dictionary: route).isEqual(to: try replacement.nativeRoute()))
        }
        XCTAssertThrowsError(try MacMLXOpenAIService.Endpoint(baseURL: fresh.baseURL,
            apiKey: fresh.apiKey, modelID: model, contextWindowTokens: 65536).nativeRoute())
    }

    func testCloudRouteDoesNotConsultLocalRuntime() throws {
        for modelID in [nil, "cloud-model"] as [String?] {
            let result = try MacLocalModelEndpointSelection.resolve(modelID: modelID) { _ in
                XCTFail("Remote route must not consult the local model service")
                throw TestError.unavailable
            }
            XCTAssertNil(result)
        }
    }

    func testUnavailableLocalModelDoesNotFallBackToCloud() {
        XCTAssertThrowsError(try MacLocalModelEndpointSelection.resolve(modelID: MacLocalModelProfile.qwen35_08B.id) { _ in
            throw TestError.unavailable
        }) { error in
            XCTAssertTrue(error is TestError)
        }
    }

    func testLocalEndpointResolvedExactlyOnceAsAnImmutableValue() throws {
        var calls = 0
        let result = try XCTUnwrap(MacLocalModelEndpointSelection.resolve(modelID: MacLocalModelProfile.qwen35_08B.id) { profile in
            calls += 1
            return MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345", apiKey: "test-instance-token", modelID: profile.id, contextWindowTokens: 32768)
        })
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(result.baseURL, "http://127.0.0.1:12345")
        XCTAssertEqual(result.apiKey, "test-instance-token")
        XCTAssertEqual(result.contextWindowTokens, 32768)
    }

    private enum TestError: Error { case unavailable }

    func testCapturedInstanceRejectsRestartAndNeverReturnsReplacementCredentials() throws {
        let modelID = MacLocalModelProfile.qwen35_08B.id
        let original = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345", apiKey: "original", modelID: modelID, contextWindowTokens: 32768)
        let replacements = [
            MacMLXOpenAIService.Endpoint(baseURL: original.baseURL, apiKey: "replacement", modelID: modelID, contextWindowTokens: 32768),
            MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:54321", apiKey: original.apiKey, modelID: modelID, contextWindowTokens: 32768),
            MacMLXOpenAIService.Endpoint(baseURL: original.baseURL, apiKey: original.apiKey, modelID: modelID, contextWindowTokens: 65536),
        ]
        for replacement in replacements {
            XCTAssertThrowsError(try MacLocalModelEndpointSelection.resolve(modelID: modelID, captured: original) { _ in replacement }) { error in
                XCTAssertTrue(error is MacLocalModelEndpointSelection.SelectionError)
            }
        }
        XCTAssertEqual(try MacLocalModelEndpointSelection.resolve(modelID: modelID, captured: original) { _ in original }, original)
        XCTAssertThrowsError(try MacLocalModelEndpointSelection.resolve(modelID: "cloud-model", captured: original) { _ in
            XCTFail("A mismatched route must not consult the runtime")
            return original
        })
    }
}
