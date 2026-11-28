// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: High-signal iOS NolaCore FFI, IOSApplicationRuntime, and LocalConversationStore contract tests against isolated temp SQLite stores.

import Foundation
import SQLite3
import XCTest
@testable import Nola

final class IOSCoreContractsTests: XCTestCase {
    private func makeTempDirectory(_ prefix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-contracts-\(prefix)-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    // MARK: - 1. Unsupported v1 / corrupt store rejection without destroying user data

    func testUnsupportedV1AndCorruptStoreRejectedWithoutDestroyingData() throws {
        let root = try makeTempDirectory("unsupported-store")

        // 1a. Legacy v1 DB (user_version = 1) must be rejected while preserving original rows.
        let v1DB = root.appendingPathComponent("legacy-v1.db")
        var conn: OpaquePointer?
        XCTAssertEqual(sqlite3_open(v1DB.path, &conn), SQLITE_OK)
        XCTAssertEqual(
            sqlite3_exec(conn, "CREATE TABLE preserved (val TEXT); INSERT INTO preserved VALUES ('keep-ios-v1'); PRAGMA user_version = 1;", nil, nil, nil),
            SQLITE_OK
        )
        sqlite3_close(conn)

        XCTAssertThrowsError(try IOSApplicationRuntime(databaseURL: v1DB))

        var roConn: OpaquePointer?
        XCTAssertEqual(sqlite3_open_v2(v1DB.path, &roConn, SQLITE_OPEN_READONLY, nil), SQLITE_OK)
        defer { sqlite3_close(roConn) }
        var stmt: OpaquePointer?
        XCTAssertEqual(sqlite3_prepare_v2(roConn, "SELECT val FROM preserved;", -1, &stmt, nil), SQLITE_OK)
        defer { sqlite3_finalize(stmt) }
        XCTAssertEqual(sqlite3_step(stmt), SQLITE_ROW)
        XCTAssertEqual(sqlite3_column_text(stmt, 0).map { String(cString: $0) }, "keep-ios-v1")

        // 1b. Corrupt non-SQLite file must be rejected without modifying raw bytes.
        let corruptDB = root.appendingPathComponent("corrupt.db")
        let sentinel = Data("corrupt-ios-db-sentinel".utf8)
        try sentinel.write(to: corruptDB, options: .atomic)
        XCTAssertThrowsError(try IOSApplicationRuntime(databaseURL: corruptDB))
        XCTAssertEqual(try Data(contentsOf: corruptDB), sentinel)
    }

    // MARK: - 2. Conversation & message persistence across cold reopen

    func testConversationAndMessagePersistenceAcrossColdReopen() throws {
        let root = try makeTempDirectory("conversations")
        let database = root.appendingPathComponent("nola-ios.db")

        let conversationID: UUID
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let store = try LocalConversationStore(applicationRuntime: runtime)
            conversationID = try store.createConversation(title: nil, language: "zh-Hans").id

            let userMessage = ConversationMessage(conversationId: conversationID, role: .user, text: "整理今天的会议纪要")
            let assistantMessage = ConversationMessage(conversationId: conversationID, role: .assistant, text: "已整理 3 条核心行动项。")
            try store.importMessages(
                conversationId: conversationID,
                messages: [userMessage, assistantMessage],
                source: .externalArchive
            )

            XCTAssertEqual(try store.refreshFirstMessageTitle(conversationId: conversationID), "整理今天的会议纪要")
            try store.updateConversationTitle(conversationId: conversationID, title: "会议纪要归档")
            XCTAssertEqual(try runtime.conversationTitleSource(id: conversationID.uuidString), "user")
        }

        // Cold reopen verifies durable SQLite v2 persistence.
        let reopenedRuntime = try IOSApplicationRuntime(databaseURL: database)
        let loaded = try XCTUnwrap(try reopenedRuntime.loadConversation(id: conversationID.uuidString))
        XCTAssertEqual(loaded.title, "会议纪要归档")
        XCTAssertEqual(loaded.messages.count, 2)
        XCTAssertEqual(loaded.messages[0].role, "user")
        XCTAssertEqual(loaded.messages[1].role, "assistant")

        let projection = try reopenedRuntime.loadConversationProjectionV1(id: conversationID.uuidString)
        XCTAssertEqual(projection.messages.count, 2)
        XCTAssertTrue(try reopenedRuntime.listConversations().contains(where: { $0.id == conversationID.uuidString }))
        XCTAssertTrue(try reopenedRuntime.deleteConversation(id: conversationID.uuidString))
        XCTAssertNil(try reopenedRuntime.loadConversation(id: conversationID.uuidString))
    }

