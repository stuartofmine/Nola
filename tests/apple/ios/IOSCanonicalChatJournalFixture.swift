// SPDX-License-Identifier: Apache-2.0
// Deterministic transport fixture for isolated protocol and receipt unit tests only.
// Never used as product UI or real-model acceptance evidence.
#if DEBUG
import Foundation
import CryptoKit
@testable import Nola

nonisolated final class IOSCanonicalChatJournalFixture: @unchecked Sendable {
    static let sourceHostID = "ui-test-canonical-host"
    private static let streamingParagraph = "The **canonical Host stream** rendered incrementally.\n\n"
        + (1...12).map { "Checkpoint \($0): Keep the original message, source owner, and visible history in their canonical order.\n\n" }.joined()

    enum Stage: CaseIterable, Equatable, Sendable {
        case reasoning, toolStarted, toolCompleted, heading, paragraph, table, terminal

        var delayNanoseconds: UInt64 {
            switch self {
            case .reasoning: 3_000_000_000
            // Leave time for genuine drag/disclosure gestures before the
            // final block; AX auto-scrolling does not pause stream following.
            case .table: 24_000_000_000
            case .heading, .paragraph: 750_000_000
            default: 300_000_000
            }
        }
    }

    final class Run: @unchecked Sendable {
        let conversationID: String
        let identity: IOSCanonicalAgentTurnIdentity
        let coreSessionID: String
        let application: IOSCanonicalAgentApplication
        let answer: String
        var transition: AgentCoreJSONValue
        var modelRequest: AgentCoreJSONValue?
        var toolRequest: AgentCoreJSONValue?
        var sequences: [String: Int] = [:]
        var published: [IOSAssistantJournalCursor] = []
        var stageIndex = 0
        var submissionIdentity: AgentCoreJSONValue = .null
        var requestDigest = ""

        init(conversationID: String, identity: IOSCanonicalAgentTurnIdentity, coreSessionID: String,
             application: IOSCanonicalAgentApplication, answer: String, transition: AgentCoreJSONValue) {
            self.conversationID = conversationID
            self.identity = identity
            self.coreSessionID = coreSessionID
            self.application = application
            self.answer = answer
            self.transition = transition
        }
    }

    private let source: IOSApplicationRuntime
    private let orderingMarker: String
    private let lock = NSLock()
    private var runs: [String: Run] = [:]
    private let diagnosticLock = NSLock()

    private func diagnostic(_ event: String) {
        guard let path = ProcessInfo.processInfo.environment["NOLA_UI_FIXTURE_DIAGNOSTICS"] else { return }
        let url = URL(fileURLWithPath: path).standardizedFileURL
        guard url.deletingLastPathComponent().path == "/tmp", url.lastPathComponent.hasPrefix("nola-ui-fixture-") else { return }
        // The simulator app cannot write the runner's host /tmp. Keep the
        // explicitly enabled fixture trace in its own sandbox.
        let sandboxURL = FileManager.default.temporaryDirectory.appendingPathComponent(url.lastPathComponent)
        diagnosticLock.lock()
        defer { diagnosticLock.unlock() }
        do {
            let line = try JSONSerialization.data(withJSONObject: ["event": event]) + Data([10])
            if !FileManager.default.fileExists(atPath: sandboxURL.path) { FileManager.default.createFile(atPath: sandboxURL.path, contents: nil) }
            let handle = try FileHandle(forWritingTo: sandboxURL)
            defer { try? handle.close() }
            try handle.seekToEnd(); try handle.write(contentsOf: line)
        } catch { /* Test diagnostics must not change the actual source transaction. */ }
    }

    init(databaseURL: URL, orderingMarker: String) throws {
        source = try IOSApplicationRuntime(databaseURL: databaseURL)
        self.orderingMarker = orderingMarker
    }

    func response(for request: URLRequest, token: String) throws -> WebAgentTransportResponse {
        diagnostic("request " + (request.url?.path ?? ""))
        if request.httpMethod == "GET", request.url?.path.hasSuffix("/agent/settings") == true {
            let settings: [String: Any] = [
                "host_identity": ["id": Self.sourceHostID,
                    "permission_protocol_version": WebAgentProvider.hostPermissionProtocolVersion],
                "model_profiles": [["profile_id": "ios-journal-fixture", "revision": 1,
                    "display_name": "Fixture model", "model_name": "fixture-model"]],
                "default_chat_model_resolution": ["kind": "resolved",
                    "profile_id": "ios-journal-fixture", "origin": "default"],
                "default_chat_model_revision": 1
            ]
            let bytes = try JSONSerialization.data(withJSONObject: settings)
            return transport(request: request, contentType: "application/json", bytes: AsyncThrowingStream {
                for byte in bytes { $0.yield(byte) }; $0.finish()
            })
        }
        guard let body = request.httpBody,
              let encrypted = try JSONSerialization.jsonObject(with: body) as? [String: Any] else {
            throw invalid("Missing encrypted fixture request")
        }
        let payload = try WebAgentProvider.decryptMobileJSON(encrypted, token: token)
        func pairedResult(_ result: AgentCoreJSONValue) throws -> WebAgentTransportResponse {
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": Self.sourceHostID, "result": Self.object(result)
            ], token: token)
            envelope["type"] = "encrypted"
            let data = try JSONSerialization.data(withJSONObject: envelope)
            return transport(request: request, contentType: "application/json", bytes: AsyncThrowingStream {
                for byte in data { $0.yield(byte) }; $0.finish()
            })
        }
        if request.url?.path.hasSuffix("/chat/agent/v2/configuration") == true {
            guard payload["operation"] as? String == "read",
                  payload["expected_host_id"] as? String == Self.sourceHostID,
                  let conversation = payload["conversation_id"] as? String,
                  let client = payload["client_id"] as? String,
                  let run = payload["expected_run_id"] as? String,
                  let assistant = payload["assistant_message_id"] as? String else {
                throw invalid("Unsupported fixture permission configuration operation")
            }
            let receipt = try command(["type": "observed_source_submission_receipt",
                "source_host_id": Self.sourceHostID, "client_id": client, "conversation_id": conversation,
                "run_id": run, "assistant_message_id": assistant])
            guard case .object(let sourceOwner) = receipt, sourceOwner["phase"] == .string("admitted") else {
                throw invalid("Fixture permission configuration has no admitted source owner")
            }
            let active = try command(["type": "active_run", "conversation_id": conversation])
            if case .object(let fields) = active,
               fields["run_id"] != .string(run) || fields["assistant_message_id"] != .string(assistant) {
                throw invalid("Fixture permission configuration active owner changed")
            }
            return try pairedResult(command(["type": "read_permission_configuration",
                "conversation_id": conversation, "source_host_id": Self.sourceHostID, "client_id": client,
                "unresolved_run_ids": [], "source_unconfirmed": false,
                "minimum_policy_revision": payload["minimum_policy_revision"] ?? NSNull()]))
        }
        if request.url?.path.hasSuffix("/chat/agent/v2/ui/queued-input") == true {
            guard payload["expected_host_id"] as? String == Self.sourceHostID,
                  let conversation = payload["conversation_id"] as? String,
                  let client = payload["client_id"] as? String, !client.isEmpty,
                  let epoch = payload["expected_source_store_epoch"] as? String,
                  try command(["type": "read_ui_authority"]) == .string(epoch) else {
                throw invalid("Fixture queued input source authority changed")
            }
            let active = try command(["type": "active_run", "conversation_id": conversation])
            let head = active == .null ? try command(["type": "peek_agent_command", "session_id": conversation]) : .null
            var dispatch: AgentCoreJSONValue = .null
            if case .object(let fields) = head, case .string(let id)? = fields["command_id"] {
                dispatch = try command(["type": "observed_source_submission_receipt", "source_host_id": Self.sourceHostID,
                    "client_id": client, "conversation_id": conversation, "run_id": id, "assistant_message_id": id + ":assistant"])
            }
            return try pairedResult(.object(["head": head, "dispatch": dispatch]))
        }
        if request.url?.path.hasSuffix("/chat/agent/v2/submission-frontier") == true {
            let result = try source.executeAgentCommand(Self.value([
                "type": "observe_submission_frontier", "source_host_id": Self.sourceHostID,
                "request": payload
            ]))
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": Self.sourceHostID, "result": Self.object(result)
            ], token: token)
            envelope["type"] = "encrypted"
            let data = try JSONSerialization.data(withJSONObject: envelope)
            return transport(request: request, contentType: "application/json", bytes: AsyncThrowingStream {
                for byte in data { $0.yield(byte) }; $0.finish()
            })
        }
        if request.url?.path.hasSuffix("/chat/agent/v2/observe") == true {
            let result = try observationResult(payload)
            guard let conversation = payload["conversation_id"] as? String,
                  let client = payload["client_id"] as? String else { throw invalid("Missing observation owner") }
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": Self.sourceHostID, "conversation_id": conversation,
                "client_id": client, "result": Self.object(result)
            ], token: token)
            envelope["type"] = "encrypted"
            let bytes = try JSONSerialization.data(withJSONObject: envelope)
            return transport(request: request, contentType: "application/json", bytes: AsyncThrowingStream {
                for byte in bytes { $0.yield(byte) }; $0.finish()
            })
        }
        if request.url?.path.hasSuffix("/agent/tool-plan/prepare") == true {
            let snapshot = try prepareToolPlan(payload)
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": Self.sourceHostID, "result": Self.object(snapshot)
            ], token: token)
            envelope["type"] = "encrypted"
            let bytes = try JSONSerialization.data(withJSONObject: envelope)
            return transport(request: request, contentType: "application/json", bytes: AsyncThrowingStream {
                for byte in bytes { $0.yield(byte) }; $0.finish()
            })
        }
        if request.url?.path.hasSuffix("/submission") == true {
            guard let operation = payload["operation"] as? String, ["observe", "cancel"].contains(operation) else {
                throw invalid("Unsupported fixture submission operation")
            }
            let result = try source.executeAgentCommand(Self.value([
                "type":operation == "cancel" ? "cancel_source_submission" : "read_source_submission_receipt",
                "identity":payload["identity"]!, "request_digest":payload["request_digest"]!]))
            var envelope = try WebAgentProvider.encryptMobileJSON(["source_host_id":Self.sourceHostID,
                "result":Self.object(result)], token: token)
            envelope["type"] = "encrypted"
            let bytes = try JSONSerialization.data(withJSONObject: envelope)
            return transport(request: request, contentType: "application/json", bytes: AsyncThrowingStream {
                for byte in bytes { $0.yield(byte) }; $0.finish()
            })
        }
        if request.url?.path.hasSuffix("/journal") == true {
            let result = try journalResult(payload)
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": Self.sourceHostID,
                "result": Self.object(result),
            ], token: token)
            envelope["type"] = "encrypted"
            let bytes = try JSONSerialization.data(withJSONObject: envelope, options: [.sortedKeys])
            return transport(request: request, contentType: "application/json", bytes: AsyncThrowingStream {
                for byte in bytes { $0.yield(byte) }
                $0.finish()
            })
        }
        guard request.url?.path.hasSuffix("/stream") == true else {
            throw invalid("Unsupported canonical fixture endpoint: \(request.url?.path ?? "")")
        }
        guard let reference = payload["tool_plan_ref"] as? [String: Any],
              reference["host_id"] as? String == Self.sourceHostID,
              reference["client_id"] as? String == payload["client_id"] as? String,
              reference["session_id"] as? String == payload["conversationId"] as? String else {
            throw invalid("Missing or mismatched fixture Host tool plan")
        }
        let preparedPlan = try source.executeAgentCommand(Self.value([
            "type": "read_prepared_host_tool_plan", "reference": reference,
            "now_ms": Self.nowMilliseconds()
        ]))
        guard preparedPlan != .null else { throw invalid("Fixture Host tool plan is unavailable") }
        let run = try begin(payload)
        let bytes = AsyncThrowingStream<UInt8, Error> { continuation in
            let worker = Task {
                do {
                    let receipt = try self.submissionReceipt(run)
                    let data = try JSONEncoder().encode(AgentCoreJSONValue.object(["type":.string("submission_receipt"), "receipt":receipt]))
                    for byte in "data: \(String(decoding: data, as: UTF8.self))\n\n".utf8 { continuation.yield(byte) }
                    for stage in Stage.allCases {
                        try await Task.sleep(nanoseconds: stage.delayNanoseconds)
                        try Task.checkCancellation()
                        self.diagnostic("advance " + String(describing: stage))
                        for event in try self.advance(run, stage: stage) {
                            let data = try JSONEncoder().encode(event)
                            let frame = "data: \(String(decoding: data, as: UTF8.self))\n\n"
                            for byte in frame.utf8 { continuation.yield(byte) }
                        }
                    }
                    continuation.finish()
                } catch {
                    self.diagnostic("stream failed " + error.localizedDescription)
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in worker.cancel() }
        }
        return transport(request: request, contentType: "text/event-stream", bytes: bytes)
    }

    func observationResult(_ payload: [String: Any]) throws -> AgentCoreJSONValue {
        lock.lock()
        defer { lock.unlock() }
        guard payload["expected_host_id"] as? String == Self.sourceHostID,
              let conversation = payload["conversation_id"] as? String,
              let client = payload["client_id"] as? String,
              let operation = payload["operation"] as? String,
              ["discover", "observe"].contains(operation) else {
            throw invalid("Invalid fixture observation owner")
        }
        let active = try command(["type": "active_run", "conversation_id": conversation])
        if operation == "discover", active == .null { return .null }
        let activeFields = try Self.object(active) as? [String: Any]
        guard let runID = (operation == "observe" ? payload["expected_run_id"] : activeFields?["run_id"]) as? String,
              let assistant = (operation == "observe" ? payload["assistant_message_id"] : activeFields?["assistant_message_id"]) as? String,
              let run = runs[runID], run.conversationID == conversation,
              run.identity.assistantMessageID == assistant,
              case .object(let owner) = run.submissionIdentity, owner["client_id"] == .string(client) else {
            throw invalid("Fixture observation Run owner mismatch")
        }
        let admission = try command(["type": "local_run_message_admission_head",
                                     "conversation_id": conversation, "run_id": runID])
        let seal = try command(["type": "local_assistant_run_terminal_seal",
                                "conversation_id": conversation, "run_id": runID])
        guard case .object(let admissionFields) = admission, let identity = admissionFields["identity"] else {
            throw invalid("Missing fixture observation admission")
        }
        let context = try source.executeAgentCommand(.object([
            "type": .string("local_run_context_usage"), "identity": identity
        ]))
        let sealed = seal != .null
        return .object([
            "protocol_version": .number(1), "conversation_id": .string(conversation),
            "client_id": .string(client), "run_id": .string(runID),
            "assistant_message_id": .string(assistant), "status": .string(sealed ? "sealed" : "running"),
            "worker": .object(["state": .string(sealed ? "absent" : "running"),
                               "generation": sealed ? .null : .string("1")]),
            "recovery": .string(sealed ? "terminal" : "observe"), "admission": admission,
            "submission_receipt": try submissionReceipt(run), "context_usage": context,
            "terminal_seal": seal, "permissions": .array([]),
            "can_submit": .bool(sealed && active == .null)
        ])
    }

    private func prepareToolPlan(_ payload: [String: Any]) throws -> AgentCoreJSONValue {
        lock.lock()
        defer { lock.unlock() }
        guard Set(payload.keys) == ["expected_host_id", "client_id", "session_id"],
              payload["expected_host_id"] as? String == Self.sourceHostID,
              let clientID = payload["client_id"] as? String,
              let sessionID = payload["session_id"] as? String else {
            throw invalid("Invalid fixture Host tool plan owner")
        }
        let tools: [[String: Any]] = [[
            "tool_id": "file.read", "route": "agent_host",
            "schema": ["type": "function", "function": [
                "name": "file.read", "description": "Read the fixture project overview",
                "parameters": ["type": "object"]
            ]]
        ]]
        let revision = SHA256.hash(data: try JSONSerialization.data(withJSONObject: tools, options: [.sortedKeys]))
            .map { String(format: "%02x", $0) }.joined()
        let now = Self.nowMilliseconds()
        let snapshot = try Self.value([
            "schema_version": 1,
            "reference": ["manifest_id": UUID().uuidString, "host_id": Self.sourceHostID,
                          "client_id": clientID, "session_id": sessionID, "catalog_revision": revision],
            "issued_at_ms": now, "expires_at_ms": now + 300_000, "tools": tools
        ])
        _ = try source.executeAgentCommand(.object([
            "type": .string("persist_prepared_host_tool_plan"), "snapshot": snapshot,
            "now_ms": .number(Double(now))
        ]))
        return snapshot
    }

    func begin(_ payload: [String: Any]) throws -> Run {
        lock.lock()
        defer { lock.unlock() }
        guard let conversationID = payload["conversationId"] as? String, !conversationID.isEmpty,
              let submissionID = payload["submission_id"] as? String,
              let userMessageID = payload["user_message_id"] as? String,
              let userUUID = UUID(uuidString: userMessageID),
              let assistantMessageID = payload["assistant_message_id"] as? String else {
            throw invalid("Missing canonical fixture turn identity")
        }
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: userUUID)
        guard identity.submissionID == submissionID, identity.userMessageID == userMessageID,
              identity.assistantMessageID == assistantMessageID, runs[submissionID] == nil else {
            throw invalid("Fixture turn identity mismatch or duplicate source admission")
        }
        let userText = (payload["messages"] as? [[String: Any]])?
            .last(where: { $0["role"] as? String == "user" })?["content"] as? String
            ?? "Exercise the canonical Host transcript"
        let coreSessionID = "ios-ui-fixture-core:\(conversationID)"
        guard let clientID = payload["client_id"] as? String else { throw invalid("Missing source client identity") }
        let submissionIdentity = try Self.value(["source_host_id":Self.sourceHostID, "client_id":clientID,
            "conversation_id":conversationID, "submission_id":submissionID,
            "user_message_id":userMessageID, "assistant_message_id":assistantMessageID])
        var semantic = payload; semantic.removeValue(forKey: "request_digest")
        let registered = try source.executeAgentCommand(.object(["type":.string("register_source_submission"),
            "identity":submissionIdentity, "request":try Self.value(semantic)]))
        struct Registered: Decodable { let receipt: IOSSourceSubmissionReceipt; let created: Bool }
        let registration = try JSONDecoder().decode(Registered.self, from: JSONEncoder().encode(registered))
        guard registration.created, payload["request_digest"] == nil || payload["request_digest"] as? String == registration.receipt.request_digest else {
            throw invalid("Source submission is not dispatchable")
        }
        let application = IOSCanonicalAgentApplication(store: source, descriptor: .init(
            conversationID: conversationID, conversationTitle: "Canonical Host transcript",
            identity: identity, userText: userText, userContentJSON: nil, contextCheckpointID: nil
        ))
        let transition = try application.begin(
            coreConfig: try Self.configuration(coreSessionID: coreSessionID, source: source),
            submission: Self.value([
                "type": "user_input", "submission_id": submissionID, "delivery": "current_turn",
                "items": [["type": "message", "id": userMessageID, "role": "user", "content": userText]],
            ]), createdAtMs: Self.nowMilliseconds()
        )
        let answer = "## Verified result\n\n" + Self.streamingParagraph
            + "| Check | Status |\n|---|---|\n| Ordering | \(orderingMarker) |"
        let run = Run(conversationID: conversationID, identity: identity, coreSessionID: coreSessionID,
            application: application, answer: answer, transition: transition)
        run.submissionIdentity = submissionIdentity
        run.requestDigest = registration.receipt.request_digest
        runs[submissionID] = run
        return run
    }

    func submissionReceipt(_ run: Run) throws -> AgentCoreJSONValue {
        try source.executeAgentCommand(.object(["type":.string("read_source_submission_receipt"),
            "identity":run.submissionIdentity, "request_digest":.string(run.requestDigest)]))
    }

    func advance(_ run: Run, stage: Stage) throws -> [AgentCoreJSONValue] {
        lock.lock()
        defer { lock.unlock() }
        guard runs[run.identity.submissionID] === run,
              run.stageIndex < Stage.allCases.count, Stage.allCases[run.stageIndex] == stage else {
            throw invalid("Fixture source stage is out of order")
        }
        switch stage {
        case .reasoning:
            let model = try request(run, type: "model_stream")
            run.modelRequest = model
            try startModel(run, request: model)
            try respond(run, request: model, payload: [
                "type": "model_delta", "channel": "reasoning",
                "part_id": try modelPartID(model, field: "reasoning_part_id"),
                "delta": "Reading the request and planning the response.",
            ])
        case .toolStarted:
            guard let model = run.modelRequest else { throw invalid("Missing first model delivery") }
            try respond(run, request: model, payload: Self.modelCompletion(items: [], toolCalls: [[
                "call_id": "ui-tool-1", "name": "file.read", "arguments": ["path": "README.md"],
            ]]))
            let tool = try request(run, type: "execute_tool")
            run.toolRequest = tool
            try respond(run, request: tool, payload: ["type": "execution_started"])
            try respond(run, request: tool, payload: [
                "type": "tool_progress", "title": "Reading README.md",
                "detail": "Inspecting the project overview.", "metadata": [:],
            ])
        case .toolCompleted:
            guard let tool = run.toolRequest else { throw invalid("Missing tool delivery") }
            try respond(run, request: tool, payload: [
                "type": "tool_completed", "call_id": "ui-tool-1", "outcome": "succeeded",
                "output": ["text": "Project overview loaded.", "resources": [[
                    "uri": "file:///tmp/README.md", "operation": "read",
                    "display_name": "README.md", "mime_type": "text/markdown",
                ]]],
                "full_output_ref": NSNull(), "artifacts": [],
            ])
            let model = try request(run, type: "model_stream")
            run.modelRequest = model
            try startModel(run, request: model)
        case .heading, .paragraph, .table:
            guard let model = run.modelRequest else { throw invalid("Missing answer delivery") }
            let delta: String
            switch stage {
            case .heading: delta = "## Verified result\n\n"
            case .paragraph: delta = Self.streamingParagraph
            default: delta = "| Check | Status |\n|---|---|\n| Ordering | \(orderingMarker) |"
            }
            try respond(run, request: model, payload: [
                "type": "model_committed_delta", "part_id": try modelPartID(model, field: "text_part_id"),
                "delta": delta,
            ])
        case .terminal:
            guard let model = run.modelRequest else { throw invalid("Missing terminal model delivery") }
            try respond(run, request: model, payload: Self.modelCompletion(items: [[
                "type": "message", "id": try modelPartID(model, field: "text_part_id"),
                "role": "assistant", "content": run.answer,
            ]], toolCalls: []))
        }
        run.stageIndex += 1
        var notifications = try journalNotifications(run)
        if stage == .terminal {
            let seal = try command([
                "type": "local_assistant_run_terminal_seal", "conversation_id": run.conversationID,
                "run_id": run.identity.submissionID,
            ])
            guard case .object(let fields) = seal, case .string(let digest)? = fields["seal_digest"] else {
                throw invalid("Source Application did not commit a terminal seal")
            }
            notifications.append(.object([
                "type": .string("assistant_run_terminal_committed_v1"),
                "session_id": .string(run.conversationID), "source_host_id": .string(Self.sourceHostID),
                "run_id": .string(run.identity.submissionID), "seal_digest": .string(digest),
            ]))
        }
        return notifications
    }

    func journalResult(_ payload: [String: Any]) throws -> AgentCoreJSONValue {
        lock.lock()
        defer { lock.unlock() }
        guard let operation = payload["operation"] as? String else { throw invalid("Missing journal operation") }
        var input = payload
        input.removeValue(forKey: "operation")
        let commandType: String
        switch operation {
        case "context_usage": commandType = "local_run_context_usage"
        case "admission_head": commandType = "local_run_message_admission_head"
        case "admissions": commandType = "read_run_message_admissions"
        case "heads": commandType = "local_run_assistant_journal_heads"
        case "read": commandType = "read_assistant_journal"
        case "terminal_seal": commandType = "local_assistant_run_terminal_seal"
        case "terminal_manifest": commandType = "read_local_assistant_run_terminal_manifest"
        case "open_snapshot": commandType = "open_assistant_journal_snapshot"
        case "parts": commandType = "read_assistant_journal_parts"
        case "changes": commandType = "read_assistant_journal_part_changes"
        case "range": commandType = "read_assistant_journal_range"
        case "record_chunk": commandType = "read_assistant_journal_record_chunk"
        case "release_snapshot": commandType = "release_assistant_journal_snapshot"
        default: throw invalid("Unsupported fixture journal operation")
        }
        input["type"] = commandType
        return try command(input)
    }

    private func journalNotifications(_ run: Run) throws -> [AgentCoreJSONValue] {
        let heads = try command([
            "type": "local_run_assistant_journal_heads", "conversation_id": run.conversationID,
            "run_id": run.identity.submissionID, "limit": 64,
        ])
        guard case .array(let values) = heads else { throw invalid("Source journal heads are missing") }
        return try values.compactMap { value in
            let cursor = try JSONDecoder().decode(IOSAssistantJournalCursor.self, from: JSONEncoder().encode(value))
            guard !run.published.contains(cursor) else { return nil }
            run.published.append(cursor)
            return .object([
                "type": .string("assistant_journal_committed_v1"), "session_id": .string(run.conversationID),
                "source_host_id": .string(Self.sourceHostID), "through": value,
            ])
        }
    }

    private func respond(_ run: Run, request: AgentCoreJSONValue, payload: [String: Any]) throws {
        guard case .object(let fields) = request, case .string(let requestID)? = fields["request_id"] else {
            throw invalid("Missing source Host request identity")
        }
        let sequence = (run.sequences[requestID] ?? 0) + 1
        let response = try DeviceAgentCoordinator.hostResponseEnvelope(
            request: request, payload: Self.value(payload), sequence: sequence
        )
        run.transition = try run.application.acceptHostResponse(response, receivedAtMs: Self.nowMilliseconds())
        run.sequences[requestID] = sequence
    }

    private func startModel(_ run: Run, request: AgentCoreJSONValue) throws {
        try respond(run, request: request, payload: ["type": "execution_started"])
        try respond(run, request: request, payload: [
            "type": "model_attempt_started", "provider": "fixture", "model": "fixture-model", "reason": "initial",
        ])
    }

    private func modelPartID(_ request: AgentCoreJSONValue, field: String) throws -> String {
        guard case .object(let envelope) = request, case .object(let payload)? = envelope["payload"],
              case .string(let value)? = payload[field] else {
            throw invalid("Source model request did not declare \(field)")
        }
        return value
    }

    private func request(_ run: Run, type: String) throws -> AgentCoreJSONValue {
        guard case .object(let fields) = run.transition, case .array(let requests)? = fields["host_requests"],
              let request = requests.first(where: { value in
                  guard case .object(let envelope) = value, case .object(let payload)? = envelope["payload"] else { return false }
                  return payload["type"] == .string(type)
              }) else { throw invalid("Source Application did not request \(type)") }
        return request
    }

    private func command(_ value: [String: Any]) throws -> AgentCoreJSONValue {
        try source.executeAgentCommand(Self.value(value))
    }

    private static func configuration(coreSessionID: String, source: IOSApplicationRuntime) throws -> AgentCoreJSONValue {
        let route = try value([
            "kind": "http", "protocol": "chat_completions", "output_token_field": "max_tokens",
            "provider": "fixture", "endpoint": "http://127.0.0.1:1/v1", "model": "fixture-model",
            "endpoint_query_sha256": NSNull(), "credential_sha256": String(repeating: "a", count: 64),
            "credential_source": ["kind": "ephemeral"], "endpoint_query_source": ["kind": "none"],
        ])
        let generation = try value([
            "temperature": 0, "top_p": 1, "max_output_tokens": 4096, "max_retries": 0,
            "parallel_tool_calls": false, "reasoning_selection": NSNull(), "sampling_override": false,
            "reasoning_tier": "balanced",
            "generation_options": [:], "thinking_enabled": false, "prompt_cache": false,
            "prompt_cache_key_sha256": NSNull(), "prompt_cache_key_prefix_sha256": NSNull(), "prompt_cache_retention": NSNull(),
        ])
        // Admission is a Core-owned contract. Use the same pure constructor as
        // production instead of embedding a schema version/resolution in fixtures.
        let output = try source.admitHostOutput(routes: [.object([
            "route": route, "max_input_tokens": .null,
            "max_output_tokens": .null, "max_total_tokens": .null,
        ])], preference: .object(["explicit": .number(4096)]))
        let binding = try IOSPreparedHostInvocation(route: route, generation: generation, outputAdmission: output)
            .binding(executionPolicy: .object([:]))
        return .object([
            "session_id": .string(coreSessionID), "model": .string("fixture-model"),
            "turn_identity": .object(["host_invocation": binding,
                "model_capability_id": .string("ios-journal-fixture"),
                "model_capability_revision": .number(1),
                "model_capability": try value([
                    "profile_id":"ios-journal-fixture", "revision":1, "instruction_profile":"generic",
                    "native_tool_calls":true, "parallel_tool_calls":false,
                    "reasoning":true, "reasoning_continuity":false, "prompt_cache":false,
                    "vision_input":false, "context_window_tokens":128000, "max_output_tokens":4096,
                    "request_continuation":false, "max_request_continuations":0,
                    "stream_text":true, "stream_reasoning":true, "stream_tool_calls":true,
                    "direct_tool_capability_ids":["file.read"]
                ])]),
            "context_window": try value(["context_window_tokens": 128000, "reserved_output_tokens": 4096, "prompt_token_limit": 123904]),
            "initial_items": .array([]), "permission_rules": .object(["rules": .array([])]),
            "tools": .array([.object([
                "type": .string("function"), "function": .object([
                    "name": .string("file.read"), "description": .string("Read the fixture project overview"),
                    "parameters": .object(["type": .string("object")]),
                ]),
            ])]),
            "host_catalog": .object([
                "host_id": .string(sourceHostID), "revision": .number(1), "availability_epoch": .number(1),
                "capabilities": .object([
                    "model.stream": capability("model.stream", route: "model", streaming: true),
                    "file.read": capability("file.read", route: "tool", streaming: false, effect: "investigate"),
                ]),
            ]),
        ])
    }

    private static func capability(_ id: String, route: String, streaming: Bool, effect: String = "execute") -> AgentCoreJSONValue {
        .object([
            "id": .string(id), "title": .string(id), "route": .string(route),
            "availability": .string("available"), "exposure": .string("visible"), "permission_domain": .null,
            "input_schema": .object(["type": .string("object")]),
            "input_normalization": .object([
                "effect": .object(["schema_version": .number(1), "fixed": .string(effect), "variants": .object([:])]),
                // This deterministic fixture only reads its synthetic overview.
                // An effect label alone is not a trusted permission contract.
                "permission_contract": .object([
                    "risk": .string(id == "file.read" ? "ordinary_read" : "unknown"),
                ]),
            ]),
            "output_policy": .object([
                "preview_char_limit": .number(4_000), "persist_full_output": .bool(true), "allow_artifacts": .bool(true),
            ]),
            "supports_streaming": .bool(streaming), "supports_argument_streaming": .bool(false),
            "parallelism": .string("serial"), "cancellation": .string("cooperative"), "replay_safety": .string("recoverable"),
        ])
    }

    private static func modelCompletion(items: [[String: Any]], toolCalls: [[String: Any]]) -> [String: Any] {
        ["type": "model_completed", "items": items, "tool_calls": toolCalls,
         "usage": ["input_tokens": 1_024, "output_tokens": 64, "total_tokens": 1_088]]
    }

    private func transport(request: URLRequest, contentType: String,
                           bytes: AsyncThrowingStream<UInt8, Error>) -> WebAgentTransportResponse {
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": contentType])!
        return .init(bytes: bytes, response: response)
    }

    private static func value(_ object: [String: Any]) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: object))
    }

    private static func object(_ value: AgentCoreJSONValue) throws -> Any {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value), options: [.fragmentsAllowed])
    }

    private static func nowMilliseconds() -> Int64 { Int64(Date().timeIntervalSince1970 * 1_000) }
    private func invalid(_ message: String) -> IOSCanonicalAgentApplicationError {
        diagnostic("invalid " + message)
        return .invalidResult(message)
    }
}
#endif
