// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS conversation configuration, drafts, knowledge persistence, memory overrides, presentation overlays, reasoning admission, context snapshots/tokens, and edited resend suite.

import AppKit
import Foundation
import PDFKit
import SQLite3
import XCTest
@testable import Nola

@MainActor
final class MacConversationDraftAndTokenSuiteTests: XCTestCase {
    private func save(_ record: MacConversationDraftRecord, to repository: MacConversationDraftRepository) async -> Bool {
        await withCheckedContinuation { continuation in repository.save(record) { continuation.resume(returning: $0) } }
    }

    func testDraftPersistenceScopeIsolationConflictDeleteAndOutboxExclusion() async throws {
        let invalidRecord = MacConversationDraftRecord(conversationID: "a", text: "", attachments: [
            .init(url: URL(string: "https://example.com/private")!, text: "", method: "", byteCount: 0, ready: false)
        ])
        XCTAssertThrowsError(try invalidRecord.validate())

        let root = FileManager.default.temporaryDirectory.appendingPathComponent("draft-suite-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("one.sqlite"))
        let otherPort = try MacApplicationPort(databaseURL: root.appendingPathComponent("two.sqlite"))

        // Same store restores draft and does not cross database scopes
        let repository = MacConversationDraftRepository(store: port)
        let record = MacConversationDraftRecord(conversationID: "conversation-a", text: "未提交的草稿", attachments: [])
        let savedRecord = await save(record, to: repository)
        XCTAssertTrue(savedRecord)
        let restored = try await MacConversationDraftRepository(store: port).load("conversation-a")
        XCTAssertEqual(restored?.text, record.text)
        let crossScope = try await MacConversationDraftRepository(store: otherPort).load("conversation-a")
        XCTAssertNil(crossScope)

        // Stale window cannot overwrite draft and delete is ordered
        let first = MacConversationDraftRepository(store: port), second = MacConversationDraftRepository(store: port)
        _ = try await first.load("a"); _ = try await second.load("a")
        let savedFirst = await save(.init(conversationID: "a", text: "first", attachments: []), to: first)
        let conflicted = await save(.init(conversationID: "a", text: "stale", attachments: []), to: second)
        XCTAssertTrue(savedFirst); XCTAssertFalse(conflicted)
        let current = try await first.load("a")
        XCTAssertEqual(current?.text, "first")
        let deleted: Bool = await withCheckedContinuation { continuation in first.delete("a") { continuation.resume(returning: $0) } }
        XCTAssertTrue(deleted)
        do { _ = try await first.load("a"); XCTFail("Deleted owners must not hydrate") } catch { }
        let resurrectedAfterDelete = await save(.init(conversationID: "a", text: "stale after delete", attachments: []), to: second)
        XCTAssertFalse(resurrectedAfterDelete)

        // Draft save and delete never enter canonical sync outbox
        let before = try port.pendingSyncChanges()
        let browser = MacConversationBrowserSnapshot(target: "managed", sessionID: "display-only-session",
            pages: [], currentTitle: "Local page", currentURL: "https://example.com/local-page",
            isConnected: false, updatedAt: .now, previewURL: nil)
        let savedPrivate = await save(.init(conversationID: "private", text: "private unsent content", attachments: [], browserSnapshot: browser), to: repository)
        XCTAssertTrue(savedPrivate)
        let reopened = try await MacConversationDraftRepository(store: port).load("private")
        XCTAssertEqual(reopened?.browserSnapshot, browser)
        XCTAssertEqual(try port.pendingSyncChanges(), before)
        XCTAssertNil(try port.loadConversationMetadata("private"))
        XCTAssertNil(try port.productSetting(key: "macos.conversation-draft." + Data("private".utf8).base64EncodedString()))
        let privateDeleted: Bool = await withCheckedContinuation { continuation in repository.delete("private") { continuation.resume(returning: $0) } }
        XCTAssertTrue(privateDeleted)
        XCTAssertEqual(try port.pendingSyncChanges(), before)
        let freshRepository = MacConversationDraftRepository(store: port)
        let lateResurrected = await save(.init(conversationID: "private", text: "late", attachments: []), to: freshRepository)
        XCTAssertFalse(lateResurrected)
    }

    func testConversationReasoningAdmissionAndContextTokenEstimation() throws {
        XCTAssertEqual(MacContextTokenEstimator.estimateText(String(repeating: "你", count: 100)), 100)
        XCTAssertEqual(MacContextTokenEstimator.estimateText(String(repeating: "a", count: 400)), 100)
        let messageOnly = MacContextTokenEstimator.estimateComponents(messageTexts: [String(repeating: "你", count: 80)])
        let complete = MacContextTokenEstimator.estimateComponents(
            messageTexts: [String(repeating: "你", count: 80)],
            draft: String(repeating: "a", count: 40),
            promptTemplate: String(repeating: "b", count: 40),
            userPreferences: String(repeating: "我", count: 10)
        )
        XCTAssertEqual(messageOnly, 87)
        XCTAssertEqual(complete, 121)
        XCTAssertGreaterThan(complete, messageOnly)

        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("admission.db"))
        let declaration = MacReasoningConfiguration(control: .budget, budgets: [1024, 4096, 8192], outputLimit: 4096)
        XCTAssertEqual(try MacConversationReasoningAdmission.validate(
            declaration: declaration, protocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, preference: "1024", port: port),
            .object(["kind": .string("budget"), "tokens": .number(1024)]))
        for preference in ["4096", "8192"] {
            XCTAssertThrowsError(try MacConversationReasoningAdmission.validate(
                declaration: declaration, protocol: .anthropic, outputLimit: 4096,
                supportsReasoning: true, preference: preference, port: port))
        }
        for invalidDeclaration in [
            MacReasoningConfiguration(control: .budget, budgets: [1024, 8192, 4096]),
            MacReasoningConfiguration(control: .budget, budgets: [1024, 4096]),
        ] {
            XCTAssertThrowsError(try MacConversationReasoningAdmission.validate(
                declaration: invalidDeclaration, protocol: .anthropic, outputLimit: 4096,
                supportsReasoning: true, preference: "1024", port: port))
        }
        XCTAssertThrowsError(try MacConversationReasoningAdmission.validate(
            declaration: .init(control: .budget), protocol: .anthropic, outputLimit: 4096,
            supportsReasoning: false, preference: "1024", port: port))
    }
}

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
        invalid[0].models[0].reasoning = .init(control: .budget)
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

nonisolated private func unexpectedPersistenceHostCall(_ operation: String) -> NSError {
    XCTFail("Persistence-only test unexpectedly invoked Host operation: \(operation)")
    return NSError(domain: "Nola.PersistenceTest.Host", code: 1)
}

actor PersistenceTestLocalRuntime: MacLocalAgentRuntime {
    private var permissionRequest: MacRemotePermissionRequest?
    func setPermissionRequest(_ request: MacRemotePermissionRequest?) {
        permissionRequest = request
    }
    func activePermissionRequest(conversationID: String, requestID: String) -> MacRemotePermissionRequest? {
        guard permissionRequest?.sessionID == conversationID,
              permissionRequest?.id == requestID else { return nil }
        return permissionRequest
    }
    func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    try request.submissionTicket?.claimDispatch()
                } catch {
                    continuation.finish(throwing: error)
                    return
                }
                request.submissionTicket?.markAccepted()
                await request.submissionAccepted?()
                continuation.finish(throwing: unexpectedPersistenceHostCall("local.run"))
            }
        }
    }
    func cancel(conversationID: String) {}
    func resolvePermission(conversationID: String, request: MacRemotePermissionRequest,
                           allow: Bool, scope: MacPermissionGrantScope) throws {
        throw unexpectedPersistenceHostCall("local.resolvePermission")
    }
    func listSessions() async throws -> [MacLocalAgentSessionSummary] {
        throw unexpectedPersistenceHostCall("local.listSessions")
    }
    func deleteSession(id: String) async throws {
        throw unexpectedPersistenceHostCall("local.deleteSession")
    }
}

