// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS application command effects, receipt recovery, receipt validation, and pending command journal suite.

import Darwin
import Foundation
import SQLite3
import XCTest
@testable import Nola
#if canImport(NolaCore)
import NolaCore
#endif

final class IOSApplicationCommandEffectTests: XCTestCase {
    func testChildConversationCatalogIsObservationAndUnknownWritesRemainOrdered() {
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "read_expert_exchange_capability", arguments: [:]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "runtime_task", arguments: ["command": ["type": "get"]]))
        XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: "runtime_task", arguments: ["command": ["type": "cancel"]]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "list_root_conversations", arguments: [:]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": "peer_exchanges"]]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": "child_history"]]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": "child_conversations"]]))
        XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": "delete_child_conversation"]]))
    }

    func testCompactionPlanningIsObservationalAndCommitKeepsMutationOrdering() {
        for type in ["step_compaction_summary", "evaluate_compaction_maintenance"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: [:]))
        }
        for kind in ["count_checkpoints", "prepare_compaction", "prepare_maintenance", "load_checkpoint"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "context", arguments: ["command": ["type": kind]]))
        }
        for kind in ["commit_maintenance", "replace_summary", "commit_checkpoint", "unknown"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: "context", arguments: ["command": ["type": kind]]))
        }
    }

    func testRejectedSummaryPlanNeverLeavesAnUncertainWriteFence() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("summary-budget-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("fixture.sqlite")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        do {
            _ = try runtime.executeAgentCommand(.object([
                "type": .string("step_compaction_summary"), "step": .object([
                    "type": .string("start"), "messages": .array([.object(["role": .string("user"), "content": .string("Fixture source")])]),
                    "context_window_tokens": .signedInteger(8_000), "output_tokens": .signedInteger(4_096),
                    "target_tokens": .signedInteger(512), "instructions": .string("Preserve source facts.")
                ])]))
            XCTFail("The summary request cannot fit this conservative input budget")
        } catch IOSApplicationRuntimeError.rejected(let detail) {
            XCTAssertTrue(detail.contains("context window leaves no room"))
        } catch { XCTFail("A pure planning failure cannot be a pending mutation: \(error)") }
        let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .application,
            validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        do {
            let transaction = try journal.beginTransaction()
            defer { transaction.close() }
            XCTAssertTrue(try transaction.load().isEmpty)
        }
        try runtime.createConversation(id: "after-budget-refusal", title: "Still usable", settingsJSON: nil,
            createdAtMs: 1, updatedAtMs: 1)
        XCTAssertNotNil(try runtime.loadConversation(id: "after-budget-refusal"))
    }

    func testCurrentRunAndRemoteRecoveryObservationsNeverEnterMutationJournal() {
        for type in ["load_committed_host_requests", "load_runtime_effect_work_schedule", "load_host_execution_state",
                     "query_claimable_memory_jobs", "build_memory_worker_plan", "read_remote_submission_intent",
                     "capture_mobile_delegation_configuration", "load_mobile_task_child_configuration", "load_mobile_task_wake_plan",
                     "inspect_mobile_permission_request", "inspect_mobile_mcp_request", "inspect_owned_task_recovery",
                     "load_today_facts", "list_recoverable_runs", "list_open_user_input_waits", "list_owned_task_user_input_waits",
                     "inspect_run_process_owners", "validate_run_process_observations",
                     "resolve_remote_model_profile", "query_automation_occurrences"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: [:]), type)
        }
        for type in ["open_assistant_journal_snapshot", "release_assistant_journal_snapshot",
                     "claim_committed_host_requests", "heartbeat_effect_delivery", "recover_observed_orphaned_run"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: [:]), type)
        }
    }

    func testExpertPlanReadsUseReadBoundaryWhileAdmissionAndClaimsRetainMutationOrdering() {
        for type in ["load_prepared_expert_resend", "load_expert_message_manifest", "load_edited_resend_child_instructions"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: [:]), "Private expert reads cannot be forwarded as renderer commands")
        }
        for kind in ["role_templates", "for_conversation", "submission", "submissions", "execution_intent"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": kind]]), kind)
        }
        for type in ["admit_expert_conversation_message", "prepare_edited_expert_resend", "prepare_edited_resend"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: [:]), type)
        }
        for kind in ["rejoin", "ensure_for_conversation", "submit", "prepare_execution", "claim_execution", "cancel_pending", "reconcile_execution"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": kind]]), kind)
        }
    }

    func testUnrecoverableContextIntentDoesNotPreventColdRecoveryOrWritesForAnotherContext() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI is unavailable") }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("fixture.db")
        var runtime: IOSApplicationRuntime? = try IOSApplicationRuntime(databaseURL: database)
        runtime = nil
        let unresolved: [String: Any] = ["client_id": "ios-application", "request_id": "a-uncertain", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
            "command": ["type": "resolve_unknown_effect_recovery", "arguments": ["input": ["conversation_id": "context-a"]]]]
        let create: [String: Any] = ["client_id": "ios-application", "request_id": "b-create", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
            "command": ["type": "create_conversation", "arguments": ["input": [
                "id": "context-b", "title": "B", "settings_json": NSNull(), "created_at_ms": 1, "updated_at_ms": 1]]]]
        let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        let initial = try journal.beginTransaction()
        try initial.save(["a-uncertain": unresolved, "b-create": create])
        initial.close()
        runtime = try IOSApplicationRuntime(databaseURL: database)
        let recovered = try runtime!.executeAgentCommand(.object([
            "type": .string("load_conversation"), "conversation_id": .string("context-b")]))
        XCTAssertNotEqual(recovered, .null)
        XCTAssertNoThrow(try runtime!.executeAgentCommand(.object([
            "type": .string("rename_conversation"), "conversation_id": .string("context-b"),
            "title": .string("B renamed"), "updated_at_ms": .signedInteger(2)])))
        XCTAssertThrowsError(try runtime!.executeAgentCommand(.object([
            "type": .string("rename_conversation"), "conversation_id": .string("context-a"),
            "title": .string("A blocked"), "updated_at_ms": .signedInteger(2)])))
        XCTAssertNil(try runtime!.productSetting(key: "read-remains-available"))
        XCTAssertNoThrow(try runtime!.executeAgentCommand(.object([
            "type": .string("load_committed_host_requests"), "run_id": .string("no-run-in-context-b")])))
        XCTAssertEqual(try runtime!.executeAgentCommand(.object([
            "type": .string("read_remote_submission_intent"), "conversation_id": .string("context-b")])), .null)
        let final = try journal.beginTransaction()
        defer { final.close() }
        XCTAssertEqual(try final.load() as NSDictionary, ["a-uncertain": unresolved] as NSDictionary)
    }

    func testExactProtocolVariantsNotNamesOrPayloadWordsChooseObservation() {
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "load_conversation", arguments: ["conversation_id": "delete 删除"]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "inspect_unknown_effect_recovery", arguments: [:]))
        XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "model", arguments: ["command": ["type": "get_profile"]]))
        for kind in ["get_connection", "list_connections", "get_account", "list_accounts"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "model", arguments: ["command": ["type": kind]]))
        }
        for kind in ["submissions", "execution_intent"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": kind]]))
        }
        for kind in ["prepare_execution", "claim_execution", "reconcile_execution", "cancel_pending"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: "agent_room", arguments: ["command": ["type": kind]]))
        }
        for kind in ["inspect", "inspect_source"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: "auxiliary_invocation", arguments: ["command": ["type": kind]]))
        }
        for kind in ["admit", "start_attempt", "record_receipt", "cancel", "observe_source"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: "auxiliary_invocation", arguments: ["command": ["type": kind]]))
        }
        for kind in ["load_auxiliary_source_slot", "load_auxiliary_operation", "list_conversation_auxiliary_sources",
                     "validate_mobile_run_recovery_authority", "load_session_projection", "load_goal_control_observation"] {
            XCTAssertTrue(IOSApplicationRuntime.isReadOnlyCommand(type: kind, arguments: [:]))
        }
        for type in ["get_future_mutation", "load_and_delete", "resolve_unknown_effect_recovery", "create_conversation"] {
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: ["type": "list", "text": "read only"]))
        }
        XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: "model", arguments: ["command": ["type": "reset_parameters"]]))
    }

    func testMissingAuxiliarySourceObservationDoesNotCreatePendingMutation() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI is unavailable") }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("fixture.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertThrowsError(try runtime.executeAuxiliaryInvocation([
            "type": .string("inspect_source"),
            "local_owner": .object(["kind": .string("account"), "account_id": .string("fixture")]),
            "source_host_id": .string("fixture-host"), "operation_id": .string("missing")
        ]))
        let transaction = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        let pending = try transaction.load()
        transaction.close()
        XCTAssertTrue(pending.isEmpty)
        _ = try runtime.putProductSetting(key: "after-missing-source", valueJSON: "true", updatedAtMs: 1)
    }

    func testReadPreservesPendingDecisionAndMutationCannotOvertakeIt() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI is unavailable") }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("fixture.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        let envelope: [String: Any] = ["client_id": "ios-application", "request_id": "original", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
            "command": ["type": "resolve_unknown_effect_recovery", "arguments": ["input": [
                "conversation_id": "conversation", "run_id": "run", "invocation_config_digest": "digest",
                "resolution_id": "resolution", "resolved_at_ms": 7,
                "choice": ["kind": "keep_unknown_blocked", "reason": "User choice"],
                "targets": [["effect_id": "effect", "request_id": "effect-request", "attempt_id": "attempt",
                    "worker_generation": "generation", "lease_owner": "owner", "lease_token": "token", "expected_attempt_revision": 1]]
            ]]]]
        let transaction = try journal.beginTransaction()
        try transaction.save(["original": envelope])
        transaction.close()
        XCTAssertNil(try runtime.productSetting(key: "missing"))
        XCTAssertThrowsError(try runtime.putProductSetting(key: "new", valueJSON: "true", updatedAtMs: 8))
        let readback = try journal.beginTransaction()
        defer { readback.close() }
        XCTAssertEqual(try readback.load() as NSDictionary, ["original": envelope] as NSDictionary)
    }
    func testRetainedReadCannotKeepGoalControlsBehindAnUncertainWriteFence() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI is unavailable") }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("fixture.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        let envelope: [String: Any] = ["client_id":"ios-application", "request_id":"read", "protocol_version":IOSApplicationRuntime.expectedServiceProtocol,
            "command":["type":"validate_mobile_run_recovery_authority", "arguments":[:]]]
        let pending = try journal.beginTransaction(); try pending.save(["read":envelope]); pending.close()
        _ = try runtime.recoverPendingCommands()
        let readback = try journal.beginTransaction(); XCTAssertTrue(try readback.load().isEmpty); readback.close()
        _ = try runtime.putProductSetting(key:"after-read", valueJSON:"true", updatedAtMs:1)
    }

}

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

