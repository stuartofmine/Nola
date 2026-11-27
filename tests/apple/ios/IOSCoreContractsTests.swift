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
}
