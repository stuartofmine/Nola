// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: High-signal macOS NolaCore FFI & MacApplicationPort contract tests against isolated temp SQLite stores.

import Foundation
import SQLite3
import XCTest
@testable import Nola

@MainActor
final class MacCoreContractsTests: XCTestCase {
    override func tearDown() {
        MacApplicationServiceSupervisor.shared.stopOwnedProcesses()
        super.tearDown()
    }

    private func makeTempDirectory(_ prefix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-contracts-\(prefix)-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    // MARK: - 1. Unsupported v1 / corrupt store rejection without destroying user data

    func testUnsupportedV1AndCorruptStoreRejectedWithoutDestroyingData() throws {
        let root = try makeTempDirectory("unsupported-store")

        // 1a. Legacy v1 SQLite store (user_version = 1) must be rejected and left untouched.
        let v1DB = root.appendingPathComponent("legacy-v1.db")
        var conn: OpaquePointer?
        XCTAssertEqual(sqlite3_open(v1DB.path, &conn), SQLITE_OK)
        XCTAssertEqual(
            sqlite3_exec(conn, "CREATE TABLE preserved (val TEXT); INSERT INTO preserved VALUES ('keep-v1'); PRAGMA user_version = 1;", nil, nil, nil),
            SQLITE_OK
        )
        sqlite3_close(conn)

        XCTAssertThrowsError(try MacApplicationPort(databaseURL: v1DB))

        // Verify original v1 data is intact and was neither deleted nor silently rewritten.
        var roConn: OpaquePointer?
        XCTAssertEqual(sqlite3_open_v2(v1DB.path, &roConn, SQLITE_OPEN_READONLY, nil), SQLITE_OK)
        defer { sqlite3_close(roConn) }
        var stmt: OpaquePointer?
        XCTAssertEqual(sqlite3_prepare_v2(roConn, "SELECT val FROM preserved;", -1, &stmt, nil), SQLITE_OK)
        defer { sqlite3_finalize(stmt) }
        XCTAssertEqual(sqlite3_step(stmt), SQLITE_ROW)
        XCTAssertEqual(sqlite3_column_text(stmt, 0).map { String(cString: $0) }, "keep-v1")

        // 1b. Future / unknown schema version (user_version = 999) must be rejected.
        let futureDB = root.appendingPathComponent("future-v999.db")
        var futureConn: OpaquePointer?
        XCTAssertEqual(sqlite3_open(futureDB.path, &futureConn), SQLITE_OK)
        XCTAssertEqual(
            sqlite3_exec(futureConn, "CREATE TABLE future_data (val TEXT); INSERT INTO future_data VALUES ('keep-future'); PRAGMA user_version = 999;", nil, nil, nil),
            SQLITE_OK
        )
        sqlite3_close(futureConn)
        XCTAssertThrowsError(try MacApplicationPort(databaseURL: futureDB))
        XCTAssertTrue(FileManager.default.fileExists(atPath: futureDB.path))

        // 1c. Corrupt non-SQLite file must be rejected without deleting or overwriting raw bytes.
        let corruptDB = root.appendingPathComponent("corrupt.db")
        let sentinelBytes = Data("not-a-valid-sqlite-header-keep-intact".utf8)
        try sentinelBytes.write(to: corruptDB, options: .atomic)
        XCTAssertThrowsError(try MacApplicationPort(databaseURL: corruptDB))
        XCTAssertEqual(try Data(contentsOf: corruptDB), sentinelBytes)
    }

    // MARK: - 2. Canonical conversation lifecycle, projection window, and archive backup/restore roundtrip

    func testCanonicalConversationLifecycleProjectionAndArchiveRestoreRoundtrip() async throws {
        let root = try makeTempDirectory("conversation-archive")
        let sourceDB = root.appendingPathComponent("source.db")
        let sourcePort = try MacApplicationPort(databaseURL: sourceDB)
        let sourceRepo = MacCanonicalConversationRepository(store: sourcePort)

        let conversationID = "conv-contract-\(UUID().uuidString.lowercased())"
        let userText = "Summarize quarterly goals for Nola 2.0."
        let assistantText = "Quarterly goals:\n1. Ship unified 8-crate core.\n2. Keep SQLite v2 data safe."

        try sourceRepo.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Initial Draft Title",
                    preview: "",
                    updatedAt: .now,
                    isPinned: false,
                    isPersisted: true
                ),
                messages: [
                    MacChatMessage(role: .user, text: userText, createdAt: .now),
                    MacChatMessage(role: .assistant, text: assistantText, createdAt: .now)
                ]
            )
        ])

        // Rename via canonical port and verify projection window reflects messages & new title.
        try sourcePort.renameConversation(
            conversationID,
            title: "Renamed Canonical Title",
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000) + 10
        )
        let metadata = try XCTUnwrap(sourcePort.loadConversationMetadata(conversationID))
        XCTAssertGreaterThan(metadata.revision, 0)
        let summary = try XCTUnwrap(try sourcePort.listConversations().first(where: { $0.id == conversationID }))
        XCTAssertEqual(summary.title, "Renamed Canonical Title")

        let window = try sourcePort.loadConversationWindow(conversationID)
        XCTAssertEqual(window.projection.title, "Renamed Canonical Title")
        let projectedMessages = try MacCanonicalConversationRepository.projectMessages(window)
        XCTAssertEqual(projectedMessages.count, 2)
        XCTAssertEqual(projectedMessages[0].role, .user)
        XCTAssertEqual(projectedMessages[0].text, userText)
        XCTAssertEqual(projectedMessages[1].role, .assistant)
        XCTAssertEqual(projectedMessages[1].text, assistantText)

        // Export canonical conversation into a MacDataBackup JSON archive and restore into a brand-new isolated SQLite DB.
        let exportedCanonical = try XCTUnwrap(try sourcePort.loadConversation(conversationID))
        XCTAssertEqual(exportedCanonical.messages.count, 2)
        let backup = MacDataBackup(
            format: MacDataBackupCodec.format,
            version: MacDataBackupCodec.version,
            exportedAt: .now,
            conversations: [MacBackupConversation(exportedCanonical)],
            activity: [],
            automations: [],
            settings: [],
            productSettings: [],
            webSources: [],
            notifications: []
        )
        let encodedBackup = try MacDataBackupCodec.encode(backup)
        let decodedBackup = try MacDataBackupCodec.decode(encodedBackup)
        XCTAssertEqual(decodedBackup.conversations.count, 1)
        XCTAssertEqual(decodedBackup.conversations[0].canonical.messages.count, 2)

        let restoredDB = root.appendingPathComponent("restored.db")
        let restoredPort = try MacApplicationPort(databaseURL: restoredDB)
        try MacArchiveRestoreWriter.restore(decodedBackup.conversations, port: restoredPort)

        let restoredRepo = MacCanonicalConversationRepository(store: restoredPort)
        let restoredLoaded = try XCTUnwrap(try restoredRepo.load(conversationID: conversationID))
        XCTAssertEqual(restoredLoaded.conversation.title, "Renamed Canonical Title")
        XCTAssertEqual(restoredLoaded.messages.count, 2)
        XCTAssertEqual(restoredLoaded.messages[0].text, userText)
        XCTAssertEqual(restoredLoaded.messages[1].text, assistantText)
    }

    // MARK: - 3. Model profile revision tracking, room/expert CRUD, and automation CRUD

    func testModelProfileRevisionRoomExpertAndAutomationCRUD() throws {
        let root = try makeTempDirectory("domain-crud")
        let database = root.appendingPathComponent("domain.db")
        let port = try MacApplicationPort(databaseURL: database)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        // 3a. Model profile persistence & CAS revision tracking
        let profileID = "contract-model-\(UUID().uuidString.lowercased())"
        let initialProfile = MacCanonicalModelProfile(
            id: profileID,
            displayName: "Contract Model",
            provider: "openai",
            modelName: "qwen3.8-flash-next-iq3_s",
            runtime: "remote",
            sourceJSON: #"{"baseURL":"https://models.example.invalid/v1","wireProtocol":"chat_completions"}"#,
            parametersJSON: #"{"temperature":0.2}"#,
            selected: true,
            enabled: true,
            revision: 0,
            createdAtMs: nowMs,
            updatedAtMs: nowMs,
            deletedAtMs: nil
        )
        try port.upsertModelProfile(initialProfile)
        let storedProfile = try XCTUnwrap(try port.modelProfile(profileID))
        XCTAssertEqual(storedProfile.modelName, "qwen3.8-flash-next-iq3_s")
        XCTAssertGreaterThan(storedProfile.revision, 0)

        let updatedProfile = MacCanonicalModelProfile(
            id: storedProfile.id,
            displayName: storedProfile.displayName,
            provider: storedProfile.provider,
            modelName: storedProfile.modelName,
            runtime: storedProfile.runtime,
            sourceJSON: storedProfile.sourceJSON,
            parametersJSON: #"{"temperature":0.6,"maxOutputTokens":2048}"#,
            selected: storedProfile.selected,
            enabled: storedProfile.enabled,
            revision: storedProfile.revision,
            createdAtMs: storedProfile.createdAtMs,
            updatedAtMs: nowMs + 100,
            deletedAtMs: nil
        )
        let nextRevision = try port.configureModelParameters(updatedProfile, expectedRevision: UInt64(storedProfile.revision))
        XCTAssertGreaterThan(nextRevision, UInt64(storedProfile.revision))
        // Stale revision must be rejected.
        XCTAssertThrowsError(try port.configureModelParameters(updatedProfile, expectedRevision: UInt64(storedProfile.revision)))

        // 3b. Room & Expert CRUD
        let roomService = MacAgentRoomService(runtime: port)
        let initialDirectory = try roomService.directory()
        let createdDirectory = try roomService.createAgent(
            name: "Architecture Reviewer",
            description: "Audits Swift & Rust boundaries",
            defaults: .object([:]),
            directory: initialDirectory
        )
        let agent = try XCTUnwrap(createdDirectory.agents.first(where: { $0.name == "Architecture Reviewer" }))
        let updatedDirectory = try roomService.updateAgent(
            agent,
            name: "Senior Architecture Reviewer",
            description: "Audits Swift & Rust FFI contracts",
            availability: "active",
            directory: createdDirectory
        )
        let updatedAgent = try XCTUnwrap(updatedDirectory.agents.first(where: { $0.id == agent.id }))
        XCTAssertEqual(updatedAgent.name, "Senior Architecture Reviewer")

        let room = try roomService.create(title: "Design Review Room")
        let joinedRoom = try roomService.join(agent: updatedAgent, room: room)
        let participant = try XCTUnwrap(joinedRoom.room.participants.first(where: { $0.agentID == updatedAgent.id && $0.active }))
        let deactivatedRoom = try roomService.deactivate(participantID: participant.id, room: joinedRoom)
        XCTAssertFalse(deactivatedRoom.room.participants.contains(where: { $0.id == participant.id && $0.active }))

        // 3c. Automation CRUD with verified canonical readback
        let supportDir = root.appendingPathComponent("Automations")
        let environment = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("Support"),
            cachesRoot: root.appendingPathComponent("Caches")
        )
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let webStore = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let automationStore = try MacAutomationStore(
            baseDirectory: supportDir,
            launchAgentDirectory: root.appendingPathComponent("LaunchAgents"),
            synchronizeSystemJobs: false,
            appEnvironment: environment,
            authorizationResolver: { _ in true },
            activityStore: MacActivityStore(baseDirectory: root.appendingPathComponent("Activity")),
            completionNotifier: { _, _ in },
            dataSourceRegistry: registry,
            webSourceStore: webStore,
            automationClient: port,
            skillCatalogClient: port
        )
        let spec = MacAutomationExecutionSpec(
            prompt: "Summarize workspace status daily.",
            executionScope: .localWorkspace(root: root.path),
            permissionProfile: "cautious",
            modelSelection: "use_current_default_at_admission",
            contextRefs: [],
            skillRefs: []
        )
        let automation = MacAutomation(
            id: UUID(),
            name: "Daily Status Digest",
            prompt: spec.prompt,
            frequency: .daily,
            weekday: 1,
            hour: 9,
            minute: 30,
            workspacePath: root.path,
            isEnabled: false,
            createdAt: .now,
            contextSources: [],
            executionSpec: spec
        )
        let verified = try automationStore.upsertWithVerification(automation)
        XCTAssertNil(verified.verificationError)
        XCTAssertEqual(try port.listAutomations().count, 1)

        // Cold reopen confirms model profile, expert/room, and automation all persisted to SQLite v2.
        let reopenedPort = try MacApplicationPort(databaseURL: database)
        let reopenedProfile = try XCTUnwrap(try reopenedPort.modelProfile(profileID))
        XCTAssertEqual(UInt64(reopenedProfile.revision), nextRevision)
        XCTAssertTrue(reopenedProfile.parametersJSON.contains("2048"))
        XCTAssertTrue(try reopenedPort.deleteModelProfile(profileID, deletedAtMs: nowMs + 200))
        XCTAssertNil(try reopenedPort.modelProfile(profileID))

        let reopenedRooms = MacAgentRoomService(runtime: reopenedPort)
        XCTAssertTrue(try reopenedRooms.directory().agents.contains(where: { $0.id == updatedAgent.id }))
        XCTAssertEqual(try reopenedRooms.load(id: room.room.id)?.room.title, "Design Review Room")

        XCTAssertEqual(try reopenedPort.listAutomations().first?.name, "Daily Status Digest")
        automationStore.delete(automation)
        XCTAssertTrue(try reopenedPort.listAutomations().isEmpty)
    }

    // MARK: - 4. Daemon UDS latency budget, discovery URL rules, and model connection lifecycle

    func testDaemonUDSLatencyAndModelConnectionLifecycle() throws {
        let root = try makeTempDirectory("uds-latency-and-connections")
        let database = root.appendingPathComponent("connections.db")
        let port = try MacApplicationPort(databaseURL: database)

        // 4a. Shared Anthropic discovery path normalization (root URL vs /v1 vs OpenAI)
        let anthropicRootRequest = try AppleModelCatalogDiscovery.request(
            endpoint: "https://api.anthropic.com",
            wireProtocol: "anthropic",
            secret: "sk-ant-test"
        )
        XCTAssertEqual(anthropicRootRequest.url?.absoluteString, "https://api.anthropic.com/v1/models")
        XCTAssertEqual(anthropicRootRequest.value(forHTTPHeaderField: "anthropic-version"), "2023-06-01")
        XCTAssertEqual(anthropicRootRequest.value(forHTTPHeaderField: "x-api-key"), "sk-ant-test")

        let anthropicV1Request = try AppleModelCatalogDiscovery.request(
            endpoint: "https://api.anthropic.com/v1",
            wireProtocol: "anthropic",
            secret: "sk-ant-test"
        )
        XCTAssertEqual(anthropicV1Request.url?.absoluteString, "https://api.anthropic.com/v1/models")

        let anthropicSubpathRequest = try AppleModelCatalogDiscovery.request(
            endpoint: "https://gateway.example.invalid/anthropic",
            wireProtocol: "anthropic",
            secret: "sk-ant-test"
        )
        XCTAssertEqual(anthropicSubpathRequest.url?.absoluteString, "https://gateway.example.invalid/anthropic/v1/models")

        let openaiRequest = try AppleModelCatalogDiscovery.request(
            endpoint: "https://api.openai.com/v1",
            wireProtocol: "chat_completions",
            secret: "sk-openai-test"
        )
        XCTAssertEqual(openaiRequest.url?.absoluteString, "https://api.openai.com/v1/models")
        XCTAssertEqual(openaiRequest.value(forHTTPHeaderField: "Authorization"), "Bearer sk-openai-test")

        // 4b. UDS latency budget: 30 sequential daemon IPC calls must stay well under 1.5s
        // (catches any regression of a 50ms accept sleep loop).
        _ = try port.listModelProfiles()
        let start = ContinuousClock.now
        for i in 0..<30 {
            if i.isMultiple(of: 2) {
                _ = try port.listModelProfiles()
            } else {
                _ = try port.listConversations()
            }
        }
        let elapsed = ContinuousClock.now - start
        XCTAssertLessThan(
            elapsed,
            .milliseconds(1_500),
            "30 sequential daemon UDS IPC requests took \(elapsed), exceeding 1.5s latency budget"
        )

        // 4c. MacModelConnectionService lifecycle: create -> refresh catalog -> configure enabled -> capability override -> disconnect/reconnect
        let service = MacModelConnectionService(application: port)
        let created = try service.create(
            name: "Contract Gateway",
            endpoint: "https://models.example.invalid/v1",
            wireProtocol: "chat_completions",
            secret: "sk-contract-lifecycle-key"
        )
        XCTAssertTrue(created.document.connection.enabled)
        XCTAssertEqual(created.document.connection.display_name, "Contract Gateway")

        let alphaModel = AppleModelCatalogDiscovery.Model(
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
        let betaModel = AppleModelCatalogDiscovery.Model(
            id: "contract-beta",
            displayName: "Contract Beta",
            modalities: ["text"]
        )
        let catalogEntries: [MacCanonicalJSONValue] = try [alphaModel, betaModel].map {
            try JSONDecoder().decode(MacCanonicalJSONValue.self, from: $0.catalogData(wireProtocol: "chat_completions"))
        }
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)
        let refreshed: MacModelConnectionSnapshot = try service.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(created.revision),
            "models": .array(catalogEntries),
            "observed_at_ms": .signedInteger(nowMs),
        ])
        XCTAssertGreaterThan(refreshed.revision, created.revision)
        XCTAssertEqual(refreshed.document.catalog?.models.count, 2)

        // Before any model is enabled (zero canonical `model` records, discovered catalog exists),
        // listModelProfiles(), MacChatModelCatalog.load, and service.loadInventory() must all succeed
        // without DecodingError.keyNotFound ("未能读取数据，因为数据丢失。").
        XCTAssertTrue(try port.listModelProfiles().isEmpty)
        let catalogBeforeEnable = try MacChatModelCatalog.load(application: port)
        XCTAssertFalse(catalogBeforeEnable.entries.contains(where: { $0.option.modelID == "contract-alpha" }))
        let inventoryBeforeEnable = try service.loadInventory()
        XCTAssertEqual(inventoryBeforeEnable.connections.count, 1)
        XCTAssertEqual(inventoryBeforeEnable.connections[0].document.catalog?.models.count, 2)
        XCTAssertTrue(inventoryBeforeEnable.enabledByConnection[created.id]?.isEmpty ?? false)

        let enabledKey = "chat_completions:contract-alpha"
        try service.enable(refreshed, modelIDs: [enabledKey])
        let enabledKeys = try service.enabledModels(connectionID: created.id)
        XCTAssertEqual(enabledKeys, [enabledKey])

        let catalog = try MacChatModelCatalog.load(application: port)
        XCTAssertTrue(catalog.entries.contains(where: {
            $0.option.source == .configured && $0.option.modelID == "contract-alpha"
        }))

        let afterEnable = try XCTUnwrap(try service.connections().first(where: { $0.id == created.id }))
        let targetCatalogModel = try XCTUnwrap(afterEnable.document.catalog?.models.first(where: { $0.model_id == "contract-alpha" }))
        let overrideCaps: MacCanonicalJSONValue = .object([
            "toolCalls": .bool(true),
            "imageInput": .bool(false),
            "reasoning": .bool(true),
            "samplingParameters": .bool(false),
        ])
        let overridden = try service.setCapabilityOverride(
            afterEnable,
            model: targetCatalogModel,
            capabilities: overrideCaps
        )
        XCTAssertGreaterThan(overridden.revision, afterEnable.revision)
        XCTAssertTrue(
            overridden.document.capability_assessments?.contains(where: {
                $0.model_id == "contract-alpha" && $0.user_override != nil
            }) == true,
            "Capability override should be persisted in capability_assessments"
        )

        let disconnected = try service.disconnect(overridden)
        XCTAssertFalse(disconnected.document.connection.enabled)
        XCTAssertEqual(
            disconnected.document.catalog?.models.count,
            2,
            "Disconnecting a connection must preserve its discovered catalog"
        )

        let reconnected = try service.reconnect(disconnected)
        XCTAssertTrue(reconnected.document.connection.enabled)
        XCTAssertEqual(
            reconnected.document.catalog?.models.count,
            2,
            "Reconnecting a connection must preserve its discovered catalog"
        )

        // 4d. Update connection (rename preserves catalog; route change rebinds Keychain secret & resets catalog)
        let renamed = try service.update(
            reconnected,
            name: "Contract Gateway Renamed",
            endpoint: "https://models.example.invalid/v1",
            wireProtocol: "chat_completions"
        )
        XCTAssertEqual(renamed.document.connection.display_name, "Contract Gateway Renamed")
        XCTAssertEqual(renamed.document.catalog?.models.count, 2, "Renaming should preserve discovered catalog")

        let rerouted = try service.update(
            renamed,
            name: "Contract Gateway v2",
            endpoint: "http://192.168.31.18:8081/v1",
            wireProtocol: "responses"
        )
        XCTAssertEqual(rerouted.document.connection.base_url, "http://192.168.31.18:8081/v1")
        XCTAssertEqual(rerouted.document.connection.protocols, ["responses"])
        XCTAssertNil(rerouted.document.catalog, "Changing endpoint/protocol should clear stale catalog")
        let reboundCred = try service.credential(
            connectionID: rerouted.id,
            endpoint: "http://192.168.31.18:8081/v1",
            wireProtocol: "responses"
        )
        XCTAssertEqual(reboundCred.secret, "sk-contract-lifecycle-key", "Keychain secret must be re-bound on endpoint/protocol update")

        // 4e. Delete connection removes connection, linked profiles, and orphan account
        try service.delete(rerouted, deleteAccount: true)
        XCTAssertFalse(
            try service.connections().contains(where: { $0.id == rerouted.id }),
            "Deleted connection must not remain in list_connections"
        )
        let catalogAfterDelete = try MacChatModelCatalog.load(application: port)
        XCTAssertFalse(
            catalogAfterDelete.entries.contains(where: { $0.option.modelID == "contract-alpha" }),
            "Models from deleted connection must be removed from chat catalog"
        )

        // 4f. Verify Nola.app Info.plist permits user-configured HTTP LAN endpoints (ATS)
        let ats = Bundle.main.object(forInfoDictionaryKey: "NSAppTransportSecurity") as? [String: Any]
        XCTAssertEqual(
            ats?["NSAllowsArbitraryLoads"] as? Bool,
            true,
            "Nola.app Info.plist must allow HTTP model endpoints via NSAppTransportSecurity"
        )

        // 4g. Verify AppleModelCatalogDiscovery parses architecture.input_modalities & max_model_len + HTTP errors
        let lanJSON = """
        {"data":[{"id":"qwen3.8-flash-next-iq3_s","max_model_len":262144,"architecture":{"input_modalities":["text","image"]},"supported_parameters":["tools","temperature"]}]}
        """.data(using: .utf8)!
        let okResponse = HTTPURLResponse(url: URL(string: "http://192.168.31.18:8081/v1/models")!, statusCode: 200, httpVersion: nil, headerFields: nil)!
        let parsedLAN = try AppleModelCatalogDiscovery.parse(data: lanJSON, response: okResponse)
        XCTAssertEqual(parsedLAN.count, 1)
        XCTAssertEqual(parsedLAN[0].contextWindow, 262_144)
        XCTAssertEqual(parsedLAN[0].modalities, ["text", "image"])
        XCTAssertEqual(parsedLAN[0].toolCalls, true)

        let unauthResponse = HTTPURLResponse(url: URL(string: "http://192.168.31.18:8081/v1/models")!, statusCode: 401, httpVersion: nil, headerFields: nil)!
        XCTAssertThrowsError(try AppleModelCatalogDiscovery.parse(data: Data(#"{"error":{"message":"Invalid key"}}"#.utf8), response: unauthResponse)) { err in
            XCTAssertTrue(err.localizedDescription.contains("401"), "Expected HTTP 401 in error: \(err.localizedDescription)")
        }

        // 4h. Verify MacDataSourceRegistry.recordValidation skips redundant writes on repeated startup refreshes
        let dsRegistry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("ds-idempotency.json"), applicationPort: port)
        dsRegistry.recordValidation(
            sourceID: "platform:reminders",
            connectorID: "eventkit.reminders",
            readerDescription: "EKEventStore.fetchReminders(incomplete)",
            schemaFields: ["identifier", "title"],
            redactedSample: "{}",
            recordCount: 3,
            maximumAgeMinutes: 15,
            observedAt: .now
        )
        let revAfterFirst = try XCTUnwrap(try port.dataSource(id: "platform:reminders")?.revision)
        for _ in 0..<5 {
            dsRegistry.recordValidation(
                sourceID: "platform:reminders",
                connectorID: "eventkit.reminders",
                readerDescription: "EKEventStore.fetchReminders(incomplete)",
                schemaFields: ["identifier", "title"],
                redactedSample: "{}",
                recordCount: 3,
                maximumAgeMinutes: 15,
                observedAt: .now
            )
        }
        let revAfterRepeat = try XCTUnwrap(try port.dataSource(id: "platform:reminders")?.revision)
        XCTAssertEqual(revAfterFirst, revAfterRepeat, "Repeated unchanged recordValidation calls must not spam SQLite revisions")
    }

    // MARK: - 5. End-to-end subsystem buttons & main-thread IPC contracts

    func testRealEndToEndSubsystemButtonsAndMainThreadContracts() async throws {
        let root = try makeTempDirectory("subsystems-e2e")
        let database = root.appendingPathComponent("subsystems.db")
        let port = try MacApplicationPort(databaseURL: database)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1_000)

        // 5a. Memory Center (MacAgentHostMemoryService / nola memory): create with metadata, revise, verify supersedes & metadata
        let runtime = MacAgentHostRuntime(
            credentialStore: MacAppleChatConfigurationStore(applicationPort: port)
        )
        let initialMemory = try await runtime.createMemory(
            MacAgentHostMemoryDraft(
                scope: "global",
                kind: "preference",
                content: "Prefer concise Swift concurrency audits with concrete line numbers.",
                summary: "Audit style preference",
                tags: ["swift", "review"],
                confidence: 0.87,
                importance: 0.78,
                sensitivity: "normal",
                expiresAt: nil,
                source: "explicit_user_memory_management",
                provenanceKind: "user_stated"
            ),
            databaseURL: database
        )
        XCTAssertEqual(initialMemory.scope, "global")
        XCTAssertEqual(initialMemory.kind, "preference")
        XCTAssertEqual(initialMemory.confidence, 0.87, accuracy: 0.001)
        XCTAssertEqual(initialMemory.provenance.source ?? initialMemory.source, "explicit_user_memory_management")

        let revisedMemory = try await runtime.reviseMemory(
            id: initialMemory.id,
            draft: MacAgentHostMemoryDraft(
                scope: "global",
                kind: "preference",
                content: "Prefer concise Swift & Rust concurrency audits with exact diagnostics.",
                summary: "Updated audit style preference",
                tags: ["swift", "rust", "review"],
                confidence: 0.94,
                importance: 0.85,
                sensitivity: "normal",
                expiresAt: nil,
                source: "explicit_user_memory_management",
                provenanceKind: "user_stated"
            ),
            databaseURL: database
        )
        XCTAssertTrue(
            revisedMemory.supersedes.contains(initialMemory.id),
            "Revised memory should record supersedes containing original id \(initialMemory.id), got \(revisedMemory.supersedes)"
        )
        XCTAssertEqual(revisedMemory.confidence, 0.94, accuracy: 0.001)
        XCTAssertEqual(revisedMemory.provenance.source ?? revisedMemory.source, "explicit_user_memory_management")

        let listedMemories = try await runtime.listMemoryRecords(databaseURL: database)
        let persistedMemory = try XCTUnwrap(listedMemories.first(where: { $0.id == revisedMemory.id }))
        XCTAssertEqual(persistedMemory.content, "Prefer concise Swift & Rust concurrency audits with exact diagnostics.")
        XCTAssertTrue(persistedMemory.supersedes.contains(initialMemory.id))
        XCTAssertEqual(persistedMemory.confidence, 0.94, accuracy: 0.001)
        XCTAssertEqual(persistedMemory.provenance.source ?? persistedMemory.source, "explicit_user_memory_management")
        try await runtime.deleteMemory(id: revisedMemory.id, databaseURL: database)
        let afterDeleteMemories = try await runtime.listMemoryRecords(databaseURL: database)
        XCTAssertFalse(afterDeleteMemories.contains(where: { $0.id == revisedMemory.id }))

        // 5b. Automation Next Run + Run History + Activity Center (including waiting conversations)
        let repo = MacCanonicalConversationRepository(store: port)
        let doneConversationID = "conv-done-\(UUID().uuidString.lowercased())"
        let waitingConversationID = "conv-waiting-\(UUID().uuidString.lowercased())"
        try repo.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: doneConversationID,
                    title: "Completed Audit Thread",
                    preview: "All checks green.",
                    updatedAt: .now,
                    isPinned: false,
                    isPersisted: true
                ),
                messages: [
                    MacChatMessage(role: .user, text: "Check subsystem contracts.", createdAt: .now),
                    MacChatMessage(role: .assistant, text: "All checks green.", createdAt: .now),
                ]
            ),
            MacStoredConversation(
                conversation: MacConversation(
                    id: waitingConversationID,
                    title: "Waiting Approval Thread",
                    preview: "Waiting on user confirmation.",
                    updatedAt: .now,
                    isPinned: false,
                    isPersisted: true
                ),
                messages: [
                    MacChatMessage(role: .user, text: "Deploy release notes.", createdAt: .now),
                ]
            ),
        ])
        // Mark waitingConversationID as having an open wait in SQLite KV so we verify needs_response: false does not hide it.
        var kvConn: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &kvConn), SQLITE_OK)
        let waitKey = "open_waits:\(waitingConversationID)"
        let insertSQL = "INSERT OR REPLACE INTO kv (key, value_json, updated_at_ms) VALUES ('\(waitKey)', '[{\"wait_id\":\"w-contract-1\"}]', \(nowMs));"
        XCTAssertEqual(sqlite3_exec(kvConn, insertSQL, nil, nil, nil), SQLITE_OK)
        sqlite3_close(kvConn)

        let appEnv = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("Support"),
            cachesRoot: root.appendingPathComponent("Caches")
        )
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let webStore = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let activityStore = MacActivityStore(baseDirectory: root.appendingPathComponent("Activity"))
        let automationStore = try MacAutomationStore(
            baseDirectory: root.appendingPathComponent("Automations"),
            launchAgentDirectory: root.appendingPathComponent("LaunchAgents"),
            synchronizeSystemJobs: false,
            appEnvironment: appEnv,
            authorizationResolver: { _ in true },
            activityStore: activityStore,
            completionNotifier: { _, _ in },
            dataSourceRegistry: registry,
            webSourceStore: webStore,
            automationClient: port,
            skillCatalogClient: port
        )
        let autoWorkspace = root.appendingPathComponent("auto-workspace", isDirectory: true)
        try FileManager.default.createDirectory(at: autoWorkspace, withIntermediateDirectories: true)
        let autoSpec = MacAutomationExecutionSpec(
            prompt: "Run daily workspace health check.",
            executionScope: .localWorkspace(root: autoWorkspace.path),
            permissionProfile: "cautious",
            modelSelection: "use_current_default_at_admission",
            contextRefs: [],
            skillRefs: []
        )
        let automation = MacAutomation(
            id: UUID(),
            name: "Nightly Contract Check",
            prompt: autoSpec.prompt,
            frequency: .daily,
            weekday: 1,
            hour: 9,
            minute: 15,
            workspacePath: autoWorkspace.path,
            isEnabled: true,
            createdAt: .now,
            contextSources: [],
            executionSpec: autoSpec
        )
        let verifiedAuto = try automationStore.upsertWithVerification(automation)
        XCTAssertNil(verifiedAuto.verificationError)

        // Real query_automation_occurrences -> future Date
        let computedNextRun = try XCTUnwrap(
            automation.computeNextRun(using: port),
            "computeNextRun(using:) should resolve a future occurrence via query_automation_occurrences"
        )
        XCTAssertGreaterThan(computedNextRun, Date(), "computeNextRun should return a future Date")

        let nextRunDeadline = ContinuousClock.now + .seconds(5)
        while automationStore.nextRun(for: automation) == nil && ContinuousClock.now < nextRunDeadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        let cachedNextRun = try XCTUnwrap(automationStore.nextRun(for: automation))
        XCTAssertGreaterThan(cachedNextRun, Date())

        // Trigger runNowAsync and wait for completion (bounded; fails fast when no model is configured in isolated DB)
        await automationStore.runNowAsync(automation)
        let runDeadline = ContinuousClock.now + .seconds(10)
        while automationStore.runningAutomationIDs.contains(automation.id) && ContinuousClock.now < runDeadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertFalse(automationStore.runningAutomationIDs.contains(automation.id))

        let persistedRuns = try port.listAutomationRuns(automationID: automation.id.uuidString.lowercased())
        XCTAssertFalse(persistedRuns.isEmpty, "runNowAsync should persist an automation run in SQLite")

        let automationActivity = try port.listActivity(
            AppleActivityQuery(source: "automation", needs_response: false, query: "", cursor: nil, limit: 50)
        )
        XCTAssertTrue(
            automationActivity.entries.contains(where: { $0.source == "automation" }),
            "list_activity(source: \"automation\") should include the triggered automation run"
        )

        let allActivity = try port.listActivity(
            AppleActivityQuery(source: nil, needs_response: false, query: "", cursor: nil, limit: 50)
        )
        XCTAssertTrue(
            allActivity.entries.contains(where: { $0.conversation_id == doneConversationID }),
            "list_activity(needs_response: false) should include completed conversations"
        )
        XCTAssertTrue(
            allActivity.entries.contains(where: { $0.conversation_id == waitingConversationID && $0.needs_response }),
            "list_activity(needs_response: false) must NOT hide waiting conversations"
        )
        let waitingOnlyActivity = try port.listActivity(
            AppleActivityQuery(source: nil, needs_response: true, query: "", cursor: nil, limit: 50)
        )
        XCTAssertTrue(
            waitingOnlyActivity.entries.contains(where: { $0.conversation_id == waitingConversationID }),
            "list_activity(needs_response: true) should return waiting conversations"
        )

        // 5c. Extensions (Skills & MCP) CRUD with cold-reopen readback verification
        let skillID = "custom-skill-\(UUID().uuidString.lowercased())"
        let createdSkill = try port.createUserSkill(
            MacCanonicalSkillInput(
                id: skillID,
                source: .user,
                version: "1.0.0",
                displayName: "Contract Audit Skill",
                summary: "Checks API invariants",
                category: "custom",
                instructionsRef: "inline://contract-audit",
                instructionsText: "# Contract Audit\nAlways verify revisions.",
                requiredTools: [],
                defaultMode: .inline,
                enabled: true,
                createdAtMs: nowMs,
                updatedAtMs: nowMs
            )
        )
        XCTAssertTrue(createdSkill.record.enabled)
        let toggledSkill = try port.setSkillEnabled(createdSkill, enabled: false, updatedAtMs: nowMs + 10)
        XCTAssertFalse(toggledSkill.record.enabled)

        let mcpID = "custom-mcp-\(UUID().uuidString.lowercased())"
        let createdMCP = try port.createMCPServer(
            MacCanonicalMCPServerInput(
                id: mcpID,
                displayName: "Contract MCP Server",
                enabled: true,
                configurationJSON: #"{"command":"/usr/bin/true","name":"Contract MCP Server"}"#,
                revision: 0,
                createdAtMs: nowMs,
                updatedAtMs: nowMs,
                deletedAtMs: nil
            )
        )
        XCTAssertTrue(createdMCP.enabled)
        let toggledMCP = try port.configureMCPServer(
            createdMCP,
            displayName: "Contract MCP Server Updated",
            enabled: false,
            configurationJSON: #"{"command":"/usr/bin/true","name":"Contract MCP Server Updated"}"#,
            updatedAtMs: nowMs + 20
        )
        XCTAssertFalse(toggledMCP.enabled)
        XCTAssertEqual(toggledMCP.displayName, "Contract MCP Server Updated")

        // Verify persisted toggle state across a fresh MacApplicationPort reopen
        let reopenedExtPort = try MacApplicationPort(databaseURL: database)
        let reloadedSkillEditor = try XCTUnwrap(try reopenedExtPort.skillEditor(skillID))
        XCTAssertFalse(reloadedSkillEditor.record.enabled)
        let reloadedMCP = try XCTUnwrap(try reopenedExtPort.listMCPServers().first(where: { $0.id == mcpID }))
        XCTAssertFalse(reloadedMCP.enabled)
        XCTAssertEqual(reloadedMCP.displayName, "Contract MCP Server Updated")

        // Delete both and verify readback after reload
        XCTAssertTrue(try reopenedExtPort.deleteUserSkill(reloadedSkillEditor, deletedAtMs: nowMs + 30))
        XCTAssertTrue(try reopenedExtPort.deleteMCPServer(reloadedMCP, deletedAtMs: nowMs + 40))

        let postDeletePort = try MacApplicationPort(databaseURL: database)
        XCTAssertFalse(
            try postDeletePort.listSkills().contains(where: { $0.id == skillID && $0.deletedAtMs == nil }),
            "Deleted user skill must not remain active after reload"
        )
        XCTAssertFalse(
            try postDeletePort.listMCPServers().contains(where: { $0.id == mcpID && $0.deletedAtMs == nil }),
            "Deleted MCP server must not remain active after reload"
        )

        // 5d. Draft Data Source Override on an unsaved draft conversation UUID (no not_found)
        let unsavedDraftConversationID = "draft-\(UUID().uuidString.lowercased())"
        let draftSourceID = "mac.platform.calendar"
        XCTAssertTrue(
            try port.setDataSourceOverride(
                conversationID: unsavedDraftConversationID,
                sourceID: draftSourceID,
                value: .enabled,
                updatedAtMs: nowMs + 50
            ),
            "set_conversation_override must succeed on an unsaved draft conversation UUID"
        )
        XCTAssertEqual(
            try port.dataSourceOverride(conversationID: unsavedDraftConversationID, sourceID: draftSourceID),
            .enabled
        )

        // 5e. Edit-and-Resend + Expert Templates (host_expert_template_operation readable_templates)
        let resendOpID = "resend-op-\(UUID().uuidString.lowercased())"
        let targetConversationID = "conv-branch-\(UUID().uuidString.lowercased())"
        let preparedResend = try port.prepareEditedResend(
            ApplePrepareEditedResend(
                operation_id: resendOpID,
                source_conversation_id: doneConversationID,
                source_message_id: "msg-src-1",
                expected_source_revision: 1,
                expected_source_message_revision: 1,
                edited_text: "Updated prompt after edit-and-resend",
                target_conversation_id: targetConversationID,
                title: "Edited Branch Conversation",
                created_at_ms: nowMs + 60
            )
        )
        XCTAssertEqual(preparedResend.run_id, resendOpID)
        XCTAssertEqual(preparedResend.input.target_conversation_id, targetConversationID)
        XCTAssertFalse(preparedResend.user_message_id.isEmpty)
        XCTAssertFalse(preparedResend.assistant_message_id.isEmpty)

        let loadedResend = try XCTUnwrap(
            try port.loadPreparedEditedResend(targetConversationID),
            "load_prepared_edited_resend should read back prepared edited resend"
        )
        XCTAssertEqual(loadedResend.run_id, resendOpID)
        XCTAssertEqual(loadedResend.input.edited_text, "Updated prompt after edit-and-resend")

        struct ReadableTemplatesPayload: Decodable {
            let revision: Int64
            let templates: [MacCanonicalJSONValue]
        }
        let readableTemplates: ReadableTemplatesPayload = try port.executeExpertHost(
            .readableTemplates(authority: .local())
        )
        XCTAssertGreaterThanOrEqual(readableTemplates.revision, 1)
        XCTAssertNotNil(readableTemplates.templates)

        // 5f. Main-thread IPC guard: scheduleReload() & loadInventory() complete asynchronously without blocking
        let presentation = MacDefaultChatModelPresentation(store: .init(application: port))
        let scheduleStart = ContinuousClock.now
        presentation.scheduleReload()
        let scheduleSyncElapsed = ContinuousClock.now - scheduleStart
        XCTAssertLessThan(
            scheduleSyncElapsed,
            .milliseconds(200),
            "scheduleReload() must return immediately on MainActor without synchronous IPC blocking"
        )
        let reloadDeadline = ContinuousClock.now + .seconds(5)
        while presentation.snapshot == nil && presentation.error == nil && ContinuousClock.now < reloadDeadline {
            try await Task.sleep(for: .milliseconds(30))
        }
        XCTAssertNotNil(presentation.snapshot, "scheduleReload() should complete asynchronously with a valid snapshot")
        XCTAssertNil(presentation.error, "scheduleReload() should not surface an error: \(presentation.error ?? "")")

        let connectionService = MacModelConnectionService(application: port)
        let inventory = try await Task.detached(priority: .userInitiated) {
            try connectionService.loadInventory()
        }.value
        XCTAssertEqual(inventory.connections.count, inventory.enabledByConnection.count)
    }

    // MARK: - 6. Strictly two permission modes & /props + OpenRouter reasoning effort discovery

    func testTwoPermissionModesAndPropsReasoningDiscovery() throws {
        // 6a. Strictly two permission modes: cautious and allow_all (no accept_edits).
        XCTAssertEqual(MacPermissionMode.allCases, [.cautious, .allowAll])
        XCTAssertEqual(try MacPermissionMode.configured(nil), .cautious)
        XCTAssertEqual(try MacPermissionMode.configured("cautious"), .cautious)
        XCTAssertEqual(try MacPermissionMode.configured("allow_all"), .allowAll)
        XCTAssertThrowsError(try MacPermissionMode.configured("accept_edits"))

        // 6b. /props URL construction strips /v1 and /models.
        let propsReq = try XCTUnwrap(
            AppleModelCatalogDiscovery.propsRequest(
                endpoint: "http://192.168.31.18:8081/v1",
                secret: "test-key"
            )
        )
        XCTAssertEqual(propsReq.url?.absoluteString, "http://192.168.31.18:8081/props")
        XCTAssertEqual(propsReq.value(forHTTPHeaderField: "Authorization"), "Bearer test-key")

        // 6c. /props enrichment populates reasoning_levels, vision, toolCalls, n_ctx, n_predict.
        let modelsURL = URL(string: "http://192.168.31.18:8081/v1/models")!
        let modelsResp = HTTPURLResponse(url: modelsURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
        let modelsJSON = Data("""
        {
          "object": "list",
          "data": [
            {
              "id": "qwen3.8-flash-next-iq3_s",
              "max_model_len": 262144,
              "architecture": { "input_modalities": ["text", "image"] }
            }
          ]
        }
        """.utf8)
        let parsed = try AppleModelCatalogDiscovery.parse(data: modelsJSON, response: modelsResp)
        XCTAssertEqual(parsed.count, 1)
        XCTAssertTrue(parsed[0].reasoningLevels.isEmpty)

        let propsURL = URL(string: "http://192.168.31.18:8081/props")!
        let propsResp = HTTPURLResponse(url: propsURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
        let propsJSON = Data("""
        {
          "model_alias": "qwen3.8-flash-next-iq3_s",
          "default_generation_settings": { "n_ctx": 262144, "params": { "n_predict": 16384 } },
          "chat_template_caps": {
            "supports_tools": true,
            "supports_tool_calls": true,
            "supports_preserve_reasoning": true
          },
          "modalities": { "vision": true }
        }
        """.utf8)
        let enriched = AppleModelCatalogDiscovery.enrichWithProps(models: parsed, data: propsJSON, response: propsResp)
        XCTAssertEqual(enriched.count, 1)
        XCTAssertEqual(enriched[0].reasoningSupported, true)
        XCTAssertEqual(enriched[0].reasoningLevels, ["none", "low", "medium", "high"])
        XCTAssertEqual(enriched[0].toolCalls, true)
        XCTAssertEqual(enriched[0].outputLimit, 16384)
        XCTAssertEqual(enriched[0].contextWindow, 262144)
        XCTAssertTrue(enriched[0].modalities.contains("image"))

        // 6d. OpenRouter supported_parameters reasoning inference.
        let orURL = URL(string: "https://openrouter.ai/api/v1/models")!
        let orResp = HTTPURLResponse(url: orURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
        let orJSON = Data("""
        {
          "data": [
            {
              "id": "deepseek/deepseek-r1",
              "name": "DeepSeek R1",
              "context_length": 131072,
              "supported_parameters": ["tools", "reasoning", "temperature"]
            }
          ]
        }
        """.utf8)
        let orParsed = try AppleModelCatalogDiscovery.parse(data: orJSON, response: orResp)
        XCTAssertEqual(orParsed.count, 1)
        XCTAssertEqual(orParsed[0].reasoningSupported, true)
        XCTAssertEqual(orParsed[0].reasoningLevels, ["none", "low", "medium", "high"])
        XCTAssertEqual(orParsed[0].toolCalls, true)
        XCTAssertEqual(orParsed[0].samplingParameters, true)

        // 6e. Core refresh_connection_catalog MUST re-sync ALREADY-ENABLED model profiles
        // so clicking "Discover" immediately unlocks reasoning effort in the composer!
        let root = try makeTempDirectory("reasoning-profile-sync")
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("reasoning.db"))
        let connService = MacModelConnectionService(application: port)
        let created = try connService.create(
            name: "Props Test Server",
            endpoint: "http://192.168.31.18:8081/v1",
            wireProtocol: "chat_completions",
            secret: ""
        )
        // Step 1: initial catalog without /props (no reasoning) -> enable model
        let initialEntries: [MacCanonicalJSONValue] = try parsed.map {
            try JSONDecoder().decode(MacCanonicalJSONValue.self, from: $0.catalogData(wireProtocol: "chat_completions"))
        }
        let initialCatalog: MacModelConnectionSnapshot = try connService.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(created.revision),
            "models": .array(initialEntries),
            "observed_at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1_000)),
        ])
        let modelKey = try XCTUnwrap(initialCatalog.document.catalog?.models.first?.id)
        try connService.enable(initialCatalog, modelIDs: [modelKey])

        let configStore = MacAppleChatConfigurationStore(applicationPort: port)
        let optionBefore = try XCTUnwrap(
            configStore.configuredModels().first(where: { $0.modelName == "qwen3.8-flash-next-iq3_s" })
        )
        let configBefore = try XCTUnwrap(configStore.load(model: optionBefore.profileID))
        XCTAssertTrue(configBefore.reasoning?.options.isEmpty ?? true)

        // Step 2: re-discover WITH /props enriched models -> profile must re-sync WITHOUT re-toggling!
        let enrichedEntries: [MacCanonicalJSONValue] = try enriched.map {
            try JSONDecoder().decode(MacCanonicalJSONValue.self, from: $0.catalogData(wireProtocol: "chat_completions"))
        }
        let _: MacModelConnectionSnapshot = try connService.command([
            "type": .string("refresh_connection_catalog"),
            "connection_id": .string(created.id),
            "expected_revision": .signedInteger(initialCatalog.revision),
            "models": .array(enrichedEntries),
            "observed_at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1_000) + 10),
        ])

        let optionAfter = try XCTUnwrap(
            configStore.configuredModels().first(where: { $0.modelName == "qwen3.8-flash-next-iq3_s" })
        )
        let loadedConfig = try XCTUnwrap(configStore.load(model: optionAfter.profileID))
        XCTAssertEqual(loadedConfig.reasoning?.control, .effort)
        XCTAssertEqual(loadedConfig.reasoning?.options, ["none", "low", "medium", "high"])
    }

    // MARK: - 7. Math display block promotion and exact typographic baseline alignment

    func testMathBlockParsingAndTypographicBaselineMetrics() throws {
        let md = """
        核心恒等式如下：

        $$S_s = \\lambda S_t + UV^\\top,\\quad \\lambda = D_s/D_t$$

        其中残差宽度为 $p = n_s + n_t$。
        """
        let blocks = MacMarkdownTextParser.parse(md)
        XCTAssertTrue(
            blocks.contains(where: {
                if case .mathBlock(let latex) = $0 {
                    return latex.contains("S_s = \\lambda S_t")
                }
                return false
            }),
            "Standalone $$...$$ equations must be promoted to dedicated .mathBlock blocks: \(blocks)"
        )

        let simple = try XCTUnwrap(MacInlineMarkdownCache.renderMath(latex: "r", display: false))
        let fraction = try XCTUnwrap(MacInlineMarkdownCache.renderMath(latex: "\\frac{D_s}{D_t}", display: false))
        let displayEq = try XCTUnwrap(MacInlineMarkdownCache.renderMath(latex: "S_s = \\lambda S_t + UV^\\top", display: true))
        XCTAssertGreaterThan(
            abs(fraction.baselineOffset),
            abs(simple.baselineOffset) + 2.0,
            "Fractions with deeper descent must get a deeper negative baselineOffset than simple symbols (simple=\(simple.baselineOffset), fraction=\(fraction.baselineOffset))"
        )
        XCTAssertGreaterThan(displayEq.image.size.width, 40)
        XCTAssertGreaterThan(displayEq.image.size.height, 14)
    }
}
