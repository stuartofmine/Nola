import Foundation
import XCTest
@testable import Nola

@MainActor
final class IOSRunConfigurationAdmissionTests: XCTestCase {
    func testRecoveryUsesFrozenGenerationAndRequiresLiveCredentialAuthorization() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 64
        parameters.temperature = 0
        parameters.seed = 9_007_199_254_740_993
        parameters.stopSequences = "  original  "
        try withProfile(protocol: .chatCompletions, sampling: true, parameters: parameters) { store, model, owner in
            let prepared = try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
                resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: owner) },
                admitOutput: { try IOSPreparedHostOutputAdmission.admit(route: $0, outputTokens: $1, using: store) })
            let original = try prepared.prepareHostInvocation()
            let frozen = IOSExactRunInvocation(conversationID: "conversation", runID: "run",
                assistantMessageID: "assistant", invocationConfigDigest: String(repeating: "a", count: 64),
                status: "running", hostInvocation: try original.binding(executionPolicy: .object([:])),
                contextBudget: .object(["context_window_tokens": .number(Double(parameters.maxContextLength)),
                    "reserved_output_tokens": .number(64)]), remoteToolPlan: .null, coreSessionID: "core",
                checkpointRevision: 1, checkpointEventCursor: 1, checkpointSHA256: String(repeating: "b", count: 64))
            let stored = try XCTUnwrap(store.modelProfile(id: model.id))
            var changed = parameters
            changed.temperature = 1.5
            changed.maxOutputTokens = 512
            changed.seed = nil
            let update = IOSCanonicalModelProfile(id: stored.id, displayName: stored.displayName,
                provider: stored.provider, modelName: stored.modelName, runtime: stored.runtime,
                sourceJSON: stored.sourceJSON, parametersJSON: String(decoding: try JSONEncoder().encode(changed), as: UTF8.self),
                selected: stored.selected, enabled: stored.enabled, revision: stored.revision,
                createdAtMs: stored.createdAtMs, updatedAtMs: stored.updatedAtMs + 1, deletedAtMs: nil)
            _ = try store.upsertModelProfileCAS(update, expectedRevision: UInt64(stored.revision))
            let restored = try IOSPreparedRunModel.recover(frozen, using: store, credentials: owner)
            XCTAssertEqual(restored.parameters.temperature, 0)
            XCTAssertEqual(restored.parameters.maxOutputTokens, 64)
            XCTAssertEqual(restored.parameters.seed, 9_007_199_254_740_993)
            XCTAssertEqual(try restored.prepareHostInvocation().generation, original.generation)
            _ = try store.deleteModelProfile(id: model.id)
            XCTAssertThrowsError(try IOSPreparedRunModel.recover(frozen, using: store, credentials: owner))
        }
    }

    func testResponsesUnsupportedOptionsFailBeforeOutputAdmissionOrRunCreation() throws {
        for edit: (inout ModelParameters) -> Void in [
            { $0.seed = 0 }, { $0.stopSequences = " END " },
            { $0.presencePenalty = 0.1 }, { $0.frequencyPenalty = -0.1 }
        ] {
            var parameters = ModelParameters.webAlignedCloudDefaults
            edit(&parameters)
            try withProfile(protocol: .responses, sampling: true, parameters: parameters) { store, model, owner in
                try assertRejectedBeforeAdmission(store: store, model: model, owner: owner, parameters: parameters)
            }
        }
    }

    func testUndeclaredSamplingNondefaultsFailDuringPreparation() throws {
        for wireProtocol: ModelWireProtocol in [.chatCompletions, .responses] {
            var parameters = ModelParameters.webAlignedCloudDefaults
            parameters.temperature = 0
            parameters.topP = 0
            try withProfile(protocol: wireProtocol, sampling: false, parameters: parameters) { store, model, owner in
                try assertRejectedBeforeAdmission(store: store, model: model, owner: owner, parameters: parameters)
            }
        }
    }

    func testDeclaredChatPreservesLargeSeedAndStopWhitespace() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.seed = 9_007_199_254_740_993
        parameters.stopSequences = "  END  \n next "
        try withProfile(protocol: .chatCompletions, sampling: true, parameters: parameters) { store, model, owner in
            let config = try prepareContainer(store: store, model: model, owner: owner)
            XCTAssertEqual(config.seed, parameters.seed)
            XCTAssertEqual(config.stopSequences, ["  END  ", " next "])
            XCTAssertTrue(try store.listConversations().isEmpty)
        }
    }

    func testDeclaredResponsesPreservesExplicitZeroSampling() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0
        parameters.topP = 0
        try withProfile(protocol: .responses, sampling: true, parameters: parameters) { store, model, owner in
            let config = try prepareContainer(store: store, model: model, owner: owner)
            XCTAssertEqual(config.temperature, 0)
            XCTAssertEqual(config.topP, 0)
            XCTAssertTrue(try store.listConversations().isEmpty)
        }
    }

    private func assertRejectedBeforeAdmission(store: IOSApplicationRuntime, model: ModelInfo,
        owner: IOSCloudCredentialOwner, parameters: ModelParameters) throws {
        XCTAssertThrowsError(try prepareContainer(store: store, model: model, owner: owner))
        var admissions = 0
        XCTAssertThrowsError(try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
            resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: owner) },
            admitOutput: { _, _ in
                admissions += 1
                XCTFail("Invalid generation configuration reached output admission")
                throw IOSApplicationRuntimeError.invalidResponse
            }))
        XCTAssertEqual(admissions, 0)
        XCTAssertTrue(try store.listConversations().isEmpty)
    }

    private func prepareContainer(store: IOSApplicationRuntime, model: ModelInfo,
        owner: IOSCloudCredentialOwner) throws -> OpenAIProvider.Config {
        try IOSContainerCloudConfiguration.prepare(selected: model) {
            try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store, credentials: owner)
        }
    }

    private func withProfile(protocol wireProtocol: ModelWireProtocol, sampling: Bool, parameters: ModelParameters,
        _ body: (IOSApplicationRuntime, ModelInfo, IOSCloudCredentialOwner) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db"))
        let vault = AdmissionCredentialVault()
        let owner = vault.owner
        let model = ModelInfo(id: "admission-fixture", name: "Fixture", source: "Fixture", isLocal: false,
            baseURL: "https://fixture.invalid/v1", apiKey: "fixture-only-key", upstreamModelId: "fixture-model",
            wireProtocol: wireProtocol, executionCapabilities: .init(samplingParameters: sampling))
        _ = try ModelSelectionManager.publishCloudModel(model, existing: nil, selected: true,
            using: store, credentials: owner)
        let stored = try XCTUnwrap(store.modelProfile(id: model.id))
        let input = IOSCanonicalModelProfile(id: stored.id, displayName: stored.displayName, provider: stored.provider,
            modelName: stored.modelName, runtime: stored.runtime, sourceJSON: stored.sourceJSON,
            parametersJSON: String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self),
            selected: stored.selected, enabled: stored.enabled, revision: stored.revision,
            createdAtMs: stored.createdAtMs, updatedAtMs: stored.updatedAtMs + 1, deletedAtMs: nil)
        _ = try store.upsertModelProfileCAS(input, expectedRevision: UInt64(stored.revision))
        try body(store, model, owner)
    }
}

private nonisolated final class AdmissionCredentialVault: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: Data] = [:]
    var owner: IOSCloudCredentialOwner {
        .init(save: { [self] account, value in lock.withLock {
            guard values[account] == nil else { return false }
            values[account] = value
            return true
        } }, read: { [self] account in lock.withLock { values[account] } },
        delete: { [self] account in lock.withLock { _ = values.removeValue(forKey: account) } })
    }
}
