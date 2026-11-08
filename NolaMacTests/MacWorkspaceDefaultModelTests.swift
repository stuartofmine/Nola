// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacWorkspaceDefaultModelTests: XCTestCase {
    func testCatalogFactsAndChosenRouteDoNotReadUnrelatedConnectionCredentials() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("models.db"))
        try MacAppleChatConfigurationStore(applicationPort: port).replaceEndpoints(["good", "bad"].map { id in
            MacRemoteEndpointConfiguration(id: id, name: id, baseURL: "https://\(id).invalid/v1", apiKey: "isolated-fixture-key",
                models: [MacRemoteModelConfiguration(id: id, modelName: "same-name", executionCapabilities: .init())])
        })
        let reader = MacAppleChatConfigurationStore(applicationPort: port, credentialReader: { account in
            if account.hasPrefix("endpoint:bad:") { throw MacApplicationPortError.rejected("Unavailable unrelated credential") }
            return "isolated-fixture-key"
        })
        XCTAssertThrowsError(try reader.endpointSnapshot())
        XCTAssertEqual(try reader.endpointSnapshot(includeCredentials: false).endpoints.count, 2)
        XCTAssertEqual(try reader.loadThrowing(model: "good")?.profileID, "good")
        XCTAssertEqual(try reader.routePlanThrowing(preferredModel: "good")?.primary.profileID, "good")
        XCTAssertThrowsError(try reader.loadThrowing(model: "bad"))
    }

    func testDefaultIdentityAndExplicitConversationSurviveCatalogEdits() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("models.db"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        var endpoints = ["a", "b"].map { id in
            MacRemoteEndpointConfiguration(id: id, name: id, baseURL: "https://\(id).invalid/v1",
                models: [MacRemoteModelConfiguration(id: "profile:\(id)", modelName: "same-name",
                    executionCapabilities: .init())])
        }
        try store.replaceEndpoints(endpoints)
        let preference = MacDefaultChatModelStore(application: port)
        let facts = try MacChatModelCatalog.load(application: port).candidates
        let saved = try preference.set(.profile("profile:b"), expectedRevision: nil, candidates: facts)
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        XCTAssertTrue(model.inheritsDefaultModel)
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:b")
        XCTAssertEqual(try store.loadThrowing()?.profileID, "profile:b")
        let a = try XCTUnwrap(model.availableModelOptions.first { $0.configurationID == "profile:a" })
        model.selectModel(a)
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:a")
        XCTAssertEqual(model.selectedRemoteConfiguration?.endpointName, "a")
        XCTAssertEqual(model.modelSelectionResolution, .resolved(profileID: "profile:a", origin: .conversation))
        _ = try preference.set(.automatic, expectedRevision: saved.revision, candidates: facts)
        model.refreshModelConfiguration()
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:a")

        endpoints[0].name = "Renamed"
        endpoints[1].models[0].priority = 100
        try store.replaceEndpoints(endpoints)
        model.refreshModelConfiguration()
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:a")
        XCTAssertEqual(model.selectedRemoteConfiguration?.endpointName, "Renamed")
        XCTAssertEqual(try preference.snapshot().selection, .automatic)
        model.restoreDefaultModelInheritance()
        XCTAssertTrue(model.inheritsDefaultModel)
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:b")
    }

    func testMissingExplicitModelBlocksSubmissionAndNeverMatchesSameProviderModelName() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("models.db"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(id: "present", name: "Present",
            baseURL: "https://present.invalid/v1", models: [MacRemoteModelConfiguration(
                id: "present-profile", modelName: "same-name", executionCapabilities: .init())])])
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.selectModel(MacChatModelOption(source: .configured, modelID: "same-name",
            displayName: "Removed", configurationID: "removed-profile"))
        XCTAssertNil(model.selectedModelOption)
        XCTAssertEqual(model.modelSelectionResolution,
            .unavailable(profileID: "removed-profile", origin: .conversation, reason: .missing))
        XCTAssertThrowsError(try store.routePlanThrowing(preferredModel: "removed-profile"))
        XCTAssertThrowsError(try store.loadThrowing(model: "same-name"), "Provider names are not canonical profile identity")
        model.draft = "Do not send to another provider"
        let count = model.selectedMessages.count
        model.send()
        XCTAssertFalse(model.isRunning)
        XCTAssertEqual(model.selectedMessages.count, count)
        XCTAssertEqual(model.draft, "Do not send to another provider")
        model.restoreDefaultModelInheritance()
        XCTAssertEqual(model.selectedModelOption?.configurationID, "present-profile")
    }
}
