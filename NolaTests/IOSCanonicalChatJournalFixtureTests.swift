// SPDX-License-Identifier: Apache-2.0
#if DEBUG
import XCTest
@testable import Nola

final class IOSCanonicalChatJournalFixtureTests: XCTestCase {
    func testSettingsDecodeThroughCurrentProductionHostContract() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"), orderingMarker: "settings")
        var request = URLRequest(url: URL(string: "http://127.0.0.1:9/api/mobile/agent/settings")!)
        request.httpMethod = "GET"
        let response = try fixture.response(for: request, token: "fixture-token")
        var data = Data()
        for try await byte in response.bytes { data.append(byte) }
        let decoded = try JSONDecoder().decode(WebAgentProvider.HostAgentSettings.self, from: data)
        XCTAssertEqual(decoded.host_identity?.id, IOSCanonicalChatJournalFixture.sourceHostID)
        XCTAssertEqual(decoded.host_identity?.permission_protocol_version, WebAgentProvider.hostPermissionProtocolVersion)
    }

    @MainActor
    func testOwnerScopedWebClientRetainsItsTransportAcrossReconfiguration() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"), orderingMarker: "scoped")
        let url = URL(string: "http://127.0.0.1:9")!
        let config = WebAgentProvider.Config(baseURL: url, token: "fixture-token", model: "fixture-model",
            expectedHostId: IOSCanonicalChatJournalFixture.sourceHostID, clientId: "fixture-client")
        let service = ModelService(provider: PlaceholderLocalProvider.shared)
        service.installTestingWebAgentTransport(baseURL: url) { try fixture.response(for: $0, token: config.token) }
        let settings = try await service.hostAgentSettings(config: config)
        XCTAssertEqual(settings.host_identity?.id, IOSCanonicalChatJournalFixture.sourceHostID)
        XCTAssertEqual(settings.host_identity?.permission_protocol_version, WebAgentProvider.hostPermissionProtocolVersion)
        let scoped = service.makeScopedWebAgentClient(config: config)
        scoped.useWebAgentProvider(config: config)
        let provider = try XCTUnwrap(scoped.currentProvider() as? WebAgentProvider)
        let payload = try await provider.prepareHostToolPlan([
            "conversationId": UUID().uuidString, "client_id": "fixture-client",
            "expected_host_id": IOSCanonicalChatJournalFixture.sourceHostID,
        ])
        XCTAssertNotNil(payload["tool_plan_ref"])
        XCTAssertNotNil(payload["model_intent"])
    }

    @MainActor
    func testPairedPermissionReadUsesActualSourceOwnerAndCoreProjection() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"), orderingMarker: "permission-read")
        let conversation = UUID().uuidString
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        _ = try fixture.begin(payload(conversationID: conversation, identity: identity))
        let config = WebAgentProvider.Config(baseURL: URL(string: "http://127.0.0.1:9")!, token: "fixture-token",
            expectedHostId: IOSCanonicalChatJournalFixture.sourceHostID, clientId: "fixture-client")
        let provider = WebAgentProvider(config: config) { try fixture.response(for: $0, token: config.token) }
        let owner = IOSRemoteSubmissionIdentity(source_host_id: IOSCanonicalChatJournalFixture.sourceHostID,
            client_id: "fixture-client", conversation_id: conversation, submission_id: identity.submissionID,
            user_message_id: identity.userMessageID, assistant_message_id: identity.assistantMessageID)
        let projection = try await provider.permissionConfiguration(owner: owner)
        try projection.validate(conversationID: conversation, runID: identity.submissionID)
        XCTAssertEqual(projection.owner.assistant_message_id, identity.assistantMessageID)
    }

    func testObservationReadsActiveAndSealedRunFromApplication() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"),
            orderingMarker: "observation")
        let conversation = UUID().uuidString
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        var request: [String: Any] = ["operation": "discover",
            "expected_host_id": IOSCanonicalChatJournalFixture.sourceHostID,
            "client_id": "fixture-client", "conversation_id": conversation]
        XCTAssertEqual(try fixture.observationResult(request), .null)
        let run = try fixture.begin(payload(conversationID: conversation, identity: identity))
        func observe() throws -> IOSRunObservation {
            try JSONDecoder().decode(IOSRunObservation.self, from: JSONEncoder().encode(fixture.observationResult(request)))
        }
        let active = try observe()
        try active.validate(conversation: conversation, client: "fixture-client", previous: nil)
        XCTAssertFalse(active.can_submit)
        XCTAssertEqual(active.status, "running")
        for stage in IOSCanonicalChatJournalFixture.Stage.allCases { _ = try fixture.advance(run, stage: stage) }
        request["operation"] = "observe"
        request["expected_run_id"] = identity.submissionID
        request["assistant_message_id"] = identity.assistantMessageID
        let terminal = try observe()
        try terminal.validate(conversation: conversation, client: "fixture-client", previous: active)
        XCTAssertTrue(terminal.can_submit)
        XCTAssertEqual(terminal.status, "sealed")
        request["client_id"] = "another-client"
        XCTAssertThrowsError(try fixture.observationResult(request))
        request["client_id"] = "fixture-client"
        request["operation"] = "discover"
        XCTAssertEqual(try fixture.observationResult(request), .null)
    }

    func testToolPlanPreparationUsesCurrentEndpointWithoutAdmittingATurn() async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("source.db")
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: database, orderingMarker: "plan")
        let conversation = UUID().uuidString
        let token = "fixture-token"
        var request = URLRequest(url: URL(string: "http://127.0.0.1:9/api/mobile/agent/tool-plan/prepare")!)
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: WebAgentProvider.encryptMobileJSON([
            "expected_host_id": IOSCanonicalChatJournalFixture.sourceHostID,
            "client_id": "fixture-client", "session_id": conversation
        ], token: token))
        let response = try fixture.response(for: request, token: token)
        var data = Data()
        for try await byte in response.bytes { data.append(byte) }
        let envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let decoded = try WebAgentProvider.decryptMobileJSON(envelope, token: token)
        let snapshot = try XCTUnwrap(decoded["result"] as? [String: Any])
        let plan = try JSONDecoder().decode(IOSRemoteHostToolPlan.self,
            from: JSONSerialization.data(withJSONObject: snapshot))
        let reference = try plan.validatedReference(hostID: IOSCanonicalChatJournalFixture.sourceHostID,
            clientID: "fixture-client", sessionID: conversation)
        XCTAssertEqual(plan.tools.map(\.tool_id), ["file.read"])
        let source = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertNil(try executeOptional(source, ["type": "active_run", "conversation_id": conversation]))
        let stored = try executeOptional(source, ["type": "read_prepared_host_tool_plan",
            "reference": reference, "now_ms": Int64(Date().timeIntervalSince1970 * 1_000)])
        XCTAssertNotNil(stored)
        request.url = URL(string: "http://127.0.0.1:9/api/mobile/unknown")
        XCTAssertThrowsError(try fixture.response(for: request, token: token))
        request.url = URL(string: "http://127.0.0.1:9/api/mobile/chat/agent/v2/stream")
        XCTAssertThrowsError(try fixture.response(for: request, token: token))
    }

    func testPreparingAndAbandoningIntentDoesNotInventAnAssistantRun() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let target = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("receiver.db"))
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let conversation = UUID().uuidString
        try IOSCanonicalRemoteConversationCommands.prepare(store: target, descriptor: .init(
            sourceHostID: IOSCanonicalChatJournalFixture.sourceHostID, clientID: "fixture-client",
            conversationID: conversation, conversationTitle: "Pending", identity: identity,
            userText: "Original request", userContentJSON: nil, contextCheckpointID: nil, createdAtMs: 1))
        let intent = try execute(target, ["type":"read_remote_submission_intent", "conversation_id":conversation])
        XCTAssertEqual(intent["phase"] as? String, "prepared")
        XCTAssertTrue(intent["request_digest"] is NSNull)
        XCTAssertNil(try executeOptional(target, ["type":"active_run", "conversation_id":conversation]))
        XCTAssertTrue(try target.loadConversationProjectionWindowV1(id: conversation, beforeOrdinal: nil,
            limit: 10, contentByteLimit: 4096).projection.messages.isEmpty)
        let cancelled = try execute(target, ["type":"abandon_prepared_remote_submission", "identity":intent["identity"]!])
        XCTAssertEqual(cancelled["phase"] as? String, "cancelled")
        XCTAssertNil(try executeOptional(target, ["type":"canonical_run_terminal", "conversation_id":conversation,
            "run_id":identity.submissionID]))
        XCTAssertNil(try executeOptional(target, ["type":"read_remote_submission_intent", "conversation_id":conversation]))
    }

    func testSourceFixtureCommitsIncrementalJournalAndOnlyThenARealTerminalSeal() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ios-source-journal-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"),
            orderingMarker: "real-source-marker")
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let conversationID = UUID().uuidString
        let run = try fixture.begin(payload(conversationID: conversationID, identity: identity))
        let target = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("receiver.db"))
        try IOSCanonicalRemoteConversationCommands.prepare(store: target, descriptor: .init(
            sourceHostID: IOSCanonicalChatJournalFixture.sourceHostID,
            clientID: "fixture-client",
            conversationID: conversationID, conversationTitle: "Source journal fixture", identity: identity,
            userText: "Exercise the canonical Host transcript", userContentJSON: nil,
            contextCheckpointID: nil, createdAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        ))
        let sourceReceipt = try object(fixture.submissionReceipt(run))
        XCTAssertNil(try executeOptional(target, ["type":"active_run", "conversation_id":conversationID]))
        XCTAssertTrue(try target.loadConversationProjectionWindowV1(id: conversationID, beforeOrdinal: nil,
            limit: 10, contentByteLimit: 4096).projection.messages.isEmpty)
        _ = try execute(target, ["type":"bind_remote_submission_request", "identity":sourceReceipt["identity"]!,
            "request":payload(conversationID: conversationID, identity: identity)])
        XCTAssertNil(try executeOptional(target, ["type":"active_run", "conversation_id":conversationID]))
        _ = try execute(target, ["type":"accept_remote_submission_receipt", "receipt":sourceReceipt])
        var notifications: [IOSAssistantJournalCursor] = []
        var terminalHint: [String: Any]?
        for stage in IOSCanonicalChatJournalFixture.Stage.allCases {
            XCTAssertEqual(try fixture.journalResult([
                "operation": "terminal_seal", "conversation_id": conversationID, "run_id": identity.submissionID,
            ]), .null, "No terminal evidence exists before the actual final ModelCompleted transition")
            for event in try fixture.advance(run, stage: stage) {
                let fields = try object(event)
                if fields["type"] as? String == "assistant_journal_committed_v1" {
                    let notification = try JSONDecoder().decode(IOSAssistantJournalNotification.self,
                        from: JSONEncoder().encode(event))
                    XCTAssertEqual(notification.session_id, conversationID)
                    XCTAssertEqual(notification.source_host_id, IOSCanonicalChatJournalFixture.sourceHostID)
                    XCTAssertEqual(notification.through.identity.run_id, identity.submissionID)
                    notifications.append(notification.through)
                    try receive(notification.through, fixture: fixture, target: target)
                } else {
                    XCTAssertEqual(fields["type"] as? String, "assistant_run_terminal_committed_v1")
                    XCTAssertEqual(stage, .terminal)
                    terminalHint = fields
                }
                XCTAssertNil(fields["snapshot"])
                XCTAssertNil(fields["conversation_projection"])
            }
        }
        XCTAssertGreaterThan(notifications.count, 3)
        XCTAssertEqual(Set(notifications.map(\.identity.source_store_epoch)).count, 1)
        let sealValue = try fixture.journalResult([
            "operation": "terminal_seal", "conversation_id": conversationID, "run_id": identity.submissionID,
        ])
        let seal = try JSONDecoder().decode(IOSAssistantRunTerminalSeal.self, from: JSONEncoder().encode(sealValue))
        XCTAssertEqual(seal.core_session_id, run.coreSessionID)
        XCTAssertNotEqual(seal.core_session_id, conversationID)
        XCTAssertEqual(seal.identity.run_id, identity.submissionID)
        XCTAssertGreaterThan(UInt64(seal.checkpoint_revision) ?? 0, 0)
        XCTAssertGreaterThan(UInt64(seal.event_cursor) ?? 0, 0)
        XCTAssertGreaterThan(UInt64(seal.heads_count) ?? 0, 0)
        XCTAssertEqual(terminalHint?["seal_digest"] as? String, seal.seal_digest)
        XCTAssertEqual(terminalHint?["run_id"] as? String, identity.submissionID)
        let sourceHostID = IOSCanonicalChatJournalFixture.sourceHostID
        _ = try execute(target, ["type": "begin_receive_assistant_run_terminal", "source_host_id": sourceHostID,
            "seal": object(sealValue)])
        var after = "0"
        while true {
            let page = try fixture.journalResult([
                "operation": "terminal_manifest", "seal": object(sealValue), "after_entry": after,
                "max_records": 64, "max_encoded_bytes": 128 * 1024,
            ])
            _ = try execute(target, ["type": "stage_assistant_run_terminal_manifest", "source_host_id": sourceHostID,
                "page": object(page)])
            let fields = try object(page)
            if fields["complete"] as? Bool == true { break }
            after = try XCTUnwrap(fields["next_entry"] as? String)
        }
        let committed = try execute(target, ["type": "commit_received_assistant_run_terminal", "source_host_id": sourceHostID,
            "seal": object(sealValue)])
        XCTAssertEqual(committed["committed"] as? Bool, true)
        // The receiver must reconstruct content after reopening, not merely
        // acknowledge a terminal hint or retain the source fixture's objects.
        for application in [target, try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("receiver.db"))] {
            let window = try application.loadConversationProjectionWindowV1(id: conversationID,
                beforeOrdinal: nil, limit: 40, contentByteLimit: 32_768)
            XCTAssertEqual(window.projection.messages.filter { $0.role == .assistant }.count, 1)
            XCTAssertNil(window.projection.activeRun)
            let through = try XCTUnwrap(notifications.last)
            let at = try JSONSerialization.jsonObject(with: JSONEncoder().encode(through))
            let snapshot = try execute(application, ["type": "open_assistant_journal_snapshot", "at": at])
            defer { _ = try? executeOptional(application, ["type": "release_assistant_journal_snapshot", "snapshot": snapshot]) }
            let page = try execute(application, ["type": "read_assistant_journal_parts", "snapshot": snapshot,
                "after_part_id": NSNull(), "max_records": 64, "max_encoded_bytes": 256 * 1024])
            let parts = try XCTUnwrap(page["parts"] as? [[String: Any]])
            var markdown = ""
            for part in parts {
                for body in (part["bodies"] as? [[String: Any]] ?? []) where body["field"] as? String == "markdown" {
                    let range = try execute(application, ["type": "read_assistant_journal_range", "snapshot": snapshot,
                        "part_id": part["part_id"]!, "content": body, "offset_bytes": "0", "limit_bytes": 32_768])
                    XCTAssertEqual(range["end_of_content"] as? Bool, true)
                    markdown += try XCTUnwrap(range["text"] as? String)
                }
            }
            XCTAssertTrue(markdown.contains("real-source-marker"))
            XCTAssertTrue(markdown.contains("canonical Host stream"))
        }
        XCTAssertThrowsError(try fixture.begin(payload(conversationID: conversationID, identity: identity)),
            "Replaying a transport request must not create a second source Run")
    }

    func testJournalTransportEncryptsActualSourceResultsAndRejectsUnknownOperations() async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ios-source-transport-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"), orderingMarker: "transport")
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let conversationID = UUID().uuidString
        _ = try fixture.begin(payload(conversationID: conversationID, identity: identity))
        var request = URLRequest(url: URL(string: "http://127.0.0.1:9/mobile/chat/agent/v2/journal")!)
        request.httpBody = try JSONSerialization.data(withJSONObject: WebAgentProvider.encryptMobileJSON([
            "operation": "terminal_seal", "conversation_id": conversationID, "run_id": identity.submissionID,
        ], token: "fixture-token"))
        let response = try fixture.response(for: request, token: "fixture-token")
        var bytes = Data()
        for try await byte in response.bytes { bytes.append(byte) }
        let encrypted = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        XCTAssertEqual(encrypted["type"] as? String, "encrypted")
        let result = try WebAgentProvider.decryptMobileJSON(encrypted, token: "fixture-token")
        XCTAssertEqual(result["source_host_id"] as? String, IOSCanonicalChatJournalFixture.sourceHostID)
        XCTAssertTrue(result["result"] is NSNull)
        XCTAssertThrowsError(try fixture.journalResult(["operation": "fabricate_terminal_seal"]))
    }

    private func receive(_ through: IOSAssistantJournalCursor, fixture: IOSCanonicalChatJournalFixture,
                         target: IOSApplicationRuntime) throws {
        let sourceHostID = IOSCanonicalChatJournalFixture.sourceHostID
        let admission = try fixture.journalResult([
            "operation": "admission_head", "conversation_id": through.identity.conversation_id,
            "run_id": through.identity.run_id,
        ])
        var admitted = try execute(target, ["type": "run_message_admission_reception_state",
            "source_host_id": sourceHostID, "through": object(admission)])
        let requiredAdmission = try object(admission)
        while admitted["record_digest"] as? String != requiredAdmission["record_digest"] as? String {
            let page = try fixture.journalResult([
                "operation": "admissions", "after": admitted, "through": requiredAdmission,
                "max_records": 32, "max_encoded_bytes": 256 * 1024,
            ])
            admitted = try execute(target, ["type": "receive_run_message_admissions",
                "source_host_id": sourceHostID, "page": object(page)])
        }
        let throughValue = try JSONSerialization.jsonObject(with: JSONEncoder().encode(through))
        let state = try execute(target, ["type": "assistant_journal_reception_state", "source_host_id": sourceHostID,
            "through": throughValue])
        var after = try XCTUnwrap(state["after"] as? [String: Any])
        while true {
            let pageValue = try fixture.journalResult([
                "operation": "read", "after": after, "through": throughValue,
                "max_records": 32, "max_encoded_bytes": 256 * 1024,
            ])
            let page = try object(pageValue)
            XCTAssertEqual(page["status"] as? String, "records")
            for record in try XCTUnwrap(page["records"] as? [[String: Any]]) {
                while let need = try executeOptional(target, ["type": "next_assistant_journal_content",
                    "source_host_id": sourceHostID, "record": record]) {
                    let header = try XCTUnwrap(record["header"] as? [String: Any])
                    let at = ["identity": header["identity"]!, "sequence": header["sequence"]!, "record_digest": header["record_digest"]!]
                    let snapshot = try fixture.journalResult(["operation": "open_snapshot", "at": at])
                    let prefix = try XCTUnwrap(need["expected_prefix"] as? [String: Any])
                    let chunk = try fixture.journalResult([
                        "operation": "record_chunk", "snapshot": object(snapshot), "part_id": need["part_id"]!,
                        "content": need["content"]!, "chunk_index": prefix["chunk_count"]!,
                    ])
                    _ = try execute(target, ["type": "stage_assistant_journal_content", "input": [
                        "source_host_id": sourceHostID, "record": record, "part_id": need["part_id"]!,
                        "content": need["content"]!, "expected_prefix": prefix, "chunks": [object(chunk)], "restart": false,
                    ]])
                    _ = try fixture.journalResult(["operation": "release_snapshot", "snapshot": object(snapshot)])
                }
                _ = try execute(target, ["type": "receive_assistant_journal_record", "source_host_id": sourceHostID,
                    "record": record, "chunks": []])
            }
            if page["complete"] as? Bool == true { break }
            after = try XCTUnwrap(page["next"] as? [String: Any])
        }
    }

    private func payload(conversationID: String, identity: IOSCanonicalAgentTurnIdentity) -> [String: Any] {
        ["conversationId": conversationID, "submission_id": identity.submissionID,
         "client_id":"fixture-client", "expected_host_id":IOSCanonicalChatJournalFixture.sourceHostID,
         "user_message_id": identity.userMessageID, "assistant_message_id": identity.assistantMessageID,
         "messages": [["role": "user", "content": "Exercise the canonical Host transcript"]]]
    }

    private func object(_ value: AgentCoreJSONValue) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? [String: Any])
    }

    private func execute(_ target: IOSApplicationRuntime, _ command: [String: Any]) throws -> [String: Any] {
        try XCTUnwrap(executeOptional(target, command))
    }

    private func executeOptional(_ target: IOSApplicationRuntime, _ command: [String: Any]) throws -> [String: Any]? {
        let value = try target.executeAgentCommand(JSONDecoder().decode(AgentCoreJSONValue.self,
            from: JSONSerialization.data(withJSONObject: command)))
        if value == .null { return nil }
        return try object(value)
    }
}
#endif