final class IOSApplicationReceiptValidationTests: XCTestCase {
    func testReceiptFailureDiagnosticContainsShapeNotPayloadOrIdentity() throws {
        let secret = "private-user-text-and-credential"
        let response: [String: Any] = ["ok": false, "error": ["message": secret],
            "result": ["request_id": secret, "client_id": secret, "status": secret,
                       "result": ["content": secret]]]
        let input: [String: Any] = ["request_id": "original", "client_id": "ios-application",
            "command": ["type": "reduce_and_commit_active_submission", "arguments": ["text": secret]]]
        let diagnostic = IOSApplicationRuntime.receiptFailureDiagnostic(
            try JSONSerialization.data(withJSONObject: response), input: input)
        XCTAssertTrue(diagnostic.contains("command=reduce_and_commit_active_submission"))
        XCTAssertTrue(diagnostic.contains("outer_ok=false"))
        XCTAssertTrue(diagnostic.contains("request_matches=false"))
        XCTAssertFalse(diagnostic.contains(secret))
        XCTAssertFalse(diagnostic.contains("original"))
    }

    private let input: [String: Any] = ["protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
        "client_id": "ios-application", "request_id": "original"]
    private func receipt(_ status: String) -> [String: Any] {
        var value: [String: Any] = ["protocol_version": IOSApplicationRuntime.expectedServiceProtocol, "client_id": "ios-application",
            "request_id": "original", "received_at": "2026-10-12T00:00:00Z", "status": status]
        if status != "accepted" { value["completed_at"] = "2026-10-12T00:00:01Z" }
        return value
    }
    private func decode(_ value: [String: Any]) throws -> [String: Any] {
        try IOSApplicationRuntime.decodeApplicationResponse(
            JSONSerialization.data(withJSONObject: value), input: input)
    }
    func testOuterNonrecoverableFailureIsNotRejectedReceipt() {
        XCTAssertThrowsError(try decode(["ok": false, "error": ["recoverable": false,
            "code": "internal", "message": "failure"]])) { error in
            guard case IOSApplicationRuntimeError.outcomePending("original") = error else {
                return XCTFail("Outer error retired the intent")
            }
        }
    }
    func testObservationFailurePreservesCoreErrorWithoutInventingPendingMutation() throws {
        var query = input
        query["command"] = ["type": "load_conversation_projection_window_v1", "arguments": [:]]
        let message = "decode canonical Core checkpoint failed: missing field control_generation"
        let bytes = try JSONSerialization.data(withJSONObject: ["ok": false,
            "error": ["code": "application_rejected", "message": message, "recoverable": false]])
        XCTAssertThrowsError(try IOSApplicationRuntime.decodeApplicationResponse(bytes, input: query)) { error in
            guard case IOSApplicationRuntimeError.rejected(let detail) = error else {
                return XCTFail("Read failure was presented as a pending mutation")
            }
            XCTAssertEqual(detail, message)
        }
        let accepted = try JSONSerialization.data(withJSONObject: ["ok": true, "result": receipt("accepted")])
        XCTAssertThrowsError(try IOSApplicationRuntime.decodeApplicationResponse(accepted, input: query)) { error in
            guard case IOSApplicationRuntimeError.invalidResponse = error else {
                return XCTFail("Read-only query cannot have durable accepted status")
            }
        }
        XCTAssertThrowsError(try IOSApplicationRuntime.decodeApplicationResponse(Data("{}".utf8), input: query)) { error in
            guard case IOSApplicationRuntimeError.invalidResponse = error else {
                return XCTFail("Malformed read response was presented as accepted")
            }
        }
    }

