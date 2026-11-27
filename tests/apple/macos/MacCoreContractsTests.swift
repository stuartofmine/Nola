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
}
