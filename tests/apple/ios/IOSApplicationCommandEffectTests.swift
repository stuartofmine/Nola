// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

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
