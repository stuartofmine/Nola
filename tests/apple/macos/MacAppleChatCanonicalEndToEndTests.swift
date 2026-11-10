// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacAppleChatCanonicalEndToEndTests: XCTestCase {
    func testNewRemoteProfilesUseCloudDefaultsWithoutLocalOnlyGenerationOptions() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        for wire in [MacModelWireProtocol.chatCompletions, .responses] {
            let id = "remote-\(wire.rawValue)"
            try store.replaceEndpoints([MacRemoteEndpointConfiguration(id: "endpoint", name: "Fixture",
                baseURL: "https://fixture.invalid/v1", models: [.init(id: id, modelName: id)], wireProtocol: wire)])
            let record = try XCTUnwrap(port.modelProfile(id))
            let parameters = try ModelParameters.decodeProfile(record.parametersJSON)
            XCTAssertEqual(parameters, .webAlignedCloudDefaults)
            XCTAssertNil(parameters.topK)
            XCTAssertNil(parameters.repetitionPenalty)
            XCTAssertEqual(try store.loadThrowing(model: id)?.wireProtocol, wire)
        }
    }

    func testCatalogRefreshPreservesExplicitGenerationOptionsForExistingProfiles() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.topK = 40
        parameters.repetitionPenalty = 1
        let endpoint = MacRemoteEndpointConfiguration(id: "endpoint", name: "Fixture",
            baseURL: "https://fixture.invalid/v1", models: [.init(id: "model", modelName: "model")], wireProtocol: .responses)
        try store.replaceEndpoints([endpoint], parametersByModelID: ["model": parameters])
        let original = try XCTUnwrap(port.modelProfile("model"))
        try store.replaceEndpoints([endpoint])
        XCTAssertEqual(try port.modelProfile("model")?.parametersJSON, original.parametersJSON,
                       "Catalog refresh must not silently remove explicit unsupported options")
    }

    func testMalformedRemoteParametersAreRejectedInsteadOfReplacedWithDefaults() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(id: "fixture", name: "Fixture",
            baseURL: "https://fixture.invalid/v1", models: [.init(id: "model", modelName: "model")])])
        let original = try XCTUnwrap(port.modelProfile("model"))
        var parameters = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(original.parametersJSON.utf8)) as? [String: Any])
        parameters["topK"] = "invalid"
        let malformed = String(decoding: try JSONSerialization.data(withJSONObject: parameters), as: UTF8.self)
        try port.upsertModelProfile(MacCanonicalModelProfile(id: original.id, displayName: original.displayName,
            provider: original.provider, modelName: original.modelName, runtime: original.runtime,
            sourceJSON: original.sourceJSON, parametersJSON: malformed, selected: original.selected,
            enabled: original.enabled, revision: original.revision, createdAtMs: original.createdAtMs,
            updatedAtMs: original.updatedAtMs + 1, deletedAtMs: nil))
        XCTAssertThrowsError(try store.loadThrowing(model: "model")) { error in
            XCTAssertTrue(error.localizedDescription.contains("parameters.topK"))
        }
        XCTAssertEqual(try port.modelProfile("model")?.parametersJSON, malformed,
            "Reading a damaged configuration must not rewrite it")
    }

    func testEphemeralCredentialsAreSharedAcrossStoresForTheSameNormalizedDatabase() throws {
        XCTAssertTrue(MacProcessRuntimePolicy.usesEphemeralCredentials)
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-vault-shared-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("test.sqlite")
        let port = try MacApplicationPort(databaseURL: database)
        let writer = MacAppleChatConfigurationStore(applicationPort: port)
        try writer.save(baseURL: "https://fixture.invalid/v1", apiKey: "fixture-only-key", model: "fixture-model",
                        executionCapabilities: .init(toolCalls: true, samplingParameters: true))
        let reopened = MacAppleChatConfigurationStore(applicationPort: try MacApplicationPort(
            databaseURL: URL(fileURLWithPath: root.path + "/./test.sqlite")))
        XCTAssertEqual(try reopened.loadThrowing()?.apiKey, "fixture-only-key")
        try writer.save(baseURL: "https://fixture.invalid/v1", apiKey: "rotated-fixture-key", model: "fixture-model",
                        executionCapabilities: .init(toolCalls: true, samplingParameters: true))
        XCTAssertEqual(try reopened.loadThrowing()?.apiKey, "rotated-fixture-key")
    }

    func testEphemeralCredentialsDoNotLeakToAnotherDatabaseWithTheSameCredentialAccount() throws {
        XCTAssertTrue(MacProcessRuntimePolicy.usesEphemeralCredentials)
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-vault-isolated-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let originalPort = try MacApplicationPort(databaseURL: root.appendingPathComponent("first.sqlite"))
        let writer = MacAppleChatConfigurationStore(applicationPort: originalPort)
        try writer.save(baseURL: "https://fixture.invalid/v1", apiKey: "first-database-only", model: "fixture-model",
                        executionCapabilities: .init(toolCalls: true, samplingParameters: true))
        let otherPort = try MacApplicationPort(databaseURL: root.appendingPathComponent("second.sqlite"))
        // Copy canonical metadata, including its credential account, but not
        // the secret. A separate test database must not inherit that account.
        for profile in try originalPort.listModelProfiles() { try otherPort.upsertModelProfile(profile) }
        let other = MacAppleChatConfigurationStore(applicationPort: otherPort)
        XCTAssertThrowsError(try other.loadThrowing())
        XCTAssertEqual(try writer.loadThrowing()?.apiKey, "first-database-only")
    }

    func testScopedConnectionSaveDoesNotValidateOrRewriteOtherUndeclaredConnection() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-scoped-legacy-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        try store.replaceEndpoints(scopedConnectionFixtures())
        let profile = try XCTUnwrap(port.modelProfile("scoped-a-model"))
        var source = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any])
        source.removeValue(forKey: "executionCapabilities")
        try port.upsertModelProfile(MacCanonicalModelProfile(
            id: profile.id, displayName: profile.displayName, provider: profile.provider,
            modelName: profile.modelName, runtime: profile.runtime,
            sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: profile.parametersJSON, selected: profile.selected, enabled: profile.enabled,
            revision: profile.revision, createdAtMs: profile.createdAtMs,
            updatedAtMs: profile.updatedAtMs + 1, deletedAtMs: nil))
        let preserved = try XCTUnwrap(port.modelProfile("scoped-a-model"))
        let initial = try store.endpointSnapshot()
        XCTAssertFalse(try XCTUnwrap(initial.endpoints.first?.models.first).hasExecutionCapabilityDeclaration)
        let newEndpoint = MacRemoteEndpointConfiguration(id: "scoped-c", name: "WSL",
            baseURL: "http://192.168.31.18:8081/v1",
            models: [.init(id: "scoped-c-model", modelName: "qwen")])
        XCTAssertNoThrow(try MacRemoteModelDraftCommit.validateConnection(
            endpointID: newEndpoint.id, drafts: initial.endpoints + [newEndpoint], store: store))
        XCTAssertThrowsError(try MacRemoteModelDraftCommit.validateConnection(
            endpointID: "scoped-a", drafts: initial.endpoints, store: store))
        let added = try MacRemoteModelDraftCommit.saveConnection(
            endpointID: newEndpoint.id, draft: newEndpoint, baseline: initial.endpoints,
            parametersByModelID: [:], expectedRevisions: initial.revisions, store: store)
        XCTAssertEqual(added.endpoints.count, 3)
        XCTAssertEqual(try port.modelProfile("scoped-a-model"), preserved)
        var changed = try XCTUnwrap(added.endpoints.first(where: { $0.id == "scoped-c" }))
        changed.models[0].displayName = "My Qwen"
        let edited = try MacRemoteModelDraftCommit.saveConnection(
            endpointID: changed.id, draft: changed, baseline: added.endpoints,
            parametersByModelID: [:], expectedRevisions: added.revisions, store: store)
        XCTAssertEqual(edited.endpoints.first(where: { $0.id == changed.id })?.models.first?.displayName, "My Qwen")
        XCTAssertEqual(try port.modelProfile("scoped-a-model"), preserved)
        changed.models[0].priority = 101
        XCTAssertThrowsError(try store.replaceConnection(endpointID: changed.id, draft: changed,
            expectedRevisions: edited.revisions))
        XCTAssertEqual(try store.endpointSnapshot().endpoints, edited.endpoints)
        changed.models[0].priority = 50
        changed.models[0].id = "scoped-a-model"
        XCTAssertThrowsError(try store.replaceConnection(endpointID: changed.id, draft: changed,
            expectedRevisions: edited.revisions))
        XCTAssertEqual(try port.modelProfile("scoped-a-model"), preserved)
    }

    func testNewModelUsesOrdinaryPurposesAndToolsWithoutGrantingSensitiveData() {
        let model = MacRemoteModelConfiguration(modelName: "new-model")
        XCTAssertEqual(model.capabilities, [.chat, .retrieval, .issueRepair])
        XCTAssertTrue(model.executionCapabilities.toolCalls)
        XCTAssertTrue(model.hasExecutionCapabilityDeclaration)
        XCTAssertFalse(model.capabilities.contains(.sensitiveData))
    }

    func testConnectionDraftSaveRetainsUnrelatedConcurrentConnectionAndParameters() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-scoped-model-save-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let original = try MacRemoteModelDraftCommit.save(
            scopedConnectionFixtures(), parametersByModelID: [:], expectedRevisions: [:], store: store
        )
        let untouchedProfile = try XCTUnwrap(port.modelProfile("scoped-a-model"))
        var otherWindow = original.endpoints
        otherWindow[1].name = "Changed elsewhere"
        var otherParameters = ModelParameters.webAlignedCloudDefaults
        otherParameters.temperature = 0.7
        let updated = try MacRemoteModelDraftCommit.save(
            otherWindow, parametersByModelID: ["scoped-b-model": otherParameters],
            expectedRevisions: original.revisions, store: store
        )
        XCTAssertEqual(updated.revisions["scoped-a-model"], original.revisions["scoped-a-model"])
        XCTAssertEqual(try port.modelProfile("scoped-a-model"), untouchedProfile,
            "A full catalog save must retain unchanged source bytes and not manufacture a conflict.")
        var selected = original.endpoints[0]
        selected.models[0].displayName = "Edited model A"
        var foreignPendingParameters = otherParameters
        foreignPendingParameters.temperature = 1.4
        let saved = try MacRemoteModelDraftCommit.saveConnection(
            endpointID: selected.id, draft: selected, baseline: original.endpoints,
            parametersByModelID: ["scoped-b-model": foreignPendingParameters],
            expectedRevisions: original.revisions, store: store
        )
        XCTAssertEqual(saved.endpoints.first(where: { $0.id == "scoped-a" })?.models.first?.displayName, "Edited model A")
        XCTAssertEqual(saved.endpoints.first(where: { $0.id == "scoped-b" }),
                       updated.endpoints.first(where: { $0.id == "scoped-b" }))
        XCTAssertEqual(try ModelParametersStore(applicationPort: port).parameterSnapshot(for: "scoped-b-model").parameters.temperature, 0.7)
    }

    func testConnectionDraftRejectsSelectedConnectionConcurrentEditAndDeletion() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-scoped-model-conflict-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let original = try MacRemoteModelDraftCommit.save(
            scopedConnectionFixtures(), parametersByModelID: [:], expectedRevisions: [:], store: store
        )
        var concurrent = original.endpoints
        concurrent[0].models.append(.init(id: "scoped-a-new", modelName: "new-model", executionCapabilities: .init()))
        let updated = try MacRemoteModelDraftCommit.save(
            concurrent, parametersByModelID: [:], expectedRevisions: original.revisions, store: store
        )
        for draft in [Optional(original.endpoints[0]), nil] {
            XCTAssertThrowsError(try MacRemoteModelDraftCommit.saveConnection(
                endpointID: "scoped-a", draft: draft, baseline: original.endpoints,
                parametersByModelID: [:], expectedRevisions: original.revisions, store: store
            )) { error in
                XCTAssertTrue(MacRemoteModelSaveFailure.isCatalogConflict(error))
            }
            XCTAssertEqual(try store.endpointSnapshot().endpoints, updated.endpoints)
        }
    }

    func testConnectionDeletionLeavesOtherConnectionAndParametersIntact() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-scoped-model-delete-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.8
        let original = try MacRemoteModelDraftCommit.save(
            scopedConnectionFixtures(), parametersByModelID: ["scoped-b-model": parameters],
            expectedRevisions: [:], store: store
        )
        let deleted = try MacRemoteModelDraftCommit.saveConnection(
            endpointID: "scoped-a", draft: nil, baseline: original.endpoints,
            parametersByModelID: [:], expectedRevisions: original.revisions, store: store
        )
        XCTAssertEqual(deleted.endpoints.map(\.id), ["scoped-b"])
        XCTAssertEqual(deleted.endpoints.first?.models.map(\.id), ["scoped-b-model"])
        XCTAssertEqual(try ModelParametersStore(applicationPort: port).parameterSnapshot(for: "scoped-b-model").parameters.temperature, 0.8)
    }

    private func scopedConnectionFixtures() -> [MacRemoteEndpointConfiguration] {
        [
            .init(id: "scoped-a", name: "Connection A", baseURL: "https://a.invalid/v1",
                  models: [.init(id: "scoped-a-model", modelName: "model-a", executionCapabilities: .init())]),
            .init(id: "scoped-b", name: "Connection B", baseURL: "https://b.invalid/v1",
                  models: [.init(id: "scoped-b-model", modelName: "model-b", executionCapabilities: .init())])
        ]
    }

    func testExecutionCapabilityDecoderMatchesSharedRequiredFieldsAndMissingOptionalDefaults() throws {
        for mask in 0..<16 {
            let fields: [String: Any] = [
                "toolCalls": mask & 1 != 0, "imageInput": mask & 2 != 0,
                "reasoning": mask & 4 != 0, "samplingParameters": mask & 8 != 0,
            ]
            let decoded = try JSONDecoder().decode(MacModelExecutionCapabilities.self,
                from: JSONSerialization.data(withJSONObject: fields))
            XCTAssertEqual(decoded.toolCalls, mask & 1 != 0)
            XCTAssertEqual(decoded.imageInput, mask & 2 != 0)
            XCTAssertEqual(decoded.reasoning, mask & 4 != 0)
            XCTAssertEqual(decoded.samplingParameters, mask & 8 != 0)
            XCTAssertFalse(decoded.samplingRequiresNoReasoning)
            XCTAssertEqual(decoded.chatOutputTokenField ?? .auto, .auto)
            XCTAssertEqual(MacModelExecutionCapabilityStatus(decoded),
                decoded.toolCalls ? .toolsEnabled : .textOnly)
            let saved = try XCTUnwrap(JSONSerialization.jsonObject(
                with: JSONEncoder().encode(decoded)) as? [String: Any])
            XCTAssertEqual(saved["samplingRequiresNoReasoning"] as? Bool, false)
            XCTAssertNil(saved["chatOutputTokenField"], "Keep the existing auto encoding omission.")
            XCTAssertEqual(try JSONDecoder().decode(MacModelExecutionCapabilities.self,
                from: JSONEncoder().encode(decoded)), decoded)
        }
        for tokenField in MacChatOutputTokenField.allCases {
            let value = MacModelExecutionCapabilities(toolCalls: true, reasoning: true,
                samplingParameters: true, samplingRequiresNoReasoning: true,
                chatOutputTokenField: tokenField)
            XCTAssertEqual(try JSONDecoder().decode(MacModelExecutionCapabilities.self,
                from: JSONEncoder().encode(value)), value)
        }
    }

    func testExecutionCapabilityDecoderRejectsMissingRequiredNullMalformedUnknownAndInconsistentFields() throws {
        let required = ["toolCalls", "imageInput", "reasoning", "samplingParameters"]
        let baseline: [String: Any] = Dictionary(uniqueKeysWithValues: required.map { ($0, false) })
        func rejects(_ fields: [String: Any], file: StaticString = #filePath, line: UInt = #line) throws {
            let data = try JSONSerialization.data(withJSONObject: fields)
            XCTAssertThrowsError(try JSONDecoder().decode(MacModelExecutionCapabilities.self,
                from: data), file: file, line: line)
        }
        for key in required {
            var missing = baseline
            missing.removeValue(forKey: key)
            try rejects(missing)
            for invalid in [NSNull(), "false", 1] as [Any] {
                var malformed = baseline
                malformed[key] = invalid
                try rejects(malformed)
            }
        }
        for invalid in [NSNull(), "false", 1] as [Any] {
            var malformed = baseline
            malformed["samplingRequiresNoReasoning"] = invalid
            try rejects(malformed)
        }
        for invalid in [NSNull(), false, 1, "max_completion"] as [Any] {
            var malformed = baseline
            malformed["chatOutputTokenField"] = invalid
            try rejects(malformed)
        }
        var unknown = baseline
        unknown["providerGuessedSupport"] = true
        try rejects(unknown)
        for (sampling, reasoning) in [(false, false), (true, false), (false, true)] {
            var inconsistent = baseline
            inconsistent["samplingRequiresNoReasoning"] = true
            inconsistent["samplingParameters"] = sampling
            inconsistent["reasoning"] = reasoning
            try rejects(inconsistent)
        }
        for invalid in ["{}", "null", "[]"] {
            XCTAssertThrowsError(try JSONDecoder().decode(MacModelExecutionCapabilities.self,
                from: Data(invalid.utf8)))
        }
    }

    func testMissingExecutionDeclarationRemainsVisibleAndBlocksAdmissionUntilConfirmed() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-capability-declaration-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let databaseURL = root.appendingPathComponent("nola.db")
        let port = try MacApplicationPort(databaseURL: databaseURL)
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "gateway", name: "Gateway", baseURL: "https://models.invalid/v1",
            models: [
                .init(id: "unknown", modelName: "arbitrary-model", priority: 90,
                    capabilities: [.chat, .issueRepair], executionCapabilities: .init()),
                .init(id: "declared", modelName: "declared-model", priority: 50,
                    executionCapabilities: .init(toolCalls: true)),
            ]
        )])
        let profile = try XCTUnwrap(port.modelProfile("unknown"))
        var source = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any])
        source.removeValue(forKey: "executionCapabilities")
        let sourceJSON = String(decoding: try JSONSerialization.data(withJSONObject: source, options: [.sortedKeys]), as: UTF8.self)
        try port.upsertModelProfile(MacCanonicalModelProfile(
            id: profile.id, displayName: profile.displayName, provider: profile.provider,
            modelName: profile.modelName, runtime: profile.runtime, sourceJSON: sourceJSON,
            parametersJSON: profile.parametersJSON, selected: profile.selected, enabled: profile.enabled,
            revision: profile.revision, createdAtMs: profile.createdAtMs,
            updatedAtMs: profile.updatedAtMs + 1, deletedAtMs: profile.deletedAtMs
        ))

        let reopened = MacAppleChatConfigurationStore(applicationPort: try MacApplicationPort(databaseURL: databaseURL))
        let untouched = try XCTUnwrap(port.modelProfile("unknown"))
        XCTAssertEqual(reopened.configuredModels().map(\.profileID), ["unknown", "declared"])
        XCTAssertEqual(reopened.configuredModels().first?.executionCapabilityStatus, .needsConfirmation)
        let snapshot = try reopened.endpointSnapshot()
        let unknown = try XCTUnwrap(snapshot.endpoints.first?.models.first(where: { $0.id == "unknown" }))
        XCTAssertFalse(unknown.hasExecutionCapabilityDeclaration)
        XCTAssertNil(unknown.declaredExecutionCapabilities)
        XCTAssertEqual(reopened.displayValues().model, "arbitrary-model")
        XCTAssertNil(reopened.load(model: "unknown"))
        XCTAssertThrowsError(try reopened.loadThrowing(model: "unknown")) {
            XCTAssertTrue($0 is MacModelCapabilityDeclarationError)
        }
        XCTAssertThrowsError(try reopened.routePlanThrowing(preferredModel: "unknown")) {
            XCTAssertTrue($0 is MacModelCapabilityDeclarationError)
        }
        XCTAssertEqual(try reopened.routePlanThrowing(preferredModel: "declared")?.fallbacks.count, 0)
        XCTAssertThrowsError(try reopened.replaceEndpoints(snapshot.endpoints))
        XCTAssertEqual(try port.modelProfile("unknown"), untouched,
            "Reading or refusing unknown capability data must never rewrite the user's model profile.")

        var draft = snapshot.endpoints
        let index = try XCTUnwrap(draft[0].models.firstIndex(where: { $0.id == "unknown" }))
        draft[0].models[index].confirmExecutionCapabilities()
        try reopened.replaceEndpoints(draft, expectedRevisions: snapshot.revisions)
        let confirmed = MacAppleChatConfigurationStore(applicationPort: try MacApplicationPort(databaseURL: databaseURL))
        XCTAssertEqual(confirmed.configuredModels().first?.executionCapabilityStatus, .textOnly)
        let admitted = try XCTUnwrap(confirmed.loadThrowing(model: "unknown"))
        XCTAssertFalse(admitted.executionCapabilities.toolCalls)
        XCTAssertEqual(admitted.capabilities, [.chat, .issueRepair])
        let saved = try XCTUnwrap(port.modelProfile("unknown"))
        let savedSource = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(saved.sourceJSON.utf8)) as? [String: Any])
        let savedDeclaration = try XCTUnwrap(savedSource["executionCapabilities"] as? [String: Any])
        XCTAssertEqual(savedDeclaration["toolCalls"] as? Bool, false)
    }

    func testModelCapabilityDraftDistinguishesMissingFromExplicitUnsupported() throws {
        var missing = MacRemoteModelConfiguration(id: "model", modelName: "any-name", executionCapabilities: nil)
        let declared = MacRemoteModelConfiguration(id: "model", modelName: "any-name", executionCapabilities: .init())
        XCTAssertEqual(missing.executionCapabilityStatus, .needsConfirmation)
        XCTAssertEqual(declared.executionCapabilityStatus, .textOnly)
        XCTAssertNotEqual(missing, declared)
        let decoded = try JSONDecoder().decode(MacRemoteModelConfiguration.self, from: JSONEncoder().encode(missing))
        XCTAssertFalse(decoded.hasExecutionCapabilityDeclaration)
        XCTAssertNil(decoded.declaredExecutionCapabilities)
        missing.executionCapabilities.toolCalls = true
        XCTAssertNil(missing.declaredExecutionCapabilities, "Editing an unknown draft alone is not confirmation.")
        missing.confirmExecutionCapabilities()
        XCTAssertEqual(missing.executionCapabilityStatus, .toolsEnabled)
        let confirmed = try JSONDecoder().decode(MacRemoteModelConfiguration.self, from: JSONEncoder().encode(missing))
        XCTAssertTrue(confirmed.declaredExecutionCapabilities?.toolCalls == true)
    }

    func testChangedModelRouteRequiresNewExplicitDeclarationAndDoesNotWriteOnFailure() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-capability-route-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let original = MacRemoteEndpointConfiguration(id: "endpoint", name: "Gateway", baseURL: "https://before.invalid/v1",
            models: [.init(id: "model", modelName: "original", executionCapabilities: .init(toolCalls: true))])
        try store.replaceEndpoints([original])
        let saved = try port.modelProfile("model")
        var changedURL = original
        changedURL.baseURL = "https://after.invalid/v1"
        var changedProtocol = original
        changedProtocol.wireProtocol = .responses
        var changedModel = original
        changedModel.models[0].modelName = "replacement"
        for draft in [changedURL, changedProtocol, changedModel] {
            XCTAssertFalse(draft.models[0].hasExecutionCapabilityDeclaration)
            XCTAssertTrue(draft.models[0].executionCapabilities.toolCalls, "Keep values editable, but not authoritative.")
            XCTAssertThrowsError(try store.replaceEndpoints([draft]))
            XCTAssertEqual(try port.modelProfile("model"), saved)
        }
        XCTAssertThrowsError(try store.save(baseURL: "https://other.invalid/v1", apiKey: "", model: "other"))
        XCTAssertEqual(try port.modelProfile("model"), saved)
        changedURL.models[0].confirmExecutionCapabilities()
        try store.replaceEndpoints([changedURL])
        XCTAssertEqual(try store.loadThrowing(model: "model")?.baseURL.absoluteString, "https://after.invalid/v1")
    }

    func testRemoteExecutionCapabilitiesSurviveColdReopenAndGateReasoning() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-execution-caps-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let databaseURL = root.appendingPathComponent("nola.db")
        let capabilities = MacModelExecutionCapabilities(
            toolCalls: true, imageInput: false, reasoning: false, samplingParameters: true
        )
        let model = MacRemoteModelConfiguration(id: "model", modelName: "fixture", executionCapabilities: capabilities)
        let endpoint = MacRemoteEndpointConfiguration(id: "endpoint", name: "Fixture",
            baseURL: "https://fixture.invalid/v1", models: [model])
        try MacAppleChatConfigurationStore(applicationPort: MacApplicationPort(databaseURL: databaseURL))
            .replaceEndpoints([endpoint])
        let reopened = MacAppleChatConfigurationStore(applicationPort: try MacApplicationPort(databaseURL: databaseURL))
        XCTAssertEqual(reopened.load(model: "model")?.executionCapabilities, capabilities)
        var invalid = model
        var reasoning = MacReasoningConfiguration()
        reasoning.control = .effort
        reasoning.efforts = ["low", "medium", "high"]
        invalid.reasoning = reasoning
        XCTAssertThrowsError(try reopened.replaceEndpoints([
            MacRemoteEndpointConfiguration(id: "endpoint", name: "Fixture",
                baseURL: "https://fixture.invalid/v1", models: [invalid])
        ]))
        var conditional = model
        conditional.executionCapabilities.reasoning = true
        conditional.executionCapabilities.samplingRequiresNoReasoning = true
        XCTAssertThrowsError(try reopened.replaceEndpoints([
            MacRemoteEndpointConfiguration(id: "endpoint", name: "Fixture",
                baseURL: "https://fixture.invalid/v1", models: [conditional], wireProtocol: .responses)
        ]))
        reasoning.efforts = ["none", "low"]
        conditional.reasoning = reasoning
        try reopened.replaceEndpoints([
            MacRemoteEndpointConfiguration(id: "endpoint", name: "Fixture",
                baseURL: "https://fixture.invalid/v1", models: [conditional], wireProtocol: .responses)
        ])
        XCTAssertTrue(reopened.load(model: "model")?.executionCapabilities.samplingRequiresNoReasoning == true)
    }

    func testRemoteModelConfigurationSurvivesRealServiceColdReopenAndClearsCanonically() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-remote-model-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("nola.sqlite")
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)

        do {
            let canonical = try MacApplicationPort(databaseURL: databaseURL)
            let configuration = MacAppleChatConfigurationStore(applicationPort: canonical)
            try configuration.save(
                baseURL: "https://models.example.test/v1",
                apiKey: "",
                model: "alpha, beta",
                executionCapabilities: .init()
            )

            XCTAssertEqual(configuration.configuredModelNames(), ["alpha", "beta"])
            XCTAssertEqual(configuration.load()?.model, "alpha")
            XCTAssertEqual(configuration.load()?.contextWindowTokens, 128_000)
            let betaID = try XCTUnwrap(configuration.configuredModels().first { $0.modelName == "beta" }?.profileID)
            XCTAssertEqual(configuration.load(model: betaID)?.baseURL.absoluteString, "https://models.example.test/v1")

            let alphaID = try XCTUnwrap(
                canonical.listModelProfiles().first(where: { $0.modelName == "alpha" })?.id
            )
            var changedParameters = ModelParameters.webAlignedCloudDefaults
            changedParameters.maxContextLength = 65_536
            try canonical.resetModelParameters(
                profileIDs: [alphaID],
                parametersJSON: String(decoding: try JSONEncoder().encode(changedParameters), as: UTF8.self),
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
            )
            XCTAssertEqual(configuration.load()?.contextWindowTokens, 65_536)
            XCTAssertEqual(configuration.load(model: betaID)?.contextWindowTokens, 128_000)

            XCTAssertThrowsError(
                try configuration.save(baseURL: "", apiKey: "", model: "replacement")
            )
            XCTAssertEqual(configuration.displayValues().baseURL, "https://models.example.test/v1")
            XCTAssertEqual(configuration.displayValues().model, "alpha")

            try configuration.save(
                baseURL: "https://replacement.example.test/v1",
                apiKey: "",
                model: "gamma, delta",
                executionCapabilities: .init()
            )
            let deltaID = try XCTUnwrap(configuration.configuredModels().first { $0.modelName == "delta" }?.profileID)
            let catalogSelection = try canonical.listModelProfiles().filter(\.selected).map(\.id)
            _ = try MacDefaultChatModelStore(application: canonical).set(.profile(deltaID), expectedRevision: nil,
                candidates: configuration.configuredModels().map { .init(profileID: $0.profileID) })
            XCTAssertEqual(configuration.load()?.model, "delta")
            XCTAssertEqual(
                try canonical.listModelProfiles().filter(\.selected).map(\.id),
                catalogSelection,
                "Changing the product chat default must not rewrite provider catalog selection."
            )
            XCTAssertEqual(
                configuration.load(model: deltaID)?.baseURL.absoluteString,
                "https://replacement.example.test/v1"
            )
        }

        do {
            // Production creates the shared port without performing IO, then
            // loads provider settings while the service is starting. Exercise
            // that cold deferred path rather than reopening an eager client.
            let reopened = MacApplicationPort(deferredDatabaseURL: databaseURL)
            let configuration = MacAppleChatConfigurationStore(applicationPort: reopened)
            XCTAssertEqual(configuration.configuredModelNames(), ["gamma", "delta"])
            XCTAssertEqual(configuration.displayValues().baseURL, "https://replacement.example.test/v1")
            XCTAssertEqual(configuration.displayValues().model, "delta")
            XCTAssertEqual(configuration.load()?.model, "delta")

            try configuration.save(baseURL: "", apiKey: "", model: "")
        }

        let finalStore = try MacApplicationPort(databaseURL: databaseURL)
        let finalConfiguration = MacAppleChatConfigurationStore(applicationPort: finalStore)
        XCTAssertTrue(finalConfiguration.configuredModelNames().isEmpty)
        XCTAssertNil(finalConfiguration.load())
    }

    func testMultipleEndpointsPreserveIdentityAndRouteByPriorityOrderAndCapability() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-remote-routing-e2e-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: canonical)

        try configuration.replaceEndpoints([
            MacRemoteEndpointConfiguration(
                id: "private",
                name: "Private Gateway",
                baseURL: "https://private.example.test/v1",
                order: 0,
                models: [
                    MacRemoteModelConfiguration(
                        id: "private-shared",
                        modelName: "shared-model",
                        displayName: "Private Shared",
                        priority: 80,
                        order: 0,
                        capabilities: [.chat, .issueRepair, .sensitiveData],
                        executionCapabilities: .init()
                    ),
                    MacRemoteModelConfiguration(
                        id: "private-search",
                        modelName: "search-model",
                        displayName: "Private Search",
                        priority: 60,
                        order: 1,
                        capabilities: [.retrieval, .sensitiveData],
                        executionCapabilities: .init()
                    ),
                ]
            ),
            MacRemoteEndpointConfiguration(
                id: "public",
                name: "Public Gateway",
                baseURL: "https://public.example.test/v1",
                order: 1,
                models: [
                    MacRemoteModelConfiguration(
                        id: "public-shared",
                        modelName: "shared-model",
                        displayName: "Public Shared",
                        priority: 90,
                        order: 0,
                        capabilities: [.chat, .retrieval],
                        executionCapabilities: .init()
                    ),
                ]
            ),
        ])

        XCTAssertEqual(configuration.endpoints().map(\.id), ["private", "public"])
        XCTAssertEqual(configuration.configuredModels().map(\.profileID), ["public-shared", "private-shared"])
        XCTAssertEqual(configuration.load()?.profileID, "public-shared")
        XCTAssertEqual(configuration.load()?.capabilities, [.chat, .retrieval])
        XCTAssertEqual(configuration.load(model: "private-shared")?.baseURL.absoluteString, "https://private.example.test/v1")
        XCTAssertEqual(
            configuration.load(model: "private-shared")?.capabilities,
            [.chat, .issueRepair, .sensitiveData]
        )
        XCTAssertEqual(configuration.load(model: "public-shared")?.baseURL.absoluteString, "https://public.example.test/v1")
        XCTAssertEqual(configuration.configuredModels(for: .retrieval).map(\.profileID), ["public-shared", "private-search"])
        XCTAssertEqual(
            try canonical.listModelProfiles().filter(\.selected).map(\.id),
            ["public-shared"],
            "The provider catalog has its own initial selection, independent of the product chat default."
        )

        let initialRoute = try XCTUnwrap(configuration.routePlan(
            preferredModel: "public-shared",
            for: .chat
        ))
        XCTAssertEqual(initialRoute.primary.profileID, "public-shared")
        XCTAssertEqual(initialRoute.fallbacks.map(\.profileID), ["private-shared"])
        XCTAssertEqual(initialRoute.fallbacks.map(\.endpointName), ["Private Gateway"])

        let fallbackStatus = MacRemoteModelRoutePresentation.status(
            for: initialRoute,
            failedCandidateIndex: 0,
            fallsBack: true,
            failureMessage: "The provider quota is exhausted."
        )
        XCTAssertEqual(fallbackStatus.title, String(localized: "Model fallback"))
        XCTAssertTrue(fallbackStatus.detail.contains("Public Gateway · shared-model"))
        XCTAssertTrue(fallbackStatus.detail.contains("The provider quota is exhausted."))
        XCTAssertTrue(fallbackStatus.detail.contains("Private Gateway · shared-model"))

        var incompatibleEndpoints = configuration.endpoints()
        let privateIndex = try XCTUnwrap(incompatibleEndpoints.firstIndex(where: { $0.id == "private" }))
        let privateModelIndex = try XCTUnwrap(incompatibleEndpoints[privateIndex].models.firstIndex(where: { $0.id == "private-shared" }))
        incompatibleEndpoints[privateIndex].models[privateModelIndex].executionCapabilities.toolCalls = true
        try configuration.replaceEndpoints(incompatibleEndpoints)
        XCTAssertTrue(try XCTUnwrap(configuration.routePlan(preferredModel: "public-shared", for: .chat)).fallbacks.isEmpty,
                      "A fallback cannot inherit the primary model's execution contract")

        let resetDate = Date().addingTimeInterval(3_600)
        configuration.markQuotaExhausted(profileID: "public-shared", until: resetDate)
        XCTAssertThrowsError(try configuration.routePlanThrowing(
            preferredModel: "public-shared", for: .chat
        ), "A cooled explicit primary must not silently select another provider before submission")
        XCTAssertGreaterThan(configuration.unavailableUntil(profileID: "public-shared"), Date())

        let quotaDetail = #"attempt=2 error={"error":{"metadata":{"headers":{"X-RateLimit-Remaining":"0","X-RateLimit-Reset":"1789257600000"}}}}"#
        XCTAssertEqual(MacRemoteQuotaFailurePolicy.failedCandidateIndex(from: quotaDetail), 0)
        let parsedReset = try XCTUnwrap(
            MacRemoteQuotaFailurePolicy.exhaustedQuotaResetDate(from: quotaDetail)
        )
        XCTAssertEqual(parsedReset.timeIntervalSince1970, 1_789_257_600, accuracy: 0.001)
    }

    func testCredentialReplacementKeepsPublishedKeyWhenCatalogCompareAndSwapFails() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-credential-cas-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: canonical)
        func endpoint(key: String) -> MacRemoteEndpointConfiguration {
            MacRemoteEndpointConfiguration(
                id: "provider", name: "Provider", baseURL: "https://models.example.test/v1",
                apiKey: key,
                models: [MacRemoteModelConfiguration(id: "model", modelName: "model", executionCapabilities: .init())]
            )
        }

        try configuration.replaceEndpoints([endpoint(key: "old-key")])
        let staleRevisions = try configuration.endpointSnapshot().revisions
        try configuration.replaceEndpoints([endpoint(key: "published-key")])
        XCTAssertEqual(try configuration.endpointSnapshot().endpoints.first?.apiKey, "published-key")

        XCTAssertThrowsError(try configuration.replaceEndpoints(
            [endpoint(key: "stale-key")], expectedRevisions: staleRevisions
        ))
        XCTAssertEqual(try configuration.endpointSnapshot().endpoints.first?.apiKey, "published-key")
        XCTAssertEqual(configuration.load()?.apiKey, "published-key")

        try configuration.replaceEndpoints([endpoint(key: "")])
        XCTAssertEqual(try configuration.endpointSnapshot().endpoints.first?.apiKey, "")
    }

    func testEndpointURLValidationMatchesWebAndRejectsEmbeddedCredentials() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-endpoint-url-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: canonical)

        for baseURL in [
            "file:///tmp/model", "javascript:alert(1)",
            "https://user:secret@example.test/v1", "https://example.test/v1#fragment",
            "https://example.test/v1#", "not a URL",
        ] {
            XCTAssertFalse(MacRemoteEndpointURL.isValid(baseURL), baseURL)
            XCTAssertThrowsError(try configuration.replaceEndpoints([
                MacRemoteEndpointConfiguration(
                    name: "Provider", baseURL: baseURL,
                    models: [MacRemoteModelConfiguration(modelName: "model")]
                )
            ]), baseURL)
        }
        XCTAssertTrue(MacRemoteEndpointURL.isValid("https://example.test/v1"))
        XCTAssertTrue(MacRemoteEndpointURL.isValid("http://127.0.0.1:8080/v1"))
        XCTAssertTrue(try configuration.endpointSnapshot().endpoints.isEmpty)
    }

    func testEditorValidationUsesSaveAdmissionWithoutWritingProfiles() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-editor-validation-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: canonical)
        let incomplete = MacRemoteEndpointConfiguration(name: "Provider", baseURL: "https://example.test/v1", models: [])
        let ready = MacRemoteEndpointConfiguration(
            name: "Provider", baseURL: "https://example.test/v1",
            models: [MacRemoteModelConfiguration(modelName: "model", executionCapabilities: .init())]
        )

        XCTAssertThrowsError(try configuration.validateEndpoints([incomplete]))
        XCTAssertNoThrow(try configuration.validateEndpoints([ready]))
        XCTAssertNoThrow(try configuration.validateEndpoints([]))
        XCTAssertTrue(try configuration.endpointSnapshot().endpoints.isEmpty)
    }

    func testDiscoveredModelCatalogIsBoundToRouteAndCredentialDraft() {
        let endpoint = MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Provider", baseURL: "https://example.test/v1",
            apiKey: "first-key", models: []
        )
        let catalog = MacDiscoveredModelCatalog(endpoint: endpoint, models: ["available-model"])
        XCTAssertEqual(catalog.models(for: endpoint), ["available-model"])

        var changed = endpoint
        changed.baseURL = "https://another.test/v1"
        XCTAssertFalse(catalog.matches(changed))
        XCTAssertTrue(catalog.models(for: changed).isEmpty)
        changed = endpoint
        changed.apiKey = "second-key"
        XCTAssertTrue(catalog.models(for: changed).isEmpty)
        changed = endpoint
        changed.wireProtocol = .responses
        XCTAssertTrue(catalog.models(for: changed).isEmpty)
    }

    func testDraggedModelOrderPreservesExplicitRoutingPriorities() {
        let models = [
            MacRemoteModelConfiguration(id: "third", modelName: "third", priority: 80, order: 2),
            MacRemoteModelConfiguration(id: "first", modelName: "first", priority: 10, order: 0),
            MacRemoteModelConfiguration(id: "second", modelName: "second", priority: 80, order: 1),
        ]
        let reordered = MacRemoteModelOrdering.moving(models, draggedID: "third", onto: "second")

        XCTAssertEqual(reordered?.map(\.id), ["first", "second", "third"])
        XCTAssertEqual(reordered?.map(\.order), [0, 1, 2])
        XCTAssertEqual(reordered?.map(\.priority), [10, 80, 80])

        let adjacent = MacRemoteModelOrdering.moving(models, draggedID: "first", onto: "second")
        XCTAssertEqual(adjacent?.map(\.id), ["third", "second", "first"])
        XCTAssertEqual(adjacent?.map(\.priority), [80, 80, 10])
        XCTAssertEqual(adjacent?.map(\.order), [0, 1, 2])
    }

    func testReindexingModelsNeverChangesMixedPriorities() {
        let models = [
            MacRemoteModelConfiguration(id: "high", modelName: "high", priority: 90, order: 4),
            MacRemoteModelConfiguration(id: "low", modelName: "low", priority: 5, order: 9),
        ]
        let reindexed = MacRemoteModelOrdering.reindexed(models)
        XCTAssertEqual(reindexed.map(\.order), [0, 1])
        XCTAssertEqual(reindexed.map(\.priority), [90, 5])
    }

    func testDraftPrimaryChatModelTracksPriorityOrderAndEligibility() {
        var first = MacRemoteModelConfiguration(id: "first", modelName: "first", priority: 60)
        var higher = MacRemoteModelConfiguration(id: "higher", modelName: "higher", priority: 90)
        var tie = MacRemoteModelConfiguration(id: "tie", modelName: "tie", priority: 90)
        let endpoints = [
            MacRemoteEndpointConfiguration(name: "one", baseURL: "https://one.test/v1", models: [first, higher]),
            MacRemoteEndpointConfiguration(name: "two", baseURL: "https://two.test/v1", models: [tie]),
        ]
        XCTAssertEqual(MacRemoteModelOrdering.primaryChatModelID(in: endpoints), "higher")
        XCTAssertEqual(MacRemoteModelOrdering.primaryChatModelID(in: Array(endpoints.reversed())), "tie")

        higher.isEnabled = false
        tie.capabilities = [.retrieval]
        XCTAssertEqual(MacRemoteModelOrdering.primaryChatModelID(in: [
            MacRemoteEndpointConfiguration(name: "one", baseURL: "https://one.test/v1", models: [first, higher]),
            MacRemoteEndpointConfiguration(name: "two", baseURL: "https://two.test/v1", models: [tie]),
        ]), "first")
        first.isEnabled = false
        XCTAssertNil(MacRemoteModelOrdering.primaryChatModelID(in: [
            MacRemoteEndpointConfiguration(name: "one", baseURL: "https://one.test/v1", models: [first, higher])
        ]))
    }

    func testEndpointReplacementCommitsNewModelParametersWithProfile() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-remote-model-parameters-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("nola.sqlite")
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let canonical = try MacApplicationPort(databaseURL: databaseURL)
        let configuration = MacAppleChatConfigurationStore(applicationPort: canonical)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxContextLength = 256_000
        parameters.maxOutputTokens = 8_192
        parameters.temperature = 0.2
        let model = MacRemoteModelConfiguration(
            id: "new-model",
            modelName: "reasoning-large",
            displayName: "Reasoning Large",
            executionCapabilities: .init()
        )

        try configuration.replaceEndpoints(
            [MacRemoteEndpointConfiguration(
                id: "provider",
                name: "Provider",
                baseURL: "https://models.example.test/v1",
                models: [model]
            )],
            parametersByModelID: [model.id: parameters]
        )

        let profile = try XCTUnwrap(canonical.modelProfile("new-model"))
        let stored = try JSONDecoder().decode(
            ModelParameters.self,
            from: try XCTUnwrap(profile.parametersJSON.data(using: .utf8))
        )
        XCTAssertEqual(stored.maxContextLength, 256_000)
        XCTAssertEqual(stored.maxOutputTokens, 8_192)
        XCTAssertEqual(stored.temperature, 0.2, accuracy: 0.001)
    }

    func testObsoleteUserDefaultsModelConfigurationIsIgnored() throws {
        let defaults = UserDefaults.standard
        let baseURLKey = "nola.mac.chat.baseURL"
        let modelKey = "nola.mac.chat.model"
        let previousBaseURL = defaults.object(forKey: baseURLKey)
        let previousModel = defaults.object(forKey: modelKey)
        defer {
            if let previousBaseURL {
                defaults.set(previousBaseURL, forKey: baseURLKey)
            } else {
                defaults.removeObject(forKey: baseURLKey)
            }
            if let previousModel {
                defaults.set(previousModel, forKey: modelKey)
            } else {
                defaults.removeObject(forKey: modelKey)
            }
        }
        defaults.set("https://obsolete.invalid/v1", forKey: baseURLKey)
        defaults.set("obsolete-model", forKey: modelKey)

        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-model-obsolete-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let canonical = try MacApplicationPort(databaseURL: workspace.appendingPathComponent("nola.db"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: canonical)

        XCTAssertTrue(configuration.configuredModelNames().isEmpty)
        XCTAssertEqual(configuration.displayValues().baseURL, "")
        XCTAssertEqual(configuration.displayValues().model, "")
        XCTAssertNil(configuration.load())
    }

    func testProviderConfigurationCannotRestoreProfilesFromStaleDefaults() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-provider-recovery-e2e-\(UUID().uuidString)", isDirectory: true)
        let suiteName = "nola-provider-recovery-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer {
            defaults.removePersistentDomain(forName: suiteName)
            try? FileManager.default.removeItem(at: root)
        }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)

        let original = MacAppleChatConfigurationStore(
            applicationPort: try MacApplicationPort(databaseURL: root.appendingPathComponent("original.db"))
        )
        try original.save(
            baseURL: "https://durable.example.test/v1",
            apiKey: "",
            model: "alpha, beta",
            executionCapabilities: .init()
        )
        let preference = MacDefaultChatModelStore(application: try MacApplicationPort(databaseURL: root.appendingPathComponent("original.db")))
        let betaID = try XCTUnwrap(original.configuredModels().first { $0.modelName == "beta" }?.profileID)
        _ = try preference.set(.profile(betaID), expectedRevision: nil,
            candidates: original.configuredModels().map { .init(profileID: $0.profileID) })

        // Obsolete client snapshots are not an Application restore command.
        // A new canonical database must stay empty even if such data survives.
        defaults.set(
            Data(#"{"schemaVersion":1,"baseURL":"https://durable.example.test/v1","modelNames":["alpha","beta"],"selectedModel":"beta"}"#.utf8),
            forKey: "nola.mac.provider.configuration.v1"
        )

        let replacementDatabase = root.appendingPathComponent("replacement.db")
        let replacementPort = try MacApplicationPort(databaseURL: replacementDatabase)
        XCTAssertTrue(try replacementPort.listModelProfiles().isEmpty)

        let restored = MacAppleChatConfigurationStore(
            applicationPort: replacementPort
        )
        XCTAssertNil(restored.load())
        XCTAssertEqual(restored.displayValues().baseURL, "")
        XCTAssertEqual(restored.displayValues().model, "")
        XCTAssertTrue(try replacementPort.listModelProfiles().isEmpty)
        XCTAssertEqual(original.load()?.model, "beta")
    }
}