    func testOnlyCompleteCanonicalOutcomesValidate() throws {
        var committed = receipt("committed")
        committed["result"] = NSNull()
        XCTAssertEqual(try decode(["ok": true, "result": committed])["status"] as? String, "committed")
        var rejected = receipt("rejected")
        rejected["error"] = ["code": "denied", "message": "denied", "recoverable": false]
        XCTAssertNoThrow(try decode(["ok": true, "result": rejected]))
        var conflict = receipt("conflict")
        conflict["conflict"] = ["scope_id": "run", "expected_revision": 1, "actual_revision": 2]
        XCTAssertNoThrow(try decode(["ok": true, "result": conflict]))
        XCTAssertNoThrow(try decode(["ok": true, "result": receipt("accepted")]))
    }
    func testMalformedTerminalReceiptsRemainPending() {
        for status in ["committed", "rejected", "conflict", "unknown"] {
            XCTAssertThrowsError(try decode(["ok": true, "result": receipt(status)]))
        }
        var rejected = receipt("rejected")
        rejected["error"] = ["message": "not enough"]
        XCTAssertThrowsError(try decode(["ok": true, "result": rejected]))
        var conflict = receipt("conflict")
        conflict["conflict"] = ["scope_id": "run", "expected_revision": -1, "actual_revision": 2]
        XCTAssertThrowsError(try decode(["ok": true, "result": conflict]))
        conflict["conflict"] = ["scope_id": "run", "expected_revision": true, "actual_revision": 2]
        XCTAssertThrowsError(try decode(["ok": true, "result": conflict]))
    }
    func testWrongIdentityMissingTimestampAndZeroRevisionRemainPending() {
        var good = receipt("committed")
        good["result"] = [:] as [String: String]
        for key in ["request_id", "client_id", "received_at", "completed_at"] {
            var missing = good
            missing.removeValue(forKey: key)
            XCTAssertThrowsError(try decode(["ok": true, "result": missing]))
        }
        var wrong = good
        wrong["request_id"] = "other"
        XCTAssertThrowsError(try decode(["ok": true, "result": wrong]))
        wrong = good
        wrong["committed_revision"] = 0
        XCTAssertThrowsError(try decode(["ok": true, "result": wrong]))
    }

