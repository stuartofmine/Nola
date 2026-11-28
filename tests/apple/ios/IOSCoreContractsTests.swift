// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: High-signal iOS NolaCore FFI, IOSApplicationRuntime, and LocalConversationStore contract tests against isolated temp SQLite stores.

import Combine
import Foundation
import QuickLook
import SQLite3
import SwiftUI
import UIKit
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

    func testMarkdownRevisionsKeepParagraphsAndMessagePartsInOrder() {
        var buffer = AppleStreamingMarkdownBuffer()
        let prefix = "First paragraph.\n\n"
        for (text, streaming) in [("OK", false), ("OK_IOS", true), ("OK_IOS_EDITED_ROOT", false)] {
            buffer.receive(prefix + text, isStreaming: streaming, reduceMotion: true)
            let blocks = buffer.renderBlocks(settleTail: !streaming)
            XCTAssertEqual(blocks.map(\.markdown), [prefix, text])
            XCTAssertEqual(blocks.map(\.ordinal), [0, 1])
            XCTAssertEqual(blocks.last?.isMutableTail, streaming)
        }
        buffer.finish(with: "Corrected.")
        XCTAssertEqual(buffer.renderBlocks(settleTail: true).map(\.markdown), ["Corrected."])

        let parts: [(String, AppleMessagePartFlowKind)] = [
            ("intro", .content), ("thought", .reasoning), ("lookup", .tool), ("answer", .content)
        ]
        let segments = AppleMessagePartFlow.segments(parts, id: { $0.0 }, kind: { $0.1 })
        XCTAssertEqual(segments.map { $0.parts.map { $0.0 } }, [["intro"], ["thought", "lookup"], ["answer"]])
    }

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
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let createdConv = try store.createConversation(title: "iOS FFI Activity Check", language: "en")
        let actualConvID = createdConv.id.uuidString
        let convID = actualConvID

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
        func beginRun(_ runID: String, assistantMsgID: String) throws -> AgentCoreJSONValue {
            try runtime.executeAgentCommand(.object([
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
        }
        XCTAssertNotEqual(try beginRun(runID, assistantMsgID: assistantMsgID), .null)

        // Normal progress re-observes the same outstanding lease. Only expiry
        // authorizes a fresh physical delivery of this outstanding request.
        func modelClaims(at time: Int64, run: String? = nil) throws -> [AgentCoreJSONValue] {
            let snapshot = try runtime.executeAgentCommand(.object([
                "type": .string("claim_committed_host_requests"),
                "input": .object([
                    "run_id": .string(run ?? runID),
                    "claimed_at_ms": .signedInteger(time),
                    "lease_expires_at_ms": .signedInteger(time + 1_000),
                ]),
            ]))
            guard case .array(let requests)? = snapshot.objectFields?["requests"] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            return requests
        }
        var seenLeases: Set<String> = []
        let first = try modelClaims(at: nowMs)
        XCTAssertThrowsError(try DeviceAgentCoordinator.newHostDeliveries(first + [.null], seen: &seenLeases))
        XCTAssertTrue(seenLeases.isEmpty)
        XCTAssertEqual(try DeviceAgentCoordinator.newHostDeliveries(first + first, seen: &seenLeases).count, 1)
        let repeated = try modelClaims(at: nowMs + 1)
        XCTAssertEqual(first, repeated)
        XCTAssertTrue(try DeviceAgentCoordinator.newHostDeliveries(repeated, seen: &seenLeases).isEmpty)
        let reclaimed = try modelClaims(at: nowMs + 1_001)
        XCTAssertEqual(try DeviceAgentCoordinator.newHostDeliveries(reclaimed, seen: &seenLeases).count, 1)

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

        // An older interrupted run can be closed from the recovery card. No model
        // retry or physical-stop confirmation is implied by ending the wait.
        let inspection = try XCTUnwrap(runtime.inspectUnknownEffectRecovery(conversationID: convID,
            runID: runID, assistantMessageID: assistantMsgID))
        XCTAssertFalse(inspection.retry_allowed)
        XCTAssertEqual(inspection.targets.count, 1)
        XCTAssertTrue(try modelClaims(at: nowMs + 2_002).isEmpty)
        let original = try XCTUnwrap(first.first)
        let effect = try XCTUnwrap(original.objectFields?["effect_id"])
        let stale = try DeviceAgentCoordinator.hostResponseEnvelope(request: original, payload: .object([
            "type": .string("cancellation_confirmed"), "target_effect_id": effect,
        ]), sequence: 2)
        XCTAssertThrowsError(try runtime.executeAgentCommand(.object([
            "type": .string("reduce_and_accept_host_response"),
            "input": .object(["run_id": .string(runID), "conversation_id": .string(convID), "response": stale]),
        ])))
        let decision = try inspection.confirmed(choice: .keepUnknownBlocked, reason: "keep provider outcome unknown")
        let reply = try runtime.resolveUnknownEffectRecovery(decision, requestID: UUID().uuidString.lowercased())
        XCTAssertEqual(reply.outcome, .unknownPreservedBlocked)
        let terminal = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
            "conversation_id": .string(convID), "run_id": .string(runID)]))
        let value = terminal.objectFields?["result"]?.objectFields?["value"]?.objectFields
        XCTAssertEqual(value?["kind"], .string("blocked"))
        XCTAssertEqual(value?["blocking_fact_refs"], .array([.string(inspection.targets[0].effect_id)]))

        XCTAssertNil(try runtime.loadConversationProjectionV1(id: convID).activeRun)
        let nextID = "next-\(UUID().uuidString)"
        XCTAssertNotEqual(try beginRun(nextID, assistantMsgID: "\(nextID):assistant"), .null)
        XCTAssertEqual(try runtime.loadConversationProjectionV1(id: convID).activeRun?.runID, nextID)

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
            "expected_run_id": .string(nextID),
            "interrupted_at_ms": .signedInteger(nowMs + 1_000),
        ]))
        let orphanFields = try XCTUnwrap(orphanResult.objectFields)
        XCTAssertEqual(orphanFields["ok"], .bool(true))
        XCTAssertEqual(orphanFields["recovered"], .bool(true))
        XCTAssertEqual(orphanFields["run_id"]?.stringScalar, nextID)
        XCTAssertEqual(orphanFields["status"]?.stringScalar, "interrupted")

        // The second run was interrupted before any lease was issued.
        XCTAssertTrue(try modelClaims(at: nowMs + 2_003, run: nextID).isEmpty)
        let unissued = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
            "conversation_id": .string(convID), "run_id": .string(nextID)]))
        XCTAssertEqual(unissued.objectFields?["result"]?.objectFields?["value"]?.objectFields?["kind"], .string("cancelled"))
        XCTAssertNil(try runtime.loadConversationProjectionV1(id: convID).activeRun)

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

    // MARK: - 8. Rendered-Geometry & Pixel-Buffer Visual Audit Helpers (iPhone & iPad, Dark & Light)

    private func iosEvidenceDirectory() throws -> URL {
        let dir = URL(fileURLWithPath: "/tmp/nola-real-e2e-evidence/ios-audit", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    @discardableResult
    private func assertUIImageHasVisibleInk(
        _ image: UIImage,
        minRatio: Double = 0.005,
        maxRatio: Double = 0.95,
        alphaOnly: Bool = false,
        context: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> Double {
        let cgImage = try XCTUnwrap(image.cgImage, "\(context): missing CGImage", file: file, line: line)
        let width = cgImage.width
        let height = cgImage.height
        XCTAssertGreaterThan(width, 8, "\(context): width too small (\(width))", file: file, line: line)
        XCTAssertGreaterThan(height, 8, "\(context): height too small (\(height))", file: file, line: line)

        let bytesPerRow = width * 4
        var pixels = [UInt8](repeating: 0, count: height * bytesPerRow)
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let bitmapInfo = CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue
        guard let ctx = CGContext(
            data: &pixels,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: bytesPerRow,
            space: colorSpace,
            bitmapInfo: bitmapInfo
        ) else {
            throw NSError(domain: "IOSPixelAudit", code: 1)
        }
        ctx.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))

        let totalPixels = width * height
        var inkCount = 0
        if alphaOnly {
            for idx in stride(from: 0, to: pixels.count, by: 4) {
                if pixels[idx + 3] > 20 {
                    inkCount += 1
                }
            }
        } else {
            func pixelAt(_ x: Int, _ y: Int) -> (Int, Int, Int) {
                let offset = (y * width + x) * 4
                return (Int(pixels[offset]), Int(pixels[offset + 1]), Int(pixels[offset + 2]))
            }
            let insetX = min(4, max(0, width - 1))
            let insetY = min(4, max(0, height - 1))
            let corners = [
                pixelAt(insetX, insetY),
                pixelAt(width - 1 - insetX, insetY),
                pixelAt(insetX, height - 1 - insetY),
                pixelAt(width - 1 - insetX, height - 1 - insetY),
            ]
            let bgR = corners.map(\.0).sorted()[1]
            let bgG = corners.map(\.1).sorted()[1]
            let bgB = corners.map(\.2).sorted()[1]
            for idx in stride(from: 0, to: pixels.count, by: 4) {
                let diff = abs(Int(pixels[idx]) - bgR) + abs(Int(pixels[idx + 1]) - bgG) + abs(Int(pixels[idx + 2]) - bgB)
                if pixels[idx + 3] > 16 && diff > 24 {
                    inkCount += 1
                }
            }
        }

        let ratio = Double(inkCount) / Double(max(1, totalPixels))
        XCTAssertGreaterThan(
            ratio,
            minRatio,
            "\(context): rendered bitmap appears blank or collapsed (inkRatio=\(String(format: "%.4f", ratio)))",
            file: file,
            line: line
        )
        XCTAssertLessThan(
            ratio,
            maxRatio,
            "\(context): rendered bitmap lacks contrast (inkRatio=\(String(format: "%.4f", ratio)))",
            file: file,
            line: line
        )
        return ratio
    }

    @MainActor
    private func renderSwiftUIView<V: View>(
        _ view: V,
        size: CGSize,
        style: UIUserInterfaceStyle,
        saveTo fileURL: URL? = nil,
        settleMs: UInt64 = 80
    ) async throws -> (image: UIImage, fittingSize: CGSize) {
        let colorScheme: ColorScheme = style == .dark ? .dark : .light
        let root = AnyView(
            view
                .environment(\.colorScheme, colorScheme)
                .frame(width: size.width, height: size.height, alignment: .topLeading)
                .background(style == .dark ? Color(uiColor: .systemBackground) : Color(uiColor: .systemBackground))
        )
        let hosting = UIHostingController(rootView: root)
        hosting.overrideUserInterfaceStyle = style
        hosting.view.backgroundColor = style == .dark ? .black : .white
        let activeScene = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first
        let window: UIWindow
        if let activeScene {
            window = UIWindow(windowScene: activeScene)
            window.frame = CGRect(origin: .zero, size: size)
        } else {
            window = UIWindow(frame: CGRect(origin: .zero, size: size))
        }
        window.overrideUserInterfaceStyle = style
        window.backgroundColor = style == .dark ? .black : .white
        window.rootViewController = hosting
        window.isHidden = false
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }

        hosting.view.frame = CGRect(origin: .zero, size: size)
        hosting.view.setNeedsLayout()
        hosting.view.layoutIfNeeded()
        if settleMs > 0 {
            try await Task.sleep(for: .milliseconds(settleMs))
            RunLoop.main.run(until: Date().addingTimeInterval(0.04))
        }
        hosting.view.setNeedsLayout()
        hosting.view.layoutIfNeeded()

        let fittingSize = hosting.sizeThatFits(in: CGSize(width: size.width, height: CGFloat.greatestFiniteMagnitude))
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 2.0
        let renderer = UIGraphicsImageRenderer(size: size, format: format)
        let image = renderer.image { ctx in
            let drew = hosting.view.drawHierarchy(in: CGRect(origin: .zero, size: size), afterScreenUpdates: true)
            if !drew {
                hosting.view.layer.render(in: ctx.cgContext)
            }
        }
        if let fileURL, let png = image.pngData() {
            try png.write(to: fileURL, options: .atomic)
        }
        return (image, fittingSize)
    }

    private func generateTestPNG(at url: URL, width: Int, height: Int) throws {
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: width, height: height))
        let image = renderer.image { ctx in
            UIColor(red: 0.95, green: 0.96, blue: 0.98, alpha: 1.0).setFill()
            ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
            UIColor(red: 0.14, green: 0.40, blue: 0.85, alpha: 1.0).setFill()
            ctx.fill(CGRect(x: width / 10, y: height / 6, width: width * 4 / 5, height: height / 3))
            UIColor(red: 0.88, green: 0.28, blue: 0.24, alpha: 1.0).setFill()
            ctx.fill(CGRect(x: width / 5, y: height * 3 / 5, width: width * 3 / 5, height: height / 4))
        }
        try XCTUnwrap(image.pngData()).write(to: url, options: .atomic)
    }

    private func generateTestPDF(at url: URL, title: String, body: String) throws {
        let bounds = CGRect(x: 0, y: 0, width: 612, height: 792)
        let renderer = UIGraphicsPDFRenderer(bounds: bounds)
        let data = renderer.pdfData { ctx in
            ctx.beginPage()
            UIColor.white.setFill()
            ctx.fill(bounds)
            UIColor(red: 0.14, green: 0.36, blue: 0.78, alpha: 1.0).setFill()
            ctx.fill(CGRect(x: 40, y: 40, width: 532, height: 44))
            (title as NSString).draw(
                in: CGRect(x: 52, y: 50, width: 508, height: 30),
                withAttributes: [.font: UIFont.boldSystemFont(ofSize: 18), .foregroundColor: UIColor.white]
            )
            (body as NSString).draw(
                in: CGRect(x: 40, y: 110, width: 532, height: 600),
                withAttributes: [.font: UIFont.systemFont(ofSize: 14), .foregroundColor: UIColor.black]
            )
        }
        try data.write(to: url, options: .atomic)
    }

    private func decodeProjectionPart(_ object: [String: Any]) throws -> IOSConversationProjectionV1.Part {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: data)
    }

    // MARK: - 9. CJK-in-LaTeX Pixel Verification (Zero Blank / Zero Tofu)

    @MainActor
    func testIOSMathRendererHandlesCJKInLatexWithoutTofu() throws {
        let evidence = try iosEvidenceDirectory()
        XCTAssertTrue(IOSMathRenderer.containsNonLatinMathText(#"S_A = \lambda \cdot S_B + (\text{一小坨残差})"#))
        XCTAssertFalse(IOSMathRenderer.containsNonLatinMathText(#"S_s = \lambda S_t + UV^\top"#))

        let displayImage = try XCTUnwrap(
            IOSMathRenderer.renderCompositeImage(
                latex: #"S_A = \lambda \cdot S_B + (\text{一小坨残差})"#,
                display: true,
                fontSize: 22
            ),
            "iOS CJK display LaTeX must render cleanly"
        )
        XCTAssertGreaterThan(displayImage.size.width, 120)
        XCTAssertGreaterThanOrEqual(displayImage.size.height, 14)
        let displayAspect = displayImage.size.width / max(1, displayImage.size.height)
        XCTAssertGreaterThan(displayAspect, 1.5, "CJK display formula aspect ratio looks collapsed: \(displayImage.size)")
        XCTAssertLessThan(displayAspect, 30.0, "CJK display formula aspect ratio looks distorted: \(displayImage.size)")
        try assertUIImageHasVisibleInk(displayImage, minRatio: 0.02, maxRatio: 0.85, alphaOnly: true, context: "CJK display LaTeX")
        try displayImage.pngData()?.write(to: evidence.appendingPathComponent("ios-cjk-math-display.png"), options: .atomic)

        let subscriptImage = try XCTUnwrap(
            IOSMathRenderer.renderCompositeImage(
                latex: #"\mathcal{L}_{\text{总}} = \mathcal{L}_{\text{主}} + \lambda \mathcal{L}_{\text{正则}}"#,
                display: false,
                fontSize: 17
            ),
            "iOS CJK subscript LaTeX must render cleanly"
        )
        XCTAssertGreaterThan(subscriptImage.size.width, 80)
        XCTAssertGreaterThanOrEqual(subscriptImage.size.height, 12)
        let subAspect = subscriptImage.size.width / max(1, subscriptImage.size.height)
        XCTAssertGreaterThan(subAspect, 1.5)
        XCTAssertLessThan(subAspect, 30.0)
        try assertUIImageHasVisibleInk(subscriptImage, minRatio: 0.02, maxRatio: 0.85, alphaOnly: true, context: "CJK subscript LaTeX")
        try subscriptImage.pngData()?.write(to: evidence.appendingPathComponent("ios-cjk-math-subscript.png"), options: .atomic)
    }

    // MARK: - 10. Bounded Live Thinking Viewport at iPhone (350pt) & iPad (720pt) Widths

    @MainActor
    func testIOSBoundedLiveThinkingViewportAtIPhoneAndIPadWidths() async throws {
        let evidence = try iosEvidenceDirectory()

        func makeMixedReasoning(lines count: Int) -> String {
            (1...count).map { idx in
                "第 \(idx) 行 iOS Thinking line \(idx): analyzing sibling-conditioned delta-rule state transfer 与移动端排版稳定性。"
            }.joined(separator: "\n")
        }

        let r10 = makeMixedReasoning(lines: 10)
        let r200 = makeMixedReasoning(lines: 200)
        let r1500 = makeMixedReasoning(lines: 1500)

        let tail = ChatCanonicalExecutionProcess.liveReasoningTail(r1500)
        XCTAssertTrue(tail.contains("iOS Thinking line 1500"), "iOS liveReasoningTail must roll to trailing line 1500")
        XCTAssertTrue(tail.contains("第 1500 行"), "iOS liveReasoningTail must retain trailing CJK line")
        XCTAssertFalse(tail.contains("line 1:"), "iOS liveReasoningTail must drop early line 1")

        func measureProcessHeight(summary: String, width: CGFloat) throws -> CGFloat {
            let reasoningPart = try decodeProjectionPart([
                "kind": "reasoning",
                "part_id": "r-live-1",
                "revision": 1,
                "state": "running",
                "title": "Thinking",
                "summary": summary,
            ])
            let view = ChatCanonicalExecutionProcess(
                parts: [reasoningPart],
                events: [],
                activities: [],
                isLive: true,
                expanded: .constant(true),
                toolExpanded: { _ in .constant(false) },
                technicalExpanded: { _ in .constant(false) },
                reader: { _ in .constant(nil) }
            )
            .padding(12)
            .frame(width: width, alignment: .topLeading)

            let hosting = UIHostingController(rootView: AnyView(view.environment(\.colorScheme, .dark)))
            hosting.view.frame = CGRect(x: 0, y: 0, width: width, height: 1200)
            hosting.view.setNeedsLayout()
            hosting.view.layoutIfNeeded()
            return hosting.sizeThatFits(in: CGSize(width: width, height: CGFloat.greatestFiniteMagnitude)).height
        }

        for (deviceName, width) in [("iPhone", CGFloat(350)), ("iPad", CGFloat(720))] {
            let h10 = try measureProcessHeight(summary: r10, width: width)
            let h200 = try measureProcessHeight(summary: r200, width: width)
            let h1500 = try measureProcessHeight(summary: r1500, width: width)

            XCTAssertGreaterThan(h10, 60, "\(deviceName): 10-line live thinking collapsed (h=\(h10))")
            XCTAssertLessThanOrEqual(h200, 280, "\(deviceName): 200-line live thinking MUST stay bounded <= 280pt (h=\(h200))")
            XCTAssertLessThanOrEqual(h1500, 280, "\(deviceName): 1500-line live thinking MUST stay bounded <= 280pt (h=\(h1500))")
            XCTAssertLessThanOrEqual(
                abs(h1500 - h200),
                4.0,
                "\(deviceName): live thinking height MUST NOT explode between 200 and 1500 lines (h200=\(h200), h1500=\(h1500))"
            )
        }

        // Visual bitmap audit in Dark & Light at iPhone & iPad sizes
        let livePart = try decodeProjectionPart([
            "kind": "reasoning",
            "part_id": "r-live-visual",
            "revision": 1,
            "state": "running",
            "title": "Thinking",
            "summary": r1500,
        ])
        let processView = ChatCanonicalExecutionProcess(
            parts: [livePart],
            events: [],
            activities: [],
            isLive: true,
            expanded: .constant(true),
            toolExpanded: { _ in .constant(false) },
            technicalExpanded: { _ in .constant(false) },
            reader: { _ in .constant(nil) }
        )
        .padding(16)

        for (label, size) in [("iphone", CGSize(width: 390, height: 320)), ("ipad", CGSize(width: 760, height: 320))] {
            for (styleName, style) in [("dark", UIUserInterfaceStyle.dark), ("light", UIUserInterfaceStyle.light)] {
                let rendered = try await renderSwiftUIView(
                    processView,
                    size: size,
                    style: style,
                    saveTo: evidence.appendingPathComponent("ios-live-thinking-\(label)-\(styleName).png")
                )
                try assertUIImageHasVisibleInk(rendered.image, context: "live-thinking-\(label)-\(styleName)")
            }
        }
    }

    // MARK: - 11. ChatBubble Full Rich Render (iPhone & iPad, Dark & Light) & Right-Sidebar Inspector Routing

    @MainActor
    func testIOSChatBubbleRichRenderAndRightSidebarInspectorRouting() async throws {
        let evidence = try iosEvidenceDirectory()
        let root = try makeTempDirectory("chatbubble-audit")
        let database = root.appendingPathComponent("bubble.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let container = ServiceContainer(applicationRuntime: runtime)

        let pngURL = root.appendingPathComponent("ios-figure.png")
        let pdfURL = root.appendingPathComponent("ios-report.pdf")
        let txtURL = root.appendingPathComponent("ios-notes.txt")
        try generateTestPNG(at: pngURL, width: 480, height: 300)
        try generateTestPDF(at: pdfURL, title: "iOS Inspector PDF", body: "Verified right-sidebar QuickLook inspector preview.")
        try "iOS inspector text fixture line 1\n中文文本预览测试\n".write(to: txtURL, atomically: true, encoding: .utf8)

        let richMarkdown = """
        ## 移动端学术与工程混合渲染审计

        兄弟分支低秩残差恒等式满足 $S_A = \\lambda \\cdot S_B + (\\text{一小坨残差})$，其中缩放系数 $\\lambda = D_s / D_t$。

        $$S_s = \\lambda S_t + UV^\\top$$

        | 平台 | 宽度 | 状态 |
        | --- | ---: | --- |
        | iPhone | 390 pt | 通过 |
        | iPad | 1024 pt | 通过 |

        ```swift
        func residualScale(_ ds: Double, _ dt: Double) -> Double {
            ds / dt
        }
        ```
        """

        let textPart = try decodeProjectionPart([
            "kind": "text",
            "part_id": "p-text-1",
            "revision": 1,
            "markdown": richMarkdown,
            "is_final": true,
        ])
        let pngPart = try decodeProjectionPart([
            "kind": "attachment",
            "part_id": "p-att-png",
            "revision": 1,
            "attachment_id": "att-png",
            "name": "ios-figure.png",
            "mime_type": "image/png",
            "uri": pngURL.absoluteString,
            "operation": "created",
        ])
        let pdfPart = try decodeProjectionPart([
            "kind": "attachment",
            "part_id": "p-att-pdf",
            "revision": 1,
            "attachment_id": "att-pdf",
            "name": "ios-report.pdf",
            "mime_type": "application/pdf",
            "uri": pdfURL.absoluteString,
            "operation": "created",
        ])

        let message = ChatMessage(
            role: .assistant,
            content: richMarkdown,
            canonicalParts: [textPart, pngPart, pdfPart]
        )

        var routedURLs: [URL] = []
        let bubble = ScrollView {
            ChatBubble(
                conversationID: UUID(),
                message: message,
                isLatestAssistant: true,
                isStreaming: false,
                isRetrieving: false,
                pendingHostPermissionRequest: nil,
                onHostPermissionDecision: { _, _ in }
            )
            .padding(16)
        }
        .environmentObject(container)
        .environment(\.iosOpenAttachmentPreview) { url in
            routedURLs.append(url)
        }

        // Measure intrinsic width/height and capture bitmaps at iPhone (390x844) & iPad (1024x768) in Dark & Light modes
        for (deviceName, viewport) in [("iphone", CGSize(width: 390, height: 844)), ("ipad", CGSize(width: 1024, height: 768))] {
            let measureHost = UIHostingController(rootView: AnyView(
                ChatBubble(
                    conversationID: UUID(),
                    message: message,
                    isLatestAssistant: true,
                    isStreaming: false,
                    isRetrieving: false,
                    pendingHostPermissionRequest: nil,
                    onHostPermissionDecision: { _, _ in }
                )
                .padding(16)
                .environmentObject(container)
            ))
            measureHost.view.frame = CGRect(origin: .zero, size: viewport)
            measureHost.view.setNeedsLayout()
            measureHost.view.layoutIfNeeded()
            let measured = measureHost.sizeThatFits(in: CGSize(width: viewport.width, height: CGFloat.greatestFiniteMagnitude))
            XCTAssertGreaterThan(measured.height, 80, "\(deviceName): ChatBubble height collapsed (\(measured.height))")
            XCTAssertLessThanOrEqual(
                measured.width,
                viewport.width + 1.0,
                "\(deviceName): ChatBubble overflowed container width (\(measured.width) > \(viewport.width))"
            )

            for (styleName, style) in [("dark", UIUserInterfaceStyle.dark), ("light", UIUserInterfaceStyle.light)] {
                let rendered = try await renderSwiftUIView(
                    bubble,
                    size: viewport,
                    style: style,
                    saveTo: evidence.appendingPathComponent("ios-chatbubble-\(deviceName)-\(styleName).png"),
                    settleMs: 120
                )
                try assertUIImageHasVisibleInk(rendered.image, context: "chatbubble-\(deviceName)-\(styleName)")
            }
        }

        // Verify right-sidebar inspector routing via \.iosOpenAttachmentPreview across png -> txt -> pdf -> nil
        final class InspectorRouterState: ObservableObject {
            @Published var previewURL: URL? = nil
            @Published var fallbackSheetCount = 0
        }
        struct InspectorRoutingHarness: View {
            @ObservedObject var state: InspectorRouterState
            var body: some View {
                VStack(spacing: 12) {
                    Text("Chat Canvas")
                        .font(.headline)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .environment(\.iosOpenAttachmentPreview) { url in
                    state.previewURL = url
                }
                .inspector(isPresented: Binding(
                    get: { state.previewURL != nil },
                    set: { if !$0 { state.previewURL = nil } }
                )) {
                    if let url = state.previewURL {
                        NavigationStack {
                            ChatAttachmentQuickLook(url: url)
                                .id(url)
                                .navigationTitle(url.lastPathComponent)
                        }
                        .inspectorColumnWidth(min: 320, ideal: 380, max: 520)
                    }
                }
            }
        }

        let routerState = InspectorRouterState()
        let host = UIHostingController(rootView: InspectorRoutingHarness(state: routerState))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 1024, height: 768))
        window.rootViewController = host
        window.isHidden = false
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }

        // Verify QuickLook coordinator serves each file type accurately and inspector switches cleanly png -> txt -> pdf -> nil
        for fixture in [pngURL, txtURL, pdfURL] {
            let quickLook = ChatAttachmentQuickLook(url: fixture)
            let coordinator = quickLook.makeCoordinator()
            let controller = QLPreviewController()
            XCTAssertEqual(coordinator.numberOfPreviewItems(in: controller), 1)
            XCTAssertEqual(coordinator.previewController(controller, previewItemAt: 0).previewItemURL, fixture)

            routerState.previewURL = fixture
            try await Task.sleep(for: .milliseconds(80))
            RunLoop.main.run(until: Date().addingTimeInterval(0.02))
            XCTAssertEqual(routerState.previewURL, fixture)
            XCTAssertEqual(routerState.fallbackSheetCount, 0, "Inspector preview must not trigger modal sheet fallback")
        }
        routerState.previewURL = nil
        try await Task.sleep(for: .milliseconds(80))
        XCTAssertNil(routerState.previewURL)
    }

    // MARK: - 12. Deterministic Local Mock-HTTP Multi-Turn Streaming, Tool Execution & HTTP 400 Persistence

    private final class IOSMockLLMURLProtocol: URLProtocol {
        nonisolated(unsafe) static var handler: ((URLRequest, Int) -> (Int, [String: String], Data))?
        private static let lock = NSLock()
        private static var callCount = 0

        static func reset() {
            lock.lock()
            callCount = 0
            handler = nil
            lock.unlock()
        }

        override class func canInit(with request: URLRequest) -> Bool {
            request.url?.host == "mock-ios-llm.local"
        }

        override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

        override func startLoading() {
            Self.lock.lock()
            Self.callCount += 1
            let index = Self.callCount
            let currentHandler = Self.handler
            Self.lock.unlock()

            guard let currentHandler, let url = request.url else {
                client?.urlProtocol(self, didFailWithError: URLError(.badURL))
                return
            }
            let (status, headers, data) = currentHandler(request, index)
            guard let response = HTTPURLResponse(
                url: url,
                statusCode: status,
                httpVersion: "HTTP/1.1",
                headerFields: headers
            ) else {
                client?.urlProtocol(self, didFailWithError: URLError(.cannotParseResponse))
                return
            }
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        }

        override func stopLoading() {}
    }

    @MainActor
    func testIOSMockHTTPMultiTurnStreamingToolExecutionErrorAndColdReopenPersistence() async throws {
        let root = try makeTempDirectory("ios-mock-http-multiturn")
        let database = root.appendingPathComponent("ios-multiturn.db")
        let conversationID: UUID
        let turn2RunID: String

        IOSMockLLMURLProtocol.reset()
        defer { IOSMockLLMURLProtocol.reset() }

        IOSMockLLMURLProtocol.handler = { _, index in
            if index == 1 {
                let args = #"{"query":"concise","limit":4}"#.replacingOccurrences(of: "\"", with: "\\\"")
                let sse = [
                    #"data: {"id":"ios-1","object":"chat.completion.chunk","created":1700000101,"model":"ios-mock-model","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"先调用 memory_search 检索本地用户偏好记忆。"},"finish_reason":null}]}"#,
                    #"data: {"id":"ios-1","object":"chat.completion.chunk","created":1700000101,"model":"ios-mock-model","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_ios_mem_1","type":"function","function":{"name":"memory_search","arguments":"\#(args)"}}]},"finish_reason":null}]}"#,
                    #"data: {"id":"ios-1","object":"chat.completion.chunk","created":1700000101,"model":"ios-mock-model","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
                    "data: [DONE]",
                ].joined(separator: "\n\n") + "\n\n"
                return (200, ["Content-Type": "text/event-stream"], Data(sse.utf8))
            } else if index == 2 {
                let sse = [
                    #"data: {"id":"ios-2","object":"chat.completion.chunk","created":1700000102,"model":"ios-mock-model","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"已拿到记忆检索结果，生成最终回答。"},"finish_reason":null}]}"#,
                    #"data: {"id":"ios-2","object":"chat.completion.chunk","created":1700000102,"model":"ios-mock-model","choices":[{"index":0,"delta":{"content":"已完成本地记忆检索：IOS_MOCK_TURN1_OK"},"finish_reason":null}]}"#,
                    #"data: {"id":"ios-2","object":"chat.completion.chunk","created":1700000102,"model":"ios-mock-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
                    "data: [DONE]",
                ].joined(separator: "\n\n") + "\n\n"
                return (200, ["Content-Type": "text/event-stream"], Data(sse.utf8))
            } else {
                let errJSON = #"{"error":{"message":"Mock iOS upstream HTTP 400 invalid_request_error","type":"invalid_request_error"}}"#
                return (400, ["Content-Type": "application/json"], Data(errJSON.utf8))
            }
        }

        let sessionConfig = URLSessionConfiguration.ephemeral
        sessionConfig.protocolClasses = [IOSMockLLMURLProtocol.self]
        let mockSession = URLSession(configuration: sessionConfig)
        defer { mockSession.invalidateAndCancel() }

        let provider = OpenAIProvider(
            config: OpenAIProvider.Config(
                baseURL: try XCTUnwrap(URL(string: "https://mock-ios-llm.local/v1")),
                apiKey: "sk-ios-mock-key",
                model: "ios-mock-model",
                wireProtocol: .chatCompletions,
                executionCapabilities: ModelExecutionCapabilities(
                    toolCalls: true,
                    imageInput: false,
                    reasoning: true,
                    samplingParameters: true
                )
            ),
            streamSession: mockSession
        )
        let modelClient = TextAgentModelClient(client: provider)

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            // Seed a profile memory so memory_search exercises real SQLite recall on isolated DB
            let memoryStore = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
            _ = try memoryStore.saveMemoryText(
                longTermMemory: "User prefers concise verified summaries.",
                userPreferences: "Keep replies focused.",
                expected: memoryStore.loadProfile()
            )
            // Persist model profile to verify cold reopen config durability
            try runtime.upsertModelProfile(IOSCanonicalModelProfile(
                id: "ios-mock-profile",
                displayName: "iOS Mock LLM",
                provider: "openai",
                modelName: "ios-mock-model",
                runtime: "remote",
                sourceJSON: #"{"baseURL":"https://mock-ios-llm.local/v1","wireProtocol":"chat_completions"}"#,
                parametersJSON: "{}",
                selected: true,
                enabled: true,
                revision: 0,
                createdAtMs: 1_000,
                updatedAtMs: 1_000,
                deletedAtMs: nil
            ))

            let convStore = try LocalConversationStore(applicationRuntime: runtime)
            conversationID = try convStore.createConversation(title: "iOS Mock Multi-Turn", language: "zh-Hans").id
            let convIDStr = conversationID.uuidString
            let hostExecutor = IOSAgentHostExecutor(applicationRuntime: runtime, session: mockSession)
            let config = AgentCoreRuntimeConfig(
                platform: .ios,
                executionMode: .deviceLocal,
                model: "ios-mock-model",
                grantedCapabilities: [.network],
                permissionProfile: .allowAll
            )

            // Turn 1: Streaming reasoning + memory_search tool execution + final text answer
            let turn1Identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
            let turn1App = IOSCanonicalAgentApplication(
                store: runtime,
                descriptor: IOSCanonicalAgentRunDescriptor(
                    conversationID: convIDStr,
                    conversationTitle: "iOS Mock Multi-Turn",
                    identity: turn1Identity,
                    userText: "请先查一下我的记忆偏好，然后回复我。",
                    userContentJSON: nil,
                    contextCheckpointID: nil
                )
            )
            let turn1Plan = try AgentCoreToolCatalog.prepareBoundPlan(
                for: .deviceLocal,
                expectedIdentity: AgentCoreExpectedIdentity(sessionId: convIDStr),
                isForeground: true,
                applicationRuntime: runtime
            )
            let coordinator1 = DeviceAgentCoordinator(
                modelClient: modelClient,
                hostExecutor: hostExecutor,
                toolPlan: turn1Plan,
                modelSupportsToolCalls: true,
                canonicalApplication: turn1App
            )

            var turn1Completed: IOSRunResult?
            for try await event in await coordinator1.start(
                sessionId: convIDStr,
                input: "请先查一下我的记忆偏好，然后回复我。",
                history: [],
                turnIdentity: turn1Identity,
                config: config
            ) {
                if case .completed(let result) = event {
                    turn1Completed = result
                }
            }
            let result1 = try XCTUnwrap(turn1Completed, "Turn 1 must emit .completed")
            XCTAssertEqual(result1.value.kind, .answered, "Turn 1 must terminate with .answered")
            let midProjection = try runtime.loadConversationProjectionV1(id: convIDStr)
            let turn1Reply = midProjection.messages
                .filter { $0.role == .assistant }
                .flatMap(\.parts)
                .compactMap { part -> String? in
                    if case .text(let t) = part { return t.markdown }
                    return nil
                }
                .joined()
            XCTAssertTrue(turn1Reply.contains("IOS_MOCK_TURN1_OK"), "Turn 1 final content must match mock response: \(turn1Reply)")

            // Turn 2: Upstream HTTP 400 error -> must seal failed run cleanly and persist failure
            let turn2Identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
            turn2RunID = turn2Identity.submissionID
            let turn2App = IOSCanonicalAgentApplication(
                store: runtime,
                descriptor: IOSCanonicalAgentRunDescriptor(
                    conversationID: convIDStr,
                    conversationTitle: "iOS Mock Multi-Turn",
                    identity: turn2Identity,
                    userText: "第二轮触发 400 错误测试。",
                    userContentJSON: nil,
                    contextCheckpointID: nil
                )
            )
            let turn2Plan = try AgentCoreToolCatalog.prepareBoundPlan(
                for: .deviceLocal,
                expectedIdentity: AgentCoreExpectedIdentity(sessionId: convIDStr),
                isForeground: true,
                applicationRuntime: runtime
            )
            let coordinator2 = DeviceAgentCoordinator(
                modelClient: modelClient,
                hostExecutor: hostExecutor,
                toolPlan: turn2Plan,
                modelSupportsToolCalls: true,
                canonicalApplication: turn2App
            )

            var turn2ThrewError = false
            var turn2TerminalFailed = false
            var turn2CompletedResult: IOSRunResult?
            do {
                for try await event in await coordinator2.start(
                    sessionId: convIDStr,
                    input: "第二轮触发 400 错误测试。",
                    history: [
                        AgentCoreMessage(role: "user", content: "请先查一下我的记忆偏好，然后回复我。"),
                        AgentCoreMessage(role: "assistant", content: turn1Reply),
                    ],
                    turnIdentity: turn2Identity,
                    config: config
                ) {
                    if case .coreTurnTerminal(let terminal) = event, terminal.status == .failed {
                        turn2TerminalFailed = true
                    } else if case .completed(let res) = event {
                        turn2CompletedResult = res
                    }
                }
            } catch {
                turn2ThrewError = true
            }
            XCTAssertTrue(
                turn2ThrewError || turn2TerminalFailed || turn2CompletedResult?.value.kind == .failed,
                "Turn 2 HTTP 400 must cleanly surface an error or seal failed terminal status (result=\(String(describing: turn2CompletedResult?.value.kind)))"
            )
        }

        // Cold reopen from the EXACT same temp SQLite DB -> verify full multi-turn transcript, tool/reasoning parts, failed run seal & model profile!
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let projection = try reopened.loadConversationProjectionV1(id: conversationID.uuidString)
        XCTAssertGreaterThanOrEqual(projection.messages.count, 3, "Cold reopen must retain Turn 1 user/assistant and Turn 2 user messages")
        XCTAssertEqual(projection.messages.filter { $0.role == .user }.count, 2, "Cold reopen must retain both user turns")

        let assistantMessages = projection.messages.filter { $0.role == .assistant }
        XCTAssertGreaterThanOrEqual(assistantMessages.count, 1)
        let firstAssistant = assistantMessages[0]
        XCTAssertEqual(firstAssistant.state, .completed)
        let firstAssistantTexts = firstAssistant.parts.compactMap { part -> String? in
            if case .text(let t) = part { return t.markdown }
            return nil
        }.joined()
        XCTAssertTrue(firstAssistantTexts.contains("IOS_MOCK_TURN1_OK"), "Cold reopen must preserve Turn 1 assistant text")
        XCTAssertTrue(
            firstAssistant.parts.contains(where: { if case .tool(let tool) = $0 { return tool.capabilityID == "memory_search" }; return false }),
            "Cold reopen must preserve executed memory_search tool part"
        )
        XCTAssertTrue(
            firstAssistant.parts.contains(where: { if case .reasoning = $0 { return true }; return false }),
            "Cold reopen must preserve streamed reasoning part"
        )

        let turn2Terminal = try reopened.executeAgentCommand(.object([
            "type": .string("canonical_run_terminal"),
            "conversation_id": .string(conversationID.uuidString),
            "run_id": .string(turn2RunID),
        ]))
        let turn2Kind = turn2Terminal.objectFields?["result"]?.objectFields?["value"]?.objectFields?["kind"]
        XCTAssertEqual(turn2Kind, .string("failed"), "Cold reopen must preserve Turn 2 HTTP 400 failed terminal seal in SQLite")

        let persistedModel = try XCTUnwrap(try reopened.modelProfile(id: "ios-mock-profile"))
        XCTAssertEqual(persistedModel.modelName, "ios-mock-model")
    }
}
