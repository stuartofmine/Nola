// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS agent host decoder/transport gate, linked/shared protocol contracts, SSE frame decoder, network failure handling, cancellation delivery process, keyboard bridge, runtime policy, code signing, and software update suite.

import CoreGraphics
import Foundation
import XCTest
@testable import Nola

final class MacTransportGateProtocolAndRuntimePolicySuiteTests: XCTestCase {
    private func gateEvent(eventID: String, sequence: UInt64, runID: String = "run-1", messageID: String = "run-1:assistant") -> MacLocalAgentEvent {
        let trace = MacAgentTraceEnvelope.decode([
            "schema_version": 1, "trace_id": "trace-1", "run_id": runID, "conversation_id": "conversation-1",
            "message_id": messageID, "turn_id": runID, "event_id": eventID, "sequence": sequence,
            "emitted_at": "2026-08-22T00:00:00Z", "kind": "phase.changed", "status": "running",
            "visibility": "user", "payload": ["phase": "processing"],
        ])!
        return MacLocalAgentEvent(kind: .status, text: "delta", title: nil, detail: nil, isError: false, eventID: eventID, trace: trace)
    }

    @MainActor
    private func decodeSSE(_ input: String, finish: Bool = true) -> [String] {
        var decoder = MacSSEFrameDecoder()
        var frames = input.utf8.compactMap { decoder.append($0) }
        if finish, let tail = decoder.finish() { frames.append(tail) }
        return frames
    }