@MainActor
final class PersistenceTestRemoteHost: MacRemoteAgentRunning {
    func permissionConfiguration(identity: MacRemoteSubmissionIdentity, target: MacRemoteSubmissionTarget,
                                 expectedRevision: Int64?, profile: MacPermissionMode?, minimumPolicyRevision: Int64?) async throws -> MacRemotePermissionConfiguration {
        throw unexpectedPersistenceHostCall("remote.permissionConfiguration")
    }
    nonisolated func captureSubmissionTarget() throws -> MacRemoteSubmissionTarget {
        throw unexpectedPersistenceHostCall("remote.captureSubmissionTarget")
    }
    func isConfigured() async -> Bool { false }
    func testSavedConnection() async throws -> MacRemoteHostConnectionReport {
        throw unexpectedPersistenceHostCall("remote.testConnection")
    }
    func requestWebManagementURL(target: String) async throws -> URL {
        throw unexpectedPersistenceHostCall("remote.managementURL")
    }
    func fetchExtensions() async throws -> [MacExtensionItem] {
        throw unexpectedPersistenceHostCall("remote.extensions")
    }
    func fetchExecutionBackends() async throws -> [MacRemoteExecutionBackend] {
        throw unexpectedPersistenceHostCall("remote.backends")
    }
    func fetchRemoteModelCatalog() async throws -> MacRemoteModelCatalogSnapshot {
        throw unexpectedPersistenceHostCall("remote.models")
    }
    func run(conversationID: String, submissionID: String, userMessageID: String,
             assistantMessageID: String, prompt: String, contextComponents: [MacCoreContextComponent],
             modelID: String, modelProviderID: String?, modelIntent: MacRemoteModelIntent?, memoryEnabled: Bool, executionBackend: String,
             externalSessionID: String?, submission: MacRemoteSubmissionControl) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: unexpectedPersistenceHostCall("remote.run")) }
    }
    func resolvePermission(request: MacRemotePermissionRequest, allow: Bool,
                           scope: MacPermissionGrantScope) async throws {
        throw unexpectedPersistenceHostCall("remote.resolvePermission")
    }
    func saveConversationSelection(conversationID: String, title: String, profile: MacRemoteModelProfile?,
        allowCreate: Bool, target: MacRemoteSubmissionTarget) async throws {}
    func sendTaskGuidance(conversationID: String, parentRunID: String, taskID: String,
                          message: String, target: MacRemoteSubmissionTarget) async throws -> AppleTaskGuidanceReceipt {
        throw unexpectedPersistenceHostCall("remote.taskGuidance")
    }
    func requestTaskCancellation(conversationID: String, parentRunID: String,
                                 taskID: String, target: MacRemoteSubmissionTarget) async throws -> AppleTaskCancellationReceipt {
        throw unexpectedPersistenceHostCall("remote.cancelTask")
    }
    func cancelRun(identity: MacRemoteSubmissionIdentity) async throws {}
    func taskEvents(conversationID: String,
                    afterCursor: Int64) async -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: unexpectedPersistenceHostCall("remote.taskEvents")) }
    }
}

