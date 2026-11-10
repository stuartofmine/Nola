import Foundation
import Testing
import SwiftUI
import UIKit
@testable import Nola

struct ModelSelectionCanonicalEndToEndTests {
    @Test @MainActor
    func manualSetupEnablesTypedModelWithoutDiscoveryAndPersistsDefault() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-manual-setup-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let service = IOSModelConnectionService(application: runtime)
        let connection = try service.create(name: "", endpoint: "https://offline.invalid/v1", wireProtocol: "chat_completions", secret: "", modelID: "user-supplied-model")
        #expect(connection.document.connection.display_name == "https://offline.invalid/v1 · chat_completions")
        #expect(connection.document.connection.account_id == nil)
        #expect(connection.document.catalog?.models.map(\.model_id) == ["user-supplied-model"])
        let profiles = try runtime.listModelProfiles()
        let profile = try #require(profiles.first)
        #expect(profiles.count == 1)
        #expect(profile.enabled)
        #expect(profile.modelName == "user-supplied-model")
        let model = try #require(ModelSelectionManager.modelInfo(from: profile))
        try ModelSelectionManager.commitSelectedModel(model, using: runtime)
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        #expect(try ModelSelectionManager.lastSelectedModel(using: reopened)?.id == profile.id)
        #expect(try ModelSelectionManager.availableModels(using: reopened).map(\.id) == [profile.id])
        let updated = try service.create(name: "", endpoint: "https://offline.invalid/v1", wireProtocol: "chat_completions", secret: "", modelID: "second-manual-model")
        #expect(updated.id == connection.id)
        #expect(try service.connections().count == 1)
        #expect(try runtime.listModelProfiles().count == 2)

    }

    @Test(arguments: ["automatic", "unknown-control"])
    @MainActor
    func canonicalAutomaticReasoningLeavesProviderDefaultsAndUnknownControlRejects(control: String) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-reasoning-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let model = ModelInfo(id: "automatic-fixture", name: "Automatic", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", wireProtocol: .chatCompletions,
            executionCapabilities: .init(toolCalls: false, imageInput: false, reasoning: false, samplingParameters: true),
            cloudCredential: .anonymous)
        let baseline = try profile(model, selected: true, parameters: .webAlignedCloudDefaults)
        var source = try #require(JSONSerialization.jsonObject(with: Data(baseline.sourceJSON.utf8)) as? [String: Any])
        source["reasoning"] = ["control": control]
        let imported = IOSCanonicalModelProfile(id: baseline.id, displayName: baseline.displayName,
            provider: baseline.provider, modelName: baseline.modelName, runtime: baseline.runtime,
            sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: baseline.parametersJSON, selected: baseline.selected, enabled: baseline.enabled,
            revision: baseline.revision, createdAtMs: baseline.createdAtMs, updatedAtMs: baseline.updatedAtMs,
            deletedAtMs: nil)
        try runtime.upsertModelProfile(imported)
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        if control == "automatic" {
            let resolved = try ModelSelectionManager.resolvedCloudModelForRun(model, using: reopened)
            #expect(resolved.id == model.id)
            #expect(resolved.cloudCredential == .anonymous)
            let config = try IOSConversationRunConfiguration.directCloudProviderConfig(model: resolved,
                parameters: .webAlignedCloudDefaults)
            #expect(config.wireProtocol == .chatCompletions)
            #expect(try reopened.modelProfile(id: model.id)?.sourceJSON == imported.sourceJSON)
        } else {
            #expect(throws: Error.self) {
                try ModelSelectionManager.resolvedCloudModelForRun(model, using: reopened)
            }
        }
    }

    @Test @MainActor
    func cloudProtocolIdentitySurvivesReopenAndSelectsResponsesAdapter() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-protocol-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let model = ModelInfo(id: "responses-fixture", name: "Responses", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", wireProtocol: .responses,
            executionCapabilities: .init(toolCalls: false, imageInput: false, reasoning: false, samplingParameters: true),
            cloudCredential: .anonymous)
        try runtime.upsertModelProfile(profile(model, selected: true, parameters: .webAlignedCloudDefaults))
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let resolved = try ModelSelectionManager.resolvedCloudModelForRun(model, using: reopened)
        #expect(resolved.wireProtocol == .responses)
        #expect(resolved.apiKey == nil)
        #expect(resolved.cloudCredential == .anonymous)
        let config = try IOSConversationRunConfiguration.directCloudProviderConfig(
            model: resolved, parameters: .webAlignedCloudDefaults)
        #expect(config.wireProtocol == .responses)
        let stored = try #require(try reopened.modelProfile(id: model.id))
        let persisted = try JSONDecoder().decode(ModelInfo.self, from: Data(stored.sourceJSON.utf8))
        #expect(persisted.apiKey == nil)
    }

    @Test
    func mobileProfileDoesNotDiscardImportedEffortRequirement() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-effort-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let model = ModelInfo(id: "effort-fixture", name: "Effort", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", wireProtocol: .responses, cloudCredential: .anonymous)
        let baseline = try profile(model, selected: true, parameters: .webAlignedCloudDefaults)
        var source = try #require(JSONSerialization.jsonObject(with: Data(baseline.sourceJSON.utf8)) as? [String: Any])
        source["capabilities"] = ["chat"]
        source["reasoning"] = ["control": "effort", "efforts": ["low", "high"], "defaultPreference": "high"]
        let imported = IOSCanonicalModelProfile(
            id: baseline.id, displayName: baseline.displayName, provider: baseline.provider,
            modelName: baseline.modelName, runtime: baseline.runtime,
            sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: baseline.parametersJSON, selected: baseline.selected, enabled: baseline.enabled,
            revision: baseline.revision, createdAtMs: baseline.createdAtMs,
            updatedAtMs: baseline.updatedAtMs, deletedAtMs: nil)
        try runtime.upsertModelProfile(imported)
        #expect(throws: Error.self) {
            try ModelSelectionManager.resolvedCloudModelForRun(model, using: runtime)
        }
        let labelModel = ModelInfo(id: "label-fixture", name: "Labels", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", wireProtocol: .responses,
            executionCapabilities: .init(toolCalls: true, reasoning: true, samplingParameters: true), cloudCredential: .anonymous)
        let labelBase = try profile(labelModel, selected: false, parameters: .webAlignedCloudDefaults)
        var labelSource = try #require(JSONSerialization.jsonObject(with: Data(labelBase.sourceJSON.utf8)) as? [String: Any])
        labelSource["capabilities"] = ["chat"]
        labelSource["reasoning"] = source["reasoning"]
        try runtime.upsertModelProfileCAS(IOSCanonicalModelProfile(
            id: labelBase.id, displayName: labelBase.displayName, provider: labelBase.provider,
            modelName: labelBase.modelName, runtime: labelBase.runtime,
            sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: labelSource), as: UTF8.self),
            parametersJSON: labelBase.parametersJSON, selected: false, enabled: true,
            revision: 0, createdAtMs: labelBase.createdAtMs,
            updatedAtMs: labelBase.updatedAtMs, deletedAtMs: nil), expectedRevision: 0)
        let editor = ModelParametersStore(applicationPort: runtime)
        #expect(try editor.reasoningEfforts(for: editor.parameterSnapshot(for: labelModel.id)) == ["low", "high"])
    }

    @Test
    func cloudExecutionCapabilitiesPersistWithoutChangingParametersOrSelection() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-execution-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let model = ModelInfo(id: "fixture", name: "Fixture", source: "api", isLocal: false,
            baseURL: "https://fixture.invalid/v1", wireProtocol: .chatCompletions, cloudCredential: .anonymous)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 8192
        try runtime.upsertModelProfile(profile(model, selected: true, parameters: parameters))
        let updated = try ModelSelectionManager.updateExecutionCapabilities(for: model,
            to: .init(toolCalls: true, imageInput: true, samplingParameters: true,
                chatOutputTokenField: .maxCompletionTokens), using: runtime)
        #expect(updated.executionCapabilities?.toolCalls == true)
        let stored = try #require(try runtime.modelProfile(id: model.id))
        #expect(stored.selected)
        #expect(stored.parametersJSON.contains("8192"))
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let restored = try #require(try reopened.modelProfile(id: model.id))
        let source = try JSONDecoder().decode(ModelInfo.self, from: Data(restored.sourceJSON.utf8))
        #expect(source.executionCapabilities?.imageInput == true)
        #expect(source.executionCapabilities?.samplingParameters == true)
        #expect(source.executionCapabilities?.chatOutputTokenField == .maxCompletionTokens)
        #expect(source.wireProtocol == .chatCompletions)
        let resolved = try ModelSelectionManager.resolvedCloudModelForRun(model, using: reopened)
        #expect(resolved.executionCapabilities?.toolCalls == true)
    }

    @Test
    func conversationSettingsRejectNonObjectsInsteadOfDiscardingTheirContents() throws {
        for json in ["[]", "null", "123", "\"text\"", "false", "{"] {
            #expect(throws: (any Error).self) {
                _ = try IOSConversationSettingsObject.decode(json)
            }
        }
        #expect(try IOSConversationSettingsObject.decode(nil).isEmpty)
        #expect(try IOSConversationSettingsObject.decode("{}").isEmpty)
        let valid = try IOSConversationSettingsObject.decode(#"{"unknown":{"nested":true}}"#)
        #expect(valid["unknown"] is [String: Any])
    }

    @Test
    func conversationStyleSurvivesReopenAndModelChangesWithoutChangingDefaults() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-style-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let first = try store.createConversation(title: "First", language: nil)
        let second = try store.createConversation(title: "Second", language: nil)
        #expect(first.promptResponseStyle == nil)
        let model = ModelInfo(id: "style-default", name: "Default", source: "local", isLocal: true)
        try runtime.upsertModelProfile(profile(model, selected: true, parameters: .localDefaults))
        try store.setPromptResponseStyle(conversationId: first.id, style: .playful)
        var changed = try #require(try store.getConversation(conversationId: first.id))
        changed.selectedModelInfo = ModelInfo(id: "other-model", name: "Other", source: "local", isLocal: true)
        try store.upsertConversation(changed)
        let reopened = try LocalConversationStore(applicationRuntime: IOSApplicationRuntime(databaseURL: database))
        #expect(try reopened.getConversation(conversationId: first.id)?.promptResponseStyle == .playful)
        #expect(try reopened.getConversation(conversationId: second.id)?.promptResponseStyle == nil)
        #expect(try runtime.modelProfile(id: model.id)?.selected == true)
        #expect(try ModelParametersStore(applicationPort: runtime).parameterSnapshot(for: model.id).parameters.responseStyle == .casual)
        try reopened.setPromptResponseStyle(conversationId: first.id, style: nil)
        #expect(try reopened.getConversation(conversationId: first.id)?.promptResponseStyle == nil)
        let encoded = try #require(runtime.loadConversation(id: first.id.uuidString)?.settingsJSON)
        let fields = try #require(JSONSerialization.jsonObject(with: Data(encoded.utf8)) as? [String: Any])
        #expect(fields["promptResponseStyle"] == nil)
    }

    @Test
    func styleEditPreservesForeignSettingsAndOrdinaryMetadataEditsPreserveUnknownFields() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-style-foreign-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversationID = UUID()
        let foreign = #"{"version":2,"modelOverrides":{"configured:fixture":{"temperature":0}},"responseStyleRawValue":"balanced","promptResponseStyle":"professional"}"#
        try runtime.createConversation(id: conversationID.uuidString, title: "Foreign", settingsJSON: foreign,
            createdAtMs: 1, updatedAtMs: 1)
        let conversation = try #require(try store.getConversation(conversationId: conversationID))
        #expect(try store.getConversation(conversationId: conversation.id)?.promptResponseStyle == .professional)
        try store.setPromptResponseStyle(conversationId: conversation.id, style: .playful)
        let encoded = try #require(runtime.loadConversation(id: conversation.id.uuidString)?.settingsJSON)
        let fields = try #require(JSONSerialization.jsonObject(with: Data(encoded.utf8)) as? [String: Any])
        #expect(fields["version"] as? Int == 2)
        #expect(fields["responseStyleRawValue"] as? String == "balanced")
        #expect(fields["modelOverrides"] is [String: Any])
        try store.updateConversationMetadata(conversationId: conversation.id, language: "zh-Hans", selectedPromptTemplateID: nil, dataSourceOverrides: nil)
        let updated = try #require(runtime.loadConversation(id: conversation.id.uuidString)?.settingsJSON)
        let updatedFields = try #require(JSONSerialization.jsonObject(with: Data(updated.utf8)) as? [String: Any])
        #expect(updatedFields["modelOverrides"] is [String: Any])
        #expect(updatedFields["responseStyleRawValue"] as? String == "balanced")
        #expect(try store.getConversation(conversationId: conversation.id)?.promptResponseStyle == .playful)
    }

    @Test
    func enteringTheDisplayedNumberDoesNotCreateAnUnsavedDraft() {
        let original = ModelParameterDraftFingerprint(
            values: ["0.7", "prompt"], numericInput: [:], displayedNumbers: ["temperature": "0.7"]
        )
        let restored = ModelParameterDraftFingerprint(
            values: ["0.7", "prompt"], numericInput: ["temperature": "0.7"], displayedNumbers: ["temperature": "0.7"]
        )
        #expect(restored == original)
        let incomplete = ModelParameterDraftFingerprint(
            values: ["0.7", "prompt"], numericInput: ["temperature": "-"], displayedNumbers: ["temperature": "0.7"]
        )
        #expect(incomplete != original)
        let cleared = ModelParameterDraftFingerprint(
            values: ["0.7", "prompt"], numericInput: ["temperature": ""], displayedNumbers: ["temperature": "0.7"]
        )
        #expect(cleared != original)
    }

    @Test
    func freeTextDelimitersCannotHideParameterChanges() {
        let first = ModelParameterDraftFingerprint(
            values: ["stop\u{1F}sequence", "prompt"], numericInput: [:], displayedNumbers: [:]
        )
        let second = ModelParameterDraftFingerprint(
            values: ["stop", "sequence\u{1F}prompt"], numericInput: [:], displayedNumbers: [:]
        )
        #expect(first != second)
        let precise = ModelParameterDraftFingerprint(
            values: ["0.123456"], numericInput: ["temperature": "0.123456"], displayedNumbers: ["temperature": "0.123456"]
        )
        let rounded = ModelParameterDraftFingerprint(
            values: ["0.12"], numericInput: ["temperature": "0.12"], displayedNumbers: ["temperature": "0.12"]
        )
        #expect(precise != rounded)
    }

    @Test @MainActor
    func openingConversationSettingsDoesNotReplaceConversationStyleWithModelDefault() async throws {
        var styleWrites: [PromptResponseStyle] = []
        let settings = ChatMoreSettingsSheet(
            selectedModel: ModelInfo(id: "scope-fixture", name: "Fixture", source: "local", isLocal: true),
            contextUsage: nil, hostUploadedSkills: [], hostUploadedDocuments: [], hostMCPServers: [],
            hostComputerUseStatus: nil, hostNativeBrowserStatus: nil, pairedHostIdentity: nil,
            conversationGenerationOverride: .init(), onSaveGenerationOverride: { _ in },
            hybridDeviceToolsEnabled: .constant(false), hybridFilesEnabled: .constant(false),
            hybridClipboardEnabled: .constant(false), hybridLocationEnabled: .constant(false),
            hybridCalendarEnabled: .constant(false), enableRetrieval: .constant(false),
            permissionMode: .constant(.cautious), autoCompactTriggerPercent: .constant(80),
            responseStyle: Binding(get: { .playful }, set: { if let style = $0 { styleWrites.append(style) } }),
            sessionMemory: .constant(""), longTermMemory: .constant(""), userPreferences: .constant(""),
            uploadedSkills: .constant([]), onSaveMemoryProfile: {}, onSetSkillEnabled: { _, _ in },
            onOpenContextSettings: {}, onOpenSkillLibrary: {}, onForgetHostPairing: {}
        )
        let controller = UIHostingController(rootView: NavigationStack { settings })
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        window.rootViewController = controller
        window.makeKeyAndVisible()
        defer { window.isHidden = true; window.rootViewController = nil }
        controller.view.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(200))
        #expect(styleWrites.isEmpty, "Merely opening session settings must not copy global model defaults into the session")
    }
    @Test
    func numericDraftsPreservePrecisionAndRejectIncompleteInput() {
        for text in ["", "-", "NaN", "inf", "1.2.3", "invalid"] {
            #expect(ModelParameterNumberParser.decimal(text) == nil)
        }
        #expect(ModelParameterNumberParser.decimal("0") == 0)
        #expect(ModelParameterNumberParser.decimal("0.123456") == 0.123456)
        #expect(ModelParameterNumberParser.decimal("0,125", locale: Locale(identifier: "fr_FR")) == 0.125)
        #expect(ModelParameterNumberParser.integer("64.5") == nil)
        #expect(ModelParameterNumberParser.integer("99999999999999999999999") == nil)
        #expect(ModelParameterNumberParser.integer("64000") == 64_000)
        #expect(ModelParameterLimits.outputTokens.contains(64_000))
        #expect(ModelParameterLimits.remoteContextTokens.contains(4_000_000))
    }

    @Test
    func twoParameterEditorSnapshotsDoNotSilentlyOverwrite() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-editors-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let model = ModelInfo(id: "editor-fixture", name: "Editor", source: "local", isLocal: true)
        try runtime.upsertModelProfile(profile(model, selected: true, parameters: .localDefaults))
        let store = ModelParametersStore(applicationPort: runtime)
        let first = try store.parameterSnapshot(for: model.id)
        let second = try store.parameterSnapshot(for: model.id)
        var winner = first.parameters
        winner.temperature = 0.2
        let revision = try store.saveParameters(winner, snapshot: first)
        var stale = second.parameters
        stale.temperature = 0.9
        do {
            try store.saveParameters(stale, snapshot: second)
            Issue.record("Stale editor must not overwrite")
        } catch IOSApplicationRuntimeError.revisionConflict(_, _, let actual) {
            #expect(actual == revision)
        }
        #expect(try store.parameterSnapshot(for: model.id).parameters.temperature == 0.2)
        #expect(try runtime.modelProfile(id: model.id)?.selected == true)
        #expect(stale.temperature == 0.9)
    }

    @Test
    func parameterRevisionReceiptCrossesSwiftFFIAndSurvivesReopen() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-cas-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let model = ModelInfo(id: "cas-fixture", name: "CAS", source: "local", isLocal: true)
        let input = try profile(model, selected: false, parameters: .localDefaults)
        let committed: UInt64
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            committed = try runtime.configureModelParameters(input, expectedRevision: 0, requestID: "create")
            #expect(committed > 0)
            #expect(try runtime.configureModelParameters(input, expectedRevision: 0, requestID: "create") == committed)
            do {
                _ = try runtime.configureModelParameters(input, expectedRevision: 0, requestID: "stale")
                Issue.record("Stale write must fail")
            } catch IOSApplicationRuntimeError.revisionConflict(let scope, let expected, let actual) {
                #expect(scope == "model_profile:cas-fixture")
                #expect(expected == 0)
                #expect(actual == committed)
            }
        }
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        #expect(try reopened.configureModelParameters(input, expectedRevision: 0, requestID: "create") == committed)
        #expect(try reopened.modelProfile(id: model.id)?.revision == Int64(committed))
    }

    @Test
    func catalogAndSelectionReadsDoNotRegisterOrSelectModels() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-model-read-only-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            #expect(try ModelSelectionManager.availableModels(using: store).isEmpty)
            #expect(try ModelSelectionManager.lastSelectedModel(using: store) == nil)
            #expect(try store.listModelProfiles().isEmpty)
            let model = ModelInfo(id: "unselected", name: "Unselected", source: "cloud", isLocal: false)
            try store.upsertModelProfile(profile(model, selected: false, parameters: .webAlignedCloudDefaults))
            let before = try #require(try store.modelProfile(id: model.id))
            #expect(try ModelSelectionManager.availableModels(using: store).map(\.id) == [model.id])
            #expect(try ModelSelectionManager.lastSelectedModel(using: store) == nil)
            #expect(try store.modelProfile(id: model.id)?.revision == before.revision)
        }
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let profiles = try reopened.listModelProfiles()
        #expect(profiles.count == 1)
        #expect(profiles.first?.selected == false)
    }

    @Test
    func selectionSourceAndParametersSurviveCanonicalColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-model-selection-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let local = ModelInfo(
            id: "mlx-community/Qwen3.5-4B-MLX-4bit",
            name: "Qwen3.5 4B",
            source: "local",
            isLocal: true
        )
        let cloud = ModelInfo(
            id: "qwen3.5-plus",
            name: "Qwen3.5 Plus",
            source: "cloud",
            isLocal: false,
            baseURL: "https://example.invalid/v1",
            cloudCredential: .anonymous
        )
        var cloudParameters = ModelParameters.webAlignedCloudDefaults
        cloudParameters.temperature = 0.31
        cloudParameters.maxContextLength = 131_072

        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            do {
                try store.upsertModelProfile(profile(local, selected: true, parameters: .localDefaults))
            } catch {
                Issue.record("Local profile upsert failed: \(error)")
                throw error
            }
            do {
                try store.upsertModelProfile(profile(cloud, selected: false, parameters: cloudParameters))
            } catch {
                Issue.record("Cloud profile upsert failed: \(error)")
                throw error
            }
            for model in [cloud, local, cloud] {
                try ModelSelectionManager.commitSelectedModel(model, using: store)
                #expect(try ModelSelectionManager.lastSelectedModel(using: store)?.id == model.id)
            }
        }

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let profiles = try reopened.listModelProfiles()
        #expect(try ModelSelectionManager.lastSelectedModel(using: reopened)?.id == cloud.id)
        let selected = try #require(profiles.first(where: { $0.id == cloud.id }))
        #expect(profiles.first(where: { $0.id == local.id })?.selected == true,
            "Global selection must not deselect a different runtime scope")
        #expect(selected.id == cloud.id)

        let sourceData = try #require(selected.sourceJSON.data(using: .utf8))
        let decodedModel = try JSONDecoder().decode(ModelInfo.self, from: sourceData)
        #expect(decodedModel.id == cloud.id)
        #expect(decodedModel.baseURL == cloud.baseURL)

        let parameterData = try #require(selected.parametersJSON.data(using: .utf8))
        let decodedParameters = try JSONDecoder().decode(ModelParameters.self, from: parameterData)
        #expect(decodedParameters.temperature == 0.31)
        #expect(decodedParameters.maxContextLength == 131_072)
    }

    @Test
    func rejectedSelectionPreservesPreviouslySelectedModel() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-model-rejection-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        let original = ModelInfo(id: "original", name: "Original", source: "cloud", isLocal: false,
            cloudCredential: .anonymous)
        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            try store.upsertModelProfile(profile(original, selected: true, parameters: .webAlignedCloudDefaults))
            try ModelSelectionManager.commitSelectedModel(original, using: store)
            let invalid = ModelInfo(id: "", name: "Invalid", source: "cloud", isLocal: false)
            var rejected = false
            do {
                try ModelSelectionManager.commitSelectedModel(invalid, using: store)
            } catch {
                rejected = true
            }
            #expect(rejected)
        }
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let profiles = try reopened.listModelProfiles()
        #expect(profiles.count == 1)
        #expect(profiles.first?.id == original.id)
        #expect(profiles.first?.selected == true)
        #expect(try ModelSelectionManager.lastSelectedModel(using: reopened)?.id == original.id)
    }

    @Test
    func defaultSelectionCASConflictPreservesWinnerAndMissingProfileDoesNotFallback() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-default-cas-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let local = ModelInfo(id: "local", name: "Local", source: "local", isLocal: true)
        try ModelSelectionManager.commitSelectedModel(local, using: store)
        let snapshot = try #require(try store.productSetting(key: "chat.default_model"))
        let automatic = "{\"version\":1,\"selection\":{\"kind\":\"automatic\"}}"
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let winner = try store.putProductSettingChecked(key: "chat.default_model", valueJSON: automatic,
            expectedRevision: snapshot.revision, updatedAtMs: now + 1)
        do {
            _ = try store.putProductSettingChecked(key: "chat.default_model", valueJSON: snapshot.valueJSON,
                expectedRevision: snapshot.revision, updatedAtMs: now + 2)
            Issue.record("A stale default selection must not overwrite the winner")
        } catch IOSApplicationRuntimeError.rejected(let message) {
            #expect(message.contains("product_setting_revision_conflict"))
        }
        #expect(try store.productSetting(key: "chat.default_model")?.revision == winner.revision)
        #expect(try ModelSelectionManager.lastSelectedModel(using: store) == nil)
        _ = try store.putProductSettingChecked(key: "chat.default_model",
            valueJSON: "{\"version\":1,\"selection\":{\"kind\":\"profile\",\"profile_id\":\"missing\"}}",
            expectedRevision: winner.revision, updatedAtMs: now + 3)
        do {
            _ = try ModelSelectionManager.lastSelectedModel(using: store)
            Issue.record("An unavailable explicit default must not silently choose another profile")
        } catch IOSApplicationRuntimeError.rejected(let message) {
            #expect(message == "The selected default chat model is unavailable.")
        }
    }

    @Test
    func modelTestResolvesOnlyAnInstalledExactLocalModel() {
        let local = ModelInfo(
            id: "mlx-community/Qwen3.5-4B-MLX-4bit",
            name: "Qwen3.5 4B",
            source: "local",
            isLocal: true
        )
        #expect(IOSModelInferenceTestRoute.localRuntimeModel(local, downloaded: true)?.name == local.id)
        #expect(IOSModelInferenceTestRoute.localRuntimeModel(local, downloaded: false)?.name == nil)
        let cloud = ModelInfo(id: local.id, name: "Cloud", source: "api", isLocal: false)
        #expect(IOSModelInferenceTestRoute.localRuntimeModel(cloud, downloaded: true)?.name == nil)
        let missing = ModelInfo(id: "unknown-local-model", name: "Unknown", source: "local", isLocal: true)
        #expect(IOSModelInferenceTestRoute.localRuntimeModel(missing, downloaded: true)?.name == nil)
    }

    private func profile(
        _ model: ModelInfo,
        selected: Bool,
        parameters: ModelParameters
    ) throws -> IOSCanonicalModelProfile {
        var fields = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(model.redactedForPersistence)) as? [String: Any])
        fields["capabilities"] = ["chat"]
        fields["reasoning"] = ["control": "automatic"]
        let source = String(decoding: try JSONSerialization.data(withJSONObject: fields), as: UTF8.self)
        let parameterJSON = try String(decoding: JSONEncoder().encode(parameters), as: UTF8.self)
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        return IOSCanonicalModelProfile(
            id: model.id,
            displayName: model.name,
            provider: model.source,
            modelName: model.effectiveUpstreamModelId,
            runtime: model.isLocal ? "mlx" : "remote",
            sourceJSON: source,
            parametersJSON: parameterJSON,
            selected: selected,
            enabled: true,
            revision: 0,
            createdAtMs: now,
            updatedAtMs: now,
            deletedAtMs: nil
        )
    }
}
