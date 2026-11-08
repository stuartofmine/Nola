import XCTest
@testable import Nola

final class IOSDraftAggregateTests: XCTestCase {
    func testStandardCanonicalProfileOwnsProjectionAndSwitchBackPreservesOverrides() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let profile = IOSCanonicalModelProfile(id: "canonical-profile", displayName: "Canonical Name", provider: "fixture",
            modelName: "upstream-wire-id", runtime: "remote",
            sourceJSON: "{\"baseURL\":\"https://fixture.invalid/v1\",\"wireProtocol\":\"responses\"}",
            parametersJSON: "{}", selected: false, enabled: true, revision: 0,
            createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil)
        try runtime.upsertModelProfile(profile)
        let model = try XCTUnwrap(ModelSelectionManager.modelInfo(from: profile))
        XCTAssertEqual(model.id, "canonical-profile")
        XCTAssertEqual(model.effectiveUpstreamModelId, "upstream-wire-id")
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.23
        let conversation = try store.createConfiguredConversation(title: "Original", language: nil, model: model, parameters: parameters)
        try registerLocal("local-other", runtime: runtime)
        try store.selectConversationModel(conversationID: conversation.id,
            model: ModelInfo(id: "local-other", name: "Other", source: "local", isLocal: true), initialParameters: .webAlignedCloudDefaults)
        try store.selectConversationModel(conversationID: conversation.id, model: model, initialParameters: .webAlignedCloudDefaults)
        XCTAssertEqual(try store.conversationParameters(conversationId: conversation.id, modelKey: "configured:canonical-profile").temperature, 0.23)
        var displayed = try XCTUnwrap(store.getConversation(conversationId: conversation.id))
        displayed.selectedModelInfo = ModelInfo(id: "wrong", name: "Wrong", source: "local", isLocal: true)
        displayed.title = "Renamed"
        try store.upsertConversation(displayed)
        XCTAssertEqual(try store.getConversation(conversationId: conversation.id)?.selectedModelInfo?.id, model.id)
        let fields = try IOSConversationSettingsObject.decode(try runtime.loadConversation(id: conversation.id.uuidString)?.settingsJSON)
        XCTAssertNil(fields["selectedModelInfo"])
        XCTAssertEqual(fields["modelSelectionID"] as? String, "configured:canonical-profile")
    }

    func testHostIsConnectionBindingNotConfiguredModel() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let host = ModelInfo(id: "web-backend:local-host", name: "Host", source: "web", isLocal: false,
            baseURL: "https://host.invalid", apiKey: "must-not-persist")
        let conversation = try store.createConfiguredConversation(title: nil, language: nil, model: host, parameters: .webAlignedCloudDefaults)
        let stored = try XCTUnwrap(runtime.loadConversation(id: conversation.id.uuidString))
        let fields = try IOSConversationSettingsObject.decode(stored.settingsJSON)
        XCTAssertNil(fields["modelSelectionID"])
        XCTAssertNil(fields["modelOverrides"])
        XCTAssertNotNil(fields["executionHostBinding"])
        XCTAssertFalse(stored.settingsJSON?.contains("must-not-persist") ?? true)
        XCTAssertTrue(try XCTUnwrap(store.getConversation(conversationId: conversation.id)?.selectedModelInfo).isWebBackend)
        XCTAssertEqual(try store.conversationParameters(conversationId: conversation.id, modelKey: "execution-host"), .webAlignedCloudDefaults)
    }
    private func registerLocal(_ id: String, runtime: IOSApplicationRuntime) throws {
        try runtime.upsertModelProfile(.init(id: id, displayName: id, provider: "local", modelName: id,
            runtime: "mlx", sourceJSON: "{}", parametersJSON: "{}", selected: false,
            enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
    }
    @MainActor
    func testUnpublishedCreationAndUnreadableSelectedConversationNeverReplaceTheDraftSelection() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        try registerLocal("fixture-local", runtime: runtime)
        let container = ServiceContainer(applicationRuntime: runtime)
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = ModelInfo(id: "fixture-local", name: "Fixture", source: "local", isLocal: true)
        let id = try XCTUnwrap(manager.ensureCurrentConversation(initialParameters: .webAlignedCloudDefaults, selectOnCreate: false))
        XCTAssertNil(manager.currentConversationId)
        XCTAssertNotNil(manager.executionForConversation(id))
        manager.currentConversationId = id
        // A catalogue removal makes the selected identity unavailable without
        // bypassing the configuration command through a full settings write.
        XCTAssertTrue(try runtime.deleteModelProfile(id: "fixture-local"))
        XCTAssertEqual(manager.ensureCurrentConversation(), id)
        let unavailable = try XCTUnwrap(container.conversationStore?.getConversation(conversationId: id))
        XCTAssertNil(unavailable.selectedModelInfo)
        XCTAssertNotNil(unavailable.modelUnavailableReason)
        XCTAssertEqual(manager.currentConversationId, id)
        XCTAssertEqual(try runtime.listConversations().count, 1)
    }
    func testConfiguredCreationCommitsIdentityAndParametersTogetherAndRejectsInvalidInput() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        for id in ["fixture-local", "second-local", "third-local"] { try registerLocal(id, runtime: runtime) }
        let model = ModelInfo(id: "fixture-local", name: "Fixture", source: "local", isLocal: true)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.42
        let created = try store.createConfiguredConversation(title: "Owned", language: nil, model: model, parameters: parameters)
        let stored = try XCTUnwrap(runtime.loadConversation(id: created.id.uuidString))
        let json = try IOSConversationSettingsObject.decode(stored.settingsJSON)
        XCTAssertEqual(json["modelSelectionID"] as? String, "local:fixture-local")
        XCTAssertNil(json["selectedModelInfo"])
        XCTAssertEqual(try store.getConversation(conversationId: created.id)?.selectedModelInfo?.id, model.id)
        XCTAssertEqual(try store.conversationParameters(conversationId: created.id, modelKey: "local:fixture-local").temperature, 0.42)
        let second = ModelInfo(id: "second-local", name: "Second", source: "local", isLocal: true)
        try store.selectConversationModel(conversationID: created.id, model: second, initialParameters: parameters)
        let changed = try XCTUnwrap(runtime.loadConversation(id: created.id.uuidString))
        XCTAssertEqual(try IOSConversationSettingsObject.decode(changed.settingsJSON)["modelSelectionID"] as? String, "local:second-local")
        XCTAssertEqual(try store.getConversation(conversationId: created.id)?.selectedModelInfo?.id, second.id)
        XCTAssertEqual(try store.conversationParameters(conversationId: created.id, modelKey: "local:second-local").temperature, 0.42)
        parameters.maxOutputTokens = 0
        let third = ModelInfo(id: "third-local", name: "Third", source: "local", isLocal: true)
        XCTAssertThrowsError(try store.selectConversationModel(conversationID: created.id, model: third, initialParameters: parameters))
        XCTAssertEqual(try store.getConversation(conversationId: created.id)?.selectedModelInfo?.id, second.id)
        XCTAssertThrowsError(try store.createConfiguredConversation(title: "Invalid", language: nil, model: model, parameters: parameters))
        XCTAssertEqual(try runtime.listConversations().count, 1)
        parameters.maxOutputTokens = ModelParameters.webAlignedCloudDefaults.maxOutputTokens
        let validAfterRejection = try store.createConfiguredConversation(title: "Still writable", language: nil,
            model: model, parameters: parameters)
        XCTAssertNotEqual(validAfterRejection.id, created.id)
        XCTAssertEqual(try runtime.listConversations().count, 2,
            "Locally rejected editor input must not leave an unconfirmed mutation ahead of valid work")
    }
}
