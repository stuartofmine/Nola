// SPDX-License-Identifier: Apache-2.0
import AppKit
import XCTest
import PDFKit
import SQLite3
@testable import Nola

final class MacConversationConfigurationTests: XCTestCase {
    func testRemoteSamplingUnsetSurvivesProfileRoundTrip() throws {
        let parameters = ModelParameters.webAlignedCloudDefaults
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(parameters)) as? [String: Any])
        object["topK"] = NSNull()
        object["repetitionPenalty"] = NSNull()
        let decoded = try JSONDecoder().decode(ModelParameters.self, from: JSONSerialization.data(withJSONObject: object))
        XCTAssertNil(decoded.topK)
        XCTAssertNil(decoded.repetitionPenalty)
        let saved = String(decoding: try JSONEncoder().encode(decoded), as: UTF8.self)
        let restored = try ModelParameters.decodeProfile(saved)
        XCTAssertNil(restored.topK)
        XCTAssertNil(restored.repetitionPenalty)
        XCTAssertEqual(LocalSamplingSettings(parameters: decoded).topK, 40)
        XCTAssertNil(decoded.topK, "Local resolution must not mutate remote intent")
        var explicit = decoded
        explicit.topK = 40
        explicit.repetitionPenalty = 1
        let explicitSaved = String(decoding: try JSONEncoder().encode(explicit), as: UTF8.self)
        let explicitRestored = try ModelParameters.decodeProfile(explicitSaved)
        XCTAssertEqual(explicitRestored.topK, 40)
        XCTAssertEqual(explicitRestored.repetitionPenalty, 1)
        object["topK"] = "invalid"
        let invalid = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
        XCTAssertThrowsError(try ModelParameters.decodeProfile(invalid)) { error in
            XCTAssertTrue(error.localizedDescription.contains("parameters.topK"))
        }
    }

    @MainActor
    func testPermissionConfigurationHasSparseCASAndExplicitConflictReload() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("permissions.sqlite"))
        let id = "permission-cas"
        _ = try port.createConversation(.init(id: id, title: "Policy", settingsJSON: #"{"retrievalEnabledOverride":false,"permissionModeRawValue":"cautious"}"#,
            createdAtMs: 1, updatedAtMs: 1))
        let repository = MacCanonicalConversationRepository(store: port)
        let original = try XCTUnwrap(port.loadConversationMetadata(id))
        let revision = try XCTUnwrap(original.configurationRevision)
        let saved = try await withCheckedThrowingContinuation { continuation in
            repository.enqueuePermissionConfiguration(conversationID: id, expectedRevision: revision, expectedRunID: nil, expectedSourceHostID: nil, mode: .allowAll) {
                continuation.resume(with: $0)
            }
        }
        XCTAssertEqual(saved.mode, .allowAll)
        XCTAssertEqual(saved.revision, revision + 1)
        _ = try port.patchConversationConfiguration(.init(conversationID: id, expectedRevision: saved.revision,
            patchJSON: #"{"memoryEnabledOverride":true}"#, updatedAtMs: 3)).committedSnapshot()
        do {
            _ = try await withCheckedThrowingContinuation { continuation in
                repository.enqueuePermissionConfiguration(conversationID: id, expectedRevision: saved.revision, expectedRunID: nil, expectedSourceHostID: nil, mode: .cautious) {
                    continuation.resume(with: $0)
                }
            }
            XCTFail("A stale permission edit must not rebase itself")
        } catch {}
        let refreshed = try await withCheckedThrowingContinuation { continuation in
            repository.reloadPermissionConfiguration(conversationID: id) { continuation.resume(with: $0) }
        }
        XCTAssertEqual(refreshed.mode, .allowAll)
        XCTAssertEqual(refreshed.revision, saved.revision + 1)
        let stored = try XCTUnwrap(port.loadConversationMetadata(id)?.settingsJSON)
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(stored.utf8)) as? [String: Any])
        XCTAssertEqual(fields["retrievalEnabledOverride"] as? Bool, false)
        XCTAssertEqual(fields["memoryEnabledOverride"] as? Bool, true)
    }

    @MainActor
    func testConfigurationReloadAndEditAcceptAbsentOverridesButRejectDamagedDocuments() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let id = "configuration-inheritance"
        _ = try port.createConversation(.init(id: id, title: "Inherited", settingsJSON: nil,
            createdAtMs: 1, updatedAtMs: 1))
        let repository = MacCanonicalConversationRepository(store: port)
        let original = try XCTUnwrap(port.loadConversationMetadata(id))
        XCTAssertNil(original.settingsJSON)
        let inherited = try await withCheckedThrowingContinuation { continuation in
            repository.reloadModelConfiguration(conversationID: id) { continuation.resume(with: $0) }
        }
        XCTAssertEqual(inherited.revision, original.configurationRevision)
        XCTAssertNil(inherited.selectionID)
        XCTAssertNil(inherited.overrides)
        XCTAssertNil(inherited.executionHostRawValue)
        var conversation = MacConversation(id: id, title: "Inherited", preview: "", updatedAt: .now,
            isPinned: false, isPersisted: true)
        conversation.configurationRevision = inherited.revision
        let edited = try await withCheckedThrowingContinuation { continuation in
            repository.enqueueModelConfiguration(conversation: conversation, selectionID: nil, host: .remote,
                changesSelection: false) { continuation.resume(with: $0) }
        }
        XCTAssertEqual(edited.executionHostRawValue, "remote")
        XCTAssertGreaterThan(edited.revision, inherited.revision)
        conversation.configurationRevision = edited.revision

        var database: OpaquePointer?
        XCTAssertEqual(sqlite3_open(port.databaseURL.path, &database), SQLITE_OK)
        defer { sqlite3_close(database) }
        // Damage only this isolated fixture; malformed and valid non-object
        // documents are neither inheritance nor a reason to rewrite settings.
        for document in ["not-json", "[]", "null", "42"] {
            let sql = "UPDATE conversation_settings SET settings_json='\(document)' WHERE conversation_id='\(id)'"
            XCTAssertEqual(sqlite3_exec(database, sql, nil, nil, nil), SQLITE_OK)
            do {
                _ = try await withCheckedThrowingContinuation { continuation in
                    repository.reloadModelConfiguration(conversationID: id) { continuation.resume(with: $0) }
                }
                XCTFail("Reload accepted damaged configuration: \(document)")
            } catch {}
            do {
                _ = try await withCheckedThrowingContinuation { continuation in
                    repository.enqueueModelConfiguration(conversation: conversation, selectionID: nil, host: .local,
                        changesSelection: false) { continuation.resume(with: $0) }
                }
                XCTFail("Edit accepted damaged configuration: \(document)")
            } catch {}
            let unchanged = try XCTUnwrap(port.loadConversationMetadata(id))
            XCTAssertEqual(unchanged.settingsJSON, document)
            XCTAssertEqual(unchanged.configurationRevision, edited.revision)
        }
    }

    func testResponsesProbeUsesConcreteRouteAndRequiresResponseEnvelope() throws {
        let base = try XCTUnwrap(URL(string: "https://fixture.invalid/v1/chat/completions"))
        XCTAssertEqual(MacResponsesRouteProbe.url(for: base).absoluteString,
                       "https://fixture.invalid/v1/responses")
        XCTAssertEqual(MacResponsesRouteProbe.url(for: try XCTUnwrap(URL(string: "https://fixture.invalid/v1/responses"))).absoluteString,
                       "https://fixture.invalid/v1/responses")
        XCTAssertTrue(MacResponsesRouteProbe.accepts(Data(#"{"object":"response","status":"completed","output":[]}"#.utf8)))
        XCTAssertTrue(MacResponsesRouteProbe.accepts(Data(#"{"object":"response","status":"incomplete","output":[]}"#.utf8)))
        XCTAssertFalse(MacResponsesRouteProbe.accepts(Data(#"{"object":"chat.completion","choices":[]}"#.utf8)))
        XCTAssertFalse(MacResponsesRouteProbe.accepts(Data(#"{"object":"response","status":"failed","output":[]}"#.utf8)))
    }

    func testModelTestUsesStoredRemoteProtocolReasoningAndParameters() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        defer { try? FileManager.default.removeItem(at: root) }
        var configuration = MacAppleChatConfiguration(
            profileID: "profile", endpointID: "endpoint", endpointName: "Fixture",
            baseURL: try XCTUnwrap(URL(string: "https://fixture.invalid/v1")),
            apiKey: "test-key", model: "reasoning-model", contextWindowTokens: 196_000
        )
        configuration.wireProtocol = .responses
        configuration.reasoning = .init(control: .effort, responses: true, efforts: ["low", "high"])
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.4
        parameters.maxOutputTokens = 4096
        configuration.parameters = parameters

        let request = try MacModelTestRunRequestFactory.remote(
            prompt: "Explain this model", configuration: configuration, environment: environment
        )
        XCTAssertEqual(request.modelID, "reasoning-model")
        XCTAssertEqual(request.endpoint, "https://fixture.invalid/v1")
        XCTAssertEqual(request.apiKey, "test-key")
        XCTAssertEqual(request.contextWindowTokens, 196_000)
        XCTAssertEqual(request.wireProtocol, .responses)
        XCTAssertEqual(request.reasoningConfiguration?.control, .effort)
        XCTAssertEqual(request.reasoningConfiguration?.outputLimit, 4096)
        XCTAssertEqual(request.reasoningPreference, "automatic")
        XCTAssertNil(request.reasoningTier)
        XCTAssertEqual(request.modelParameters?.maxOutputTokens, 4096)
        XCTAssertEqual(request.modelParameters?.temperature, 0.4)
        XCTAssertTrue(request.samplingOverride)
        XCTAssertFalse(request.memoryEnabled)
        XCTAssertEqual(request.permissionMode, .cautious)
        XCTAssertEqual(request.modelPurpose, .chat)
        XCTAssertTrue(request.modelTestMode)
        XCTAssertTrue(request.activeSkillDescriptors.isEmpty)
        XCTAssertNil(request.activeMCPServerConfigurationData)
    }

    func testLocalModelTestUsesItsSavedParametersAndChatProtocol() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        defer { try? FileManager.default.removeItem(at: root) }
        var parameters = ModelParameters.localDefaults
        parameters.maxContextLength = 96_000
        let request = try MacModelTestRunRequestFactory.local(
            prompt: "Hello", profile: .qwen35_08B, parameters: parameters, environment: environment
        )
        XCTAssertEqual(request.modelID, MacLocalModelProfile.qwen35_08B.id)
        XCTAssertEqual(request.contextWindowTokens, 96_000)
        XCTAssertEqual(request.modelParameters?.maxContextLength, 96_000)
        XCTAssertEqual(request.wireProtocol, .chatCompletions)
        XCTAssertFalse(request.memoryEnabled)
        XCTAssertEqual(request.permissionMode, .cautious)
        XCTAssertTrue(request.modelTestMode)
        XCTAssertTrue(request.activeSkillDescriptors.isEmpty)
        XCTAssertNil(request.activeMCPServerConfigurationData)
    }

    @MainActor
    func testOutOfRangePriorityIsRejectedWithoutChangingSavedCatalog() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let initial = [MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Fixture", baseURL: "https://fixture.invalid/v1",
            models: [.init(id: "model", modelName: "fixture-model", priority: 50, executionCapabilities: .init())]
        )]
        let saved = try MacRemoteModelDraftCommit.save(
            initial, parametersByModelID: [:], expectedRevisions: [:], store: store
        )

        for invalidPriority in [-1, 101] {
            var draft = saved.endpoints
            draft[0].models[0].priority = invalidPriority
            XCTAssertThrowsError(try store.validateEndpoints(draft))
            XCTAssertThrowsError(try MacRemoteModelDraftCommit.save(
                draft, parametersByModelID: [:], expectedRevisions: saved.revisions, store: store
            ))
            XCTAssertEqual(try store.endpointSnapshot().endpoints, saved.endpoints)
        }
    }

    @MainActor
    func testMissingEndpointProtocolDoesNotInferResponsesFromReasoningMetadata() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        try port.upsertModelProfile(MacCanonicalModelProfile(
            id: "deepseek", displayName: "DeepSeek", provider: "openai-compatible",
            modelName: "deepseek-flash", runtime: "remote",
            sourceJSON: #"{"schemaVersion":1,"endpointID":"endpoint","endpointName":"DeepSeek","baseURL":"https://api.example.invalid/chat/completions","endpointEnabled":true,"endpointOrder":0,"modelEnabled":true,"modelOrder":0,"priority":100,"capabilities":["chat"],"executionCapabilities":{"toolCalls":false,"imageInput":false,"reasoning":true,"samplingParameters":false,"samplingRequiresNoReasoning":false,"chatOutputTokenField":"auto"},"reasoning":{"control":"automatic","responses":true,"budgets":[1024,4096,8192],"efforts":[],"outputLimit":16384}}"#,
            parametersJSON: String(decoding: try JSONEncoder().encode(ModelParameters.webAlignedCloudDefaults), as: UTF8.self), selected: true, enabled: true, revision: 1,
            createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))

        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let endpoint = try XCTUnwrap(store.endpointSnapshot().endpoints.first)
        XCTAssertEqual(endpoint.wireProtocol, .chatCompletions)
        XCTAssertEqual(endpoint.models.first?.reasoning?.responses, false)
        XCTAssertEqual(store.load(model: "deepseek")?.wireProtocol, .chatCompletions)
        XCTAssertEqual(store.load(model: "deepseek")?.reasoning?.responses, false)
    }

    func testRemoteParameterRouteKeepsProfileIdentitySeparateFromVisibleModelName() throws {
        let route = MacModelParametersRoute(
            modelName: "stable-profile-id", isLocal: false, displayName: "  Research model  "
        )
        XCTAssertEqual(route.displayTitle, "Research model")
        XCTAssertEqual(route.modelName, "stable-profile-id")
        let restored = try JSONDecoder().decode(
            MacModelParametersRoute.self, from: JSONEncoder().encode(route)
        )
        XCTAssertEqual(restored, route)
        XCTAssertEqual(
            route,
            MacModelParametersRoute(modelName: "stable-profile-id", isLocal: false, displayName: "Renamed")
        )
        XCTAssertEqual(MacModelParametersRoute(modelName: "stable-profile-id", isLocal: false).displayTitle,
                       "stable-profile-id")
    }

    @MainActor
    func testManualRemoteModelsPersistWithoutProviderCatalogDiscovery() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let endpoints: [MacRemoteEndpointConfiguration] = [
            .init(id: "responses", name: "Responses fixture", baseURL: "https://unreachable.invalid/v1",
                  models: [.init(id: "responses-model", modelName: "manual-responses-model",
                                 executionCapabilities: .init(reasoning: true),
                                 reasoning: .init(control: .effort, responses: true, efforts: ["low", "high"]))],
                  wireProtocol: .responses),
            .init(id: "anthropic", name: "Messages fixture", baseURL: "https://unreachable.invalid/v1",
                  models: [.init(id: "messages-model", modelName: "manual-messages-model",
                                 executionCapabilities: .init(reasoning: true),
                                 reasoning: .init(control: .budget))],
                  wireProtocol: .anthropic)
        ]

        let saved = try MacRemoteModelDraftCommit.save(
            endpoints, parametersByModelID: [:], expectedRevisions: [:], store: store
        )
        XCTAssertEqual(saved.endpoints.map(\.id), ["responses", "anthropic"])
        XCTAssertEqual(saved.endpoints.flatMap(\.models).map(\.modelName),
                       ["manual-responses-model", "manual-messages-model"])
        XCTAssertEqual(store.load(model: "responses-model")?.wireProtocol, .responses)
        XCTAssertEqual(store.load(model: "messages-model")?.wireProtocol, .anthropic)

        var invalid = endpoints
        invalid[1].models[0].reasoning = .init(control: .effort, efforts: ["high"])
        XCTAssertThrowsError(try MacRemoteModelDraftCommit.save(
            invalid, parametersByModelID: [:], expectedRevisions: saved.revisions, store: store
        ), "Save must still reject protocol-incompatible reasoning locally")
        XCTAssertEqual(store.endpoints().map(\.id), ["responses", "anthropic"],
                       "Invalid edits must not replace the persisted model catalog")
    }

    @MainActor
    func testStaleModelCatalogWindowCannotOverwriteAnotherWindowsEditOrMembership() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let initial = [MacRemoteEndpointConfiguration(
            id: "shared", name: "Original", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "model-a", modelName: "model-a", executionCapabilities: .init())]
        )]
        let first = try MacRemoteModelDraftCommit.save(
            initial, parametersByModelID: [:], expectedRevisions: [:], store: store
        )

        var secondWindow = first.endpoints
        secondWindow[0].name = "Changed by second window"
        secondWindow[0].models.append(.init(id: "model-b", modelName: "model-b", executionCapabilities: .init()))
        let committed = try MacRemoteModelDraftCommit.save(
            secondWindow, parametersByModelID: [:], expectedRevisions: first.revisions, store: store
        )

        var staleFirstWindow = first.endpoints
        staleFirstWindow[0].name = "Stale first window"
        XCTAssertThrowsError(try MacRemoteModelDraftCommit.save(
            staleFirstWindow, parametersByModelID: [:], expectedRevisions: first.revisions, store: store
        )) { error in
            XCTAssertTrue(MacRemoteModelSaveFailure.isCatalogConflict(error))
        }
        let current = try store.endpointSnapshot()
        XCTAssertEqual(current.endpoints, committed.endpoints)
        XCTAssertEqual(current.revisions, committed.revisions)
    }

    @MainActor
    func testStaleModelCatalogWindowCannotOverwriteParameterRevision() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let initial = [MacRemoteEndpointConfiguration(
            id: "shared", name: "Original", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "model-a", modelName: "model-a", executionCapabilities: .init())]
        )]
        let snapshot = try MacRemoteModelDraftCommit.save(
            initial, parametersByModelID: [:], expectedRevisions: [:], store: store
        )
        let parameterStore = ModelParametersStore(applicationPort: port)
        let parameters = try parameterStore.parameterSnapshot(for: "model-a")
        var changed = parameters.parameters
        changed.temperature = 1.2
        _ = try parameterStore.saveParameters(changed, snapshot: parameters)

        var stale = snapshot.endpoints
        stale[0].name = "Stale metadata"
        XCTAssertThrowsError(try MacRemoteModelDraftCommit.save(
            stale, parametersByModelID: [:], expectedRevisions: snapshot.revisions, store: store
        )) { error in
            XCTAssertTrue(MacRemoteModelSaveFailure.isCatalogConflict(error))
        }
        XCTAssertEqual(try parameterStore.parameterSnapshot(for: "model-a").parameters.temperature, 1.2)
        XCTAssertEqual(try store.endpointSnapshot().endpoints.first?.name, "Original")
    }

    @MainActor
    func testMacMetadataEditPreservesMobileToneAndUnknownSettings() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let repository = MacCanonicalConversationRepository(store: port)
        var conversation = MacConversation(id: "fixture", title: "Fixture", preview: "", updatedAt: .now, isPinned: false)
        _ = try port.createConversation(.init(id: conversation.id, title: conversation.title,
            settingsJSON: #"{"promptResponseStyle":"professional","mobileExtension":{"key":"value"},"contextLimitOverride":4096}"#,
            createdAtMs: 1, updatedAtMs: 1))
        conversation = try XCTUnwrap(repository.load(conversationID: conversation.id)?.conversation)
        conversation.contextLimitOverride = nil
        conversation.isPinned = true
        try repository.saveMetadata(conversation: conversation, knowledge: nil)
        let json = try XCTUnwrap(port.loadConversationSettings(conversation.id))
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(fields["promptResponseStyle"] as? String, "professional")
        XCTAssertEqual((fields["mobileExtension"] as? [String: String])?["key"], "value")
        XCTAssertEqual(fields["isPinned"] as? Bool, true)
        XCTAssertNil(fields["contextLimitOverride"], "Clearing an owned optional field must still work")
    }

    @MainActor
    func testInvalidPDFShowsAnErrorInsteadOfABlankPDFView() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("invalid.pdf")
        try Data("not a PDF".utf8).write(to: url)
        let view = MacNativeDocumentPreview.makePreview(for: url)
        XCTAssertFalse(view is PDFView)
        XCTAssertEqual(view.accessibilityIdentifier(), "mac-document-preview-error")
    }
    @MainActor
    func testRemoteParameterEditorsRejectStaleDraftAndAdvanceOwnReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: port)
        try configuration.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "fixture", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "model", modelName: "fixture", executionCapabilities: .init())]
        )])
        let store = ModelParametersStore(applicationPort: port)
        let originalSource = try port.modelProfile("model")?.sourceJSON
        let originalSelected = try port.modelProfile("model")?.selected
        let first = try store.parameterSnapshot(for: "model")
        let stale = try store.parameterSnapshot(for: "model")
        var params = first.parameters
        params.temperature = 0.123456
        let receipt = try store.saveParameters(params, snapshot: first)
        XCTAssertGreaterThan(receipt.revision, first.revision)
        XCTAssertThrowsError(try store.saveParameters(stale.parameters, snapshot: stale))
        XCTAssertEqual(try store.parameterSnapshot(for: "model").parameters.temperature, 0.123456)
        params.maxOutputTokens = 64_000
        let second = try store.saveParameters(params, snapshot: receipt)
        XCTAssertGreaterThan(second.revision, receipt.revision)
        XCTAssertEqual(try store.parameterSnapshot(for: "model").parameters.maxOutputTokens, 64_000)
        XCTAssertEqual(try port.modelProfile("model")?.sourceJSON, originalSource)
        XCTAssertEqual(try port.modelProfile("model")?.selected, originalSelected)
    }
    func testNumericDraftParsingDoesNotSubstituteOldValues() {
        for text in ["", "-", "abc", "NaN", "inf", "1.2.3"] {
            XCTAssertNil(ModelParameterNumberParser.decimal(text), text)
        }
        XCTAssertEqual(ModelParameterNumberParser.decimal("0"), 0)
        XCTAssertEqual(ModelParameterNumberParser.decimal("0.123456"), 0.123456)
        XCTAssertEqual(ModelParameterNumberParser.decimal("0,125", locale: Locale(identifier: "de_DE")), 0.125)
        XCTAssertNil(ModelParameterNumberParser.integer("64.5"))
        XCTAssertNil(ModelParameterNumberParser.integer("99999999999999999999999"))
        XCTAssertEqual(ModelParameterNumberParser.integer("64000"), 64_000)
    }

    @MainActor
    func testResponsesEffortDoesNotRequireAnthropicMinimumOutputBudget() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let reasoning = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low"], outputLimit: 512)
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "fixture", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "model", modelName: "fixture",
                                                  executionCapabilities: .init(reasoning: true), reasoning: reasoning)],
            wireProtocol: .responses
        )])
        XCTAssertEqual(store.load(model: "model")?.reasoning?.outputLimit, 512)
    }

    func testRelativeDocumentLinksResolveAgainstConversationWorkspace() throws {
        let workspace = URL(fileURLWithPath: "/tmp/session", isDirectory: true)
        let relative = try XCTUnwrap(URL(string: "reports/main.pdf"))
        XCTAssertNil(MacCodeReferenceParser.parse(relative))
        let reference = try XCTUnwrap(MacCodeReferenceParser.parse(relative, relativeTo: workspace))
        XCTAssertEqual(reference.url.path, "/tmp/session/reports/main.pdf")
        XCTAssertTrue(MacFilePreviewPolicy.usesDocumentPreview(reference.url))
        XCTAssertNil(MacCodeReferenceParser.parse(try XCTUnwrap(URL(string: "https://example.com/main.pdf")), relativeTo: workspace))
        XCTAssertNil(MacCodeReferenceParser.parse(try XCTUnwrap(URL(string: "//example.com/main.pdf")), relativeTo: workspace))
    }

    func testFileLinksDoNotDecodeLiteralPercentSequencesTwice() throws {
        let file = URL(fileURLWithPath: "/tmp/report%20final.pdf")
        XCTAssertEqual(try XCTUnwrap(MacCodeReferenceParser.parse(file)).url.path, file.path)
    }
    func testRemovingSessionOverridesRestoresDefaultsWithoutRetainingPreviousValues() {
        var defaults = ModelParameters.webAlignedCloudDefaults
        defaults.temperature = 1.1
        defaults.maxOutputTokens = 64_000
        var override = MacConversationModelOverride(temperature: 0, maxOutputTokens: 1024)
        XCTAssertEqual(override.applying(to: defaults).temperature, 0)
        override = .init()
        XCTAssertEqual(override.applying(to: defaults).temperature, 1.1)
        XCTAssertEqual(override.applying(to: defaults).maxOutputTokens, 64_000)
    }
    func testEditorTokenBoundsPreserveLargeExplicitValues() {
        XCTAssertTrue(ModelParameterLimits.outputTokens.contains(64_000))
        XCTAssertTrue(ModelParameterLimits.outputTokens.contains(1_000_000))
        XCTAssertFalse(ModelParameterLimits.outputTokens.contains(0))
        XCTAssertTrue(ModelParameterLimits.remoteContextTokens.contains(4_000_000))
        XCTAssertFalse(ModelParameterLimits.remoteContextTokens.contains(4_000_001))
    }
    @MainActor
    func testRealPDFOpensInPDFKitInsteadOfTheCodeDecoder() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("preview.pdf")
        let page = NSView(frame: NSRect(x: 0, y: 0, width: 400, height: 300))
        let label = NSTextField(labelWithString: "Nola PDF preview")
        label.frame = NSRect(x: 30, y: 140, width: 300, height: 30)
        page.addSubview(label)
        try page.dataWithPDF(inside: page.bounds).write(to: url)
        let preview = try XCTUnwrap(MacNativeDocumentPreview.makePreview(for: url) as? PDFView)
        XCTAssertEqual(preview.document?.pageCount, 1)
        XCTAssertTrue(preview.document?.string?.contains("Nola PDF preview") == true)
        XCTAssertTrue(preview.autoScales)
    }
    func testSparseSessionParametersWinIncludingZeroAndUnspecifiedInherits() throws {
        var defaults = ModelParameters.webAlignedCloudDefaults
        defaults.temperature = 1.2
        defaults.topP = 0.8
        let override = MacConversationModelOverride(temperature: 0, maxOutputTokens: 2048)
        let resolved = override.applying(to: defaults)
        XCTAssertEqual(resolved.temperature, 0)
        XCTAssertEqual(resolved.topP, 0.8)
        XCTAssertEqual(resolved.maxOutputTokens, 2048)
        XCTAssertEqual(defaults.temperature, 1.2, "Must not modify global defaults")
        XCTAssertEqual(try JSONDecoder().decode(MacConversationModelOverride.self, from: JSONEncoder().encode(override)), override)
    }

    func testCommonDocumentsAreNotSentToTextDecoder() {
        for name in ["paper.pdf", "photo.png", "photo.jpg", "slides.pptx", "sheet.xlsx", "report.docx"] {
            XCTAssertTrue(MacFilePreviewPolicy.usesDocumentPreview(URL(fileURLWithPath: "/tmp/\(name)")), name)
        }
        for name in ["main.swift", "README.md", "notes.txt", "main.py"] {
            XCTAssertFalse(MacFilePreviewPolicy.usesDocumentPreview(URL(fileURLWithPath: "/tmp/\(name)")), name)
        }
    }

    @MainActor
    func testProtocolAndGlobalModelParametersSurviveReload() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.25
        parameters.maxOutputTokens = 4096
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "fixture", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "model", modelName: "gpt-5.5",
                                                  executionCapabilities: .init(reasoning: true),
                                                  reasoning: .init(control: .effort, efforts: ["high"]))], wireProtocol: .responses
        )], parametersByModelID: ["model": parameters])
        let reloaded = try XCTUnwrap(store.load(model: "model"))
        XCTAssertEqual(reloaded.wireProtocol, .responses)
        XCTAssertEqual(reloaded.reasoning?.options, ["high"])
        XCTAssertEqual(reloaded.parameters.temperature, 0.25)
        XCTAssertEqual(reloaded.parameters.maxOutputTokens, 4096)
        XCTAssertEqual(store.endpoints().first?.wireProtocol, .responses)
        XCTAssertEqual(store.routePlan(preferredModel: "model")?.primary.wireProtocol, .responses)
    }
}