    func testTransportGateLinkedProtocolKeyboardMapperAndRuntimePolicy() throws {
        let trace: [String: Any] = [
            "schema_version": 1, "trace_id": "goal-trace", "run_id": "goal-run-1",
            "conversation_id": "conversation-1", "message_id": "goal-run-1:assistant",
            "turn_id": "goal-run-1", "event_id": "seal-replay", "sequence": 22,
            "emitted_at": "2026-08-22T00:00:00Z", "kind": "phase.changed",
            "status": "running", "visibility": "user", "payload": ["phase": "processing"],
        ]
        let line = String(decoding: try JSONSerialization.data(withJSONObject: ["type": "phase", "trace": trace]), as: UTF8.self)
        XCTAssertEqual(MacAgentHostRuntime.goalEventRunID(line: line), "goal-run-1")
        XCTAssertNil(MacAgentHostRuntime.goalEventRunID(line: "{\"type\":\"status\"}"))

        var dupGate = MacAgentHostTransportGate(runID: "run-1", assistantMessageID: "run-1:assistant")
        let ev1 = gateEvent(eventID: "event-1", sequence: 1)
        XCTAssertEqual(try dupGate.accept(ev1), .accepted)
        XCTAssertEqual(try dupGate.accept(ev1), .duplicate)

        var orderGate = MacAgentHostTransportGate(runID: "run-1", assistantMessageID: "run-1:assistant")
        XCTAssertEqual(try orderGate.accept(gateEvent(eventID: "event-2", sequence: 2)), .accepted)
        XCTAssertThrowsError(try orderGate.accept(gateEvent(eventID: "event-1", sequence: 1)))

        var foreignGate = MacAgentHostTransportGate(runID: "run-expected", assistantMessageID: "run-expected:assistant")
        XCTAssertThrowsError(try foreignGate.accept(gateEvent(eventID: "foreign-event", sequence: 1, runID: "run-foreign", messageID: "run-expected:assistant")))

        XCTAssertEqual(MacApplicationServiceClient.protocolVersion, 1)
        try MacApplicationServiceClient.validateLinkedContract(abiVersion: 1, serviceProtocol: 1)
        for version in [UInt32(0), 2, 54, UInt32.max] {
            XCTAssertThrowsError(try MacApplicationServiceClient.validateLinkedContract(abiVersion: 1, serviceProtocol: version))
        }
        for abi in [UInt32(0), 2, UInt32.max] {
            XCTAssertThrowsError(try MacApplicationServiceClient.validateLinkedContract(abiVersion: abi, serviceProtocol: 1))
        }
        XCTAssertEqual(MacApplicationServiceClient.protocolVersion, 1)

        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "A"), 0)
        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "enter"), 36)
        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "left_arrow"), 123)
        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "escape"), 53)
        let flags = MacKeyboardEventMapper.flags(for: ["CMD", "Shift", "alt", "ctrl"])
        XCTAssertTrue(flags.contains(.maskCommand) && flags.contains(.maskShift) && flags.contains(.maskAlternate) && flags.contains(.maskControl))

        XCTAssertTrue(MacProcessRuntimePolicy.isStableInstalledApplication(URL(fileURLWithPath: "/Applications/Nola.app")))
        XCTAssertTrue(MacProcessRuntimePolicy.isStableInstalledApplication(URL(fileURLWithPath: "/Users/example/Applications/Nola.app")))
        XCTAssertFalse(MacProcessRuntimePolicy.isStableInstalledApplication(URL(fileURLWithPath: "/Users/example/Library/Developer/Xcode/DerivedData/Nola/Build/Products/Debug/Nola.app")))
        XCTAssertFalse(MacProcessRuntimePolicy.isStableInstalledApplication(URL(fileURLWithPath: "/Users/example/Nola/Build/Products/Debug/Nola.app")))
        XCTAssertFalse(MacProcessRuntimePolicy.isStableInstalledApplication(URL(fileURLWithPath: "/usr/local/bin/nola")))
    }

    @MainActor
    func testSharedProtocolDiscoverySoftwareUpdateAndSSEFrameDecoding() async throws {
        let validSetup = #"{"type":"nola.server.connection","protocol_version":1,"origin":"http://192.168.1.8:5001","host_id":"host-fixture","access_token":"fixture-token"}"#
        XCTAssertEqual(AppleHostConnectionSetup.decode(validSetup)?.hostID, "host-fixture")
        for invalid in [validSetup.replacingOccurrences(of: #""protocol_version":1"#, with: #""protocol_version":true"#),
                        validSetup.replacingOccurrences(of: #""protocol_version":1"#, with: #""protocol_version":2"#),
                        validSetup.replacingOccurrences(of: "192.168.1.8:5001", with: "user:pass@192.168.1.8:5001"),
                        validSetup.replacingOccurrences(of: "192.168.1.8:5001", with: "8.8.8.8:5001"),
                        validSetup.replacingOccurrences(of: "192.168.1.8:5001", with: "192.168.1.8:5001?token=secret")] {
            XCTAssertNil(AppleHostConnectionSetup.decode(invalid))
        }
        XCTAssertTrue(AppleHostConnectionSetup.sameOrigin(URL(string: "https://Studio.local")!, URL(string: "https://studio.local:443/api/mobile")!))
        XCTAssertFalse(AppleHostConnectionSetup.sameOrigin(URL(string: "http://one.local")!, URL(string: "http://two.local")!))

        let subIdentity = AppleRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        let cancelObj = try AppleRemoteCancellationRequest(identity: subIdentity, requestDigest: "digest").object()
        XCTAssertEqual(Set(cancelObj.keys), ["identity", "request_digest"])
        XCTAssertEqual((cancelObj["identity"] as? [String: Any])?["submission_id"] as? String, "run")
        XCTAssertThrowsError(try AppleRemoteCancellationRequest(identity: subIdentity, requestDigest: ""))

        let permCmd = SetConversationPermissionCommand(conversationID: "conversation", expectedRevision: 7,
            expectedRunID: nil, expectedSourceHostID: nil, profile: "cautious", updatedAtMs: 100)
        let permObj = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(permCmd)) as? [String: Any])
        XCTAssertEqual(permObj["type"] as? String, "set_conversation_permission")
        XCTAssertEqual(permObj["expected_revision"] as? Int, 7)
        XCTAssertTrue(permObj["expected_run_id"] is NSNull && permObj["expected_source_host_id"] is NSNull)

        for selection in [SelectConversationExecutionCommand<MacCanonicalJSONValue>.Selection.inherit, .location(executionHost: "remote")] {
            let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(selection)) as? [String: Any])
            XCTAssertNil(obj["modelSelectionID"]); XCTAssertNil(obj["executionHostBinding"])
            if obj["kind"] as? String == "location" { XCTAssertEqual(obj["execution_host"] as? String, "remote") }
            else { XCTAssertEqual(obj["kind"] as? String, "inherit") }
        }

        let badCancelledWire = Data(#"{"kind":"cancelled","cancellation_id":"cancel-1","observed_effect_refs":["effect-1"],"unknown_effect_refs":["effect-1"]}"#.utf8)
        XCTAssertFalse((try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: badCancelledWire)).hasRequiredEvidenceFields)
        let answeredWire = Data(#"{"kind":"answered","answer_part_refs":["part-1"]}"#.utf8)
        let answeredVal = try JSONDecoder().decode(AppleRunResult<MacCanonicalJSONValue>.Value.self, from: answeredWire)
        XCTAssertTrue(answeredVal.hasRequiredEvidenceFields)
        XCTAssertEqual(try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(answeredVal)),
                       try JSONDecoder().decode(MacCanonicalJSONValue.self, from: answeredWire))

        let jIdentity = AppleAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant")
        let jCursor = AppleAssistantJournalCursor(identity: jIdentity, sequence: "9007199254740993", record_digest: "digest")
        let jObj = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleAssistantJournalReadRequest<MacCanonicalJSONValue>.records(after: jCursor, through: jCursor))) as? [String: Any])
        XCTAssertEqual(jObj["operation"] as? String, "read")
        XCTAssertEqual(jObj["max_records"] as? Int, 32)
        XCTAssertEqual(jObj["max_encoded_bytes"] as? Int, 256 * 1024)
        XCTAssertEqual((jObj["after"] as? [String: Any])?["sequence"] as? String, "9007199254740993")

        let discReq = try AppleModelCatalogDiscovery.request(endpoint: "https://example.test/v1/", wireProtocol: "anthropic", secret: "test-secret")
        XCTAssertEqual(discReq.url?.path, "/v1/models"); XCTAssertEqual(discReq.httpMethod, "GET")
        XCTAssertEqual(discReq.value(forHTTPHeaderField: "x-api-key"), "test-secret")
        XCTAssertNil(discReq.value(forHTTPHeaderField: "Authorization"))
        let discResp = try XCTUnwrap(HTTPURLResponse(url: try XCTUnwrap(discReq.url), statusCode: 200, httpVersion: nil, headerFields: nil))
        let discModels = try AppleModelCatalogDiscovery.parse(data: Data(#"{"data":[{"id":"a","display_name":"A"},{"id":"b"},{"id":""}]}"#.utf8), response: discResp)
        XCTAssertEqual(discModels.map(\.id), ["a", "b"]); XCTAssertEqual(discModels.map(\.displayName), ["A", "b"])
        XCTAssertThrowsError(try AppleModelCatalogDiscovery.parse(data: Data(#"{"models":[]}"#.utf8), response: discResp))
        for levels in [#""supported_reasoning_levels":[{"effort":"none"},{"effort":"low"},{"effort":"high"},{"effort":"max"},{"effort":"high"}]"#,
                       #""reasoning_levels":["none","low","high","max"]"#] {
            let body = "{\"data\":[{\"id\":\"fixture\",\"display_name\":\"Readable\",\"context_window\":128000,\"max_output_tokens\":32000," + levels + "}]}"
            let m = try XCTUnwrap(AppleModelCatalogDiscovery.parse(data: Data(body.utf8), response: discResp).first)
            XCTAssertEqual(m.reasoningLevels, ["none", "low", "high", "max"])
            let entry = try XCTUnwrap(JSONSerialization.jsonObject(with: m.catalogData(wireProtocol: "chat_completions")) as? [String: Any])
            XCTAssertEqual(entry["reasoning_levels"] as? [String], m.reasoningLevels)
            XCTAssertEqual(entry["context_window"] as? Int, 128000)
            XCTAssertEqual((entry["execution_capabilities"] as? [String: Any])?["reasoning"] as? Bool, true)
        }
        let plainModels = try AppleModelCatalogDiscovery.parse(data: Data(#"{"data":[{"id":"gpt-looking-name"},{"id":"empty","supported_reasoning_levels":[],"reasoning_levels":["high"]}]}"#.utf8), response: discResp)
        XCTAssertTrue(plainModels.allSatisfy { $0.reasoningLevels.isEmpty && $0.reasoningSupported == nil })

        do {
            let _: SoftwareUpdateSnapshot = try await AppleSoftwareUpdateClient.call("install")
            XCTFail("The check/download ABI must never install or quit the Host")
        } catch {
            XCTAssertTrue(error is AppleSoftwareUpdateClient.UpdateFailure)
        }
        let updateSnap = try JSONDecoder().decode(SoftwareUpdateSnapshot.self, from: Data(#"{"status":"available","current_version":"1.0.0","current_build":1,"channel":"internal","target":"ios","release":{"release_id":"signed-payload-digest","version":"1.0.1","build":2,"notes":"<literal release notes>","artifact":{"kind":"store","url":"https://apps.apple.com/app/id123"}}}"#.utf8))
        XCTAssertEqual(updateSnap.release?.release_id, "signed-payload-digest")
        XCTAssertEqual(updateSnap.release?.notes, "<literal release notes>")
        XCTAssertEqual(updateSnap.release?.artifact.kind, "store")

        // SSE frame decoder comprehensive contract
        XCTAssertEqual(decodeSSE("data: first\n\ndata: second\n\ndata: third\n\n", finish: false), ["first", "second", "third"])
        XCTAssertEqual(decodeSSE("data: {\ndata:   \"type\": \"delta\",\ndata:   \"text\": \"你好\"\ndata: }\n\n"), ["{\n  \"type\": \"delta\",\n  \"text\": \"你好\"\n}"])
        for newline in ["\n", "\r\n", "\r"] {
            XCTAssertEqual(decodeSSE(["data: alpha", "data: beta", "", "data: gamma", "", ""].joined(separator: newline)), ["alpha\nbeta", "gamma"])
        }
        var splitDec = MacSSEFrameDecoder()
        for byte in "data: one\r".utf8 { XCTAssertNil(splitDec.append(byte)) }
        XCTAssertNil(splitDec.append(10)); XCTCT: do {
            XCTAssertEqual(splitDec.append(13), "one")
            XCTAssertNil(splitDec.append(10)); XCTAssertNil(splitDec.finish())
        }
        XCTAssertEqual(decodeSSE("data: a\r\ndata: b\r\rdata: c\n\ndata: d\r\n\r\n"), ["a\nb", "c", "d"])
        XCTAssertEqual(decodeSSE(": heartbeat\n\nevent: delta\nid: 4\nretry: 500\nunknown: value\ndata: payload\n: inline comment\n\n"), ["payload"])
        XCTAssertEqual(decodeSSE(": heartbeat\nevent: ignored\nid: 4\n\n"), [])
        XCTAssertEqual(decodeSSE("Data: ignored\n data: ignored\ndata:  keep one space\ndata:\tkeep tab\ndata:trailing  \n\n"), [" keep one space\n\tkeep tab\ntrailing  "])
        XCTAssertEqual(decodeSSE("\n\n: comment\n\ndata:\n\ndata\n\n"), ["", ""])
        XCTAssertEqual(decodeSSE("data: one\ndata:\n\n"), ["one\n"])
        XCTAssertEqual(decodeSSE("\u{FEFF}data: 你好 👋🏽 café\n\ndata: \u{FEFF}payload\n\n"), ["你好 👋🏽 café", "\u{FEFF}payload"])
        XCTAssertEqual(decodeSSE("data: first\n\n\u{FEFF}data: ignored\n\n"), ["first"])
        var mbDec = MacSSEFrameDecoder()
        for byte in "data: 中文🙂e\u{301}".utf8 { XCTAssertNil(mbDec.append(byte)) }
        XCTAssertNil(mbDec.append(10)); XCTAssertEqual(mbDec.append(10), "中文🙂e\u{301}"); XCTAssertNil(mbDec.finish())
        for wire in ["data: tail", "data: tail\n", "data: tail\r", "data: tail\r\n"] {
            var eofDec = MacSSEFrameDecoder()
            XCTAssertTrue(wire.utf8.compactMap { eofDec.append($0) }.isEmpty)
            XCTAssertEqual(eofDec.finish(), "tail")
            XCTAssertNil(eofDec.finish())
        }
        XCTAssertEqual(decodeSSE("data: done\n\n: final heartbeat"), ["done"])
        XCTAssertEqual(decodeSSE("data: done\n\n"), ["done"])
        XCTAssertEqual(decodeSSE("event: unfinished metadata"), [])
        XCTAssertEqual(decodeSSE(""), [])
    }
}

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
        let common: [String: Any] = ["protocol_version": 1, "session_id": core,
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
        let permission = #"{"type":"permission_request", "allow_conversation": true,"protocol_version":1,"request_id":"permission","nonce":"nonce","host_id":"host","client_id":"client","session_id":"canonical-conversation","tool_name":"write_file"}"#
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

final class MacNetworkFailureTests: XCTestCase {
    func testOfflineCodeAloneDoesNotMeanPermissionDenied() {
        XCTAssertEqual(MacNetworkFailure.classify(URLError(.notConnectedToInternet)), .offline)
        let proseOnly = NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [
            NSLocalizedDescriptionKey: "unsatisfied (Local network prohibited)",
            NSURLErrorFailingURLErrorKey: URL(string: "http://192.168.1.2/v1/models")!,
        ])
        XCTAssertEqual(MacNetworkFailure.classify(proseOnly), .offline)
        XCTAssertNil(MacNetworkFailure.classify(NSError(domain: "Provider", code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (Local network prohibited)",
        ])))
        XCTAssertNil(MacNetworkFailure.classify(URLError(.timedOut)))
    }

    func testExplicitUnderlyingOSPathPolicyIsDistinguishedFromOffline() {
        let denied = NSError(domain: "kCFErrorDomainCFNetwork", code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (Local network prohibited), interface: en0[802.11]",
        ])
        let wrapped = NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [NSUnderlyingErrorKey: denied])
        XCTAssertEqual(MacNetworkFailure.classify(wrapped), .localNetworkDenied)
        XCTAssertNotEqual(MacNetworkFailure.message(for: wrapped), MacNetworkFailure.message(for: URLError(.notConnectedToInternet)))
        let ordinary = NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (No network route)",
        ])
        XCTAssertEqual(MacNetworkFailure.classify(ordinary), .offline)
    }

    func testActualDiscoveryPreservesPolicyEvidenceForTheUIWithoutNetworkAccess() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [DeniedModelCatalogProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        do {
            _ = try await MacModelCatalogClient.fetch(baseURL: URL(string: "http://catalog.invalid/v1/responses")!, apiKey: "fixture-key", session: session)
            XCTFail("Expected the injected URLSession policy denial")
        } catch {
            XCTAssertEqual(MacNetworkFailure.classify(error), .localNetworkDenied)
        }
        XCTAssertEqual(MacModelCatalogClient.catalogURL(URL(string: "http://catalog.invalid/v1/chat/completions")!).path, "/v1/models")
    }

    func testBuiltMacAppDeclaresLocalNetworkUsage() {
        XCTAssertFalse((Bundle.main.object(forInfoDictionaryKey: "NSLocalNetworkUsageDescription") as? String)?.isEmpty ?? true)
    }

    func testDiscoveryUsesDraftCredentialAndReturnsSortedUniqueModelIDs() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SuccessfulModelCatalogProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let models = try await MacModelCatalogClient.fetch(
            baseURL: URL(string: "http://catalog.invalid/v1/responses")!,
            apiKey: "current-draft-key", session: session
        )
        XCTAssertEqual(models, ["chat-fast", "reasoning-large"])
    }

    func testDiscoveryNormalizesBaseAndInferenceRoutesWithoutDuplicatingVersion() {
        for route in ["/v1", "/v1/", "/v1/responses", "/v1/chat/completions"] {
            XCTAssertEqual(
                MacModelCatalogClient.catalogURL(URL(string: "http://catalog.invalid" + route)!).path,
                "/v1/models", route
            )
        }
    }

    func testDiscoveryPreservesHTTPFailureWithoutLeakingResponseBody() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [RejectedModelCatalogProtocol.self]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        for status in [401, 403, 404, 405, 429, 500] {
            do {
                _ = try await MacModelCatalogClient.fetch(
                    baseURL: URL(string: "http://catalog.invalid/status/\(status)")!,
                    apiKey: "fixture-key", session: session
                )
                XCTFail("Expected HTTP rejection")
            } catch {
                XCTAssertEqual(error as? MacModelCatalogHTTPError, MacModelCatalogHTTPError(statusCode: status))
                XCTAssertFalse(MacNetworkFailure.message(for: error).contains("secret-response-marker"))
                XCTAssertFalse(MacNetworkFailure.message(for: error).isEmpty)
            }
        }
    }
}

private final class SuccessfulModelCatalogProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        XCTAssertEqual(request.url?.path, "/v1/models")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer current-draft-key")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "application/json")
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(#"{"data":[{"id":"reasoning-large"},{"id":"chat-fast"},{"id":"reasoning-large"}]}"#.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

private final class RejectedModelCatalogProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let url = request.url!
        let status = Int(url.deletingLastPathComponent().lastPathComponent)!
        let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("secret-response-marker".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

private final class DeniedModelCatalogProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        XCTAssertEqual(request.url?.path, "/v1/models")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer fixture-key")
        client?.urlProtocol(self, didFailWithError: NSError(domain: NSURLErrorDomain, code: -1009, userInfo: [
            "_NSURLErrorNWPathKey": "unsatisfied (Local network prohibited)",
        ]))
    }
    override func stopLoading() {}
}

final class MacCancellationDeliveryProcessTests: XCTestCase {
    private func fixture(body: String) throws -> (directory: URL, executable: URL) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-cancel-transport-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let executable = directory.appendingPathComponent("control-fixture")
        try ("#!/bin/sh\nIFS= read -r frame\n" + body + "\n")
            .write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        return (directory, executable)
    }

    private func request() throws -> MacRunCancellationRequest {
        try .init(conversationID: "conversation", expectedRunID: "run",
                  assistantMessageID: "assistant", cancellationID: "cancel", requestedAtMs: 1)
    }

    func testOwnedChildReceivesOnlyControlFrameWithoutCreatingDatabaseOrTerminal() async throws {
        let fixture = try fixture(body: "printf '%s' \"$frame\" > received.json\nprintf '%s\\n' \"$@\" > arguments.txt")
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let database = fixture.directory.appendingPathComponent("isolated.db")
        try await MacCancellationDeliveryProcess.run(executable: fixture.executable,
            databaseURL: database, request: request())
        let data = try Data(contentsOf: fixture.directory.appendingPathComponent("received.json"))
        let frame = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: String])
        XCTAssertEqual(frame, ["operation": "cancel", "session_id": "conversation",
            "expected_run_id": "run", "assistant_message_id": "assistant", "cancellation_id": "cancel"])
        let arguments = try String(contentsOf: fixture.directory.appendingPathComponent("arguments.txt"), encoding: .utf8)
        XCTAssertEqual(arguments.split(separator: "\n").map(String.init),
                       ["run", "--json", "--host-request-stdin", "--session", "conversation"])
        XCTAssertFalse(FileManager.default.fileExists(atPath: database.path),
                       "Transport success itself cannot manufacture a canonical terminal record")
    }

    func testLargeErrorOutputIsDrainedAndRetainedBounded() async throws {
        let fixture = try fixture(body: "i=0; while [ \"$i\" -lt 5000 ]; do printf 'control-fixture-output-0123456789\\n' >&2; i=$((i+1)); done; exit 7")
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        do {
            try await MacCancellationDeliveryProcess.run(executable: fixture.executable,
                databaseURL: fixture.directory.appendingPathComponent("isolated.db"), request: request())
            XCTFail("Expected owned child failure")
        } catch MacLocalAgentRuntimeError.processFailed(let status, let detail) {
            XCTAssertEqual(status, 7)
            XCTAssertEqual(detail.utf8.count, 4_000)
        }
    }

    func testDeadlineStopsOnlyOwnedControlChildAndDoesNotReportSuccess() async throws {
        let fixture = try fixture(body: "exec /bin/sleep 60")
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        do {
            try await MacCancellationDeliveryProcess.run(executable: fixture.executable,
                databaseURL: fixture.directory.appendingPathComponent("isolated.db"),
                request: request(), timeout: .milliseconds(50))
            XCTFail("A timed-out control transport is not success")
        } catch MacLocalAgentRuntimeError.processFailed { }
    }
}