final class MacConversationKnowledgePersistenceTests: XCTestCase {
    @MainActor
    func testRecallFailureIsNotAnEmptyResultAndClearsForNextQuery() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("recall-error-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try repository.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(id: "recall-error", title: "Recall", preview: "", updatedAt: .now, isPinned: false),
                messages: [MacChatMessage(role: .user, text: "Original evidence", createdAt: .now)]
            )
        ])
        let model = MacWorkspaceModel(
            initialConversationID: "recall-error", canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = "recall-error"
        model.conversationRecallQuery = String(repeating: "x", count: 501)
        await waitForRecallSearch(model)
        XCTAssertTrue(model.conversationRecallFailed)
        XCTAssertTrue(model.conversationRecallResults.isEmpty)
        model.retryConversationRecallSearch()
        XCTAssertFalse(model.conversationRecallFailed)
        await waitForRecallSearch(model)
        XCTAssertTrue(model.conversationRecallFailed)
        model.conversationRecallQuery = "Original evidence"
        await waitForRecallSearch(model)
        XCTAssertFalse(model.conversationRecallFailed)
        XCTAssertEqual(model.conversationRecallResults.count, 1)
    }

    @MainActor
    func testConversationKnowledgeEndToEndSurvivesSwitchReloadRestartAndRepeatedCompaction() async throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-conversation-knowledge-e2e-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        let applicationPort = try MacApplicationPort(
            databaseURL: baseDirectory.appendingPathComponent("nola.sqlite")
        )
        let store = MacCanonicalConversationRepository(store: applicationPort)
        let firstConversationID = "knowledge-first-\(UUID().uuidString)"
        try store.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: firstConversationID,
                    title: "Atlas",
                    preview: "",
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [
                    MacChatMessage(
                        role: .user,
                        text: "The original Atlas decision requires blue-green deployment in Shanghai.",
                        createdAt: Date(timeIntervalSince1970: 100)
                    )
                ]
            )
        ])
        let model = MacWorkspaceModel(
            initialConversationID: firstConversationID,
            canonicalRepository: store,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = firstConversationID
        model.selectedKnowledgePointsText = "- Atlas uses blue-green deployment"
        model.selectedWikiText = "# Atlas\n\nPrimary region: Shanghai"
        XCTAssertTrue(model.selectedKnowledgeIsSaving)

        model.newConversation()
        let secondConversationID = try XCTUnwrap(model.selectedConversationID)
        model.selectedKnowledgePointsText = "- Borealis is private"
        model.selectedWikiText = "# Borealis\n\nVisibility: private"

        for index in 1...3 {
            try commitCanonicalTestContextCheckpoint(
                store: applicationPort,
                id: "archive-\(index)",
                conversationID: firstConversationID,
                summary: index == 3
                    ? "The retained rollback owner is Mina."
                    : "Earlier compacted continuation \(index).",
                estimatedTokens: 16,
                payloadDigest: String(repeating: "a", count: 64),
                createdAtMs: Int64(200 + index) * 1_000
            )
        }

        // Selection only exposes the second owner's pending state. Flush and
        // await each original owner's exact metadata/configuration receipt.
        try await model.waitForModelConfigurationCommit(conversationID: firstConversationID)
        try await model.waitForModelConfigurationCommit(conversationID: secondConversationID)
        XCTAssertFalse(model.selectedKnowledgeIsSaving)
        XCTAssertFalse(model.selectedKnowledgeSaveFailed)
        for (id, markdown) in [(firstConversationID, "- Atlas uses blue-green deployment"),
                               (secondConversationID, "- Borealis is private")] {
            let persisted = try XCTUnwrap(applicationPort.loadConversation(id))
            let settings = try XCTUnwrap(persisted.settingsJSON)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(settings.utf8)) as? [String: Any])
            let knowledge = try XCTUnwrap(object["knowledge"] as? [String: Any])
            XCTAssertEqual(Set(knowledge.keys), Set(["keyPointsMarkdown", "wikiMarkdown", "updatedAt"]),
                "Writes own only Markdown; Core derives the committed knowledge timestamp")
            XCTAssertNotNil(knowledge["updatedAt"] as? NSNumber)
            XCTAssertEqual(knowledge["keyPointsMarkdown"] as? String, markdown)
        }

        model.reloadPersistentData()
        model.selectedConversationID = firstConversationID
        XCTAssertEqual(model.selectedKnowledgePointsText, "- Atlas uses blue-green deployment")
        XCTAssertTrue(model.selectedWikiText.contains("Shanghai"))

        let restarted = MacWorkspaceModel(
            initialConversationID: secondConversationID,
            canonicalRepository: store,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        restarted.selectedConversationID = secondConversationID
        await restarted.loadSelectedConversationIfNeeded()
        XCTAssertEqual(restarted.selectedKnowledgePointsText, "- Borealis is private")
        XCTAssertTrue(restarted.selectedWikiText.contains("Visibility: private"))

        restarted.selectedConversationID = firstConversationID
        await restarted.loadSelectedConversationIfNeeded()
        XCTAssertEqual(restarted.selectedKnowledgePointsText, "- Atlas uses blue-green deployment")
        XCTAssertTrue(restarted.selectedWikiText.contains("Primary region: Shanghai"))
        await restarted.refreshContextArchiveCount()
        XCTAssertEqual(restarted.selectedContextArchiveCount, 3)
        XCTAssertEqual(
            restarted.conversationRecallResults,
            []
        )

        restarted.conversationRecallQuery = "blue-green"
        await waitForRecallSearch(restarted)
        XCTAssertEqual(restarted.conversationRecallResults.first?.source, .originalMessage)
        restarted.conversationRecallQuery = "rollback owner Mina"
        await waitForRecallSearch(restarted)
        XCTAssertTrue(restarted.conversationRecallResults.isEmpty)

        // Switching while a debounced request is pending must not publish
        // another conversation's originals or checkpoint summaries.
        restarted.conversationRecallQuery = "blue-green"
        restarted.selectedConversationID = secondConversationID
        await waitForRecallSearch(restarted)
        XCTAssertTrue(restarted.conversationRecallResults.isEmpty)
    }

    @MainActor
    private func waitForRecallSearch(
        _ model: MacWorkspaceModel,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async {
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while model.isConversationRecallSearching, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertFalse(model.isConversationRecallSearching, "Recall search timed out", file: file, line: line)
    }

    @MainActor
    func testKnowledgeWikiAndOriginalRecallSurviveRepeatedCompactionArchives() async throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-conversation-knowledge-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        let applicationPort = try MacApplicationPort(
            databaseURL: baseDirectory.appendingPathComponent("nola.sqlite")
        )
        let store = MacCanonicalConversationRepository(store: applicationPort)
        let conversationID = "conversation-\(UUID().uuidString)"
        let conversation = MacConversation(
            id: conversationID,
            title: "Architecture review",
            preview: "",
            updatedAt: .now,
            isPinned: false
        )
        let originalMessage = MacChatMessage(
            role: .user,
            text: "The deployment decision is to use blue-green releases in Shanghai.",
            createdAt: Date(timeIntervalSince1970: 100)
        )
        let knowledge = MacConversationKnowledge(
            keyPointsMarkdown: "- Blue-green deployment is required",
            wikiMarkdown: "# Deployment\n\nPrimary region: Shanghai",
            updatedAt: Date(timeIntervalSince1970: 200)
        )
        try store.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: conversation,
                messages: [originalMessage],
                knowledge: knowledge
            )
        ])
        for index in 1...3 {
            try commitCanonicalTestContextCheckpoint(
                store: applicationPort,
                id: "archive-\(index)",
                conversationID: conversationID,
                summary: index == 3
                    ? "The unresolved rollback owner must be assigned before release."
                    : "Earlier continuation summary \(index).",
                estimatedTokens: 16,
                payloadDigest: String(repeating: "a", count: 64),
                createdAtMs: Int64(200 + index) * 1_000
            )
        }

        let restored = try XCTUnwrap(store.loadAll().first)
        XCTAssertEqual(restored.knowledge, knowledge)
        XCTAssertEqual(restored.messages.count, 1)
        let restoredMessage = try XCTUnwrap(restored.messages.first)
        XCTAssertEqual(restoredMessage.id, originalMessage.id)
        XCTAssertEqual(restoredMessage.role, originalMessage.role)
        XCTAssertEqual(restoredMessage.text, originalMessage.text)
        XCTAssertEqual(restoredMessage.createdAt, originalMessage.createdAt)
        XCTAssertEqual(
            restoredMessage.canonicalItemIDs,
            Set([originalMessage.id.uuidString, "\(originalMessage.id.uuidString):text"])
        )

        let originalResults = try await store.searchConversationAsync(
            conversationID: conversationID,
            query: "blue-green"
        )
        XCTAssertEqual(originalResults.first?.source, .originalMessage)
        XCTAssertTrue(originalResults.first?.content.contains("Shanghai") == true)
        XCTAssertEqual(originalResults.first?.messageID, originalMessage.id.uuidString)
        XCTAssertEqual(originalResults.first?.partID, "\(originalMessage.id.uuidString):text")
        XCTAssertEqual(originalResults.first?.messageOrdinal, 1)
        XCTAssertEqual(originalResults.first?.startCharacter, 0)
        XCTAssertLessThanOrEqual(originalResults.first?.snippet.count ?? Int.max, 1_600)

        let summaryResults = try await store.searchConversationAsync(
            conversationID: conversationID,
            query: "rollback owner"
        )
        XCTAssertTrue(summaryResults.isEmpty)
    }

    @MainActor
    func testNativeRecallBridgeHonorsCrossConversationScopeAndExactSource() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-cross-recall-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        // Keep the current conversation lexically disjoint: token-based recall
        // may legitimately match “之前” in “我之前问了什么”. This fixture tests
        // scope isolation, not exact-phrase search semantics.
        try repository.restoreBackupSnapshot([("current", "你好"),
            ("earlier", "之前约定 search current：Tokyo launch is October 14.")].map { id, text in
            MacStoredConversation(conversation: MacConversation(id: id, title: id, preview: "",
                updatedAt: .now, isPinned: false),
                messages: [MacChatMessage(role: .user, text: text, createdAt: .now)])
        })
        func recall(_ arguments: [String: Any], owner: String = "current") throws -> [String: Any] {
            let data = MacConversationRecallBridge.responseData(requestID: UUID().uuidString,
                conversationID: owner, arguments: arguments, store: store)
            return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        }
        func hits(_ arguments: [String: Any]) throws -> [[String: Any]] {
            let envelope = try recall(arguments)
            XCTAssertEqual(envelope["is_error"] as? Bool, false)
            let output = try XCTUnwrap(envelope["output"] as? [String: Any])
            return try XCTUnwrap(output["hits"] as? [[String: Any]])
        }
        XCTAssertTrue(try hits(["query": "之前约定"]).isEmpty)
        let history = try hits(["query": "之前约定", "scope": "local_user_history"])
        XCTAssertEqual(history.count, 1)
        let source = try XCTUnwrap(history.first?["source"] as? [String: Any])
        XCTAssertEqual(source["conversation_id"] as? String, "earlier")
        XCTAssertEqual(try hits(["query": "之前约定", "scope": "conversations",
            "conversation_ids": ["earlier"]]).count, 1)
        XCTAssertTrue(try hits(["query": "之前约定", "scope": "conversations",
            "conversation_ids": ["current"]]).isEmpty)
        var read: [String: Any] = ["conversation_id": "earlier",
            "message_id": try XCTUnwrap(source["message_id"]),
            "part_id": try XCTUnwrap(source["part_id"]),
            "expected_conversation_revision": try XCTUnwrap(source["conversation_revision"])]
        let original = try recall(read)
        XCTAssertEqual(original["is_error"] as? Bool, false)
        XCTAssertEqual((original["output"] as? [String: Any])?["text"] as? String,
            "之前约定 search current：Tokyo launch is October 14.")
        read["conversation_id"] = "current"
        XCTAssertEqual(try recall(read)["is_error"] as? Bool, true,
            "Exact source identities must never resolve against another conversation.")
        XCTAssertEqual(try recall(["query": "之前约定"], owner: "unknown")["is_error"] as? Bool, true)
        let invalidArguments: [[String: Any]] = [
            ["query": "之前约定", "scope": "conversations"],
            ["query": "之前约定", "scope": "current_conversation", "conversation_ids": ["earlier"]],
            ["message_id": "incomplete"], ["query": "之前约定", "requester_conversation_id": "earlier"],
        ]
        for invalid in invalidArguments {
            XCTAssertEqual(try recall(invalid)["is_error"] as? Bool, true)
        }
    }

    @MainActor
    func testNativeRecallBridgeReturnsOnlyCanonicalOriginalEvidence() throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-recall-bridge-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        try FileManager.default.createDirectory(at: baseDirectory, withIntermediateDirectories: true)
        let store = try MacApplicationPort(
            databaseURL: baseDirectory.appendingPathComponent("nola.db")
        )
        let repository = MacCanonicalConversationRepository(store: store)
        let conversationID = "bridge-conversation"
        try repository.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Bridge",
                    preview: "",
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [
                    MacChatMessage(
                        role: .user,
                        text: "Keep the Atlas migration behind a feature flag.",
                        createdAt: .now
                    )
                ],
                knowledge: MacConversationKnowledge(
                    keyPointsMarkdown: "- Atlas requires a feature flag",
                    wikiMarkdown: "# Atlas\n\nOwner: Platform",
                    updatedAt: .now
                )
            )
        ])

        let data = MacConversationRecallBridge.responseData(
            requestID: "request-1",
            conversationID: conversationID,
            arguments: ["query": "Atlas migration"],
            store: store
        )
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(payload["type"] as? String, "device_tool_result")
        XCTAssertEqual(payload["is_error"] as? Bool, false)
        let output = try XCTUnwrap(payload["output"] as? [String: Any])
        XCTAssertEqual(output["kind"] as? String, "search")
        XCTAssertNil(output["knowledge_points"])
        XCTAssertNil(output["wiki"])
        let matches = try XCTUnwrap(output["hits"] as? [[String: Any]])
        XCTAssertEqual(matches.count, 1)
        XCTAssertEqual(matches.first?["snippet"] as? String, "Keep the Atlas migration behind a feature flag.")
        let source = try XCTUnwrap(matches.first?["source"] as? [String: Any])
        let readData = MacConversationRecallBridge.responseData(requestID: "read-1",
            conversationID: conversationID, arguments: [
                "message_id": try XCTUnwrap(source["message_id"]),
                "part_id": try XCTUnwrap(source["part_id"]),
                "expected_conversation_revision": try XCTUnwrap(source["conversation_revision"]),
                "limit_bytes": 8,
            ], store: store)
        let read = try XCTUnwrap(JSONSerialization.jsonObject(with: readData) as? [String: Any])
        XCTAssertEqual(read["is_error"] as? Bool, false)
        let original = try XCTUnwrap(read["output"] as? [String: Any])
        XCTAssertEqual(original["text"] as? String, "Keep the")
        XCTAssertEqual(original["next_offset_bytes"] as? Int, 8)
        XCTAssertEqual(original["end_of_content"] as? Bool, false)
        XCTAssertThrowsError(try MacConversationRecallBridge.canonicalQuery(
            conversationID: conversationID, arguments: ["message_id": "m", "part_id": "p", "query": "mixed"]))
        XCTAssertThrowsError(try MacConversationRecallBridge.canonicalQuery(
            conversationID: conversationID, arguments: ["query": "Atlas", "scope": "unknown"]))
        let scoped = try MacConversationRecallBridge.canonicalQuery(conversationID: conversationID,
            arguments: ["query": "Atlas", "scope": "local_user_history", "after_conversation_id": "cursor"])
        XCTAssertEqual((scoped["scope"] as? [String: String])?["kind"], "local_user_history")
        XCTAssertEqual(scoped["after_conversation_id"] as? String, "cursor")
        let historyData = MacConversationRecallBridge.responseData(requestID: "history-1",
            conversationID: conversationID,
            arguments: ["query": "Atlas migration", "scope": "local_user_history"], store: store)
        let history = try XCTUnwrap(JSONSerialization.jsonObject(with: historyData) as? [String: Any])
        XCTAssertEqual(history["is_error"] as? Bool, false)
        let historyOutput = try XCTUnwrap(history["output"] as? [String: Any])
        XCTAssertEqual((historyOutput["hits"] as? [[String: Any]])?.count, 1)
        let changedSourceData = MacConversationRecallBridge.responseData(requestID: "changed-source",
            conversationID: conversationID, arguments: [
                "message_id": try XCTUnwrap(source["message_id"]),
                "part_id": try XCTUnwrap(source["part_id"]),
                "expected_conversation_revision": Int64.max,
            ], store: store)
        let changedSource = try XCTUnwrap(JSONSerialization.jsonObject(with: changedSourceData) as? [String: Any])
        XCTAssertEqual(changedSource["is_error"] as? Bool, true)
    }
}

