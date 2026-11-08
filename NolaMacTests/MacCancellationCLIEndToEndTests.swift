import Foundation
import XCTest
@testable import Nola

/// Opt-in production-helper boundary: no provider credentials, model transport,
/// user workspace or mocked cancellation dispatcher participates in these tests.
@MainActor
final class MacCancellationCLIEndToEndTests: XCTestCase {
    @MainActor private struct Fixture {
        let root: URL
        let database: URL
        let port: MacApplicationPort
        let client: MacApplicationServiceClient
        let repository: MacCanonicalConversationRepository
        let request: MacRunCancellationRequest
        let modelEffectID: String

        init() throws {
            root = FileManager.default.temporaryDirectory.appendingPathComponent("NolaCancelCLI-\(UUID())")
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            database = root.appendingPathComponent("isolated.db")
            port = try MacApplicationPort(databaseURL: database)
            client = try MacApplicationServiceClient(databaseURL: database)
            repository = MacCanonicalConversationRepository(store: port)
            request = try MacRunCancellationRequest(conversationID: "conversation", expectedRunID: "run",
                assistantMessageID: "assistant", cancellationID: "cancellation",
                requestedAtMs: Int64(Date().timeIntervalSince1970 * 1000))
            let capability: [String: Any] = [
                "id": "model.stream", "title": "Never dispatched", "route": "model", "availability": "available",
                "exposure": "internal", "input_schema": ["type": "object"],
                "output_policy": ["preview_char_limit": 4000, "persist_full_output": true, "allow_artifacts": true],
                "supports_streaming": true, "parallelism": "serial", "cancellation": "cooperative",
                "replay_safety": "recoverable"
            ]
            // Admit the real frozen invocation even though cancellation must
            // never resolve its ephemeral credential or dispatch its model.
            let route: [String: Any] = [
                "kind": "http", "protocol": "chat_completions", "output_token_field": "max_tokens", "siwc_account": NSNull(),
                "provider": "fixture", "endpoint": "http://127.0.0.1:1/v1",
                "endpoint_query_sha256": NSNull(), "model": "never-executed-model",
                "credential_sha256": String(repeating: "a", count: 64),
                "credential_source": ["kind": "ephemeral"], "endpoint_query_source": ["kind": "none"]
            ]
            let outputAdmission = try Self.command(client, ["type": "admit_host_output",
                "routes": [["route": route, "max_input_tokens": NSNull(),
                    "max_output_tokens": NSNull(), "max_total_tokens": NSNull()]],
                "preference": "default"])
            let binding: [String: Any] = [
                "version": 1, "primary": route, "fallbacks": [],
                "generation": ["temperature": 0, "top_p": 0, "max_output_tokens": 16384,
                    "max_retries": 2, "parallel_tool_calls": NSNull(), "reasoning_selection": NSNull(),
                    "reasoning_tier": "balanced",
                    "sampling_override": false, "generation_options": [:], "thinking_enabled": false,
                    "prompt_cache": false, "prompt_cache_key_sha256": NSNull(),
                    "prompt_cache_key_prefix_sha256": NSNull(), "prompt_cache_retention": NSNull()],
                "output_admission": outputAdmission,
                "execution_policy_sha256": String(repeating: "b", count: 64)
            ]
            _ = try Self.command(client, ["type": "begin_assistant_turn", "input": [
                "conversation_id": "conversation", "conversation_title": "Isolated cancel",
                "user_message_id": "user", "user_part_id": "user:text", "user_text": "Do not execute",
                "assistant_message_id": "assistant",
                "core_config": ["session_id": "conversation", "model": "never-executed-model",
                    "turn_identity": ["host_invocation": binding,
                        "model_capability_id": "mac-journal-fixture", "model_capability_revision": 1,
                        "model_capability": MacAssistantJournalTestFixture.modelCapabilityProfile(
                            toolCapabilityID: nil, contextWindow: 128_000, maxOutput: 4096)],
                    "host_catalog": ["host_id": "isolated-host", "revision": 1, "availability_epoch": 1,
                        "capabilities": ["model.stream": capability]]],
                "submission": ["type": "user_input", "items": [["type": "message", "id": "user",
                    "role": "user", "content": "Do not execute"]], "submission_id": "run",
                    "submitted_at_epoch_ms": request.requestedAtMs, "delivery": "current_turn"],
                "created_at_ms": request.requestedAtMs
            ]])
            let admitted = try Self.command(client, ["type": "load_exact_run_invocation",
                "conversation_id": "conversation", "expected_run_id": "run", "assistant_message_id": "assistant"])
            XCTAssertEqual(admitted["run_id"] as? String, "run")
            XCTAssertEqual(admitted["assistant_message_id"] as? String, "assistant")
            let pending = try Self.command(client, ["type": "load_committed_host_requests", "run_id": "run"])
            let model = try XCTUnwrap((pending["requests"] as? [[String: Any]])?.first {
                ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream"
            })
            modelEffectID = try XCTUnwrap(model["effect_id"] as? String)
        }