    func testRecoveryQueriesOriginalReceiptBeforeAnyReplay() throws {
        var calls = 0
        var stored = receipt("committed")
        stored["result"] = NSNull()
        let recovered = try IOSApplicationRuntime.recoverApplicationEnvelope(input) { query in
            calls += 1
            XCTAssertEqual((query["command"] as? [String: Any])?["type"] as? String, "command_receipt")
            var response = self.receipt("committed")
            response["request_id"] = query["request_id"]
            response["result"] = stored
            return response
        }
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(recovered["request_id"] as? String, "original")
    }

    func testOnlyExplicitMissingReceiptPermitsExactReplay() throws {
        var calls: [[String: Any]] = []
        _ = try IOSApplicationRuntime.recoverApplicationEnvelope(input) { request in
            calls.append(request)
            var response = self.receipt("committed")
            response["request_id"] = request["request_id"]
            response["result"] = NSNull()
            return response
        }
        XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(calls[1] as NSDictionary, input as NSDictionary)
        calls = []
        XCTAssertThrowsError(try IOSApplicationRuntime.recoverApplicationEnvelope(input) { request in
            calls.append(request)
            throw IOSApplicationRuntimeError.outcomePending("lookup")
        })
        XCTAssertEqual(calls.count, 1)
    }
}