@discardableResult
func commitCanonicalTestContextCheckpoint(
    store: MacApplicationPort,
    id: String,
    conversationID: String,
    runID: String? = nil,
    policyVersion: String = "agent-core-v1",
    summary: String,
    estimatedTokens: Int64,
    measuredTokens: Int64? = nil,
    window: Int64 = 0,
    promptLimit: Int64 = 0,
    payloadDigest: String,
    createdAtMs: Int64
) throws -> MacCanonicalContextAggregate {
    let componentID = "\(id):summary"
    let checkpoint = MacCanonicalContextCheckpoint(
        id: id,
        conversationID: conversationID,
        runID: runID,
        modelProfileID: "test-model",
        policyVersion: policyVersion,
        coveredThroughMessageID: nil,
        coveredThroughSequence: nil,
        summaryPartID: componentID,
        retainedTailStartMessageID: nil,
        tokenUsage: MacCanonicalContextTokenUsage(
            estimated: estimatedTokens,
            measured: measuredTokens,
            window: window,
            promptLimit: promptLimit
        ),
        payloadDigest: payloadDigest,
        createdAtMs: createdAtMs
    )
    let component = MacCanonicalContextComponent(
        id: componentID,
        checkpointID: id,
        kind: "conversation_summary",
        sourceID: nil,
        sourceURI: nil,
        title: nil,
        mimeType: "text/plain",
        authorization: "not_required",
        observedAtMs: createdAtMs,
        maximumAgeMinutes: nil,
        freshness: "fresh",
        contentReference: MacCanonicalContextContentReference(
            type: "inline_preview",
            text: summary,
            truncated: false,
            artifactID: nil,
            blobID: nil
        ),
        contentDigest: payloadDigest,
        estimatedTokens: estimatedTokens,
        provenance: MacCanonicalContextProvenance(
            providerID: "test",
            collectionMethod: "compaction",
            sourceRevision: nil,
            disclosedToModel: true
        )
    )
    return try store.commitContextCheckpoint(
        MacCanonicalContextCommit(checkpoint: checkpoint, components: [component])
    )
}

@MainActor
final class MacConversationMemoryOverrideTests: XCTestCase {
    func testMemoryInheritanceUsesOnlyTheOwningStoreForEveryOverride() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let first = try MacApplicationPort(databaseURL: root.appendingPathComponent("first.db"))
        let second = try MacApplicationPort(databaseURL: root.appendingPathComponent("second.db"))
        MacAssistantPreferences.setUserPreferences("first owner preference", in: first)
        MacAssistantPreferences.setUserPreferences("second owner preference", in: second)
        XCTAssertEqual(MacCanonicalConversationRepository(store: first).userPreferences, "first owner preference")
        XCTAssertEqual(MacCanonicalConversationRepository(store: second).userPreferences, "second owner preference")
        let firstModel = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: first),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        let secondModel = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: second),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        defer {
            firstModel.presentationDidDetach()
            secondModel.presentationDidDetach()
        }
        for global in [false, true] {
            MacAssistantPreferences.setMemoryEnabled(global, in: first)
            MacAssistantPreferences.setMemoryEnabled(!global, in: second)
            for override: Bool? in [nil, true, false] {
                firstModel.memoryEnabledOverride = override
                secondModel.memoryEnabledOverride = override
                XCTAssertEqual(firstModel.memoryEnabled, override ?? global)
                XCTAssertEqual(secondModel.memoryEnabled, override ?? !global)
            }
        }
        firstModel.memoryEnabledOverride = nil
        MacAssistantPreferences.setMemoryEnabled(false, in: first)
        XCTAssertFalse(firstModel.memoryEnabled)
        MacAssistantPreferences.setMemoryEnabled(true, in: first)
        XCTAssertTrue(firstModel.memoryEnabled)
    }

    func testThreeStatesPersistWithoutChangingAnotherConversation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("memory.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try repository.restoreBackupSnapshot(["one", "two"].map { id in
            MacStoredConversation(conversation: MacConversation(id: id, title: id, preview: "",
                updatedAt: .now, isPinned: false, isPersisted: true), messages: [])
        })
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        defer { model.presentationDidDetach() }
        model.selectedConversationID = "one"
        XCTAssertNil(model.memoryEnabledOverride)
        for value: Bool? in [true, false, nil] {
            model.memoryEnabledOverride = value
            let deadline = Date().addingTimeInterval(3)
            while try repository.load(conversationID: "one")?.conversation.memoryEnabledOverride != value,
                  Date() < deadline {
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTAssertEqual(try repository.load(conversationID: "one")?.conversation.memoryEnabledOverride, value)
            XCTAssertNil(try repository.load(conversationID: "two")?.conversation.memoryEnabledOverride)
            model.selectedConversationID = "two"
            XCTAssertNil(model.memoryEnabledOverride)
            model.selectedConversationID = "one"
            XCTAssertEqual(model.memoryEnabledOverride, value)
        }
        let reopened = try MacApplicationPort(databaseURL: root.appendingPathComponent("memory.db"))
        XCTAssertNil(try MacCanonicalConversationRepository(store: reopened).load(conversationID: "one")?.conversation.memoryEnabledOverride)
    }

    func testChangingOverrideDoesNotMutateAlreadyConstructedRunRequest() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let repository = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        defer { model.presentationDidDetach() }
        model.memoryEnabledOverride = true
        let request = try MacLocalAgentRunRequest(conversationID: "frozen", prompt: "fixture",
            projectURL: root, attachmentURLs: [], memoryEnabled: model.memoryEnabled,
            appEnvironment: environment, applicationPort: port)
        model.memoryEnabledOverride = false
        XCTAssertFalse(model.memoryEnabled)
        XCTAssertTrue(request.memoryEnabled)
        model.memoryEnabledOverride = nil
        XCTAssertNil(model.memoryEnabledOverride)
        XCTAssertEqual(model.memoryEnabled, MacAssistantPreferences.memoryEnabled(in: port))
        XCTAssertTrue(request.memoryEnabled)
    }
}

final class MacConversationPresentationOverlayTests: XCTestCase {
    func testHistoryScopeKeepsPageMembershipWithoutDroppingOffPageLiveState() {
        let historical = MacChatMessage(role: .assistant, text: "history", createdAt: .distantPast)
        let live = MacChatMessage(role: .assistant, text: "live", createdAt: .distantPast)
        var overlay = MacConversationPresentationOverlay()
        XCTAssertTrue(overlay.mutateMessage(canonical: [historical, live], messageID: live.id,
            createAssistantIfMissing: false) { $0.text = "live update" })
        XCTAssertTrue(overlay.mutateMessage(canonical: [historical], messageID: historical.id,
            createAssistantIfMissing: false) { $0.text = "history correction" })
        let history = overlay.renderedMessages(canonical: [historical], scope: .canonicalPage)
        XCTAssertEqual(history.map(\.id), [historical.id])
        XCTAssertEqual(history.first?.text, "history correction")
        let latest = overlay.renderedMessages(canonical: [live], scope: .canonicalPage)
        XCTAssertEqual(latest.first?.text, "live update", "Hiding an off-page row must not discard its live state")
        XCTAssertEqual(Set(overlay.renderedMessages(canonical: [historical]).map(\.id)),
            Set([historical.id, live.id]))
    }

    func testPendingSubmissionDoesNotAppendToHistoryPage() {
        let old = MacChatMessage(role: .user, text: "history", createdAt: .distantPast)
        let user = MacChatMessage(role: .user, text: "new question", createdAt: .distantPast)
        let assistant = MacChatMessage(role: .assistant, text: "", createdAt: .distantPast)
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(.init(submissionID: "pending", userMessage: user, assistantMessage: assistant))
        XCTAssertEqual(overlay.renderedMessages(canonical: [old], scope: .canonicalPage), [old])
        XCTAssertEqual(overlay.renderedMessages(canonical: [], scope: .latest).map(\.id), [user.id, assistant.id])
    }

    func testLargeCanonicalRefreshUsesIdentityIndexAndPreservesOrder() {
        let messageCount = 10_000
        let canonical = (0..<messageCount).map { index in
            MacChatMessage(
                id: UUID(),
                role: .assistant,
                text: "message \(index)",
                createdAt: .distantPast
            )
        }
        let previous = canonical.reversed()

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: canonical,
            previous: Array(previous)
        )

