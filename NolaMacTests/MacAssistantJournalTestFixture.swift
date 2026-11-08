// SPDX-License-Identifier: Apache-2.0
// Test transport fixtures author and import actual Application journals and terminal manifests.
import Foundation
import XCTest
@testable import Nola

/// Shared real-UI export contract. Each terminal comes from Application's seal
/// validator; rendered text and trace state never establish execution success.
@MainActor
enum MacCanonicalTranscriptEvidence {
    static func persist(_ messages: [MacChatMessage], to url: URL) throws {
        let rows = try messages.map { message -> [String: Any] in
            var row: [String: Any] = ["id": message.id.uuidString, "role": message.role.rawValue,
                "text": message.text, "traceCount": message.presentation?.traceItems.count ?? 0]
            if message.role == .assistant {
                let reference = try XCTUnwrap(message.journalReference)
                let identity = reference.cursor.identity
                let port = try MacApplicationPort(databaseURL: reference.databaseURL)
                let terminal = try XCTUnwrap(port.canonicalRunTerminal(
                    conversationID: identity.conversation_id, runID: identity.run_id))
                XCTAssertEqual(terminal.assistantMessageID, identity.assistant_message_id)
                row["canonicalTerminal"] = ["conversation_id": terminal.conversationID,
                    "run_id": terminal.runID, "assistant_message_id": terminal.assistantMessageID,
                    "status": terminal.status.rawValue, "result_digest": terminal.resultDigest,
                    "completed_at_ms": terminal.completedAtMs,
                    "result": try JSONSerialization.jsonObject(with: JSONEncoder().encode(terminal.result))]
            }
            return row
        }
        try JSONSerialization.data(withJSONObject: rows, options: [.prettyPrinted, .sortedKeys])
            .write(to: url, options: .atomic)
    }
}

// Originating-request fixture: prepares and freezes the actual semantic input,
// but deliberately creates no Run. Only the source fixture's receipt admits it.
extension MacApplicationPort {
    @discardableResult
    func prepareFixtureRemoteSubmission(_ input: MacPrepareRemoteSubmissionInput) throws -> MacRemoteSubmissionIntent {
        let prepared = try prepareRemoteSubmission(input)
        let request: [String: Any] = ["conversationId":input.conversationID, "expected_host_id":input.sourceHostID,
            "client_id":input.clientID, "submission_id":input.submissionID, "user_message_id":input.userMessageID,
            "assistant_message_id":input.assistantMessageID, "messages":[["role":"user", "content":input.userText]]]
        return try bindRemoteSubmissionRequest(identity: prepared.identity,
            request: JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONSerialization.data(withJSONObject: request)))
    }
}