    // MARK: - 3. Model profile save, load, CAS revision, and delete

    func testModelProfileSaveLoadRevisionAndDelete() throws {
        let root = try makeTempDirectory("models")
        let database = root.appendingPathComponent("nola-ios.db")
        let profileID = "ios-contract-model"

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            try runtime.upsertModelProfile(IOSCanonicalModelProfile(
                id: profileID,
                displayName: "LAN Qwen",
                provider: "openai",
                modelName: "qwen3.8-flash-next-iq3_s",
                runtime: "remote",
                sourceJSON: #"{"baseURL":"https://models.example.invalid/v1","wireProtocol":"chat_completions"}"#,
                parametersJSON: "{}",
                selected: true,
                enabled: true,
                revision: 0,
                createdAtMs: 1_000,
                updatedAtMs: 1_000,
                deletedAtMs: nil
            ))

            let parameterStore = ModelParametersStore(applicationPort: runtime)
            var params = ModelParameters.webAlignedCloudDefaults
            params.temperature = 0.35
            params.maxContextLength = 65_536
            params.responseStyle = .professional
            try parameterStore.saveParameters(params, for: profileID, isLocal: false)
        }

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let saved = try XCTUnwrap(try reopened.modelProfile(id: profileID))
        XCTAssertEqual(saved.displayName, "LAN Qwen")
        XCTAssertGreaterThan(saved.revision, 0)

        let loadedParams = ModelParametersStore(applicationPort: reopened).loadParameters(for: profileID, isLocal: false)
        XCTAssertEqual(loadedParams.temperature, 0.35, accuracy: 0.001)
        XCTAssertEqual(loadedParams.maxContextLength, 65_536)
        XCTAssertEqual(loadedParams.responseStyle, .professional)

