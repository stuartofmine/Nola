import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacAgentHostDecoderCooperativeTests: XCTestCase {
    // Unlike synchronous XCTest main-thread calls, this uses the same small
    // cooperative worker stack as the actual remote byte-stream reader.
    private func decodeOnWorker(_ line: String, conversation: String) async -> MacLocalAgentEvent? {
        await Task.detached {
            MacAgentHostRuntime.decode(line: line, conversationID: conversation)
        }.value
    }

    func testJournalAndTerminalHintsDecodeOnCooperativeWorkerWithoutCompletionAuthority() async throws {
        let digest = String(repeating: "a", count: 64)
        let identity: [String: Any] = ["source_store_epoch": "epoch", "conversation_id": "conversation",
            "run_id": "run", "assistant_message_id": "assistant"]
        let objects: [[String: Any]] = [
            ["type": "assistant_journal_committed_v1", "session_id": "conversation",
             "source_host_id": "host", "through": ["identity": identity,
                 "sequence": "18446744073709551615", "record_digest": digest]],
            ["type": "assistant_run_terminal_committed_v1", "session_id": "conversation",
             "source_host_id": "host", "run_id": "run", "seal_digest": digest],
        ]
        for object in objects {
            let line = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
            let decoded = await decodeOnWorker(line, conversation: "conversation")
            let event = try XCTUnwrap(decoded)
            XCTAssertFalse(event.isError)
            XCTAssertEqual(event.kind, .status)
            XCTAssertEqual(event.canonicalRunID, "run")
            XCTAssertNil(event.coreTurnTerminal, "A notification is not committed completion")
            XCTAssertTrue(event.text.isEmpty)
            let copied = event.addingArtifactURLs([])
            XCTAssertEqual(copied.assistantJournalCursor, event.assistantJournalCursor)
            XCTAssertEqual(copied.assistantRunTerminalSealDigest, event.assistantRunTerminalSealDigest)
            if let cursor = event.assistantJournalCursor {
                XCTAssertEqual(cursor.sequence, "18446744073709551615")
            }
        }
    }

    func testInvalidFamilyPayloadsStayProtocolFailuresOnCooperativeWorker() async throws {
        for type in ["core_session_projection_updated", "remote_assistant_projection_committed_v1",
                     "conversation_projection_committed_v1", "core_part_updated", "turn_completed",
                     "core_task_updated", "core_skill_runtime_updated", "core_capabilities_activated",
                     "core_permission_projection_updated", "permission_request", "user_input_wait_changed",
                     "context_usage"] {
            let data = try JSONSerialization.data(withJSONObject: ["type": type, "canonical_run_id": "bound-run"])
            let decoded = await decodeOnWorker(String(decoding: data, as: UTF8.self), conversation: "conversation")
            let event = try XCTUnwrap(decoded, type)
            XCTAssertTrue(event.isError, type)
            XCTAssertEqual(event.terminalReason, "protocol_error", type)
            XCTAssertEqual(event.canonicalRunID, "bound-run", type)
            XCTAssertNil(event.assistantJournalCursor, type)
            XCTAssertNil(event.assistantRunTerminalSealDigest, type)
        }
    }

    func testUnknownJSONAndUnknownTraceDoNotBecomeVisibleAssistantText() async throws {
        let lines = [
            #"{"type":"future_event","text":"untrusted answer"}"#,
            #"{"type":"future_event","trace":{"schema_version":1,"trace_id":"trace","run_id":"run","conversation_id":"conversation","turn_id":"turn","event_id":"event","sequence":1,"emitted_at":"2026-09-28T00:00:00Z","kind":"future.unknown","span_id":"span","status":"running","visibility":"summary","payload":{"delta":"untrusted answer"}}}"#
        ]
        for line in lines {
            let decoded = await decodeOnWorker(line, conversation: "conversation")
            let event = try XCTUnwrap(decoded)
            XCTAssertTrue(event.isError)
            XCTAssertEqual(event.terminalReason, "protocol_error")
            XCTAssertNil(event.assistantJournalCursor)
            XCTAssertFalse(event.text.contains("untrusted answer"))
        }
    }

    func testDistinctCoreSessionWitnessValidatesEveryCoreFamilyWithoutRewritingIdentity() throws {
        let conversation = "canonical-conversation"
        let core = "actual-core-session"
        let common: [String: Any] = ["protocol_version": 2, "session_id": core,
            "turn_id": "owned-run", "cursor": 1, "revision": 1]
        let payloads: [[String: Any]] = [
            ["type": "core_task_updated", "task_projection_revision": 1,
             "task": ["id": "task", "owner_session_id": core, "owner_turn_id": "owned-run",
                      "kind": "agent", "state": "running", "cancel_requested": false]],
            ["type": "core_skill_runtime_updated", "skill_runtime_revision": 1,
             "update": ["type": "catalog_replaced", "revision": 1]],
            ["type": "core_permission_projection_updated", "permission_projection_revision": 1,
             "permission_projection": ["revision": 1, "pending": []]],
            ["event": ["type": "turn_completed", "turn_id": "owned-run"]],
        ]
        for payload in payloads {
            let object = common.merging(payload, uniquingKeysWith: { _, new in new })
            let line = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
            let accepted = try XCTUnwrap(MacAgentHostRuntime.decode(line: line,
                conversationID: conversation, coreSessionID: core))
            XCTAssertFalse(accepted.isError, line)
            XCTAssertEqual(accepted.validatedCoreSessionID, core)
            XCTAssertEqual(accepted.validatedCoreConversationID, conversation)
            XCTAssertEqual(accepted.addingArtifactURLs([]).validatedCoreSessionID, core)
            XCTAssertTrue(MacAgentHostRuntime.decode(line: line, conversationID: conversation)?.isError == true,
                "Missing authoritative witness must fail, even when a caller supplies a canonical conversation")
            XCTAssertTrue(MacAgentHostRuntime.decode(line: line, conversationID: conversation,
                coreSessionID: conversation)?.isError == true, "Canonical identity cannot stand in for Core identity")
            XCTAssertTrue(MacAgentHostRuntime.decode(line: line, conversationID: conversation,
                controlSessionID: conversation)?.isError == true, "Authenticated control identity is not a Core witness")
        }
        let permission = #"{"type":"permission_request", "allow_conversation": true,"protocol_version":4,"request_id":"permission","nonce":"nonce","host_id":"host","client_id":"client","session_id":"canonical-conversation","tool_name":"write_file"}"#
        let accepted = try XCTUnwrap(MacAgentHostRuntime.decode(line: permission,
            conversationID: conversation, coreSessionID: core, controlSessionID: conversation))
        XCTAssertEqual(accepted.permissionRequest?.sessionID, conversation,
            "Nonce challenge owns the authenticated Host control session")
        XCTAssertEqual(accepted.validatedCoreSessionID, core,
            "Core projection ownership stays independent of the control session")
        XCTAssertEqual(accepted.validatedControlSessionID, conversation)
        XCTAssertTrue(MacAgentHostRuntime.decode(line: permission, conversationID: conversation,
            coreSessionID: core, controlSessionID: "foreign-control")?.isError == true)
    }

    func testTerminalNumericFieldsAndRequiredNullableExactWitnessDecodeStrictly() throws {
        let terminal: [String: Any] = ["session_id": "actual-core", "turn_id": "run", "cursor": 1, "revision": 1,
            "event": ["type": "turn_failed", "session_id": "actual-core",
                      "turn_id": "run", "error": "fixture failure"]]
        let valid = try XCTUnwrap(MacAgentHostRuntime.decode(
            line: String(decoding: try JSONSerialization.data(withJSONObject: terminal), as: UTF8.self),
            conversationID: "canonical", coreSessionID: "actual-core"))
        XCTAssertEqual(valid.coreTurnTerminal?.sessionID, "actual-core")
        XCTAssertEqual(valid.terminalReason, "fixture failure")
        XCTAssertEqual(valid.validatedCoreSessionID, "actual-core")
        for (key, foreignValue) in [("session_id", "canonical"), ("turn_id", "foreign-turn")] {
            var object = terminal
            var inner = try XCTUnwrap(object["event"] as? [String: Any])
            inner[key] = foreignValue
            object["event"] = inner
            XCTAssertEqual(MacAgentHostRuntime.decode(
                line: String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self),
                conversationID: "canonical", coreSessionID: "actual-core")?.terminalReason, "protocol_error")
        }
        for key in ["cursor", "revision"] {
            for malformed: Any in [true, -1, 1.5, NSNull(), "1"] {
                var object = terminal
                object[key] = malformed
                let line = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
                XCTAssertEqual(MacAgentHostRuntime.decode(line: line, conversationID: "canonical",
                    coreSessionID: "actual-core")?.terminalReason, "protocol_error")
            }
        }
        var exact: [String: Any] = ["conversation_id": "canonical", "run_id": "run",
            "assistant_message_id": "assistant", "invocation_config_digest": String(repeating: "a", count: 64),
            "status": "queued", "host_invocation": [:], "core_session_id": NSNull(),
            "checkpoint_revision": NSNull(), "checkpoint_event_cursor": NSNull(), "checkpoint_sha256": NSNull()]
        func decode(_ value: [String: Any]) throws -> MacExactRunInvocation {
            try JSONDecoder().decode(MacExactRunInvocation.self,
                from: JSONSerialization.data(withJSONObject: value))
        }
        XCTAssertNil(try decode(exact).coreSessionID)
        for key in ["core_session_id", "checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256"] {
            var missing = exact
            missing.removeValue(forKey: key)
            XCTAssertThrowsError(try decode(missing), "missing \(key) is not an explicit pre-checkpoint null")
        }
        exact["core_session_id"] = "actual-core"
        XCTAssertThrowsError(try decode(exact), "partial witness must fail")
        exact["checkpoint_revision"] = 1
        exact["checkpoint_event_cursor"] = 2
        exact["checkpoint_sha256"] = String(repeating: "b", count: 64)
        XCTAssertEqual(try decode(exact).coreSessionID, "actual-core")
    }
}