final class IOSPendingCommandJournalTests: XCTestCase {
    func testExpertHostNamespaceDoesNotConsumeRendererIntentsAndScopesIndependentOwners() throws {
        let database = try directory().appendingPathComponent("product.db")
        func operation(parent: String, child: String) -> [String: Any] {
            ["kind": "admit", "input": ["conversation_id": parent, "selected_participants": [["context_conversation_id": child]]],
             "context": ["actor": ["kind": "local_user", "id": "actor"], "request_host_id": NSNull()]]
        }
        func hostEnvelope(_ id: String, operation: [String: Any]) -> [String: Any] {
            ["request_id": id, "client_id": "native-expert-host", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
             "command": ["type": "host_expert_template_operation", "arguments": ["operation": operation]]]
        }
        let ordinary = ApplePendingCommandJournal(databaseURL: database, namespace: .application,
            validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        let source = try ordinary.beginTransaction()
        try source.save(["ordinary": envelope("ordinary")]); source.close()
        let host = ApplePendingCommandJournal(databaseURL: database, namespace: .expertHost,
            validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
        let privateScope = try host.beginTransaction()
        XCTAssertTrue(try privateScope.load().isEmpty, "A renderer intent can never become a trusted Host operation")
        let a = operation(parent: "parent-a", child: "child-a"), b = operation(parent: "parent-b", child: "child-b")
        try privateScope.save(["a": hostEnvelope("a", operation: a), "b": hostEnvelope("b", operation: b)])
        XCTAssertEqual(try privateScope.load().count, 2)
        XCTAssertTrue(AppleExpertHostIntentScope.intersects(a, operation(parent: "other-parent", child: "child-a")))
        XCTAssertFalse(AppleExpertHostIntentScope.intersects(a, b))
        XCTAssertTrue(AppleExpertHostIntentScope.intersects(a, ["kind": "unknown"]))
        XCTAssertThrowsError(try privateScope.save(["a": hostEnvelope("a", operation: a),
            "overlap": hostEnvelope("overlap", operation: operation(parent: "other-parent", child: "child-a"))]))
        XCTAssertEqual(try privateScope.load().count, 2, "A rejected overlap must not rewrite prior intents")
        privateScope.close()
        let ordinaryAgain = try ordinary.beginTransaction()
        defer { ordinaryAgain.close() }
        XCTAssertNotNil(try ordinaryAgain.load()["ordinary"])
    }

    private func directory() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
                                               attributes: [.posixPermissions: 0o700])
        addTeardownBlock { try FileManager.default.removeItem(at: root) }
        return root
    }
    private func envelope(_ id: String = "original", content: String = "original payload") -> [String: Any] {
        ["request_id": id, "client_id": "ios-application", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
         "expected_revision": 7, "command": ["type": "fixture", "arguments": ["content": content]]]
    }