        private static func command(_ client: MacApplicationServiceClient, _ command: [String: Any]) throws -> [String: Any] {
            let result = try client.execute(commandData: JSONSerialization.data(withJSONObject: command))
            return try XCTUnwrap(JSONSerialization.jsonObject(with: result) as? [String: Any])
        }

        func sqlite(_ query: String) throws -> String {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
            process.arguments = ["-readonly", database.path, query]
            let output = Pipe()
            process.standardOutput = output
            try process.run()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            XCTAssertEqual(process.terminationStatus, 0)
            return String(decoding: data, as: UTF8.self)
        }

        func taskSnapshot() throws -> [String: String] {
            // A rejected command may legitimately add an audit receipt. It
            // must not mutate task state, ownership, transcript or outbox.
            try Dictionary(uniqueKeysWithValues: [
                "runs", "core_checkpoints", "messages", "message_parts", "effect_outbox",
                "effect_delivery_attempts", "session_run_leases", "run_events",
                "host_event_inbox", "committed_transitions", "trace_journal_runs", "trace_journal_events"
            ].map { ($0, try sqlite(".dump \($0)")) })
        }
    }

    func testBundledCancelOnlyProcessSettlesNeverDispatchedRunWithoutNewMessages() async throws {
        let executable = try XCTUnwrap(MacAgentHostExecutableLocator.resolve(environment: [:]), "Bundled agent Host is required")
        let fixture = try Fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let before = try fixture.repository.displayProjection(conversationID: "conversation")
        let receipt = try fixture.port.requestRunCancellation(fixture.request)
        XCTAssertEqual(receipt.disposition, .cancellationCommitted)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
        try await MacCancellationDeliveryProcess.run(executable: executable, databaseURL: fixture.database, request: fixture.request)
        let terminal = try XCTUnwrap(fixture.repository.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
        XCTAssertEqual(terminal.result.value.kind, .cancelled)
        XCTAssertEqual(terminal.runID, "run")
        XCTAssertEqual(terminal.assistantMessageID, "assistant")
        let after = try fixture.repository.displayProjection(conversationID: "conversation")
        XCTAssertEqual(after.messages.map(\.messageID), before.messages.map(\.messageID))
        XCTAssertEqual(after.messages.filter { $0.role == .user }.count, 1)
        XCTAssertEqual(try fixture.sqlite("SELECT count(*) FROM runs;"), "1\n")
        let escaped = fixture.modelEffectID.replacingOccurrences(of: "'", with: "''")
        XCTAssertEqual(try fixture.sqlite("SELECT count(*) FROM effect_delivery_attempts WHERE effect_id='\(escaped)' AND json_extract(record_json,'$.execution_started_at_ms') IS NOT NULL;"), "0\n",
            "Cancel-only dispatch must not start the original model request")
    }

    func testWrongAssistantCancelFrameDoesNotMutateIsolatedDatabase() async throws {
        let executable = try XCTUnwrap(MacAgentHostExecutableLocator.resolve(environment: [:]))
        let fixture = try Fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        _ = try fixture.port.requestRunCancellation(fixture.request)
        let before = try fixture.taskSnapshot()
        let wrong = try MacRunCancellationRequest(conversationID: "conversation", expectedRunID: "run",
            assistantMessageID: "wrong-assistant", cancellationID: fixture.request.cancellationID,
            requestedAtMs: fixture.request.requestedAtMs)
        do {
            try await MacCancellationDeliveryProcess.run(executable: executable, databaseURL: fixture.database, request: wrong)
            XCTFail("Wrong assistant must fail before cancellation delivery")
        } catch { }
        XCTAssertEqual(try fixture.taskSnapshot(), before)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
    }
}