        XCTAssertEqual(refreshed.map(\.id), canonical.map(\.id))
        XCTAssertEqual(refreshed.map(\.text), canonical.map(\.text))
    }

    func testCanonicalRefreshKeepsCompletedNodesButUpdatesTraceMetadata() {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        let node = MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "Existing answer")
        live.append([node])
        live.finish()
        let old = MacChatMessage(id: id, role: .assistant, text: node.text, createdAt: .now, presentation: live)
        var canonical = old
        var fresh = MacAssistantPresentation.processing()
        fresh.append([MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: node.text)])
        fresh.upsertTool(externalID: "tool", title: "Read", detail: "Completed", state: .completed)
        fresh.finish()
        canonical.presentation = fresh
        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(in: [canonical], previous: [old])
        XCTAssertEqual(refreshed.first?.presentation?.textNodes, live.textNodes)
        XCTAssertEqual(refreshed.first?.presentation?.traceItems, fresh.traceItems)
        canonical.text = "Authoritative correction"
        XCTAssertEqual(
            MacConversationPresentationOverlay.preservingCompletedText(in: [canonical], previous: [old]),
            [canonical]
        )
    }

    func testStaleCanonicalRefreshCannotReopenCompletedMarkdownPresentation() throws {
        let id = UUID()
        let node = MacAssistantTextNode(
            blockID: UUID(),
            kind: .markdownBlock,
            text: "## Stable result"
        )
        var completed = MacAssistantPresentation.processing()
        completed.append([node])
        completed.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: node.text,
            createdAt: .now,
            presentation: completed
        )

        var stale = MacAssistantPresentation.processing()
        stale.append([MacAssistantTextNode(
            blockID: UUID(),
            kind: .markdownBlock,
            text: node.text
        )])
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: node.text,
            createdAt: previous.createdAt,
            presentation: stale
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )
        let presentation = try XCTUnwrap(refreshed.first?.presentation)

        XCTAssertTrue(presentation.isComplete)
        XCTAssertEqual(presentation.textNodes, [node])
    }

    func testTerminalTextOnlyRefreshKeepsCompletedLivePresentationTree() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        let nodes = [
            MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "First sentence."),
            MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: " Second sentence."),
        ]
        live.append(nodes)
        live.upsertTool(externalID: "transient-tool", title: "Read", detail: "Completed", state: .completed)
        live.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: nodes.map(\.text).joined(),
            createdAt: .now,
            presentation: live
        )
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: previous.text,
            createdAt: previous.createdAt
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )

        XCTAssertEqual(try XCTUnwrap(refreshed.first?.presentation).textNodes, nodes)
        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).traceItems.isEmpty)
        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).isComplete)
    }

    func testCanonicalRefreshDoesNotPreserveTransportOnlyModelFallbackDisclosure() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        live.append([MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "Recovered answer")])
        live.upsertStatus(
            externalID: "model-route:event-2",
            title: "Model fallback",
            detail: "Failed model: Primary · first\nFalling back to: Backup · second",
            state: .completed
        )
        live.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: "Recovered answer",
            createdAt: .now,
            presentation: live
        )
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: previous.text,
            createdAt: previous.createdAt
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )

        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).traceItems.isEmpty)
    }

    func testSuccessfulCanonicalRefreshDropsTransportOnlyModelProtocolFailure() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        live.append([MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "Verified answer")])
        live.upsertStatus(
            externalID: "model-protocol:event-1",
            title: "Response failed",
            detail: "The invalid tool request was rejected before retrying.",
            state: .failed
        )
        live.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: "Verified answer",
            createdAt: .now,
            presentation: live
        )
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: previous.text,
            createdAt: previous.createdAt
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )

        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).traceItems.isEmpty)
    }

    func testFailedCanonicalRefreshReplacesProtocolPreviewsWithCommittedFailure() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        for index in 1...3 {
            live.upsertStatus(externalID: "model-protocol:event-\(index)",
                             title: "Response failed", detail: "Retrying", state: .failed)
        }
        live.finish()
        let previous = MacChatMessage(id: id, role: .assistant, text: "", createdAt: .now, presentation: live)
        var committed = MacAssistantPresentation.processing()
        committed.upsertStatus(externalID: "model-attempt:terminal-attempt",
                               title: "Model request failed",
                               detail: "The model repeatedly emitted pseudo tool execution or unverified command output",
                               state: .failed)
        committed.finish()
        let canonical = MacChatMessage(id: id, role: .assistant, text: "", createdAt: previous.createdAt,
                                       presentation: committed, deliveryState: .failed)

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(in: [canonical], previous: [previous])
        let traces = try XCTUnwrap(refreshed.first?.presentation).traceItems
        XCTAssertEqual(traces, committed.traceItems)
        XCTAssertEqual(traces.count, 1)
        XCTAssertEqual(traces[0].state, .failed)
        XCTAssertTrue(traces[0].detail.contains("unverified command output"))
    }

    func testPendingSubmissionShadowsCanonicalMessagesWithoutChangingTheirOrder() {
        let userID = UUID()
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: userID, role: .user, text: "canonical user", createdAt: .distantPast),
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical partial", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(MacPendingSubmissionOverlay(
            submissionID: "submission-1",
            userMessage: MacChatMessage(id: userID, role: .user, text: "pending user", createdAt: .now),
            assistantMessage: MacChatMessage(id: assistantID, role: .assistant, text: "revealed text", createdAt: .now)
        ))

        let rendered = overlay.renderedMessages(canonical: canonical)

        XCTAssertEqual(rendered.map(\.id), [userID, assistantID])
        XCTAssertEqual(rendered.map(\.text), ["pending user", "revealed text"])
        XCTAssertEqual(canonical.map(\.text), ["canonical user", "canonical partial"])
    }

    func testCanonicalAcceptanceMovesOnlyAssistantPresentationIntoTransientState() {
        let userID = UUID()
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: userID, role: .user, text: "canonical user", createdAt: .distantPast),
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical final", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(MacPendingSubmissionOverlay(
            submissionID: "submission-2",
            userMessage: MacChatMessage(id: userID, role: .user, text: "pending user", createdAt: .now),
            assistantMessage: MacChatMessage(id: assistantID, role: .assistant, text: "paced partial", createdAt: .now)
        ))

        overlay.acceptCanonical(
            canonical,
            retainPendingAssistantPresentation: true
        )

        XCTAssertNil(overlay.pendingAssistantMessageID)
        XCTAssertEqual(
            overlay.renderedMessages(canonical: canonical).map(\.text),
            ["canonical user", "paced partial"]
        )
        XCTAssertEqual(canonical.last?.text, "canonical final")
    }

    func testCanonicalCompletionAfterRunFinishedCannotRestoreStalePresentationOverride() {
        let userID = UUID()
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: userID, role: .user, text: "canonical user", createdAt: .distantPast),
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical final", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(MacPendingSubmissionOverlay(
            submissionID: "submission-finished-first",
            userMessage: MacChatMessage(id: userID, role: .user, text: "pending user", createdAt: .now),
            assistantMessage: MacChatMessage(id: assistantID, role: .assistant, text: "stale partial", createdAt: .now)
        ))

        overlay.acceptCanonical(
            canonical,
            retainPendingAssistantPresentation: false
        )

        XCTAssertTrue(overlay.isEmpty)
        XCTAssertEqual(overlay.renderedMessages(canonical: canonical), canonical)
    }

    func testDiscardingTransientMessageReturnsRenderingToCanonicalProjection() {
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical final", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        XCTAssertTrue(overlay.mutateMessage(
            canonical: canonical,
            messageID: assistantID,
            createAssistantIfMissing: false
        ) { $0.text = "animated partial" })
        XCTAssertEqual(overlay.renderedMessages(canonical: canonical).first?.text, "animated partial")

        overlay.discardTransientMessage(assistantID)

        XCTAssertTrue(overlay.isEmpty)
        XCTAssertEqual(overlay.renderedMessages(canonical: canonical), canonical)
    }

    func testUnknownMessageCannotEnterPresentationWithoutExplicitAssistantShell() {
        var overlay = MacConversationPresentationOverlay()
        let unknownID = UUID()

        XCTAssertFalse(overlay.mutateMessage(
            canonical: [],
            messageID: unknownID,
            createAssistantIfMissing: false
        ) { $0.text = "forbidden" })
        XCTAssertTrue(overlay.isEmpty)

        XCTAssertTrue(overlay.mutateMessage(
            canonical: [],
            messageID: unknownID,
            createAssistantIfMissing: true
        ) { $0.text = "temporary shell" })
        XCTAssertEqual(
            overlay.renderedMessages(canonical: []).first?.role,
            .assistant
        )
    }
}

private final class SnapshotSourceURLProtocol: URLProtocol, @unchecked Sendable {
    @MainActor static var handlers: [String: (URLRequest) -> Void] = [:]
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Task { @MainActor in
            Self.handlers[request.url!.host!]?(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: 200,
                httpVersion: nil, headerFields: ["Content-Type": "text/plain"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data("fetched original content".utf8))
            client?.urlProtocolDidFinishLoading(self)
        }
    }
    override func stopLoading() {}
}