        XCTAssertTrue(try reopened.deleteModelProfile(id: profileID))
        XCTAssertNil(try reopened.modelProfile(id: profileID))
    }

    // MARK: - 4. Memory and Data Source CRUD across cold reopen

    func testMemoryAndDataSourceCRUDAcrossColdReopen() throws {
        let root = try makeTempDirectory("memory-sources")
        let database = root.appendingPathComponent("nola-ios.db")

        let savedRevision: Int64
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let memoryStore = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
            let baseline = memoryStore.loadProfile()
            let updatedMemory = try memoryStore.saveMemoryText(
                longTermMemory: "User prefers concise Chinese technical summaries.",
                userPreferences: "Always include verification commands.",
                expected: baseline
            )
            XCTAssertEqual(updatedMemory.longTermMemory, "User prefers concise Chinese technical summaries.")

            let contract: [String: AgentCoreDataSourceJSON] = [
                "sourceId": .string("ios-contract-source"),
                "connectorId": .string("ios.file"),
                "connectorVersion": .number(1),
                "readerDescription": .string("Bounded local file read"),
                "readRecipe": .object([
                    "method": .string("localFileRead"),
                    "operationId": .string("ios.file.read"),
                    "configurationKeys": .array([.string("bookmarkReference")]),
                    "outputSchema": .object([
                        "type": .string("object"),
                        "properties": .object(["title": .object(["type": .string("string")])])
                    ])
                ]),
                "schemaFields": .array([.string("title")]),
                "redactedSample": .string("sample"),
                "recordCount": .number(1),
                "maximumAgeMinutes": .number(60),
                "validationState": .string("verified"),
                "validatedAt": .string("2026-11-27T00:00:00Z"),
                "observedAt": .string("2026-11-27T00:00:00Z")
            ]
            let source = try runtime.saveDataSource(IOSCanonicalDataSourceSubmission(
                draft: IOSCanonicalDataSourceDraft(
                    id: "ios-contract-source",
                    name: "Contract Notes File",
                    kind: .localFile,
                    configuration: IOSCanonicalDataSourceConfiguration(
                        connectorID: "ios.file",
                        locationKind: .localPath,
                        location: "file:///Documents/notes.md",
                        parameters: ["metadataJSON": .string(#"{"bookmarkReference":"ios-contract-source"}"#)],
                        secretReferences: []
                    ),
                    accessPolicy: "conversationOptIn",
                    authorization: IOSCanonicalDataSourceAuthorization(
                        state: .authorized,
                        capability: "filesystem.read",
                        observedAtMs: 1_000
                    ),
                    freshnessPolicy: IOSCanonicalDataSourceFreshnessPolicy(
                        maximumAgeMinutes: 60,
                        allowStaleFallback: false
                    ),
                    capabilities: ["read", "context"],
                    privacy: "local-first",
                    createdAtMs: 1_000,
                    updatedAtMs: 1_000
                ),
                validationContract: contract,
                enabled: true
            ))
            savedRevision = source.revision
            XCTAssertTrue(source.defaultEnabled)
        }

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let reopenedMemory = CanonicalChatMemoryProfileStore(applicationRuntime: reopened).loadProfile()
        XCTAssertEqual(reopenedMemory.longTermMemory, "User prefers concise Chinese technical summaries.")
        XCTAssertEqual(reopenedMemory.userPreferences, "Always include verification commands.")

        let sources = try reopened.listDataSources()
        XCTAssertEqual(sources.count, 1)
        XCTAssertEqual(sources.first?.name, "Contract Notes File")
        XCTAssertTrue(try reopened.deleteDataSource(id: "ios-contract-source", expectedRevision: savedRevision, deletedAtMs: 2_000))
        XCTAssertTrue(try reopened.listDataSources().isEmpty)
    }

    // MARK: - 5. IOSModelConnectionService full flow, protocol switching (.anthropic), and discovery URL

    private final class InMemoryCredentialBox: @unchecked Sendable {
        private let lock = NSLock()
        private var storage: [String: Data] = [:]

        func makeOwner() -> IOSCloudCredentialOwner {
            IOSCloudCredentialOwner(
                save: { [weak self] account, data in
                    guard let self else { return false }
                    self.lock.lock()
                    defer { self.lock.unlock() }
                    guard self.storage[account] == nil else { return false }
                    self.storage[account] = data
                    return true
                },
                read: { [weak self] account in
                    guard let self else { return nil }
                    self.lock.lock()
                    defer { self.lock.unlock() }
                    return self.storage[account]
                },
                delete: { [weak self] account in
                    guard let self else { return }
                    self.lock.lock()
                    defer { self.lock.unlock() }
                    self.storage.removeValue(forKey: account)
                }
            )
        }
    }

    func testModelConnectionServiceLifecycleProtocolSwitchAndAnthropicDiscoveryURL() throws {
        // 5a. AppleModelCatalogDiscovery.request Anthropic /v1/models normalization & headers
        let anthropicRoot = try AppleModelCatalogDiscovery.request(
            endpoint: "https://api.anthropic.com",
            wireProtocol: "anthropic",
            secret: "sk-ant-contract-test"
        )
        XCTAssertEqual(anthropicRoot.url?.absoluteString, "https://api.anthropic.com/v1/models")
        XCTAssertEqual(anthropicRoot.value(forHTTPHeaderField: "anthropic-version"), "2023-06-01")
        XCTAssertEqual(anthropicRoot.value(forHTTPHeaderField: "x-api-key"), "sk-ant-contract-test")
        XCTAssertNil(anthropicRoot.value(forHTTPHeaderField: "Authorization"))

        let anthropicV1 = try AppleModelCatalogDiscovery.request(
            endpoint: "https://api.anthropic.com/v1",
            wireProtocol: "anthropic",
            secret: "sk-ant-contract-test"
        )
        XCTAssertEqual(anthropicV1.url?.absoluteString, "https://api.anthropic.com/v1/models")

        let anthropicSubpath = try AppleModelCatalogDiscovery.request(
            endpoint: "https://gateway.example.invalid/anthropic",
            wireProtocol: "anthropic",
            secret: "sk-ant-contract-test"
        )
        XCTAssertEqual(anthropicSubpath.url?.absoluteString, "https://gateway.example.invalid/anthropic/v1/models")

        let openaiRequest = try AppleModelCatalogDiscovery.request(
            endpoint: "https://api.openai.com/v1",
            wireProtocol: "chat_completions",
            secret: "sk-openai-contract-test"
        )
        XCTAssertEqual(openaiRequest.url?.absoluteString, "https://api.openai.com/v1/models")
        XCTAssertEqual(openaiRequest.value(forHTTPHeaderField: "Authorization"), "Bearer sk-openai-contract-test")
        XCTAssertNil(openaiRequest.value(forHTTPHeaderField: "x-api-key"))

        // 5b. IOSModelConnectionService full lifecycle against isolated temp SQLite store
        let root = try makeTempDirectory("model-connections")
        let database = root.appendingPathComponent("connections.db")
        let credentialBox = InMemoryCredentialBox()
        let credentials = credentialBox.makeOwner()
        let endpoint = "https://models.example.invalid/v1"
        let secret = "sk-ios-contract-secret"

        let createdID: String
        let anthropicProfileID: String
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let service = IOSModelConnectionService(application: runtime, credentials: credentials)

            // create (import_connection + save_account + initial profile)
            let created = try service.create(
                name: "iOS Contract Gateway",
                endpoint: endpoint,
                wireProtocol: "chat_completions",
                secret: secret,
                modelID: "contract-alpha"
            )
            createdID = created.id
            XCTAssertTrue(created.document.connection.enabled)
            XCTAssertEqual(created.document.connection.display_name, "iOS Contract Gateway")
            XCTAssertEqual(created.document.connection.base_url, endpoint)
            XCTAssertEqual(created.document.connection.protocols, ["chat_completions"])

            // list_connections
            let listed = try service.connections()
            XCTAssertTrue(listed.contains(where: { $0.id == created.id }))
            XCTAssertEqual(
                try service.credential(connectionID: created.id, endpoint: endpoint, wireProtocol: "chat_completions").secret,
                secret
            )

            // save_connection via disconnect + reconnect
            let disconnected = try service.disconnect(created)
            XCTAssertFalse(disconnected.document.connection.enabled)
            let reconnected = try service.reconnect(disconnected)
            XCTAssertTrue(reconnected.document.connection.enabled)
            XCTAssertGreaterThan(reconnected.revision, disconnected.revision)

            // refresh_connection_catalog
            let alpha = AppleModelCatalogDiscovery.Model(
                id: "contract-alpha",
                displayName: "Contract Alpha",
                reasoningLevels: ["low", "high"],
                contextWindow: 128_000,
                outputLimit: 8_192,
                modalities: ["text"],
                reasoningSupported: true,
                toolCalls: true,
                samplingParameters: true
            )
            let beta = AppleModelCatalogDiscovery.Model(
                id: "contract-beta",
                displayName: "Contract Beta",
                modalities: ["text"]
            )
            let catalogEntries: [AgentCoreJSONValue] = try [alpha, beta].map {
                try JSONDecoder().decode(AgentCoreJSONValue.self, from: $0.catalogData(wireProtocol: "chat_completions"))
            }
            let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)
            let refreshed: IOSModelConnectionSnapshot = try service.command([
                "type": .string("refresh_connection_catalog"),
                "connection_id": .string(reconnected.id),
                "expected_revision": .signedInteger(reconnected.revision),
                "models": .array(catalogEntries),
                "observed_at_ms": .signedInteger(nowMs),
            ])
            XCTAssertGreaterThan(refreshed.revision, reconnected.revision)
            XCTAssertEqual(refreshed.document.catalog?.models.count, 2)

            // configure_connection_models
            let enabledKey = "chat_completions:contract-alpha"
            try service.enable(refreshed, modelIDs: [enabledKey])
            let enabledKeys = try service.enabledModels(connectionID: refreshed.id)
            XCTAssertTrue(enabledKeys.contains(enabledKey))

            // configureProtocol: .chatCompletions -> .anthropic -> .chatCompletions
            let storedAlpha = try XCTUnwrap(try runtime.listModelProfiles().first(where: { profile in
                profile.enabled && profile.deletedAtMs == nil && profile.modelName == "contract-alpha"
            }))
            let initialModel = try XCTUnwrap(ModelSelectionManager.modelInfo(from: storedAlpha))
            XCTAssertEqual(initialModel.wireProtocol, .chatCompletions)

            let anthropicModel = try service.configureProtocol(for: initialModel, wireProtocol: .anthropic)
            anthropicProfileID = anthropicModel.id
            XCTAssertEqual(anthropicModel.wireProtocol, .anthropic)
            XCTAssertEqual(anthropicModel.baseURL, endpoint)
            XCTAssertEqual(anthropicModel.effectiveUpstreamModelId, "contract-alpha")

            let connectionsAfterSwitch = try service.connections()
            let anthropicConn = try XCTUnwrap(connectionsAfterSwitch.first(where: {
                $0.document.connection.base_url == endpoint && $0.document.connection.protocols == ["anthropic"]
            }))
            XCTAssertEqual(
                try service.credential(connectionID: anthropicConn.id, endpoint: endpoint, wireProtocol: "anthropic").secret,
                secret
            )

            // Switch back from .anthropic -> .chatCompletions
            let switchedBack = try service.configureProtocol(for: anthropicModel, wireProtocol: .chatCompletions)
            XCTAssertEqual(switchedBack.wireProtocol, .chatCompletions)
            XCTAssertEqual(switchedBack.baseURL, endpoint)
            XCTAssertEqual(switchedBack.effectiveUpstreamModelId, "contract-alpha")

            // Switching to the same wire protocol is an idempotent no-op
            let sameProtocol = try service.configureProtocol(for: switchedBack, wireProtocol: .chatCompletions)
            XCTAssertEqual(sameProtocol.id, switchedBack.id)
            XCTAssertEqual(sameProtocol.wireProtocol, .chatCompletions)
        }

        // Cold reopen confirms connections and Anthropic profile persisted in SQLite v2
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let reopenedService = IOSModelConnectionService(application: reopened, credentials: credentials)
        let reopenedConnections = try reopenedService.connections()
        XCTAssertTrue(reopenedConnections.contains(where: { $0.id == createdID }))
        XCTAssertTrue(reopenedConnections.contains(where: { $0.document.connection.protocols == ["anthropic"] }))
        let persistedAnthropic = try XCTUnwrap(try reopened.modelProfile(id: anthropicProfileID))
        let persistedAnthropicInfo = try XCTUnwrap(ModelSelectionManager.modelInfo(from: persistedAnthropic))
        XCTAssertEqual(persistedAnthropicInfo.wireProtocol, .anthropic)
        XCTAssertEqual(persistedAnthropicInfo.baseURL, endpoint)
        XCTAssertEqual(persistedAnthropic.modelName, "contract-alpha")
    }

    // MARK: - 6. Local FFI daemon contracts (activity, today facts, orphan recovery, trace replay, memory jobs, interrupt)

    func testLocalFFIDaemonContractsActivityTodayFactsOrphanReplayMemoryAndInterrupt() throws {
        let root = try makeTempDirectory("ffi-daemon-contracts")
        let database = root.appendingPathComponent("daemon-contracts.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        // Seed a conversation and an automation run
        let convID = "conv-ios-ffi-\(UUID().uuidString.lowercased())"
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let createdConv = try store.createConversation(title: "iOS FFI Activity Check", language: "en")
        let actualConvID = createdConv.id.uuidString

        let autoRunID = "auto-run-\(UUID().uuidString.lowercased())"
        _ = try runtime.enqueueAutomationRun(IOSCanonicalAutomationRun(
            id: autoRunID,
            automationID: "auto-ios-daily",
            definitionRevision: 1,
            occurrenceKey: "2026-11-27T09:00:00Z",
            trigger: "schedule",
            status: "completed",
            attempt: 1,
            workerID: nil,
            leaseToken: nil,
            leaseUntilMs: nil,
            scheduledForMs: nowMs,
            startedAtMs: nowMs,
            finishedAtMs: nowMs + 500,
            nextRetryAtMs: nil,
            resultJSON: #"{"summary":"Completed iOS automation check"}"#,
            errorJSON: nil,
            createdAtMs: nowMs,
            updatedAtMs: nowMs + 500
        ))

        // 6a. list_activity with needs_response: false (must include conversations & automation runs)
        let allActivity = try IOSWorkspaceReadFacts.activity(
            runtime: runtime,
            query: AppleActivityQuery(source: nil, needs_response: false, query: "", cursor: nil, limit: 50)
        )
        XCTAssertTrue(
            allActivity.entries.contains(where: { $0.conversation_id == actualConvID && $0.source == "conversation" }),
            "list_activity(needs_response: false) must include conversation entries"
        )
        XCTAssertTrue(
            allActivity.entries.contains(where: { $0.run_id == autoRunID && $0.source == "automation" }),
            "list_activity(needs_response: false) must include automation runs"
        )

        // 6b. list_activity with source: "automation" (filters strictly to automation runs)
        let automationOnly = try IOSWorkspaceReadFacts.activity(
            runtime: runtime,
            query: AppleActivityQuery(source: "automation", needs_response: false, query: "", cursor: nil, limit: 50)
        )
        XCTAssertFalse(automationOnly.entries.isEmpty)
        XCTAssertTrue(automationOnly.entries.allSatisfy { $0.source == "automation" })
        XCTAssertTrue(automationOnly.entries.contains(where: { $0.run_id == autoRunID }))

        // 6c. load_today_facts
        let todaySnapshot = try IOSWorkspaceReadFacts.snapshot(runtime: runtime)
        XCTAssertTrue(
            todaySnapshot.conversations.contains(where: {
                $0.conversation_id == actualConvID && $0.title == "iOS FFI Activity Check" && $0.revision >= 1
            }),
            "load_today_facts must project the created conversation"
        )
        XCTAssertFalse(todaySnapshot.recent_conversations.isEmpty)

        // 6d. Begin an active run and test interrupt cancellation via reduce_and_commit_active_submission
        let runID = "run-ios-ffi-\(UUID().uuidString.lowercased())"
        let assistantMsgID = "\(runID):assistant"
        let beginResult = try runtime.executeAgentCommand(.object([
            "type": .string("begin_assistant_turn"),
            "input": .object([
                "conversation_id": .string(convID),
                "conversation_title": .string("Interruptible Run"),
                "user_message_id": .string("\(runID):user"),
                "assistant_message_id": .string(assistantMsgID),
                "user_text": .string("Start a run that will be interrupted"),
                "created_at_ms": .signedInteger(nowMs),
                "core_config": .object([
                    "session_id": .string(convID),
                    "model": .string("qwen3.8-flash-next-iq3_s"),
                ]),
                "submission": .object([
                    "submission_id": .string(runID),
                    "type": .string("user_message"),
                ]),
            ]),
        ]))
        XCTAssertNotEqual(beginResult, .null)

        let interruptResult = try runtime.executeAgentCommand(.object([
            "type": .string("reduce_and_commit_active_submission"),
            "input": .object([
                "conversation_id": .string(convID),
                "run_id": .string(runID),
                "transition_id": .string("tx-interrupt-\(UUID().uuidString.lowercased())"),
                "submission": .object([
                    "type": .string("interrupt"),
                    "cancellation_id": .string("cancel-ios-1"),
                    "reason": .string("user_stop"),
                ]),
            ]),
        ]))
        let controlFields = try XCTUnwrap(interruptResult.objectFields?["control"]?.objectFields)
        XCTAssertEqual(controlFields["turn_status"]?.stringScalar, "cancelled")

        // 6e. load_exact_run_trace_replay
        let tracePage = try runtime.executeAgentCommand(.object([
            "type": .string("load_exact_run_trace_replay"),
            "conversation_id": .string(convID),
            "run_id": .string(runID),
            "assistant_message_id": .string(assistantMsgID),
            "after_sequence": .signedInteger(0),
            "limit": .signedInteger(256),
        ]))
        let traceFields = try XCTUnwrap(tracePage.objectFields)
        XCTAssertEqual(Set(traceFields.keys), ["trace_id", "through_sequence", "next_sequence", "events"])
        XCTAssertFalse((traceFields["trace_id"]?.stringScalar ?? "").isEmpty)
        if case .array = traceFields["events"] {
            // Valid array payload
        } else {
            XCTFail("Expected events array in load_exact_run_trace_replay response")
        }

        // 6f. recover_observed_orphaned_run
        let orphanResult = try runtime.executeAgentCommand(.object([
            "type": .string("recover_observed_orphaned_run"),
            "conversation_id": .string(convID),
            "expected_run_id": .string(runID),
            "interrupted_at_ms": .signedInteger(nowMs + 1_000),
        ]))
        let orphanFields = try XCTUnwrap(orphanResult.objectFields)
        XCTAssertEqual(orphanFields["ok"], .bool(true))
        XCTAssertEqual(orphanFields["recovered"], .bool(true))
        XCTAssertEqual(orphanFields["run_id"]?.stringScalar, runID)
        XCTAssertEqual(orphanFields["status"]?.stringScalar, "interrupted")

        // 6g. query_claimable_memory_jobs
        let claimableJobs = try runtime.executeAgentCommand(.object([
            "type": .string("query_claimable_memory_jobs"),
            "input": .object([
                "conversation_id": .string(convID),
                "now_epoch_ms": .signedInteger(nowMs + 2_000),
            ]),
        ]))
        if case .array(let jobs) = claimableJobs {
            XCTAssertTrue(jobs.isEmpty)
        } else {
            XCTFail("Expected JSON array from query_claimable_memory_jobs, got \(claimableJobs)")
        }
    }

    // MARK: - 7. OpenAIProvider Anthropic wire protocol validation & request building

    func testOpenAIProviderAnthropicWireProtocolValidationAndRequestBuilding() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://api.anthropic.com"))
        let fullCaps = ModelExecutionCapabilities(
            toolCalls: true,
            imageInput: true,
            reasoning: true,
            samplingParameters: true
        )

        // 7a. validateGenerationConfiguration accepts .anthropic (no longer rejected)
        let validConfig = OpenAIProvider.Config(
            baseURL: baseURL,
            apiKey: "sk-ant-test-key",
            model: "claude-sonnet-4-5",
            wireProtocol: .anthropic,
            executionCapabilities: fullCaps,
            temperature: 0.4,
            topP: 0.9,
            maxCompletionTokens: 2_048,
            presencePenalty: 0,
            frequencyPenalty: 0,
            seed: nil,
            stopSequences: ["END_MARKER"]
        )
        XCTAssertNoThrow(try OpenAIProvider.validateGenerationConfiguration(validConfig))

        // Unsupported Anthropic parameters (presencePenalty / frequencyPenalty / seed) remain rejected
        var badPresence = validConfig
        badPresence.presencePenalty = 0.5
        XCTAssertThrowsError(try OpenAIProvider.validateGenerationConfiguration(badPresence))

        var badSeed = validConfig
        badSeed.seed = 42
        XCTAssertThrowsError(try OpenAIProvider.validateGenerationConfiguration(badSeed))

        // 7b. agentCapabilities reflects .anthropic execution capabilities
        let provider = OpenAIProvider(config: validConfig)
        let caps = provider.agentCapabilities
        XCTAssertTrue(caps.streamText)
        XCTAssertTrue(caps.streamReasoning)
        XCTAssertTrue(caps.nativeToolCalls)
        XCTAssertTrue(caps.visionInput)
        XCTAssertTrue(caps.reasoning)
        XCTAssertEqual(caps.maxOutputTokens, 2_048)

        // 7c. anthropicURL normalization across root, /v1, subpath, and OpenAI-style endpoints
        XCTAssertEqual(try provider.anthropicURL().absoluteString, "https://api.anthropic.com/v1/messages")

        let v1Provider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: try XCTUnwrap(URL(string: "https://api.anthropic.com/v1")),
            apiKey: "sk-ant-test-key",
            model: "claude-sonnet-4-5",
            wireProtocol: .anthropic
        ))
        XCTAssertEqual(try v1Provider.anthropicURL().absoluteString, "https://api.anthropic.com/v1/messages")

        let subpathProvider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: try XCTUnwrap(URL(string: "https://gateway.example.invalid/anthropic")),
            apiKey: "sk-ant-test-key",
            model: "claude-sonnet-4-5",
            wireProtocol: .anthropic
        ))
        XCTAssertEqual(try subpathProvider.anthropicURL().absoluteString, "https://gateway.example.invalid/anthropic/v1/messages")

        let chatPathProvider = OpenAIProvider(config: OpenAIProvider.Config(
            baseURL: try XCTUnwrap(URL(string: "https://gateway.example.invalid/v1/chat/completions")),
            apiKey: "sk-ant-test-key",
            model: "claude-sonnet-4-5",
            wireProtocol: .anthropic
        ))
        XCTAssertEqual(try chatPathProvider.anthropicURL().absoluteString, "https://gateway.example.invalid/v1/messages")

        // 7d. anthropicRequest builds Anthropic Messages API headers + converted tool/message body
        let coreEnvelope: [String: Any] = [
            "system_prompt": "You are a concise assistant.",
            "messages": [
                ["role": "user", "content": "Check weather in Shanghai"],
                [
                    "role": "assistant",
                    "content": "Checking weather.",
                    "tool_calls": [
                        [
                            "id": "call_weather_1",
                            "type": "function",
                            "function": [
                                "name": "get_weather",
                                "arguments": #"{"city":"Shanghai"}"#,
                            ],
                        ]
                    ],
                ],
                [
                    "role": "tool",
                    "tool_call_id": "call_weather_1",
                    "content": #"{"temp_c":22}"#,
                ],
            ],
            "tools": [
                [
                    "type": "function",
                    "function": [
                        "name": "get_weather",
                        "description": "Lookup current weather",
                        "parameters": [
                            "type": "object",
                            "properties": ["city": ["type": "string"]],
                            "required": ["city"],
                        ],
                    ],
                ]
            ],
            "tool_choice": "auto",
        ]
        let envelopeData = try JSONSerialization.data(withJSONObject: coreEnvelope)
        let envelopeText = String(decoding: envelopeData, as: UTF8.self)
        let input = ModelInput(
            dateRange: .init(start: "", end: ""),
            candidates: [],
            userPrefs: [
                "prepared_core_payload": "true",
                "prepared_core_payload_json": envelopeText,
            ]
        )

        let request = try provider.anthropicRequest(input: input)
        XCTAssertEqual(request.httpMethod, "POST")
        XCTAssertEqual(request.url?.absoluteString, "https://api.anthropic.com/v1/messages")
        XCTAssertEqual(request.value(forHTTPHeaderField: "anthropic-version"), "2023-06-01")
        XCTAssertEqual(request.value(forHTTPHeaderField: "x-api-key"), "sk-ant-test-key")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "text/event-stream")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Content-Type"), "application/json")
        XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))

        let bodyData = try XCTUnwrap(request.httpBody)
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: bodyData) as? [String: Any])
        XCTAssertEqual(body["model"] as? String, "claude-sonnet-4-5")
        XCTAssertEqual(body["max_tokens"] as? Int, 2_048)
        XCTAssertEqual(body["stream"] as? Bool, true)
        XCTAssertEqual(body["system"] as? String, "You are a concise assistant.")
        XCTAssertEqual(body["stop_sequences"] as? [String], ["END_MARKER"])
        XCTAssertEqual(body["temperature"] as? Double ?? -1, 0.4, accuracy: 0.001)
        XCTAssertNil(body["top_p"], "Anthropic requests must not send top_p alongside temperature")

        let tools = try XCTUnwrap(body["tools"] as? [[String: Any]])
        XCTAssertEqual(tools.count, 1)
        XCTAssertEqual(tools[0]["name"] as? String, "get_weather")
        XCTAssertNotNil(tools[0]["input_schema"])
        let toolChoice = try XCTUnwrap(body["tool_choice"] as? [String: Any])
        XCTAssertEqual(toolChoice["type"] as? String, "auto")

        let messages = try XCTUnwrap(body["messages"] as? [[String: Any]])
        XCTAssertEqual(messages.count, 3)
        XCTAssertEqual(messages[0]["role"] as? String, "user")
        XCTAssertEqual(messages[1]["role"] as? String, "assistant")
        let assistantBlocks = try XCTUnwrap(messages[1]["content"] as? [[String: Any]])
        XCTAssertTrue(assistantBlocks.contains(where: { $0["type"] as? String == "tool_use" && $0["id"] as? String == "call_weather_1" }))
        XCTAssertEqual(messages[2]["role"] as? String, "user")
        let toolResultBlocks = try XCTUnwrap(messages[2]["content"] as? [[String: Any]])
        XCTAssertTrue(toolResultBlocks.contains(where: { $0["type"] as? String == "tool_result" && $0["tool_use_id"] as? String == "call_weather_1" }))
    }
}