    private func contextEnvelope(_ id: String, owner: String, type: String = "reduce_and_accept_host_response") -> [String: Any] {
        ["request_id": id, "client_id": "ios-application", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
         "command": ["type": type, "arguments": ["input": ["conversation_id": owner]]]]
    }

    func testIndependentContextsSurviveReopenAndOnlyExactReceiptOwnerIsRetired() throws {
        let database = try directory().appendingPathComponent("product.db")
        let a = contextEnvelope("a", owner: "context-a")
        let b = contextEnvelope("b", owner: "context-b")
        let initial = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        try initial.save(["a": a, "b": b])
        initial.close()
        let reopened = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        defer { reopened.close() }
        var pending = try reopened.load()
        XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("c", owner: "context-c"), pending: pending, resolve: { $0 }))
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("a-next", owner: "context-a"), pending: pending, resolve: { $0 }), "a")
        pending.removeValue(forKey: "b")
        try reopened.save(pending)
        XCTAssertEqual(try JSONSerialization.data(withJSONObject: reopened.load()["a"]!, options: [.sortedKeys]),
                       try JSONSerialization.data(withJSONObject: a, options: [.sortedKeys]))
    }

    func testTypedOwnershipRejectsMalformedKnownCommandsAndKeepsUnknownCommandsGlobal() throws {
        let a = contextEnvelope("a", owner: "a")
        XCTAssertEqual(try IOSPendingCommandOwner.decode(contextEnvelope("x", owner: "a", type: "future_conversation_command")), .global)
        var malformed = a
        malformed["command"] = ["type": "reduce_and_accept_host_response", "arguments": ["conversation_id": "a"]]
        XCTAssertThrowsError(try IOSPendingCommandOwner.decode(malformed))
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: envelope("global"), pending: ["a": a], resolve: { $0 }), "a")
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: a, pending: ["global": envelope("global")], resolve: { $0 }), "global")
    }

    func testJournalSnapshotLeasesAreContextWritesNotGlobalHistoryBarriers() throws {
        let cursor: [String: Any] = ["identity": ["conversation_id": "context-b", "run_id": "run-b",
            "assistant_message_id": "assistant-b", "source_store_epoch": "epoch"],
            "sequence": "1", "record_digest": String(repeating: "a", count: 64)]
        let commands: [(String, [String: Any])] = [
            ("open_assistant_journal_snapshot", ["at": cursor]),
            ("release_assistant_journal_snapshot", ["snapshot": ["snapshot_id": "snapshot-b", "at": cursor]])
        ]
        for (type, arguments) in commands {
            let request: [String: Any] = ["request_id": "snapshot", "client_id": "ios", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
                "command": ["type": type, "arguments": arguments]]
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: arguments))
            XCTAssertEqual(try IOSPendingCommandOwner.decode(request), .conversation("context-b"))
            XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: request,
                pending: ["a": contextEnvelope("a", owner: "context-a")], resolve: { $0 }))
            XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: request,
                pending: ["b": contextEnvelope("b", owner: "context-b")], resolve: { $0 }), "b")
        }
    }

    func testQueuedAdmissionsSerializeWithTheirConversationOnly() throws {
        for type in ["begin_queued_user_turn", "promote_queued_assistant_turn"] {
            let queued = contextEnvelope("queued", owner: "context-a", type: type)
            XCTAssertEqual(try IOSPendingCommandOwner.decode(queued), .conversation("context-a"))
            XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(
                for: contextEnvelope("b", owner: "context-b"), pending: ["queued": queued], resolve: { $0 }))
            XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(
                for: contextEnvelope("a", owner: "context-a"), pending: ["queued": queued], resolve: { $0 }), "queued")
        }
    }

    func testAutomationAdmissionUsesItsConversationOwnerWithoutBlockingAnotherContext() throws {
        let automation = contextEnvelope("automation", owner: "context-a", type: "begin_automation_assistant_turn")
        XCTAssertEqual(try IOSPendingCommandOwner.decode(automation), .conversation("context-a"))
        XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(
            for: contextEnvelope("b", owner: "context-b"), pending: ["automation": automation], resolve: { $0 }))
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(
            for: contextEnvelope("a", owner: "context-a"), pending: ["automation": automation], resolve: { $0 }), "automation")
    }

    func testRemoteJournalConsumerWritesRetainExactContextOrdering() throws {
        let identity: [String: Any] = ["conversation_id": "remote-a"]
        let cursor: [String: Any] = ["identity": identity]
        let record: [String: Any] = ["header": cursor]
        let cases: [(String, [String: Any])] = [
            ("accept_remote_run_context_usage", ["record": cursor]),
            ("receive_run_message_admissions", ["page": ["through": cursor]]),
            ("receive_assistant_journal_record", ["record": record]),
            ("stage_assistant_journal_content", ["input": ["record": record]]),
            ("begin_receive_assistant_run_terminal", ["seal": cursor]),
            ("stage_assistant_run_terminal_manifest", ["page": ["seal": cursor]]),
            ("commit_received_assistant_run_terminal", ["seal": cursor])
        ]
        for (type, arguments) in cases {
            let pending: [String: Any] = ["request_id": "remote", "client_id": "ios", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
                "command": ["type": type, "arguments": arguments]]
            XCTAssertEqual(try IOSPendingCommandOwner.decode(pending), .conversation("remote-a"), type)
            XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("b", owner: "remote-b"),
                pending: ["remote": pending], resolve: { $0 }), type)
            XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("a", owner: "remote-a"),
                pending: ["remote": pending], resolve: { $0 }), "remote", type)
        }
    }

    func testRunOnlyCommandUsesCanonicalConversationOwnerRatherThanIndependentRunLane() throws {
        let response = contextEnvelope("response", owner: "context-a")
        let claim: [String: Any] = ["request_id": "claim", "client_id": "ios", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
            "command": ["type": "claim_committed_host_requests", "arguments": ["input": ["run_id": "run-a"]]]]
        XCTAssertEqual(try IOSPendingCommandOwner.decode(claim), .run("run-a"))
        let resolve: (IOSPendingCommandOwner) throws -> IOSPendingCommandOwner = {
            $0 == .run("run-a") ? .conversation("context-a") : $0
        }
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: claim, pending: ["response": response], resolve: resolve), "response")
        XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: claim,
            pending: ["other": contextEnvelope("other", owner: "context-b")], resolve: resolve))
    }

    func testReopenPreservesCompleteEnvelopeAndExplicitRetirement() throws {
        let database = try directory().appendingPathComponent("product.db")
        let original = envelope(content: "中文 / quote \" backslash \\ newline\n")
        let first = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        try first.save(["original": original])
        first.close()
        let reopened = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        let pending = try reopened.load()
        XCTAssertEqual(try JSONSerialization.data(withJSONObject: pending["original"]!, options: [.sortedKeys]),
                       try JSONSerialization.data(withJSONObject: original, options: [.sortedKeys]))
        try reopened.save([:]) // Runtime may do this only after a validated terminal receipt.
        reopened.close()
        let retired = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        defer { retired.close() }
        XCTAssertTrue(try retired.load().isEmpty)
        var info = stat()
        XCTAssertEqual(lstat(database.appendingPathExtension("pending-commands.json").path, &info), 0)
        XCTAssertEqual(info.st_mode & 0o777, 0o600)
    }

    func testBoundUsesSerializedFrameAndRejectsForeignMapIdentity() throws {
        let database = try directory().appendingPathComponent("product.db")
        let transaction = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        defer { transaction.close() }
        // Near the legal frame ceiling must not be truncated to an arbitrary
        // small intent limit, including commands carrying source/artifact data.
        let large = envelope(content: String(repeating: "x", count: ApplePendingCommandJournal.maximumEnvelopeBytes - 1024))
        try transaction.save(["original": large])
        XCTAssertEqual(try transaction.load().count, 1)
        XCTAssertThrowsError(try transaction.save(["foreign": envelope()]))
        XCTAssertThrowsError(try transaction.save(["original": envelope(), "second": envelope("second")]))
        XCTAssertThrowsError(try transaction.save(["original": envelope(content:
            String(repeating: "x", count: ApplePendingCommandJournal.maximumEnvelopeBytes))]))
        XCTAssertEqual(try transaction.load().count, 1, "invalid replacement leaves the previous intent intact")
    }

    func testRejectsSymlinkNonPrivateAndMalformedJournalWithoutTreatingAsEmpty() throws {
        for kind in ["symlink", "public", "null", "empty"] {
            let root = try directory()
            let database = root.appendingPathComponent("product.db")
            let journal = database.appendingPathExtension("pending-commands.json")
            if kind == "symlink" {
                let target = root.appendingPathComponent("target")
                try Data("{}".utf8).write(to: target)
                try FileManager.default.createSymbolicLink(at: journal, withDestinationURL: target)
            } else {
                let content = kind == "null" ? "null" : kind == "empty" ? "" : "{}"
                try Data(content.utf8).write(to: journal)
                try FileManager.default.setAttributes([.posixPermissions: kind == "public" ? 0o644 : 0o600],
                                                     ofItemAtPath: journal.path)
            }
            XCTAssertThrowsError(try {
                let scope = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
                defer { scope.close() }
                _ = try scope.load()
            }(), kind)
        }
    }

    func testSeparateInstancesCannotReadBetweenLoadSendAndSave() throws {
        let database = try directory().appendingPathComponent("product.db")
        let first = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        let secondJournal = ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        let attempted = DispatchSemaphore(value: 0)
        let acquired = DispatchSemaphore(value: 0)
        let finished = expectation(description: "second transaction observes original intent")
        DispatchQueue.global().async {
            attempted.signal()
            do {
                let second = try secondJournal.beginTransaction()
                defer { second.close() }
                acquired.signal()
                XCTAssertEqual(try second.load().keys.sorted(), ["original"])
            } catch { XCTFail("Journal transaction failed: \(error)") }
            finished.fulfill()
        }
        XCTAssertEqual(attempted.wait(timeout: .now() + 1), .success)
        XCTAssertEqual(acquired.wait(timeout: .now() + 0.05), .timedOut)
        try first.save(["original": envelope()])
        first.close()
        wait(for: [finished], timeout: 2)
    }
}