@MainActor
final class MacContextSelectionSnapshotTests: XCTestCase {
    func testBrowserPreviewIdentityDoesNotDependOnFrameTimestamps() throws {
        let endpoint = try XCTUnwrap(URL(string: "http://127.0.0.1:8123/preview/session?token=fixture"))
        let identity = MacManagedBrowserDocumentIdentity(owner: "conversation", sessionID: "browser-session", endpoint: endpoint)
        XCTAssertEqual(identity, .init(owner: "conversation", sessionID: "browser-session", endpoint: endpoint))
        XCTAssertNotEqual(identity, .init(owner: "other-conversation", sessionID: "browser-session", endpoint: endpoint))
        XCTAssertNotEqual(identity, .init(owner: "conversation", sessionID: "new-browser-session", endpoint: endpoint))
    }

    func testCanonicalEstimateAndCacheDiagnosticRetainIndependentRequestSources() throws {
        let data = Data(#"{"used_tokens":2000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":true,"measured_at_revision":10,"measurement":{"request_id":"next","attempt_id":null,"context_generation":4},"prompt_cache_measurement":{"request_id":"paid","attempt_id":null,"context_generation":3},"prompt_cache_diagnostic":{"schemaVersion":1,"status":"hit","identityStable":false,"reasons":["tool_manifest_changed"],"cacheReadInputTokens":800,"cacheWriteInputTokens":null}}"#.utf8)
        let usage = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ContextUsage.self, from: data)
        let snapshot = MacConversationContextSnapshot(canonical: usage)
        XCTAssertTrue(snapshot.isEstimated)
        XCTAssertEqual(snapshot.measurement?.requestID, "next")
        XCTAssertEqual(snapshot.promptCacheMeasurement?.requestID, "paid")
        XCTAssertNil(snapshot.promptCacheMeasurement?.attemptID)
        XCTAssertEqual(snapshot.promptCacheDiagnostic?.reasons, ["tool_manifest_changed"])
        let restored = try JSONDecoder().decode(MacConversationContextSnapshot.self, from: JSONEncoder().encode(snapshot))
        XCTAssertEqual(restored, snapshot)
    }

    func testCacheOnlyChangeMustRefreshInspectorWithinSameContextBucket() throws {
        // Reproduce the measured latest-conversation values without user data.
        let miss = MacConversationContextSnapshot(usedTokens: 15_455,
            contextWindowTokens: 128_000, promptTokenLimit: 111_616, isEstimated: false,
            cacheUsage: MacPromptCacheUsage(input_tokens: 15_455, read_tokens: 0,
                write_tokens: nil, hit_rate_bps: 0, is_compaction: false))
        let hit = MacConversationContextSnapshot(usedTokens: 15_485,
            contextWindowTokens: 128_000, promptTokenLimit: 111_616, isEstimated: false,
            cacheUsage: MacPromptCacheUsage(input_tokens: 15_485, read_tokens: 15_450,
                write_tokens: nil, hit_rate_bps: 9_977, is_compaction: false))
        XCTAssertNotEqual(miss.cacheUsage?.rateText, hit.cacheUsage?.rateText)
        XCTAssertEqual(miss.cacheUsage?.hit_rate_bps, 0)
        XCTAssertEqual(hit.cacheUsage?.hit_rate_bps, 9_977)
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: miss, next: hit))
    }

    func testUnknownAndMeasuredMissAreDistinctEvenWithUnchangedOccupancy() {
        let unknown = cacheSnapshot(usage: MacPromptCacheUsage(input_tokens: 1000,
            read_tokens: nil, write_tokens: nil, hit_rate_bps: nil, is_compaction: false))
        let miss = cacheSnapshot(usage: MacPromptCacheUsage(input_tokens: 1000,
            read_tokens: 0, write_tokens: nil, hit_rate_bps: 0, is_compaction: false))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: unknown, next: miss))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: miss, next: unknown))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: miss, next: cacheSnapshot()))
    }

    func testReadWriteAndDiagnosticChangesPublishIndependently() {
        let original = cacheSnapshot()
        let diagnostic = MacPromptCacheDiagnostic(schemaVersion: 1, status: "hit", identityStable: true,
            identitySHA256: nil, reasons: ["provider_reported_cache_read"], fragmentChanges: nil,
            cacheReadInputTokens: 800, cacheWriteInputTokens: nil)
        for snapshot in [cacheSnapshot(read: 0), cacheSnapshot(write: 0),
                         cacheSnapshot(read: 800), cacheSnapshot(write: 100),
                         cacheSnapshot(diagnostic: diagnostic)] {
            XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: original, next: snapshot))
            XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: snapshot, next: original))
        }
    }

    func testUnchangedCacheEvidenceStillCoalescesOccupancyWithinSameBucket() {
        let current = cacheSnapshot(used: 1000)
        XCTAssertFalse(MacContextSnapshotPublicationPolicy.shouldPublish(current: current, next: current))
        XCTAssertFalse(MacContextSnapshotPublicationPolicy.shouldPublish(current: current, next: cacheSnapshot(used: 1001)))
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: current, next: cacheSnapshot(used: 1300)))
    }

    private func cacheSnapshot(used: Int = 1000, read: Int? = nil, write: Int? = nil,
                               diagnostic: MacPromptCacheDiagnostic? = nil,
                               usage: MacPromptCacheUsage? = nil) -> MacConversationContextSnapshot {
        MacConversationContextSnapshot(usedTokens: used, contextWindowTokens: 128_000,
            promptTokenLimit: 111_616, isEstimated: false, cachedInputTokens: read,
            cacheWriteInputTokens: write, promptCacheDiagnostic: diagnostic, cacheUsage: usage)
    }

    func testCacheUsageSurvivesCanonicalAndPersistedSnapshotRoundTrip() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":800,"write_tokens":100,"hit_rate_bps":8000,"is_compaction":true}}"#.utf8)
        let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ContextUsage.self, from: data)
        XCTAssertEqual(projection.cacheUsage?.hit_rate_bps, 8000)
        let snapshot = MacConversationContextSnapshot(usedTokens: 1000, contextWindowTokens: 128000, promptTokenLimit: 120000, isEstimated: false, cacheUsage: projection.cacheUsage)
        let restored = try JSONDecoder().decode(MacConversationContextSnapshot.self, from: JSONEncoder().encode(snapshot))
        XCTAssertEqual(restored.cacheUsage, projection.cacheUsage)
        XCTAssertTrue(restored.cacheUsage?.is_compaction == true)
        let unknown = MacPromptCacheUsage(input_tokens: 1000, read_tokens: nil, write_tokens: nil, hit_rate_bps: nil, is_compaction: false)
        let miss = MacPromptCacheUsage(input_tokens: 1000, read_tokens: 0, write_tokens: 1000, hit_rate_bps: 0, is_compaction: false)
        XCTAssertNotEqual(unknown.rateText, miss.rateText)
    }

    func testConversationCacheTotalsAreIndependentOfLastRequestAndOccupancy() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":900,"read_tokens":0,"write_tokens":0,"hit_rate_bps":0,"is_compaction":false},"conversation_cache_usage":{"input_tokens":1000,"read_tokens":100,"write_tokens":0,"hit_rate_bps":1000,"is_compaction":false,"reported_requests":2,"unreported_requests":1}}"#.utf8)
        let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ContextUsage.self, from: data)
        let before = MacConversationContextSnapshot(usedTokens: 1000, contextWindowTokens: 128000,
            promptTokenLimit: 120000, isEstimated: false, cacheUsage: projection.cacheUsage)
        let after = MacConversationContextSnapshot(usedTokens: 1000, contextWindowTokens: 128000,
            promptTokenLimit: 120000, isEstimated: false, cacheUsage: projection.cacheUsage,
            conversationCacheUsage: projection.conversationCacheUsage)
        XCTAssertEqual(after.cacheUsage?.hit_rate_bps, 0)
        XCTAssertEqual(after.conversationCacheUsage?.hit_rate_bps, 1000)
        XCTAssertTrue(MacContextSnapshotPublicationPolicy.shouldPublish(current: before, next: after))
        let restored = try JSONDecoder().decode(MacConversationContextSnapshot.self, from: JSONEncoder().encode(after))
        XCTAssertEqual(restored.conversationCacheUsage, after.conversationCacheUsage)
    }

    private func fixture() throws -> (URL, MacDataSourceRegistry, MacWebSourceStore, MacWebSource) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-source-snapshot-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SnapshotSourceURLProtocol.self]
        let session = URLSession(configuration: configuration)
        addTeardownBlock { session.invalidateAndCancel() }
        let store = MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry, urlSession: session)
        let source = MacWebSource(id: UUID(), name: "Original source", url: "https://\(UUID().uuidString.lowercased()).invalid/original",
            format: .text, isEnabled: true, cachedText: "original cache", lastFetchedAt: .now,
            lastError: nil, createdAt: .now)
        store.upsert(source)
        registry.recordValidation(sourceID: source.dataSourceID, connectorID: "mac.web.http",
            readerDescription: "HTTP GET", schemaFields: ["text"], redactedSample: "text",
            recordCount: 1, maximumAgeMinutes: 60, observedAt: .now, defaultPolicy: .defaultEnabled)
        return (root, registry, store, source)
    }

    func testCapturedWebSelectionDoesNotReselectNameURLCacheOrEnablement() async throws {
        let (_, _, store, source) = try fixture()
        let selection = store.captureContextSelection(conversationID: "original-conversation")
        XCTAssertEqual(selection.sources.count, 1)
        var edited = source
        edited.name = "New source"; edited.url = "https://other.invalid/new"
        edited.cachedText = "other cache"; edited.isEnabled = false
        store.upsert(edited)
        let context = await store.promptWithSources("", selection: selection)
        XCTAssertTrue(context.contains("Original source"))
        XCTAssertTrue(context.contains(source.url))
        XCTAssertTrue(context.contains("original cache"))
        XCTAssertFalse(context.contains("other cache"))
    }

    func testFetchReturnsFrozenRequestContentWithoutOverwritingEditedSource() async throws {
        let (_, _, store, original) = try fixture()
        var source = original; source.lastFetchedAt = .distantPast
        store.upsert(source)
        let selection = store.captureContextSelection(conversationID: "c")
        let host = try XCTUnwrap(URL(string: source.url)?.host)
        SnapshotSourceURLProtocol.handlers[host] = { request in
            XCTAssertEqual(request.url?.absoluteString, source.url)
            var edited = source
            edited.name = "Edited"; edited.url = "https://edited.invalid/new"; edited.cachedText = "edited cache"
            store.upsert(edited)
        }
        defer { SnapshotSourceURLProtocol.handlers[host] = nil }
        let context = await store.promptWithSources("", selection: selection)
        XCTAssertTrue(context.contains("Original source"))
        XCTAssertTrue(context.contains("fetched original content"))
        XCTAssertFalse(context.contains("edited cache"))
        XCTAssertEqual(store.sources.first?.cachedText, "edited cache")
        XCTAssertEqual(store.sources.first?.name, "Edited")
    }

    func testLivePolicyRevocationDuringFetchSuppressesCapturedContextAndCacheWrite() async throws {
        let (_, registry, store, original) = try fixture()
        var source = original; source.lastFetchedAt = .distantPast
        store.upsert(source)
        let selection = store.captureContextSelection(conversationID: "c")
        let host = try XCTUnwrap(URL(string: source.url)?.host)
        SnapshotSourceURLProtocol.handlers[host] = { _ in registry.setPolicy(.blocked, for: source.dataSourceID) }
        defer { SnapshotSourceURLProtocol.handlers[host] = nil }
        let context = await store.promptWithSources("prompt", selection: selection)
        XCTAssertEqual(context, "prompt")
        XCTAssertEqual(store.sources.first?.cachedText, "original cache")
    }

    func testPlatformSelectionFreezesEnabledSetAndStepGuidance() throws {
        let (root, registry, _, _) = try fixture()
        // Conversation overrides are canonical writes, not an in-memory UI
        // toggle. Create the actual owner before requesting its override.
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        _ = try application.createConversation(.init(id: "captured", title: "Captured",
            settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1))
        let id = MacDataSourceRegistry.platformSourceID("health")
        registry.recordValidation(sourceID: id, connectorID: "health", readerDescription: "summary",
            schemaFields: ["steps"], redactedSample: "steps", recordCount: 1,
            maximumAgeMinutes: 60, observedAt: .now, defaultPolicy: .defaultEnabled)
        let key = MacRunNotificationPreferenceKeys.stepGuidance
        let suiteName = "nola-context-selection-\(UUID().uuidString)"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { preferences.removePersistentDomain(forName: suiteName) }
        preferences.set(false, forKey: key)
        let service = MacPlatformDataService(registry: registry, startsPeriodicRefresh: false,
            preferences: preferences)
        let selection = service.captureContextSelection(conversationID: "captured")
        registry.setConversationOverride(false, sourceID: id, conversationID: "captured")
        XCTAssertNil(registry.persistenceError)
        XCTAssertEqual(registry.conversationOverride(sourceID: id, conversationID: "captured"), false)
        preferences.set(true, forKey: key)
        XCTAssertTrue(selection.kinds.contains(.health))
        XCTAssertFalse(selection.stepGuidance)
        XCTAssertEqual(selection.conversationID, "captured")
        XCTAssertFalse(service.captureContextSelection(conversationID: "captured").kinds.contains(.health))
    }
}

