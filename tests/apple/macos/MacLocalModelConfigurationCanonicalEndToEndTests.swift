// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacLocalModelConfigurationCanonicalEndToEndTests: XCTestCase {
    func testStaleEditorCannotOverwriteNewerParameters() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-mac-cas-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("state.db"))
        let configuration = MacLocalModelConfigurationStore(applicationPort: port)
        let profile = MacLocalModelProfile.qwen35_08B
        let snapshot = try configuration.parameterSnapshot(for: profile)
        var first = snapshot.parameters
        first.temperature = 0.2
        let revision = try configuration.saveParameters(first, for: profile, expectedRevision: snapshot.revision)
        var stale = snapshot.parameters
        stale.temperature = 0.9
        XCTAssertThrowsError(try configuration.saveParameters(stale, for: profile, expectedRevision: snapshot.revision)) { error in
            guard case MacApplicationServiceError.revisionConflict(let scope, let expected, let actual) = error else {
                return XCTFail("Expected typed conflict, got \(error)")
            }
            XCTAssertEqual(scope, "model_profile:\(profile.id)")
            XCTAssertEqual(expected, snapshot.revision)
            XCTAssertEqual(actual, revision)
        }
        XCTAssertEqual(try configuration.parameterSnapshot(for: profile).parameters.temperature, 0.2, accuracy: 0.001)
        first.temperature = 0.3
        let next = try configuration.saveParameters(first, for: profile, expectedRevision: revision)
        XCTAssertGreaterThan(next, revision)
    }

    func testFirstParameterConfigurationDoesNotSelectDefaultCatalogModel() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-local-unselected-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("product.db"))
        let configuration = MacLocalModelConfigurationStore(applicationPort: canonical)
        let profile = MacLocalModelProfile.qwen35_08B
        try configuration.saveParameters(.localDefaults, for: profile)
        let stored = try XCTUnwrap(canonical.modelProfile(profile.id))
        XCTAssertFalse(stored.selected, "Editing default catalog parameters must not select or start the model.")
        XCTAssertEqual(stored.provider, "local")
        XCTAssertEqual(stored.runtime, "mlx")
    }

    func testParameterEditPreservesCanonicalSourceAndSelection() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-local-parameter-only-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("product.db"))
        let profile = MacLocalModelProfile.qwen35_08B
        let source = #"{"modelID":"owned-model","format":"mlx","installedDirectory":"/owned/model"}"#
        try canonical.upsertModelProfile(MacCanonicalModelProfile(
            id: profile.id, displayName: "Owned model", provider: "local",
            modelName: profile.id, runtime: "mlx", sourceJSON: source,
            parametersJSON: "{}", selected: false, enabled: true,
            revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        let configuration = MacLocalModelConfigurationStore(applicationPort: canonical)
        XCTAssertThrowsError(try configuration.parameterSnapshot(for: profile),
                             "Incomplete stored parameters must not silently become an editable default snapshot.")
        var parameters = ModelParameters.localDefaults
        parameters.temperature = 0.21
        try configuration.saveParameters(parameters, for: profile)
        let stored = try XCTUnwrap(canonical.modelProfile(profile.id))
        XCTAssertFalse(stored.selected)
        XCTAssertEqual(stored.sourceJSON, source)
        XCTAssertEqual(stored.displayName, "Owned model")
        XCTAssertEqual(configuration.parameters(for: profile).temperature, 0.21, accuracy: 0.001)
    }

    func testParametersAndSelectionSurviveRealServiceColdReopen() throws {
        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-local-model-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let database = workspace.appendingPathComponent("nola.db")
        let profile = MacLocalModelProfile.qwen35_4B
        var parameters = ModelParameters()
        parameters.maxContextLength = 131_072
        parameters.temperature = 0.35

        do {
            let canonical = try MacApplicationPort(databaseURL: database)
            let configuration = MacLocalModelConfigurationStore(applicationPort: canonical)
            try configuration.saveParameters(parameters, for: profile)
            try configuration.select(profile)
            XCTAssertEqual(configuration.selectedModelID, profile.id)
        }

        let reopened = try MacApplicationPort(databaseURL: database)
        let configuration = MacLocalModelConfigurationStore(applicationPort: reopened)
        XCTAssertEqual(configuration.selectedModelID, profile.id)
        XCTAssertEqual(configuration.selectedProfile.id, profile.id)
        XCTAssertEqual(configuration.parameters(for: profile).maxContextLength, 131_072)
        XCTAssertEqual(configuration.parameters(for: profile).temperature, 0.35, accuracy: 0.001)
        let stored = try XCTUnwrap(reopened.modelProfile(profile.id))
        XCTAssertTrue(stored.selected)
        XCTAssertEqual(stored.provider, "local")
        XCTAssertEqual(stored.runtime, "mlx")
    }
}
