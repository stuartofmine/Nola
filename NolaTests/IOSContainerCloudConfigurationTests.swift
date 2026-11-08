import Foundation
import XCTest
@testable import Nola

@MainActor
final class IOSContainerCloudConfigurationTests: XCTestCase {
    func testCanonicalProfileAndParametersWinOverStaleSelectedValues() throws {
        try withStore { store, vault in
            let model = cloudModel(secret: "current-fixture-key")
            _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
                using: store, credentials: vault.owner)
            let observed = try XCTUnwrap(store.modelProfile(id: model.id))
            var parameters = ModelParameters.webAlignedCloudDefaults
            parameters.temperature = 0.37
            parameters.maxOutputTokens = 2_048
            let updated = IOSCanonicalModelProfile(id: observed.id, displayName: observed.displayName,
                provider: observed.provider, modelName: observed.modelName, runtime: observed.runtime,
                sourceJSON: observed.sourceJSON,
                parametersJSON: String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self),
                selected: observed.selected, enabled: observed.enabled, revision: observed.revision,
                createdAtMs: observed.createdAtMs, updatedAtMs: observed.updatedAtMs + 1, deletedAtMs: nil)
            _ = try store.upsertModelProfileCAS(updated, expectedRevision: UInt64(observed.revision))
            let stale = cloudModel(endpoint: "https://stale.invalid/v1", secret: "stale-key", wireProtocol: .chatCompletions)
            let config = try prepare(stale, store: store, vault: vault)
            XCTAssertEqual(config.baseURL.absoluteString, model.baseURL)
            XCTAssertEqual(config.apiKey, "current-fixture-key")
            XCTAssertEqual(config.wireProtocol, .responses)
            XCTAssertEqual(config.temperature, 0.37)
            XCTAssertEqual(config.maxCompletionTokens, 2_048)
            XCTAssertEqual(config.model, model.effectiveUpstreamModelId)
        }
    }

    func testEveryPreparationRevalidatesCredentialBeforeAnyDedupe() throws {
        try withStore { store, vault in
            let model = cloudModel(secret: "saved-key")
            _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
                using: store, credentials: vault.owner)
            _ = try prepare(model, store: store, vault: vault)
            vault.removeAll()
            XCTAssertThrowsError(try prepare(model, store: store, vault: vault),
                "The selected ModelInfo's old key must not replace a revoked credential")
        }
    }

    func testExplicitAnonymousIsAcceptedButUnpublishedOrMissingSelectionIsRejected() throws {
        try withStore { store, vault in
            let model = cloudModel(secret: "")
            XCTAssertThrowsError(try prepare(model, store: store, vault: vault))
            XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: nil) { _ in
                XCTFail("A missing selection must not invoke a fallback resolver")
                throw IOSCloudCredentialError.unavailable
            })
            _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
                using: store, credentials: vault.owner)
            XCTAssertEqual(try prepare(model, store: store, vault: vault).apiKey, "")
        }
    }

    func testImportedConnectionProfilesAdmitAnonymousAndAccountCredentials() throws {
        for secret in ["", "fixture-account-key"] {
            try withStore { store, vault in
                let service = IOSModelConnectionService(application: store, credentials: vault.owner)
                let connection = try service.create(name: "WSL fixture", endpoint: "http://127.0.0.1:18080/v1",
                    wireProtocol: "chat_completions", secret: secret, modelID: "qwen3.8-flash-next-iq3_s")
                let catalog: IOSModelConnectionSnapshot = try service.command([
                    "type": .string("refresh_connection_catalog"), "connection_id": .string(connection.id),
                    "expected_revision": .signedInteger(connection.revision), "observed_at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000) + 1),
                    "models": .array([.object([
                        "model_id": .string("qwen3.8-flash-next-iq3_s"), "display_name": .string("Qwen fixture"),
                        "protocol": .string("chat_completions"), "context_window": .null, "output_limit": .null,
                        "modalities": .array([]), "reasoning_levels": .array([]), "execution_capabilities": .null
                    ])])])
                try service.enable(catalog, modelIDs: ["chat_completions:qwen3.8-flash-next-iq3_s"])
                let profile = try XCTUnwrap(store.listModelProfiles().first)
                XCTAssertFalse(profile.sourceJSON.contains("cloudCredential"), "Connections use account admission, not a per-profile credential field")
                let selected = try XCTUnwrap(ModelSelectionManager.modelInfo(from: profile))
                let config = try prepare(selected, store: store, vault: vault)
                XCTAssertEqual(config.apiKey, secret)
                XCTAssertEqual(config.model, "qwen3.8-flash-next-iq3_s")
                XCTAssertEqual(config.maxCompletionTokens, 16_384)
                let resolved = try ModelSelectionManager.resolvedCloudProfileForRun(selected, using: store, credentials: vault.owner)
                if secret.isEmpty {
                    XCTAssertEqual(resolved.model.cloudCredential, .anonymous)
                } else {
                    vault.removeAll()
                    XCTAssertThrowsError(try prepare(selected, store: store, vault: vault)) {
                        XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .credential)
                    }
                }
                _ = try service.disconnect(try XCTUnwrap(service.connections().first))
                XCTAssertThrowsError(try prepare(selected, store: store, vault: vault))
            }
        }
    }

    func testPreparationDiagnosticsIdentifyStageWithoutIncludingUnderlyingSecrets() throws {
        let selected = cloudModel(secret: "fixture-secret")
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { _ in
            throw NSError(domain: "private-token", code: 1)
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .profile) }
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { _ in
            throw IOSCloudRunCredentialResolutionError(underlyingError: NSError(domain: "private-token", code: 1))
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .credential) }
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { model in
            IOSResolvedCloudRunProfile(model: model, profileID: model.id, revision: 1, parametersJSON: "[]")
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .parameters) }
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 3
        let json = String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self)
        XCTAssertThrowsError(try IOSContainerCloudConfiguration.prepare(selected: selected) { model in
            IOSResolvedCloudRunProfile(model: model, profileID: model.id, revision: 1, parametersJSON: json)
        }) { XCTAssertEqual($0 as? IOSContainerCloudConfigurationFailure, .providerValidation) }
    }

    func testExplicitWorkspaceSelectionSurvivesModeChangesAndDeferredRefreshes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("owner.db"))
        let vault = ContainerCredentialVault()
        let defaultModel = ModelInfo(id: "default-profile", name: "Old default", source: "Fixture", isLocal: false,
            baseURL: "https://default.invalid/v1", apiKey: "", upstreamModelId: "old-default", wireProtocol: .chatCompletions)
        let chosen = ModelInfo(id: "conversation-profile", name: "Qwen", source: "Fixture", isLocal: false,
            baseURL: "https://conversation.invalid/v1", apiKey: "", upstreamModelId: "qwen-current", wireProtocol: .chatCompletions)
        _ = try ModelSelectionManager.publishCloudModel(defaultModel, existing: nil, selected: true, using: store, credentials: vault.owner)
        _ = try ModelSelectionManager.publishCloudModel(chosen, existing: nil, selected: false, using: store, credentials: vault.owner)
        _ = try store.putProductSettingChecked(key: "chat.default_model",
            valueJSON: "{\"version\":1,\"selection\":{\"kind\":\"profile\",\"profile_id\":\"default-profile\"}}",
            expectedRevision: nil, updatedAtMs: 1)
        let container = ServiceContainer(applicationRuntime: store)
        func assertChosen() throws {
            let service = try XCTUnwrap(container.llmClient as? ModelService)
            let provider = try XCTUnwrap(service.currentProvider() as? OpenAIProvider)
            XCTAssertEqual(provider.auxiliaryModelIdentity, "qwen-current")
            XCTAssertEqual(container.configurationModel?.id, chosen.id)
        }
        container.useLocalModel = false
        container.reconfigureIfPossible(selectedModel: chosen)
        container.useLocalModel = false // @Published emits even when the value is unchanged.
        await Task.yield()
        container.reconfigureIfPossible()
        try assertChosen()
        XCTAssertEqual(try ModelSelectionManager.lastSelectedModel(using: store)?.id, defaultModel.id)

        let local = ModelInfo(id: "fixture-local", name: "Local fixture", source: "Local", isLocal: true)
        container.useLocalModel = true
        container.reconfigureIfPossible(selectedModel: local)
        await Task.yield()
        XCTAssertEqual(container.configurationModel?.id, local.id)
        XCTAssertFalse((container.llmClient as? ModelService)?.currentProvider() is OpenAIProvider)
        container.useLocalModel = false
        container.reconfigureIfPossible(selectedModel: chosen)
        await Task.yield()
        container.reconfigureIfPossible()
        try assertChosen()

        _ = try store.deleteModelProfile(id: chosen.id)
        container.reconfigureIfPossible()
        XCTAssertFalse((container.llmClient as? ModelService)?.currentProvider() is OpenAIProvider,
            "Retaining an explicit owner must not bypass profile revocation")
    }

    private func prepare(_ selected: ModelInfo, store: IOSApplicationRuntime,
        vault: ContainerCredentialVault) throws -> OpenAIProvider.Config {
        try IOSContainerCloudConfiguration.prepare(selected: selected) {
            try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: vault.owner)
        }
    }

    private func cloudModel(endpoint: String = "https://provider.invalid/v1", secret: String,
        wireProtocol: ModelWireProtocol = .responses) -> ModelInfo {
        ModelInfo(id: "container-profile", name: "Fixture", source: "Fixture", isLocal: false,
            baseURL: endpoint, apiKey: secret, upstreamModelId: "fixture-model", wireProtocol: wireProtocol,
            executionCapabilities: .init(samplingParameters: true))
    }

    private func withStore(_ body: (IOSApplicationRuntime, ContainerCredentialVault) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try body(IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db")), ContainerCredentialVault())
    }
}

private nonisolated final class ContainerCredentialVault: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: Data] = [:]
    func removeAll() { lock.withLock { values.removeAll() } }
    var owner: IOSCloudCredentialOwner {
        .init(save: { [self] account, value in lock.withLock {
            guard values[account] == nil else { return false }
            values[account] = value
            return true
        } }, read: { [self] account in lock.withLock { values[account] } },
        delete: { [self] account in lock.withLock { _ = values.removeValue(forKey: account) } })
    }
}