final class MacCodeSigningIdentityTests: XCTestCase {
    func testMacTargetPinsStableDevelopmentSigningForTCCPermissions() throws {
        let project = try projectSource()
        let macConfigurationStart = try XCTUnwrap(
            project.range(of: "B10000000000000000000008 /* Debug */")
        )
        let macConfigurationEnd = try XCTUnwrap(
            project.range(
                of: "D50000000000000000000008 /* Debug */",
                range: macConfigurationStart.upperBound..<project.endIndex
            )
        )
        let macConfigurations = project[macConfigurationStart.lowerBound..<macConfigurationEnd.lowerBound]

        XCTAssertEqual(macConfigurations.components(separatedBy: "CODE_SIGN_IDENTITY = \"Apple Development\";").count - 1, 2)
        XCTAssertEqual(macConfigurations.components(separatedBy: "DEVELOPMENT_TEAM = 65G58B4R2Q;").count - 1, 2)
        XCTAssertEqual(macConfigurations.components(separatedBy: "PRODUCT_BUNDLE_IDENTIFIER = com.gaojc.Nola.mac;").count - 1, 2)
    }

    func testEmbeddedAgentToolsUseAdHocSigningWithoutASecondKeychainIdentity() throws {
        let project = try projectSource()
        let phaseStart = try XCTUnwrap(
            project.range(of: "F1000000000000000000000C /* Embed Nola Runtime Products */")
        )
        let phaseEnd = try XCTUnwrap(
            project.range(
                of: "/* End PBXShellScriptBuildPhase section */",
                range: phaseStart.upperBound..<project.endIndex
            )
        )
        let phase = project[phaseStart.lowerBound..<phaseEnd.lowerBound]

        XCTAssertTrue(phase.contains("${SRCROOT}/scripts/embed_mac_runtime.sh"))
        XCTAssertTrue(phase.contains("$(SRCROOT)/scripts/lib/install_mac_executable.sh"))
        XCTAssertFalse(phase.contains("EXPANDED_CODE_SIGN_IDENTITY"))
        let embed = try repositorySource("scripts/embed_mac_runtime.sh")
        let installer = try repositorySource("scripts/lib/install_mac_executable.sh")
        XCTAssertTrue(embed.contains(". \"${SRCROOT}/scripts/lib/install_mac_executable.sh\""))
        XCTAssertTrue(embed.contains("MACOS=\"${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/MacOS\""))
        for executable in ["nola-agent-host", "nola-service", "nola-mlx-runtime"] {
            XCTAssertTrue(embed.contains("\"$MACOS/\(executable)\" \"${CODE_SIGNING_ALLOWED:-NO}\""))
        }
        XCTAssertTrue(installer.contains("if [[ \"$sign\" == YES ]]; then"))
        let sign = try XCTUnwrap(installer.range(of: "/usr/bin/codesign --force --sign - --timestamp=none \"$staged\" || exit 1"))
        let verify = try XCTUnwrap(installer.range(of: "/usr/bin/codesign --verify \"$staged\" || exit 1"))
        let install = try XCTUnwrap(installer.range(of: "mv -f \"$staged\" \"$destination\" || exit 1"))
        XCTAssertLessThan(sign.lowerBound, verify.lowerBound)
        XCTAssertLessThan(verify.lowerBound, install.lowerBound)
        for source in [embed, installer] {
            XCTAssertFalse(source.contains("EXPANDED_CODE_SIGN_IDENTITY"))
            XCTAssertFalse(source.contains("Products/Debug"))
            XCTAssertFalse(source.contains("security unlock-keychain"))
        }
    }