@MainActor
final class MacEditedResendTests: XCTestCase {
    func testAdmissionDropsTransientRowsThatBelongedToEditedSuffix() {
        let prefix = MacChatMessage(role: .user, text: "prefix", createdAt: .now)
        let old = MacChatMessage(role: .assistant, text: "original", createdAt: .now)
        let replacement = MacChatMessage(role: .user, text: "edited", createdAt: .now)
        let streaming = MacChatMessage(role: .assistant, text: "new", createdAt: .now)
        var overlay = MacConversationPresentationOverlay()
        XCTAssertTrue(overlay.mutateMessage(canonical: [prefix, old], messageID: old.id, createAssistantIfMissing: false) { $0.text += " finish" })
        overlay.beginSubmission(.init(submissionID: "obsolete", userMessage: prefix, assistantMessage: old))
        let visible = [prefix, replacement, streaming]
        overlay.reconcileAfterRewind(visible)
        XCTAssertEqual(overlay.renderedMessages(canonical: visible), visible)
        XCTAssertTrue(overlay.isEmpty)
    }

    func testInitialUserRevisionEnablesInPlaceEditWithoutInventingAVersion() throws {
        let messageJSON = """
        {"message_id":"first-user","role":"user","state":"completed","revision":0,"created_at_ms":1,
         "parts":[{"kind":"text","part_id":"p","revision":0,"markdown":"first","is_final":true}]}
        """
        let original = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self, from: Data(messageJSON.utf8))
        guard case .text(let body)? = original.parts.first else { return XCTFail("Missing original user text") }
        XCTAssertTrue(body.isFinal)
        XCTAssertEqual(body.revision, 0)

