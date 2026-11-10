// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacLocalRunModelRouteTests: XCTestCase {
    func testCapturedRouteKeepsIdentityAndCapabilitiesAfterModelCenterEdit() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-run-model-route-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = MacAppleChatConfigurationStore(
            applicationPort: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        var endpoint = MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Before", baseURL: "https://before.invalid/v1",
            models: [MacRemoteModelConfiguration(
                id: "profile", modelName: "original-model",
                executionCapabilities: MacModelExecutionCapabilities(
                    toolCalls: true, imageInput: false, reasoning: false, samplingParameters: true
                )
            )]
        )
        try store.replaceEndpoints([endpoint])
        let captured = MacLocalRunModelRoute(
            configuredProvider: try XCTUnwrap(store.load(model: "profile")),
            routePlan: try XCTUnwrap(store.routePlan(preferredModel: "profile"))
        )

        endpoint.baseURL = "https://after.invalid/v1"
        endpoint.wireProtocol = .responses
        endpoint.models[0].modelName = "replacement-model"
        endpoint.models[0].executionCapabilities.samplingParameters = false
        endpoint.models[0].confirmExecutionCapabilities()
        try store.replaceEndpoints([endpoint])
        store.markQuotaExhausted(profileID: "profile", until: .now.addingTimeInterval(3_600))

        XCTAssertEqual(store.load(model: "profile")?.model, "replacement-model")
        XCTAssertNil(store.routePlan(preferredModel: "profile"))
        let frozen = try XCTUnwrap(captured.validatedProvider(requiresSelectedProfile: true))
        XCTAssertEqual(frozen.profileID, "profile")
        XCTAssertEqual(frozen.model, "original-model")
        XCTAssertEqual(frozen.baseURL.absoluteString, "https://before.invalid/v1")
        XCTAssertEqual(frozen.wireProtocol, .chatCompletions)
        XCTAssertTrue(frozen.executionCapabilities.samplingParameters)
    }

    func testFrozenRouteValidationPreservesExplicitProfileAndCoolingRules() throws {
        let primary = MacAppleChatConfiguration(
            profileID: "selected", baseURL: URL(string: "https://selected.invalid")!,
            apiKey: "", model: "selected"
        )
        let fallback = MacAppleChatConfiguration(
            profileID: "fallback", baseURL: URL(string: "https://fallback.invalid")!,
            apiKey: "", model: "fallback"
        )
        let cooled = MacLocalRunModelRoute(configuredProvider: primary, routePlan: nil)
        XCTAssertThrowsError(try cooled.validatedProvider(requiresSelectedProfile: false))
        let substituted = MacLocalRunModelRoute(
            configuredProvider: primary,
            routePlan: MacRemoteModelRoutePlan(primary: fallback, fallbacks: [])
        )
        XCTAssertThrowsError(try substituted.validatedProvider(requiresSelectedProfile: true))
        XCTAssertEqual(try substituted.validatedProvider(requiresSelectedProfile: false)?.profileID, "fallback")
        let local = MacLocalRunModelRoute(configuredProvider: nil, routePlan: nil)
        XCTAssertNil(try local.validatedProvider(requiresSelectedProfile: false))
    }
}
