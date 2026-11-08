import XCTest
@testable import Nola

final class CanonicalConversationStoreEndToEndTests: XCTestCase {
    func testDuplicateCreationRejectsWithoutAdoptingOrChangingExistingConfiguration() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-create-receipt-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.db"))
        let id = UUID().uuidString
        let original = "{\"language\":\"zh-Hans\"}"
        try runtime.createConversation(id: id, title: "Original", settingsJSON: original, createdAtMs: 1, updatedAtMs: 1)
        XCTAssertThrowsError(try runtime.createConversation(id: id, title: "Conflicting", settingsJSON: "{}", createdAtMs: 2, updatedAtMs: 2))
        let saved = try XCTUnwrap(runtime.loadConversation(id: id))
        XCTAssertEqual(saved.title, "Original")
        XCTAssertEqual(try IOSConversationSettingsObject.decode(saved.settingsJSON)["language"] as? String, "zh-Hans")
    }

    func testStoredPermissionRejectsExplicitInvalidValuesAndPreservesSparseValidSettings() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-permission-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.db"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let id = try store.createConversation(title: "Permission", language: "en").id
        let invalidProfiles: [Any] = ["moderate", "allow-all", "unknown", true]
        for (index, value) in invalidProfiles.enumerated() {
            // Invalid wire input has no durable rejection receipt. Its original
            // intent remains pending; unrelated mutations must not overtake it.
            let invalidRuntime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("invalid-\(index).db"))
            let invalidStore = try LocalConversationStore(applicationRuntime: invalidRuntime)
            let invalidID = try invalidStore.createConversation(title: "Invalid permission", language: "en").id
            let json = String(decoding: try JSONSerialization.data(withJSONObject:
                ["permissionProfile": value]), as: UTF8.self)
            let before = try XCTUnwrap(invalidRuntime.loadConversation(id: invalidID.uuidString))
            XCTAssertThrowsError(try invalidRuntime.patchConversationConfiguration(id: invalidID.uuidString,
                expectedRevision: XCTUnwrap(before.configurationRevision), patchJSON: json, updatedAtMs: 1).requireCommitted()) { error in
                guard case IOSApplicationRuntimeError.outcomePending = error else {
                    return XCTFail("Invalid wire input must retain its unconfirmed intent: \(error)")
                }
            }
            XCTAssertEqual(try invalidRuntime.loadConversation(id: invalidID.uuidString)?.settingsJSON, before.settingsJSON)
        }
        for mode in IOSPermissionMode.allCases {
            let json = String(decoding: try JSONSerialization.data(withJSONObject:
                ["permissionProfile": mode.rawValue]), as: UTF8.self)
            try runtime.patchConversationConfiguration(id: id.uuidString,
                expectedRevision: XCTUnwrap(runtime.loadConversation(id: id.uuidString)?.configurationRevision),
                patchJSON: json, updatedAtMs: 2).requireCommitted()
            XCTAssertEqual(try store.getConversation(conversationId: id)?.permissionMode, mode)
        }
        try runtime.patchConversationConfiguration(id: id.uuidString,
            expectedRevision: XCTUnwrap(runtime.loadConversation(id: id.uuidString)?.configurationRevision),
            patchJSON: "{\"permissionProfile\":null}", updatedAtMs: 3).requireCommitted()
        XCTAssertNil(try IOSConversationSettingsObject.decode(runtime.loadConversation(id: id.uuidString)?.settingsJSON)["permissionModeRawValue"])
        XCTAssertEqual(try store.getConversation(conversationId: id)?.permissionMode, .cautious)
    }

    func testPerModelConversationLimitsOverrideAndResetWithoutErasingMacFields() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-model-override-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("nola-product.db")
        let model = ModelInfo(id: "profile-a", name: "Model A", source: "fixture", isLocal: false)
        let key = IOSConversationGenerationOverride.modelKey(for: model)
        let conversationID: UUID

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let store = try LocalConversationStore(applicationRuntime: runtime)
            conversationID = UUID()
            let settings = "{}"
            let withMacFields = try IOSConversationGenerationOverrides.patching(
                settings, modelKey: key, override: .init()
            )
            var root = try IOSConversationSettingsObject.decode(withMacFields)
            root["modelOverrides"] = [key: ["reasoningPreference": "high", "wireProtocol": "responses"]]
            root["foreignClientField"] = ["preserve": true]
            let seeded = String(decoding: try JSONSerialization.data(withJSONObject: root), as: UTF8.self)
            try runtime.createConversation(id: conversationID.uuidString, title: "Limits", settingsJSON: seeded,
                createdAtMs: 1, updatedAtMs: 1)

            try store.setGenerationOverride(
                conversationId: conversationID, modelKey: key,
                override: .init(maxContextLength: 32_000, maxOutputTokens: 2048)
            )
            XCTAssertEqual(try store.generationOverride(conversationId: conversationID, modelKey: key),
                           .init(maxContextLength: 32_000, maxOutputTokens: 2048))
            var ordinaryEdit = try XCTUnwrap(store.getConversation(conversationId: conversationID))
            ordinaryEdit.sessionMemory = "unrelated edit"
            try store.upsertConversation(ordinaryEdit)
            XCTAssertEqual(try store.generationOverride(conversationId: conversationID, modelKey: key).maxOutputTokens,
                           2048, "An unrelated iOS metadata save must not erase session model limits")
            var defaults = ModelParameters.webAlignedCloudDefaults
            defaults.maxContextLength = 128_000
            defaults.maxOutputTokens = 16_384
            let effective = try store.generationOverride(conversationId: conversationID, modelKey: key).applying(to: defaults)
            XCTAssertEqual(effective.maxContextLength, 32_000)
            XCTAssertEqual(effective.maxOutputTokens, 2048)
            XCTAssertThrowsError(try store.setGenerationOverride(
                conversationId: conversationID, modelKey: key,
                override: .init(maxContextLength: 0, maxOutputTokens: nil)
            ))
        }

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let store = try LocalConversationStore(applicationRuntime: runtime)
            XCTAssertEqual(try store.generationOverride(conversationId: conversationID, modelKey: key).maxOutputTokens, 2048)
            try store.setGenerationOverride(conversationId: conversationID, modelKey: key, override: .init())
            let reset = try store.generationOverride(conversationId: conversationID, modelKey: key)
            XCTAssertEqual(reset.maxContextLength, 32_000)
            XCTAssertEqual(reset.maxOutputTokens, 2048)
            let settings = try IOSConversationSettingsObject.decode(runtime.loadConversation(id: conversationID.uuidString)?.settingsJSON)
            let modelFields = try XCTUnwrap((settings["modelOverrides"] as? [String: Any])?[key] as? [String: Any])
            XCTAssertEqual(modelFields["reasoningPreference"] as? String, "high")
            XCTAssertEqual(modelFields["wireProtocol"] as? String, "responses")
            XCTAssertEqual((settings["foreignClientField"] as? [String: Bool])?["preserve"], true)
            XCTAssertEqual(modelFields["maxContextLength"] as? Int, 32_000)
            XCTAssertEqual(modelFields["maxOutputTokens"] as? Int, 2048)
        }
    }

    @MainActor
    func testDirectCloudRunFreezesConversationOutputWithoutMutatingModelDefaults() throws {
        let model = ModelInfo(
            id: "profile-a", name: "Fixture", source: "fixture", isLocal: false,
            baseURL: "https://provider.invalid/v1", apiKey: "fixture-key",
            upstreamModelId: "upstream-model", executionCapabilities: .init(samplingParameters: true)
        )
        var defaults = ModelParameters.webAlignedCloudDefaults
        defaults.maxOutputTokens = 16_384
        defaults.temperature = 0.4
        let session = IOSConversationGenerationOverride(maxContextLength: 32_000, maxOutputTokens: 2048)
        let effective = session.applying(to: defaults)
        let request = try IOSConversationRunConfiguration.directCloudProviderConfig(
            model: model, parameters: effective
        )
        XCTAssertEqual(request.model, "upstream-model")
        XCTAssertEqual(request.maxCompletionTokens, 2048)
        XCTAssertEqual(request.temperature, 0.4)
        XCTAssertEqual(defaults.maxOutputTokens, 16_384)
        let inherited = try IOSConversationRunConfiguration.directCloudProviderConfig(
            model: model, parameters: IOSConversationGenerationOverride().applying(to: defaults)
        )
        XCTAssertEqual(inherited.maxCompletionTokens, 16_384)
    }

    private final class DeterministicSummaryClient: LLMClient {
        func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
            ModelOutput(items: [], explain: "Persisted compact summary")
        }

        func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
            AsyncThrowingStream { continuation in continuation.finish() }
        }

        func chat(
            message: String,
            systemPrompt: String?,
            enableThinking: Bool,
            timeout: TimeInterval?
        ) async throws -> String {
            "Persisted compact summary"
        }
    }

    func testAutomationOccurrenceLeaseRetryAndOutcomeSurviveColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-automation-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("nola-product.db")
        let run = IOSCanonicalAutomationRun(
            id: "run-1", automationID: "brief", definitionRevision: 1, occurrenceKey: "brief:2026-08-18",
            trigger: "background", status: "queued", attempt: 0,
            workerID: nil, leaseToken: nil, leaseUntilMs: nil,
            scheduledForMs: 1_000, startedAtMs: nil, finishedAtMs: nil,
            nextRetryAtMs: nil, resultJSON: nil, errorJSON: nil,
            createdAtMs: 900, updatedAtMs: 900
        )
        let encoded = try JSONEncoder().encode(run)
        XCTAssertEqual(try JSONDecoder().decode(IOSCanonicalAutomationRun.self, from: encoded), run)
        var missingRevision = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        missingRevision.removeValue(forKey: "definition_revision")
        XCTAssertThrowsError(try JSONDecoder().decode(IOSCanonicalAutomationRun.self,
            from: JSONSerialization.data(withJSONObject: missingRevision)))
        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            _ = try store.upsertAutomationDefinition(.init(id: "brief", name: "Brief", scheduleFormat: "ios.scheduled_url_report", enabled: true, revision: 0,
                executionSpec: .agent(.init(prompt: "Brief", executionScope: .deviceLocal(hostID: "ios-device"), permissionProfile: "cautious", modelSelection: "use_current_default_at_admission", contextRefs: [], skillRefs: [])),
                metadataJSON: "{}", createdAtMs: 900, updatedAtMs: 900, deletedAtMs: nil))
            XCTAssertEqual(try store.enqueueAutomationRun(run).id, "run-1")
            var duplicate = run
            duplicate = IOSCanonicalAutomationRun(
                id: "run-duplicate", automationID: duplicate.automationID,
                definitionRevision: duplicate.definitionRevision,
                occurrenceKey: duplicate.occurrenceKey, trigger: duplicate.trigger,
                status: duplicate.status, attempt: duplicate.attempt,
                workerID: nil, leaseToken: nil, leaseUntilMs: nil,
                scheduledForMs: duplicate.scheduledForMs, startedAtMs: nil,
                finishedAtMs: nil, nextRetryAtMs: nil, resultJSON: nil,
                errorJSON: nil, createdAtMs: 901, updatedAtMs: 901
            )
            XCTAssertEqual(try store.enqueueAutomationRun(duplicate).id, "run-1")
            XCTAssertTrue(try store.claimAutomationRun(id: "run-1", workerID: "ios-a", leaseToken: "lease-a", nowMs: 1_000, leaseUntilMs: 1_100))
            XCTAssertTrue(try store.retryAutomationRun(id: "run-1", leaseToken: "lease-a",
                nextRetryAtMs: 1_150, errorJSON: "{\"kind\":\"preparation\"}", updatedAtMs: 1_050))
        }
        do {
            let reopened = try IOSApplicationRuntime(databaseURL: database)
            XCTAssertFalse(try reopened.claimAutomationRun(id: "run-1", workerID: "ios-b", leaseToken: "lease-b", nowMs: 1_101, leaseUntilMs: 1_300),
                "A cold process cannot claim before the original owner's explicitly scheduled retry")
            XCTAssertTrue(try reopened.claimAutomationRun(id: "run-1", workerID: "ios-b", leaseToken: "lease-b", nowMs: 1_150, leaseUntilMs: 1_300))
            XCTAssertTrue(try reopened.retryAutomationRun(id: "run-1", leaseToken: "lease-b", nextRetryAtMs: 1_500, errorJSON: "{\"kind\":\"network\"}", updatedAtMs: 1_200))
            XCTAssertFalse(try reopened.claimAutomationRun(id: "run-1", workerID: "ios-c", leaseToken: "lease-c", nowMs: 1_499, leaseUntilMs: 1_700))
            XCTAssertTrue(try reopened.claimAutomationRun(id: "run-1", workerID: "ios-c", leaseToken: "lease-c", nowMs: 1_500, leaseUntilMs: 1_700))
            XCTAssertNil(try reopened.reconcileAutomationExecution(occurrenceID: "run-1", reconciledAtMs: 1_600), "A lease without an admitted Run has no business result")
        }
        let finalStore = try IOSApplicationRuntime(databaseURL: database)
        let persisted = try XCTUnwrap(finalStore.loadAutomationRun(id: "run-1"))
        XCTAssertNil(try finalStore.observeAutomationExecution(occurrenceID: "run-1").receipt)
        XCTAssertEqual(persisted.attempt, 3)
        XCTAssertEqual(persisted.definitionRevision, run.definitionRevision)
        XCTAssertEqual(try finalStore.listAutomationRuns(automationID: "brief").count, 1)
        XCTAssertFalse(try finalStore.claimAutomationRun(id: "run-1", workerID: "ios-d", leaseToken: "lease-d",
            nowMs: 1_800, leaseUntilMs: 2_000), "Lease expiry is not authority to launch a replacement worker")
        let expired = try XCTUnwrap(finalStore.loadAutomationRun(id: "run-1"))
        XCTAssertEqual(expired.attempt, 3)
        XCTAssertEqual(expired.workerID, "ios-c")
        XCTAssertFalse(try finalStore.retryAutomationRun(id: "run-1", leaseToken: "lease-c",
            nextRetryAtMs: 2_000, errorJSON: "{\"kind\":\"expired\"}", updatedAtMs: 1_801))
        XCTAssertTrue(try finalStore.observeAutomationWorker(lease: .init(occurrenceID: "run-1",
            definitionRevision: run.definitionRevision, workerID: "ios-c", leaseToken: "lease-c"),
            observation: .launchFailed(detail: "Unadmitted worker stopped; no business result was observed"), observedAtMs: 1_802))
        let closed = try XCTUnwrap(finalStore.loadAutomationRun(id: "run-1"))
        XCTAssertEqual(closed.status, "launch_failed")
        XCTAssertEqual(closed.attempt, 3)
        XCTAssertNil(closed.leaseToken)
        let observed = try finalStore.observeAutomationExecution(occurrenceID: "run-1")
        XCTAssertNil(observed.binding)
        XCTAssertNil(observed.terminal)
        XCTAssertNil(observed.receipt, "Worker launch failure must not become an admitted Run's terminal result")
        try? FileManager.default.removeItem(at: root)
    }

    func testConversationSettingsAndOrdinalTranscriptSurviveColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-conversation-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("nola-product.db")
        let conversationID: UUID
        let timestamp = Date(timeIntervalSince1970: 1_787_000_000)

        do {
            let canonical = try IOSApplicationRuntime(databaseURL: database)
            let store = try LocalConversationStore(applicationRuntime: canonical)
            var conversation = try store.createConversation(title: "Canonical", language: "zh-Hans")
            conversationID = conversation.id
            let model = ModelInfo(
                id: "qwen3.5-4b",
                name: "Qwen 3.5",
                source: "local",
                isLocal: true
            )
            try canonical.upsertModelProfile(.init(id: model.id, displayName: model.name,
                provider: "local", modelName: model.id, runtime: "mlx", sourceJSON: "{}", parametersJSON: "{}",
                selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
            try store.selectConversationModel(conversationID: conversationID, model: model, initialParameters: .webAlignedCloudDefaults)
            conversation = try XCTUnwrap(store.getConversation(conversationId: conversationID))
            conversation.executionBackend = "nola"
            conversation.dataSourceOverrides = ["health": true]
            try store.upsertConversation(conversation)
            try store.importMessages(
                conversationId: conversation.id,
                messages: [
                    ConversationMessage(
                        conversationId: conversation.id,
                        role: .user,
                        text: "first",
                        createdAt: timestamp
                    ),
                    ConversationMessage(
                        conversationId: conversation.id,
                        role: .assistant,
                        text: "second",
                        createdAt: timestamp
                    ),
                ],
                source: .externalArchive
            )
        }

        do {
            let canonical = try IOSApplicationRuntime(databaseURL: database)
            let reopened = try LocalConversationStore(applicationRuntime: canonical)
            let conversation = try XCTUnwrap(reopened.getConversation(conversationId: conversationID))
            XCTAssertEqual(conversation.selectedModelInfo?.id, "qwen3.5-4b")
            XCTAssertEqual(conversation.executionBackend, "nola")
            XCTAssertEqual(conversation.dataSourceOverrides, ["health": true])
            XCTAssertEqual(
                try reopened.getMessages(conversationId: conversationID, limit: 0).map(\.text),
                ["first", "second"]
            )
        }

        try? FileManager.default.removeItem(at: root)
    }

    @MainActor
    func testCoreCompactionPreservesCanonicalTranscriptAcrossColdReopen() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-compaction-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("nola-product.db")
        let defaultsName = "nola-ios-compaction-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: defaultsName))
        defer {
            defaults.removePersistentDomain(forName: defaultsName)
            try? FileManager.default.removeItem(at: root)
        }

        let conversationID: UUID
        var expected: [(UUID, String)] = []
        do {
            let canonical = try IOSApplicationRuntime(databaseURL: database)
            let store = try LocalConversationStore(applicationRuntime: canonical)
            let conversation = try store.createConversation(title: "Compaction", language: "en")
            conversationID = conversation.id
            for index in 0..<12 {
                let message = ConversationMessage(
                    conversationId: conversation.id,
                    role: index.isMultiple(of: 2) ? .user : .assistant,
                    text: "message-\(index) " + String(repeating: "canonical transcript payload ", count: 80),
                    createdAt: Date(timeIntervalSince1970: 1_787_000_000 + Double(index))
                )
                expected.append((message.id, message.text))
            }
            try store.importMessages(
                conversationId: conversation.id,
                messages: expected.enumerated().map { index, item in
                    ConversationMessage(
                        id: item.0,
                        conversationId: conversation.id,
                        role: index.isMultiple(of: 2) ? .user : .assistant,
                        text: item.1,
                        createdAt: Date(timeIntervalSince1970: 1_787_000_000 + Double(index))
                    )
                },
                source: .externalArchive
            )

            let profileStore = CanonicalChatMemoryProfileStore(applicationRuntime: canonical
            )
            let manager = DefaultContextManager(store: store, profileStore: profileStore)
            manager.llmClient = DeterministicSummaryClient()
            let firstUsage = try await manager.compactConversation(
                for: conversation.id,
                maxTokens: 8_000,
                preserveRecentMessages: 2,
                force: true
            )
            XCTAssertGreaterThan(firstUsage.archivedMessageCount, 0)
            let firstEntryCount = try XCTUnwrap(
                store.getConversation(conversationId: conversation.id)
            ).compactedMemoryEntries.count
            _ = try await manager.compactConversation(
                for: conversation.id,
                maxTokens: 8_000,
                preserveRecentMessages: 2,
                force: true
            )
            XCTAssertEqual(
                try XCTUnwrap(store.getConversation(conversationId: conversation.id))
                    .compactedMemoryEntries.count,
                firstEntryCount
            )
        }

        let reopenedCanonical = try IOSApplicationRuntime(databaseURL: database)
        let reopened = try LocalConversationStore(applicationRuntime: reopenedCanonical)
        let actual = try reopened.getMessages(conversationId: conversationID, limit: 0)
        XCTAssertEqual(actual.map(\.id), expected.map(\.0))
        XCTAssertEqual(actual.map(\.text), expected.map(\.1))
        XCTAssertTrue(actual.allSatisfy { $0.meta?["archivedFromContext"] == nil })
        let persistedConversation = try XCTUnwrap(
            reopened.getConversation(conversationId: conversationID)
        )
        XCTAssertFalse(persistedConversation.compactedMemoryEntries.isEmpty)
        XCTAssertEqual(persistedConversation.sessionMemory, "Persisted compact summary")
    }
}