nonisolated final class MacAssistantJournalTestFixture: @unchecked Sendable {
    /// Facts of this deterministic transport, not permissive production defaults.
    static func modelCapabilityProfile(toolCapabilityID: String?, contextWindow: Int, maxOutput: Int) -> [String: Any] {
        ["profile_id": "mac-journal-fixture", "revision": 1, "instruction_profile": "generic",
         "native_tool_calls": toolCapabilityID != nil, "parallel_tool_calls": false,
         "reasoning": true, "reasoning_continuity": false, "prompt_cache": false,
         "vision_input": false, "context_window_tokens": contextWindow, "max_output_tokens": maxOutput,
         "request_continuation": false, "max_request_continuations": 0,
         "stream_text": true, "stream_reasoning": true, "stream_tool_calls": toolCapabilityID != nil,
         "direct_tool_capability_ids": toolCapabilityID.map { [$0] } ?? []]
    }
    private final class ReceiverSources: @unchecked Sendable {
        final class Weak { weak var source: MacAssistantJournalTestFixture?; init(_ source: MacAssistantJournalTestFixture) { self.source = source } }
        let lock = NSRecursiveLock()
        var values: [String: Weak] = [:]
    }
    private static let receiverSources = ReceiverSources()
    static func forReceiver(receiver: MacApplicationPort, conversationID: String, runID: String,
                            assistantMessageID: String, sourceHostID: String = "fixture-host",
                            contextUsage: ScriptedCanonicalContextUsage? = nil) throws -> MacAssistantJournalTestFixture {
        let key = receiver.databaseURL.path + "\n" + runID
        receiverSources.lock.lock(); defer { receiverSources.lock.unlock() }
        if let source = receiverSources.values[key]?.source {
            guard source.conversationID == conversationID, source.assistantMessageID == assistantMessageID,
                  source.sourceHostID == sourceHostID else { throw MacRemoteHostError.invalidResponse }
            return source
        }
        let source = try MacAssistantJournalTestFixture(receiver: receiver, conversationID: conversationID,
            runID: runID, assistantMessageID: assistantMessageID, sourceHostID: sourceHostID, contextUsage: contextUsage)
        receiverSources.values = receiverSources.values.filter { $0.value.source != nil }
        receiverSources.values[key] = ReceiverSources.Weak(source)
        return source
    }
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let sourceHostID: String
    let coreSessionID: String
    let source: MacApplicationServiceClient
    let submissionIdentity: MacRemoteSubmissionIdentity
    let submissionRequest: MacCanonicalJSONValue
    let requestDigest: String
    let databaseURL: URL
    private let directory: URL
    private let invocationDigest: String
    private var model: [String: Any]
    private var sequence = 2
    private var answer = ""
    private var terminal: MacAssistantRunTerminalSeal?
    private let contextUsage: ScriptedCanonicalContextUsage?

    init(conversationID: String, runID: String, assistantMessageID: String,
         sourceHostID: String = "fixture-host", userText: String = "Original request",
         userMessageID: String? = nil, userPartID: String? = nil, coreSessionID: String? = nil,
         tool: ScriptedCanonicalTool? = nil, clientID: String = "fixture-client",
         semanticRequest: [String: Any]? = nil, contextUsage: ScriptedCanonicalContextUsage? = nil,
         executeInitialTool: Bool = true) throws {
        self.contextUsage = contextUsage
        self.conversationID = conversationID
        self.runID = runID
        self.assistantMessageID = assistantMessageID
        self.sourceHostID = sourceHostID
        self.coreSessionID = coreSessionID ?? "fixture-core:\(conversationID)"
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("mac-source-journal-\(UUID())")
        databaseURL = directory.appendingPathComponent("source.db")
        source = try MacApplicationServiceClient(databaseURL: databaseURL)
        let userMessageID = userMessageID ?? "\(runID):user"
        let userPartID = userPartID ?? "\(userMessageID):text"
        submissionIdentity = .init(source_host_id: sourceHostID, client_id: clientID, conversation_id: conversationID,
            submission_id: runID, user_message_id: userMessageID, assistant_message_id: assistantMessageID)
        submissionRequest = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONSerialization.data(withJSONObject:
            semanticRequest ?? ["conversationId":conversationID, "expected_host_id":sourceHostID, "client_id":clientID,
                "submission_id":runID, "user_message_id":userMessageID, "assistant_message_id":assistantMessageID,
                "messages":[["role":"user", "content":userText]]]))
        struct Registered: Decodable { let receipt: MacSourceSubmissionReceipt; let created: Bool }
        let registered: Registered = try Self.result(source, ["type":"register_source_submission",
            "identity":Self.object(submissionIdentity), "request":Self.object(submissionRequest)])
        guard registered.created else { throw MacRemoteHostError.invalidResponse }
        requestDigest = registered.receipt.request_digest
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let capability: [String: Any] = [
            "id": "model.stream", "title": "Fixture model", "route": "model", "availability": "available",
            "exposure": "internal", "input_schema": ["type": "object"],
            "output_policy": ["preview_char_limit": 4_000, "persist_full_output": true, "allow_artifacts": true],
            "supports_streaming": true, "parallelism": "serial", "cancellation": "cooperative", "replay_safety": "recoverable",
        ]
        var capabilities = ["model.stream": capability]
        if let tool {
            var descriptor = capability
            descriptor["id"] = tool.capabilityID
            descriptor["title"] = tool.title
            descriptor["route"] = "tool"
            descriptor["exposure"] = "visible"
            descriptor["supports_streaming"] = false
            // This deterministic Host only returns the supplied in-memory
            // fixture output; it never opens a file or navigates a browser.
            descriptor["input_normalization"] = ["origin": ["kind": "builtin"],
                "permission_contract": ["risk": "ordinary_read"],
                "effect": ["schema_version": 1, "fixed": "investigate", "variants": [:]]]
            capabilities[tool.capabilityID] = descriptor
        }
        // Observation and steering use the exact frozen Run contract even
        // though this deterministic Host supplies model responses itself.
        let outputTokens = contextUsage.map { max(64, $0.window - $0.promptLimit) } ?? 4_096
        let route: [String: Any] = [
            "kind": "http", "protocol": "chat_completions", "output_token_field": "max_tokens", "siwc_account": NSNull(),
            "provider": "fixture", "endpoint": "http://127.0.0.1:1/v1", "model": "fixture-model",
            "endpoint_query_sha256": NSNull(), "credential_sha256": String(repeating: "a", count: 64),
            "credential_source": ["kind": "ephemeral"], "endpoint_query_source": ["kind": "none"],
        ]
        let outputAdmission = try Self.result(source, ["type": "admit_host_output",
            "routes": [["route": route, "max_input_tokens": NSNull(),
                "max_output_tokens": NSNull(), "max_total_tokens": NSNull()]],
            "preference": ["explicit": outputTokens]])
        let binding: [String: Any] = [
            "version": 8, "primary": route, "fallbacks": [],
            "generation": ["temperature": 0, "top_p": 1, "max_output_tokens": outputTokens,
                "max_retries": 0, "parallel_tool_calls": false, "reasoning_selection": NSNull(),
                "reasoning_tier": "balanced",
                "sampling_override": false, "generation_options": [:], "thinking_enabled": false,
                "prompt_cache": false, "prompt_cache_key_sha256": NSNull(),
                "prompt_cache_key_prefix_sha256": NSNull(), "prompt_cache_retention": NSNull()],
            "output_admission": outputAdmission,
            "execution_policy_sha256": String(repeating: "b", count: 64),
        ]
        let begun: [String: Any] = try Self.result(source, ["type": "begin_assistant_turn", "input": [
            "conversation_id": conversationID, "conversation_title": "Source fixture",
            "user_message_id": userMessageID, "user_part_id": userPartID,
            "user_text": userText, "assistant_message_id": assistantMessageID, "created_at_ms": now,
            "core_config": ["session_id": self.coreSessionID, "model": "fixture-model",
                "turn_identity": ["host_invocation": binding,
                    "model_capability_id": "mac-journal-fixture", "model_capability_revision": 1,
                    "model_capability": Self.modelCapabilityProfile(toolCapabilityID: tool?.capabilityID,
                        contextWindow: contextUsage?.window ?? 128_000, maxOutput: outputTokens)],
                "context_window": ["context_window_tokens": contextUsage?.window ?? 128_000,
                    "reserved_output_tokens": outputTokens,
                    "prompt_token_limit": contextUsage?.promptLimit ?? 123_904],
                "tools": tool.map { [["type": "function", "function": ["name": $0.capabilityID,
                    "description": $0.title, "parameters": ["type": "object"]]]] } ?? [],
                "host_catalog": ["host_id": sourceHostID, "revision": 1, "availability_epoch": 1,
                    "capabilities": capabilities]],
            "submission": ["type": "user_input", "submission_id": runID, "delivery": "current_turn",
                "submitted_at_epoch_ms": now,
                "items": [["type": "message", "id": userMessageID, "role": "user", "content": userText]]],
        ]])
        invocationDigest = try XCTUnwrap(begun["invocation_config_digest"] as? String)
        let claimed: [String: Any] = try Self.result(source, ["type": "claim_committed_host_requests", "input": [
            "run_id": runID, "worker_generation": "fixture-generation", "lease_owner": "fixture-worker",
            "claimed_at_ms": now, "lease_expires_at_ms": now + 240_000, "limit": 64,
        ]])
        model = try XCTUnwrap((claimed["requests"] as? [[String: Any]])?.first {
            ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream"
        })
        let originalModel = model
        _ = try accept(sequence: 1, payload: ["type": "execution_started"])
        _ = try accept(sequence: 2, payload: ["type": "model_attempt_started", "provider": "fixture",
            "model": "fixture-model", "reason": "initial"])
        let current: [String: Any] = try Self.result(source, ["type": "load_committed_host_requests", "run_id": runID])
        model = try XCTUnwrap((current["requests"] as? [[String: Any]])?.first {
            $0["effect_id"] as? String == originalModel["effect_id"] as? String
        })
        model["delivery"] = originalModel["delivery"]
        if let tool, executeInitialTool { try performTool(tool) }
    }

    convenience init(receiver: MacApplicationPort, conversationID: String, runID: String,
                     assistantMessageID: String, sourceHostID: String = "fixture-host",
                     contextUsage: ScriptedCanonicalContextUsage? = nil) throws {
        if let intent = try receiver.remoteSubmissionIntent(conversationID: conversationID),
           intent.phase == .bound, let digest = intent.request_digest {
            let request = try receiver.remoteSubmissionRequest(identity: intent.identity, digest: digest)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
            let messages = object["messages"] as? [[String: Any]]
            try self.init(conversationID: conversationID, runID: runID, assistantMessageID: assistantMessageID,
                sourceHostID: sourceHostID, userText: messages?.last?["content"] as? String ?? "Original request",
                userMessageID: intent.identity.user_message_id, clientID: intent.identity.client_id, semanticRequest: object,
                contextUsage: contextUsage)
            try ingestAdmissions(into: receiver)
            return
        }
        let window = try receiver.loadConversationWindow(conversationID)
        let assistantIndex = try XCTUnwrap(window.projection.messages.firstIndex { $0.messageID == assistantMessageID })
        let user = try XCTUnwrap(window.projection.messages[..<assistantIndex].last { $0.role == .user })
        let original = try MacCanonicalConversationRepository.projectMessage(user)
        try self.init(conversationID: conversationID, runID: runID, assistantMessageID: assistantMessageID,
            sourceHostID: sourceHostID, userText: original.text, userMessageID: user.messageID,
            userPartID: try XCTUnwrap(user.parts.first?.partID), contextUsage: contextUsage)
    }

    deinit { try? FileManager.default.removeItem(at: directory) }

    func append(_ text: String) throws -> MacAssistantJournalCursor {
        guard terminal == nil else { throw MacRemoteHostError.invalidResponse }
        let part = try XCTUnwrap((model["payload"] as? [String: Any])?["text_part_id"] as? String)
        sequence += 1
        _ = try accept(sequence: sequence, payload: ["type": "model_committed_delta", "part_id": part, "delta": text])
        answer += text
        return try XCTUnwrap(heads().last)
    }

    func appendReasoning(_ text: String) throws -> MacAssistantJournalCursor {
        guard terminal == nil else { throw MacRemoteHostError.invalidResponse }
        let part = try XCTUnwrap((model["payload"] as? [String: Any])?["reasoning_part_id"] as? String)
        sequence += 1
        _ = try accept(sequence: sequence, payload: ["type": "model_delta", "channel": "reasoning",
            "part_id": part, "delta": text])
        return try XCTUnwrap(heads().last)
    }

    @discardableResult
    func continueAfterOutputLimit() throws -> MacAssistantJournalCursor {
        guard terminal == nil, !answer.isEmpty else { throw MacRemoteHostError.invalidResponse }
        let part = try XCTUnwrap((model["payload"] as? [String: Any])?["text_part_id"] as? String)
        sequence += 1
        _ = try accept(sequence: sequence, payload: [
            "type": "model_output_limited", "items": [["type": "message", "id": part,
                "role": "assistant", "content": answer]], "tool_calls": [],
            "usage": ["input_tokens": 8, "output_tokens": max(1, answer.utf8.count / 4)],
        ])
        model = try XCTUnwrap(claim().first {
            ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream"
        })
        answer = ""
        sequence = 2
        _ = try accept(sequence: 1, payload: ["type": "execution_started"])
        _ = try accept(sequence: 2, payload: ["type": "model_attempt_started", "provider": "fixture",
            "model": "fixture-model", "reason": "initial"])
        return try XCTUnwrap(heads().last)
    }

    @discardableResult
    func finish(_ text: String? = nil) throws -> MacAssistantRunTerminalSeal {
        if let terminal { return terminal }
        // An empty model completion requests recovery, not a successful seal.
        // Supply an actual fixture response when this scenario has no deltas.
        let finalText = text ?? (answer.isEmpty ? "Fixture completed." : answer)
        if answer.isEmpty { _ = try append(finalText) }
        else { XCTAssertEqual(answer, finalText, "The committed stream and final model content must agree") }
        sequence += 1
        _ = try accept(sequence: sequence, payload: [
            "type": "model_completed", "items": [["type": "message", "id": try XCTUnwrap((model["payload"] as? [String: Any])?["text_part_id"] as? String),
                "role": "assistant", "content": answer]], "tool_calls": [],
            "usage": ["input_tokens": contextUsage?.used ?? 8, "output_tokens": max(1, answer.utf8.count / 4),
                "total_tokens": (contextUsage?.used ?? 8) + max(1, answer.utf8.count / 4)],
        ])
        let seal: MacAssistantRunTerminalSeal? = try Self.result(source, [
            "type": "local_assistant_run_terminal_seal", "conversation_id": conversationID, "run_id": runID,
        ])
        terminal = try XCTUnwrap(seal, "The source Application must seal its actual terminal transition")
        return terminal!
    }

    func heads() throws -> [MacAssistantJournalCursor] {
        try Self.result(source, ["type": "local_run_assistant_journal_heads", "conversation_id": conversationID,
            "run_id": runID, "limit": 64])
    }

    func admittedHead() throws -> MacAssistantJournalCursor {
        let admission = try XCTUnwrap(submissionReceipt().admission)
        return try Self.result(source, ["type": "local_assistant_journal_head",
            "identity": Self.object(admission.identity)])
    }

    func committedContextUsage() throws -> MacCommittedRunContextUsage? {
        let admission = try XCTUnwrap(submissionReceipt().admission)
        return try Self.result(source, ["type": "local_run_context_usage", "identity": Self.object(admission.identity)])
    }

    func ingest(into target: MacApplicationPort, through: MacAssistantJournalCursor) throws {
        try ingestAdmissions(into: target)
        var after = try target.assistantJournalReceptionState(sourceHostID: sourceHostID, through: through).after
        while after != through {
            let page: MacAssistantJournalPage = try Self.result(source, ["type": "read_assistant_journal",
                "after": Self.object(after), "through": Self.object(through), "max_records": 32, "max_encoded_bytes": 256 * 1024])
            guard case .records(let records) = page else { throw MacRemoteHostError.invalidResponse }
            for record in records.records {
                while let need = try target.nextAssistantJournalContent(sourceHostID: sourceHostID, record: record) {
                    let snapshot: MacAssistantJournalSnapshot = try Self.result(source, [
                        "type": "open_assistant_journal_snapshot", "at": Self.object(record.cursor),
                    ])
                    let chunk: MacAssistantJournalChunk
                    do {
                        chunk = try Self.result(source, ["type": "read_assistant_journal_record_chunk",
                            "snapshot": Self.object(snapshot), "part_id": need.part_id, "content": Self.object(need.content),
                            "chunk_index": need.expected_prefix.chunk_count])
                    } catch {
                        try? release(snapshot)
                        throw error
                    }
                    try release(snapshot)
                    try target.stageAssistantJournalContent(sourceHostID: sourceHostID, record: record,
                        need: need, chunk: chunk)
                }
                _ = try target.receiveAssistantJournalRecord(sourceHostID: sourceHostID, record: record)
            }
            guard records.next != after || records.complete else { throw MacRemoteHostError.invalidResponse }
            after = records.next
            if records.complete { break }
        }
        XCTAssertTrue(try target.assistantJournalReceptionState(sourceHostID: sourceHostID, through: through).complete)
    }

    func ingestAdmissions(into target: MacApplicationPort, through required: MacRunMessageAdmissionCursor? = nil) throws {
        let receipt = try submissionReceipt()
        if let intent = try target.remoteSubmissionIntent(conversationID: conversationID) {
            if intent.phase == .prepared {
                _ = try target.bindRemoteSubmissionRequest(identity: intent.identity, request: submissionRequest)
            }
            _ = try target.acceptRemoteSubmissionReceipt(receipt)
        } else if try target.canonicalRunTerminal(conversationID: conversationID, runID: runID) == nil {
            try target.adoptObservedRemoteRun(receipt)
        }
        let through: MacRunMessageAdmissionCursor = try required ?? Self.result(source, [
            "type": "local_run_message_admission_head", "conversation_id": conversationID, "run_id": runID,
        ])
        var after = try target.runMessageAdmissionReceptionState(sourceHostID: sourceHostID, through: through)
        while after != through {
            let page: MacRunMessageAdmissionPage = try Self.result(source, [
                "type": "read_run_message_admissions", "after": Self.object(after), "through": Self.object(through),
                "max_records": 32, "max_encoded_bytes": 256 * 1024,
            ])
            guard page.after == after, page.through == through, page.next != after else { throw MacRemoteHostError.invalidResponse }
            let received = try target.receiveRunMessageAdmissions(sourceHostID: sourceHostID, page: page)
            guard received == page.next else { throw MacRemoteHostError.invalidResponse }
            after = received
            if page.complete { break }
        }
        XCTAssertEqual(after, through)
    }

    func submissionReceipt() throws -> MacSourceSubmissionReceipt {
        let receipt: MacSourceSubmissionReceipt? = try Self.result(source, ["type":"read_source_submission_receipt",
            "identity":Self.object(submissionIdentity), "request_digest":requestDigest])
        return try XCTUnwrap(receipt)
    }

    @discardableResult
    func commitTerminal(into target: MacApplicationPort) throws -> MacAssistantRunTerminalSeal {
        let seal = try finish()
        try ingestAdmissions(into: target, through: seal.admission)
        if let usage = try committedContextUsage() {
            _ = try target.acceptRemoteRunContextUsage(sourceHostID: sourceHostID, record: usage)
        }
        for head in try heads() { try ingest(into: target, through: head) }
        _ = try target.beginReceiveAssistantRunTerminal(sourceHostID: sourceHostID, seal: seal)
        var after = "0"
        while true {
            let page: MacAssistantRunTerminalManifestPage = try Self.result(source, [
                "type": "read_local_assistant_run_terminal_manifest", "seal": Self.object(seal), "after_entry": after,
                "max_records": 64, "max_encoded_bytes": 128 * 1024,
            ])
            _ = try target.stageAssistantRunTerminalManifest(sourceHostID: sourceHostID, page: page)
            if page.complete { break }
            guard page.next_entry != after else { throw MacRemoteHostError.invalidResponse }
            after = page.next_entry
        }
        let reception = try target.commitReceivedAssistantRunTerminal(sourceHostID: sourceHostID, seal: seal)
        XCTAssertTrue(reception.committed)
        return seal
    }

    func event(through: MacAssistantJournalCursor) -> MacLocalAgentEvent {
        MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
            assistantJournalCursor: through).bindingCanonicalRun(runID)
    }

    func terminalEvent(seal: MacAssistantRunTerminalSeal) -> MacLocalAgentEvent {
        MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
            assistantRunTerminalSealDigest: seal.seal_digest).bindingCanonicalRun(runID)
    }

    private func release(_ snapshot: MacAssistantJournalSnapshot) throws {
        _ = try source.execute(commandData: JSONSerialization.data(withJSONObject: [
            "type": "release_assistant_journal_snapshot", "snapshot": Self.object(snapshot),
        ]))
    }

    func performTool(_ tool: ScriptedCanonicalTool, outcome: String = "succeeded") throws {
        sequence += 1
        _ = try accept(sequence: sequence, payload: ["type": "model_completed", "items": [],
            "tool_calls": [["call_id": tool.callID, "name": tool.capabilityID, "arguments": tool.input]],
            "usage": ["input_tokens": 8, "output_tokens": 3, "total_tokens": 11]])
        var requests = try claim()
        let call = try XCTUnwrap(requests.first { ($0["payload"] as? [String: Any])?["type"] as? String == "execute_tool" })
        let originalModel = model
        model = call
        _ = try accept(sequence: 1, payload: ["type": "execution_started"])
        _ = try accept(sequence: 2, payload: ["type": "tool_progress", "title": tool.title,
            "detail": tool.summary, "metadata": [:]])
        _ = try accept(sequence: 3, payload: ["type": "tool_completed", "call_id": tool.callID,
            "output": tool.output, "outcome": outcome, "full_output_ref": NSNull(), "artifacts": [[
                "uri": tool.resourceURI, "name": URL(string: tool.resourceURI)?.lastPathComponent ?? "Fixture file",
                "mime_type": "text/plain", "operation": "read",
            ]]])
        requests = try claim()
        model = try XCTUnwrap(requests.first { ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream" })
        XCTAssertNotEqual(model["effect_id"] as? String, originalModel["effect_id"] as? String)
        sequence = 2
        _ = try accept(sequence: 1, payload: ["type": "execution_started"])
        _ = try accept(sequence: 2, payload: ["type": "model_attempt_started", "provider": "fixture",
            "model": "fixture-model", "reason": "initial"])
    }

    private func claim() throws -> [[String: Any]] {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let claimed: [String: Any] = try Self.result(source, ["type": "claim_committed_host_requests", "input": [
            "run_id": runID, "worker_generation": "fixture-generation", "lease_owner": "fixture-worker",
            "claimed_at_ms": now, "lease_expires_at_ms": now + 240_000, "limit": 64,
        ]])
        return try XCTUnwrap(claimed["requests"] as? [[String: Any]])
    }

    private func accept(sequence: Int, payload: [String: Any]) throws -> [String: Any] {
        let keys: Set<String> = ["request_id", "attempt_id", "delegated_invocation_id", "effect_id", "host_id",
            "lease_id", "session_id", "turn_id", "catalog_revision", "availability_epoch", "delivery"]
        var response = model.filter { keys.contains($0.key) }
        response["sequence"] = sequence
        response["payload"] = payload
        return try Self.result(source, ["type": "reduce_and_accept_host_response", "input": [
            "conversation_id": conversationID, "run_id": runID, "invocation_config_digest": invocationDigest,
            "response": response, "received_at_ms": Int64(Date().timeIntervalSince1970 * 1_000),
        ]])
    }

    private static func result(_ source: MacApplicationServiceClient, _ input: [String: Any]) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: source.execute(
            commandData: JSONSerialization.data(withJSONObject: input))) as? [String: Any])
    }

    private static func result<T: Decodable>(_ source: MacApplicationServiceClient, _ input: [String: Any]) throws -> T {
        let data = try source.execute(commandData: JSONSerialization.data(withJSONObject: input))
        return try JSONDecoder().decode(T.self, from: data)
    }

    private static func object<T: Encodable>(_ value: T) throws -> Any {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value), options: [.fragmentsAllowed])
    }
}
