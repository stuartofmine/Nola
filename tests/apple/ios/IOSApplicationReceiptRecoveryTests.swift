// SPDX-License-Identifier: Apache-2.0
import XCTest
import SQLite3
@testable import Nola
#if canImport(NolaCore)
import NolaCore
#endif

final class IOSApplicationReceiptRecoveryTests: XCTestCase {
    func testIncompatibleStoreReportsOriginalErrorAndPreservesDatabase() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("incompatible.db")
        do { _ = try IOSApplicationRuntime(databaseURL: database) }
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &connection), SQLITE_OK)
        XCTAssertEqual(sqlite3_exec(connection,
            "UPDATE store_metadata SET value='4' WHERE key='schema_epoch'; PRAGMA wal_checkpoint(TRUNCATE);",
            nil, nil, nil), SQLITE_OK)
        XCTAssertEqual(sqlite3_close(connection), SQLITE_OK)
        let original = try Data(contentsOf: database)
        for _ in 0..<2 {
            XCTAssertThrowsError(try IOSApplicationRuntime(databaseURL: database)) { error in
                guard case IOSApplicationRuntimeError.openFailed(let detail) = error else {
                    return XCTFail("Expected original Store open failure, received \(error)")
                }
                XCTAssertTrue(detail.contains(database.path))
                XCTAssertTrue(detail.lowercased().contains("unsupported database format"), detail)
            }
            XCTAssertEqual(try Data(contentsOf: database), original)
        }
    }

    func testColdOpenReplaysOriginalCancellationWithoutAdvancingRunAgain() throws {
        #if canImport(NolaCore) && DEBUG
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("cancel.db")
        let operation: IOSCanonicalCancellationOperation = try {
            let fixture = try IOSCanonicalChatJournalFixture(databaseURL: database, orderingMarker: "cancel-replay")
            let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
            let run = try fixture.begin([
                "conversationId": UUID().uuidString, "submission_id": identity.submissionID,
                "client_id": "fixture-client", "expected_host_id": IOSCanonicalChatJournalFixture.sourceHostID,
                "user_message_id": identity.userMessageID, "assistant_message_id": identity.assistantMessageID,
                "messages": [["role": "user", "content": "Cancellation replay fixture"]],
            ])
            // Enter a real model delivery before interruption, so cancellation
            // must await that Host effect rather than infer a terminal outcome.
            _ = try fixture.advance(run, stage: .reasoning)
            return try run.application.prepareCancellation(reason: "user requested stop")
        }()
        var command = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(operation.command)) as? [String: Any])
        let type = try XCTUnwrap(command.removeValue(forKey: "type") as? String)
        let requestID = "original-cancellation"
        let envelope: [String: Any] = ["protocol_version": nola_core_application_service_protocol_version(),
            "client_id": "ios-application", "request_id": requestID,
            "command": ["type": type, "arguments": command]]
        func retainOriginalCancellation() throws {
            let transaction = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
            defer { transaction.close() }
            try transaction.save([requestID: envelope])
        }
        try retainOriginalCancellation()
        var runtime: IOSApplicationRuntime? = try IOSApplicationRuntime(databaseURL: database)
        let lookup: AgentCoreJSONValue = .object(["type": .string("command_receipt"), "request_id": .string(requestID)])
        let receipt = try runtime!.executeAgentCommand(lookup)
        guard case .object(let fields) = receipt, case .object(let result)? = fields["result"],
              case .object(let control)? = result["control"] else { return XCTFail("Missing original cancellation receipt") }
        XCTAssertEqual(fields["status"], .string("committed"))
        XCTAssertEqual(fields["request_id"], .string(requestID))
        XCTAssertEqual(control["turn_status"], .string("cancelling"))
        let checkpointQuery: AgentCoreJSONValue = .object(["type": .string("load_checkpoint"),
            "conversation_id": .string(operation.conversationID)])
        let checkpoint = try runtime!.executeAgentCommand(checkpointQuery)
        runtime = nil
        // Lost acknowledgement after commit: cold recovery must return the same
        // receipt and checkpoint, not introduce a successor model or transition.
        try retainOriginalCancellation()
        runtime = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(try runtime!.executeAgentCommand(lookup), receipt)
        XCTAssertEqual(try runtime!.executeAgentCommand(checkpointQuery), checkpoint)
        let transaction = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        defer { transaction.close() }
        XCTAssertTrue(try transaction.load().isEmpty)
        runtime = nil
        #else
        XCTFail("NolaCore and canonical debug fixture are required")
        #endif
    }

    func testOpeningReplaysExactPendingEnvelopeWithoutRepeatingCommittedWrite() throws {
        #if canImport(NolaCore)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("receipt.db")
        let requestID = "interrupted-create"
        let envelope: [String: Any] = [
            "protocol_version": nola_core_application_service_protocol_version(),
            "client_id": "ios-application", "request_id": requestID,
            "command": ["type": "create_conversation", "arguments": ["input": [
                "id": "receipt-conversation", "title": "Original", "created_at_ms": 1, "updated_at_ms": 1
            ]]]
        ]
        let journal = database.appendingPathExtension("pending-commands.json")
        let retained = [requestID: envelope]
        func retainOriginalCommand() throws {
            let transaction = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
            defer { transaction.close() }
            try transaction.save(retained)
        }
        try retainOriginalCommand()
        var runtime: IOSApplicationRuntime? = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(try runtime!.listConversations().count, 1)
        runtime = nil
        // Simulate a crash after Core committed but before the client removed
        // its durable delivery record. The original request identity survives.
        try retainOriginalCommand()
        runtime = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(try runtime!.listConversations().count, 1)
        let remaining = try JSONSerialization.jsonObject(with: Data(contentsOf: journal)) as? [String: Any]
        XCTAssertEqual(remaining?.count, 0)
        let lookup: AgentCoreJSONValue = .object([
            "type": .string("command_receipt"), "request_id": .string(requestID)
        ])
        let receipt = try runtime!.executeAgentCommand(lookup)
        guard case .object(let fields) = receipt else { return XCTFail("Missing durable receipt") }
        XCTAssertEqual(fields["request_id"], .string(requestID))
        runtime = nil
        #else
        XCTFail("NolaCore is required for the native receipt recovery acceptance test")
        #endif
    }
}