    func testDefaultMacTestBuildCannotTouchSigningKeychain() throws {
        let project = try projectSource()
        let appTestStart = try XCTUnwrap(
            project.range(of: "E50000000000000000000001 /* Test */")
        )
        let testBundleStart = try XCTUnwrap(
            project.range(
                of: "E50000000000000000000002 /* Test */",
                range: appTestStart.upperBound..<project.endIndex
            )
        )
        let testBundleEnd = try XCTUnwrap(
            project.range(
                of: "E50000000000000000000003 /* Test */",
                range: testBundleStart.upperBound..<project.endIndex
            )
        )

        let appTestConfiguration = project[
            appTestStart.lowerBound..<testBundleStart.lowerBound
        ]
        let testBundleConfiguration = project[
            testBundleStart.lowerBound..<testBundleEnd.lowerBound
        ]
        XCTAssertTrue(appTestConfiguration.contains("CODE_SIGNING_ALLOWED = NO;"))
        XCTAssertTrue(testBundleConfiguration.contains("CODE_SIGNING_ALLOWED = NO;"))
    }

    func testStableDeveloperIdentityCanReuseTCCGrantAcrossBuilds() {
        let identity = MacCodeSigningIdentitySnapshot(
            bundleIdentifier: "com.gaojc.Nola.mac",
            signingIdentifier: "com.gaojc.Nola.mac",
            teamIdentifier: "65G58B4R2Q",
            designatedRequirement: "identifier com.gaojc.Nola.mac and anchor apple generic"
        )
        XCTAssertTrue(identity.isStable)
    }

    func testAdHocCDHashIdentityCannotReuseVisibleOlderTCCGrant() {
        let identity = MacCodeSigningIdentitySnapshot(
            bundleIdentifier: "com.gaojc.Nola.mac",
            signingIdentifier: "Nola",
            teamIdentifier: nil,
            designatedRequirement: "cdhash H\"0123456789abcdef\""
        )
        XCTAssertFalse(identity.isStable)
    }

    func testMismatchedSigningIdentifierIsNotStable() {
        let identity = MacCodeSigningIdentitySnapshot(
            bundleIdentifier: "com.gaojc.Nola.mac",
            signingIdentifier: "com.gaojc.Nola.debug",
            teamIdentifier: "65G58B4R2Q",
            designatedRequirement: "identifier com.gaojc.Nola.debug and anchor apple generic"
        )
        XCTAssertFalse(identity.isStable)
    }

    private func projectSource() throws -> String {
        let projectURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "NolaProject.pbxproj",
                withExtension: "txt"
            )
        )
        return try String(contentsOf: projectURL, encoding: .utf8)
    }

    private func repositorySource(_ path: String) throws -> String {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        return try String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
    }
}