        let draft = AppleMessageEditDraft(sourceConversationID: "thread", sourceMessageID: "first-user",
            expectedSourceRevision: 2, expectedMessageRevision: original.revision, text: "replacement")
        XCTAssertTrue(draft.hasSourceWitness)
        let request = try XCTUnwrap(draft.inPlacePreparation())
        XCTAssertEqual(request.expected_message_revision, 0)
        XCTAssertEqual(request.expected_revision, 2)
        let missing = AppleMessageEditDraft(sourceConversationID: "thread", sourceMessageID: "first-user",
            expectedSourceRevision: 2, expectedMessageRevision: nil, text: "replacement")
        XCTAssertNil(missing.inPlacePreparation())
    }

    func testInPlaceEditKeepsOwnerAndStableRetryIdentity() throws {
        var draft = AppleMessageEditDraft(sourceConversationID: "existing-conversation", sourceMessageID: "user-message",
            expectedSourceRevision: 7, expectedMessageRevision: 3, text: "original")
        draft.text = "edited"
        let input = try XCTUnwrap(draft.inPlacePreparation())
        XCTAssertEqual(input.conversation_id, "existing-conversation")
        XCTAssertEqual(input.message_id, "user-message")
        XCTAssertEqual(input.expected_revision, 7)
        XCTAssertEqual(input.expected_message_revision, 3)
        XCTAssertEqual(input, draft.inPlacePreparation())
        XCTAssertEqual(input.edited_text, "edited")
        draft.text = "another edit"
        XCTAssertNotEqual(input.operation_id, draft.inPlacePreparation()?.operation_id)
        draft.text = "  "
        XCTAssertNil(draft.inPlacePreparation())
    }

    func testExpertConfigurationOnlyChangesExplicitSelections() throws {
        let original: MacCanonicalJSONValue = .object([
            "modelSelectionID": .string("configured:original"),
            "selectedPromptTemplateID": .string(UUID().uuidString),
            "modelOverrides": .object(["configured:original": .object(["temperature": .number(0.3)])]),
            "language": .string("zh-Hans"), "memoryEnabledOverride": .bool(false),
            "retrievalEnabledOverride": .bool(true), "contextLimitOverride": .number(16000)
        ])
        var draft = try AppleExpertConfigurationDraft(defaults: original)
        XCTAssertEqual(try draft.applying(to: original), original)
        draft.modelID = "configured:changed"
        draft.templateID = ""
        guard case .object(let updated) = try draft.applying(to: original), case .object(let fields) = original else {
            return XCTFail("Expected a defaults object")
        }
        XCTAssertEqual(updated["modelSelectionID"], .string("configured:changed"))
        XCTAssertNil(updated["selectedPromptTemplateID"])
        for key in ["modelOverrides", "language", "memoryEnabledOverride", "retrievalEnabledOverride", "contextLimitOverride"] {
            XCTAssertEqual(updated[key], fields[key])
        }
        draft.modelID = ""; draft.templateID = ""
        guard case .object(let inherited) = try draft.applying(to: original) else { return XCTFail() }
        XCTAssertNil(inherited["modelSelectionID"])
        XCTAssertNil(inherited["selectedPromptTemplateID"])
        XCTAssertEqual(inherited["modelOverrides"], fields["modelOverrides"])
        XCTAssertThrowsError(try draft.applying(to: .null))
    }

    func testExpertConfigurationPreservesExactAbsentAndNullTemplate() throws {
        for initial: MacCanonicalJSONValue in [.object([:]), .object(["selectedPromptTemplateID": .null])] {
            XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: initial).applying(to: initial), initial)
        }
    }

    func testExpertModelAndTemplateUpdateUsesCoreCASAndPreservesExistingContexts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("experts.db"))
        for id in ["original", "changed"] {
            let source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture-endpoint", "endpointName": "Fixture",
                "baseURL": "https://fixture.invalid/v1", "endpointEnabled": true, "endpointOrder": 0,
                "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
                "wireProtocol": "chat_completions", "credentialAccount": "fixture-account"]
            try port.upsertModelProfile(.init(id: id, displayName: id, provider: "openai-compatible", modelName: id,
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        }
        let service = MacAgentRoomService(runtime: port)
        let initial: MacCanonicalJSONValue = .object(["modelSelectionID": .string("configured:original"), "language": .string("zh-Hans")])
        let directory = try service.createAgent(name: "Original", description: "role", defaults: initial, directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let room = try service.create(title: "Existing context")
        let joined = try service.join(agent: agent, room: room)
        let context = try XCTUnwrap(joined.room.participants.first?.contextConversationID)
        let before = try port.loadConversationMetadata(context)
        var draft = try AppleExpertConfigurationDraft(defaults: initial)
        draft.modelID = "configured:changed"; draft.templateID = UUID().uuidString
        let updatedDefaults = try draft.applying(to: initial)
        let updated = try service.updateAgent(agent, name: "Edited", description: "role", availability: "active",
            directory: directory, defaults: updatedDefaults)
        XCTAssertEqual(updated.agents.first?.defaults, updatedDefaults)
        XCTAssertEqual(try port.loadConversationMetadata(context)?.settingsJSON, before?.settingsJSON)
        XCTAssertThrowsError(try service.updateAgent(agent, name: "Stale", description: "role", availability: "active",
            directory: directory, defaults: initial))
        XCTAssertEqual(try service.directory().agents.first?.defaults, updatedDefaults)
    }

    private func credential(profile: String = "profile", endpoint: String = "https://fixture.invalid/v1?token=scope", model: String = "model", secret: String = "fixture-key") -> MacRunModelCredential {
        .init(profileID: profile, endpointID: "endpoint", account: "private-account", endpoint: endpoint,
              model: model, wireProtocol: .chatCompletions, secretDigest: MacRunModelCredential.digest(secret))
    }
    private var route: MacCanonicalJSONValue {
        .object(["kind": .string("http"), "siwc_account": .null, "protocol": .string(MacModelWireProtocol.chatCompletions.rawValue),
                 "model": .string("model"), "endpoint": .string("https://fixture.invalid/v1"),
                 "endpoint_query_sha256": .string(MacRunModelCredential.digest("token=scope")),
                 "credential_sha256": .string(MacRunModelCredential.digest("fixture-key")), "output_token_field": .string("max_tokens"),
                 "provider": .null, "credential_source": .object(["kind": .string("ephemeral")]),
                 "endpoint_query_source": .object(["kind": .string("ephemeral")])])
    }
    func testFrozenRouteRequiresCurrentExactPrivateCredentialAndUniqueProfile() throws {
        XCTAssertEqual(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: [credential()]), 0)
        for changed in [credential(endpoint: "https://elsewhere.invalid/v1?token=scope"), credential(model: "other"),
                        credential(secret: "rotated"), credential(endpoint: "https://fixture.invalid/v1?token=changed") ] {
            XCTAssertFalse(MacEditedResendModelResolver.matchesHTTP(route, credential: changed))
            XCTAssertThrowsError(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: [changed]))
        }
        XCTAssertThrowsError(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: [credential(), credential(profile: "another")]))
        XCTAssertThrowsError(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: []))
    }

    func testChatGPTResendRequiresTheExactReadyAccountWithoutAnAPIKey() throws {
        let id = UUID().uuidString.lowercased()
        let raw: MacCanonicalJSONValue = .object(["account_id": .string(id), "account_revision": .number(3),
            "credential_ref": .string("siwc:" + id), "client_id": .string("fixture-client"), "subject": .string("fixture-subject")])
        func account(revision: Int64 = 3, state: String = "ready", source: String = "nola_chatgpt", reference: String? = nil) -> MacModelAccountSnapshot {
            .init(revision: revision, account: .init(id: id, display_name: "Fixture",
                source: source, credential_ref: reference ?? "siwc:" + id, state: state))
        }
        XCTAssertTrue(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account()))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(revision: 4)))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(state: "needs_reauthorization")))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(source: "api")))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(reference: "siwc:foreign")))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(.null, account: account()))
    }

    func testEditedNativePayloadUsesNewOwnerAndOnlyFrozenModelFields() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let operation = UUID().uuidString.lowercased(), target = UUID().uuidString.lowercased()
        var request = try MacLocalAgentRunRequest(conversationID: target, submissionID: operation,
            userMessageID: operation + ":user", assistantMessageID: operation + ":assistant", prompt: " edited text \n",
            projectURL: environment.sessionWorkspaceURL(for: target), attachmentURLs: [], modelID: "model",
            endpoint: "https://fixture.invalid/v1?token=scope", apiKey: "fixture-key",
            reasoningPreference: "automatic", modelParameters: .webAlignedCloudDefaults, wireProtocol: .chatCompletions,
            permissionMode: .cautious, databaseURLOverride: port.databaseURL, appEnvironment: environment, applicationPort: port)
        request.editedResendOperationID = operation
        request.editedResendModel = .init(version: 1, primary: route, fallbacks: .array([]), generation: .object([
            "max_retries": .number(1), "parallel_tool_calls": .null, "reasoning_selection": .null,
            "reasoning_tier": .string("balanced"), "sampling_override": .bool(false), "generation_options": .object([:]),
            "temperature": .number(0.2), "top_p": .number(0.8), "max_output_tokens": .number(2000),
            "thinking_enabled": .bool(false), "prompt_cache": .bool(false), "prompt_cache_retention": .null,
            "prompt_cache_key_sha256": .null, "prompt_cache_key_prefix_sha256": .null
        ]), output_admission: .object(["version": .number(1), "preference": .string("default"), "configured_context_window": .number(32000),
            "routes": .array([.object(["route": route, "max_input_tokens": .null, "max_output_tokens": .number(2000), "max_total_tokens": .null])]),
            "resolved": .object(["tokens": .number(2000), "source": .string("known_capability_default")])]), model_capability: .object([
            "profile_id": .string("fixture-model"), "revision": .number(1), "instruction_profile": .string("generic"),
            "native_tool_calls": .bool(true), "parallel_tool_calls": .bool(false), "reasoning": .bool(false),
            "reasoning_continuity": .bool(false), "prompt_cache": .bool(false), "vision_input": .bool(false),
            "context_window_tokens": .number(32000), "max_output_tokens": .number(2000),
            "request_continuation": .bool(false), "max_request_continuations": .number(0),
            "stream_text": .bool(true), "stream_reasoning": .bool(true), "stream_tool_calls": .bool(true),
            "direct_tool_capability_ids": .array([])
        ]))
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(payload["edited_resend_operation_id"] as? String, operation)
        XCTAssertEqual(payload["user_message_id"] as? String, operation + ":user")
        XCTAssertEqual(payload["assistant_message_id"] as? String, operation + ":assistant")
        XCTAssertEqual(payload["prompt"] as? String, " edited text \n")
        XCTAssertEqual((payload["context_components"] as? [Any])?.count, 0)
        XCTAssertEqual((payload["attachments"] as? [Any])?.count, 0)
        XCTAssertNil(payload["external_session_id"])
        XCTAssertNil(payload["expected_parent_message_id"] as? String)
        XCTAssertNil(payload["max_output_tokens"], "A default admission must not turn into an explicit preference")
        XCTAssertEqual(payload["temperature"] as? Double, 0.2)
        XCTAssertEqual(payload["context_window_tokens"] as? Int, 32000)
        XCTAssertEqual((payload["mac_launch"] as? [String: Any])?["project_path"] as? String,
                       environment.sessionWorkspaceURL(for: target).standardizedFileURL.resolvingSymlinksInPath().path)
        XCTAssertNil(payload["resume"])
        XCTAssertNil(payload["fresh_resend_policy"], "Only the trusted Host may attest its actual Core configuration")
        for field in ["max_output_tokens", "omitted"] {
            var frozenRoute = try XCTUnwrap(request.editedResendModel)
            guard case .object(var fields) = frozenRoute.primary else { return XCTFail() }
            fields["output_token_field"] = .string(field)
            frozenRoute = .init(version: frozenRoute.version, primary: .object(fields), fallbacks: frozenRoute.fallbacks,
                generation: frozenRoute.generation, output_admission: frozenRoute.output_admission, model_capability: frozenRoute.model_capability)
            request.editedResendModel = frozenRoute
            let adapted = try await MacAgentHostRuntime().initialRequestData(for: request)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: adapted) as? [String: Any])
            XCTAssertEqual(object["chat_output_token_field"] as? String, "auto")
        }

    }
}
