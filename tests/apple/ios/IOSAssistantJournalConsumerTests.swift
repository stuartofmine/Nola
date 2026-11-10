// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

private nonisolated final class JournalCommandProbe: ConversationRemoteTurnStore, @unchecked Sendable {
    private(set) var commands = 0
    func prepareRemoteSubmission(_ descriptor: IOSRemoteConversationTurnDescriptor) throws {}
    func executeRemoteJournalCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        commands += 1
        throw IOSApplicationRuntimeError.rejected("Unexpected replica write")
    }
}

private nonisolated final class JournalRuntimeStore: ConversationRemoteTurnStore, @unchecked Sendable {
    let runtime: IOSApplicationRuntime
    init(_ runtime: IOSApplicationRuntime) { self.runtime = runtime }
    func prepareRemoteSubmission(_ descriptor: IOSRemoteConversationTurnDescriptor) throws {
        try IOSCanonicalRemoteConversationCommands.prepare(store: runtime, descriptor: descriptor)
    }
    func executeRemoteJournalCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        try runtime.executeAgentCommand(command)
    }
}

@MainActor final class IOSAssistantJournalConsumerTests: XCTestCase {
    func testContextUsageComesFromRealSourceAdmissionAndSurvivesReceiverReopen() async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime is unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let fixture = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"), orderingMarker: "context")
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let conversation = UUID().uuidString
        let run = try fixture.begin(["conversationId": conversation, "submission_id": identity.submissionID,
            "user_message_id": identity.userMessageID, "assistant_message_id": identity.assistantMessageID,
            "client_id": "fixture-client", "expected_host_id": IOSCanonicalChatJournalFixture.sourceHostID,
            "messages": [["role": "user", "content": "Measure source context"]]])
        for stage in IOSCanonicalChatJournalFixture.Stage.allCases { _ = try fixture.advance(run, stage: stage) }
        let receiverURL = directory.appendingPathComponent("receiver.db")
        let runtime = try IOSApplicationRuntime(databaseURL: receiverURL)
        let receipt = try fixture.submissionReceipt(run)
        _ = try runtime.executeAgentCommand(.object(["type": .string("adopt_observed_remote_run"), "receipt": receipt]))
        let config = WebAgentProvider.Config(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "fixture-token", expectedHostId: IOSCanonicalChatJournalFixture.sourceHostID, clientId: "fixture-client")
        let provider = WebAgentProvider(config: config) { try fixture.response(for: $0, token: config.token) }
        let consumer = try IOSAssistantJournalConsumer(provider: provider, store: JournalRuntimeStore(runtime),
            conversationID: conversation, runID: identity.submissionID, assistantID: identity.assistantMessageID)
        // No context notification was delivered. Terminal catch-up itself must
        // receive the usage fact before declaring the Run recovered.
        let terminalRecovered = try await consumer.recoverTerminal()
        XCTAssertTrue(terminalRecovered)
        let terminalUsage = try runtime.executeAgentCommand(.object([
            "type": .string("load_conversation_context_usage_v1"), "conversation_id": .string(conversation),
        ]))
        let terminalTotals = try JSONSerialization.jsonObject(with: JSONEncoder().encode(terminalUsage)) as! [String: Any]
        XCTAssertEqual((terminalTotals["used_tokens"] as? NSNumber)?.uint64Value, 1024)
        let received = try await consumer.recoverContextUsage()
        let accepted = try XCTUnwrap(received)
        XCTAssertEqual(accepted.usage.used_tokens, "1024")
        XCTAssertFalse(accepted.usage.estimated)
        let reopened = try IOSApplicationRuntime(databaseURL: receiverURL)
        let result = try reopened.executeAgentCommand(.object(["type": .string("load_conversation_context_usage_v1"),
            "conversation_id": .string(conversation)]))
        let usage = try JSONSerialization.jsonObject(with: JSONEncoder().encode(result)) as! [String: Any]
        XCTAssertEqual((usage["used_tokens"] as? NSNumber)?.uint64Value, 1024)
        XCTAssertEqual(usage["estimated"] as? Bool, false)
        // A well-formed DTO is not Core authority: changing its digest must fail.
        let forged = IOSCommittedRunContextUsage(protocol_version: accepted.protocol_version, identity: accepted.identity,
            admission: accepted.admission, event_sequence: accepted.event_sequence,
            attempt_sequence: accepted.attempt_sequence, run_cache_usage: accepted.run_cache_usage, usage: accepted.usage,
            record_digest: String(repeating: "0", count: 64))
        do { _ = try await consumer.recoverContextUsage(forged); XCTFail("Core must reject a forked record") }
        catch {}
        let foreignOwner = IOSAssistantJournalIdentity(source_store_epoch: accepted.identity.source_store_epoch,
            conversation_id: conversation, run_id: "another-run", assistant_message_id: identity.assistantMessageID)
        let foreign = IOSCommittedRunContextUsage(protocol_version: 1, identity: foreignOwner,
            admission: accepted.admission, event_sequence: accepted.event_sequence,
            attempt_sequence: accepted.attempt_sequence, run_cache_usage: accepted.run_cache_usage, usage: accepted.usage,
            record_digest: accepted.record_digest)
        do { _ = try await consumer.recoverContextUsage(foreign); XCTFail("Consumer must reject another Run before acceptance") }
        catch {}
    }

    func testContextRecordDecimalPrecisionAndOwnerValidation() throws {
        let owner = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant")
        let admission = IOSRunMessageAdmissionCursor(identity: owner, sequence: "1", record_digest: String(repeating: "a", count: 64))
        let value = IOSCommittedRunContextUsage(protocol_version: 1, identity: owner, admission: admission,
            event_sequence: "9007199254740993", attempt_sequence: "9007199254740994", run_cache_usage: nil,
            usage: .init(used_tokens: "9007199254740993",
                context_window_tokens: "18446744073709551615", prompt_token_limit: "18446744073709551614", estimated: false),
            record_digest: String(repeating: "b", count: 64))
        try value.validate(owner: owner)
        XCTAssertEqual(try JSONDecoder().decode(IOSCommittedRunContextUsage.self, from: JSONEncoder().encode(value)), value)
        let wrongOwner = IOSAssistantJournalIdentity(source_store_epoch: "other", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant")
        XCTAssertThrowsError(try value.validate(owner: wrongOwner))
        var raw = try JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as! [String: Any]
        raw["event_sequence"] = 1
        XCTAssertThrowsError(try JSONDecoder().decode(IOSCommittedRunContextUsage.self, from: JSONSerialization.data(withJSONObject: raw)))
        raw["event_sequence"] = "01"
        let invalid = try JSONDecoder().decode(IOSCommittedRunContextUsage.self, from: JSONSerialization.data(withJSONObject: raw))
        XCTAssertThrowsError(try invalid.validate(owner: owner))
    }
    private func provider(source: String = "paired") -> WebAgentProvider {
        let config = WebAgentProvider.Config(baseURL: URL(string: "http://127.0.0.1:8123")!, token: "fixture-token", expectedHostId: "paired")
        return WebAgentProvider(config: config) { request in
            XCTAssertTrue(request.url!.path.hasSuffix("/chat/agent/v2/journal"))
            let encrypted = try JSONSerialization.jsonObject(with: request.httpBody!) as! [String: Any]
            let payload = try WebAgentProvider.decryptMobileJSON(encrypted, token: config.token)
            XCTAssertEqual(payload["operation"] as? String, "terminal_seal")
            XCTAssertEqual(payload["conversation_id"] as? String, "conversation")
            XCTAssertEqual(payload["run_id"] as? String, "run")
            var wire = try WebAgentProvider.encryptMobileJSON(["source_host_id": source, "result": NSNull()], token: config.token)
            wire["type"] = "encrypted"
            let data = try JSONSerialization.data(withJSONObject: wire)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream { continuation in
                for byte in data { continuation.yield(byte) }
                continuation.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
        }
    }

    func testEOFWithoutSealDoesNotCommitTerminal() async throws {
        let store = JournalCommandProbe()
        let consumer = try IOSAssistantJournalConsumer(provider: provider(), store: store,
            conversationID: "conversation", runID: "run", assistantID: "assistant")
        let completed = try await consumer.recoverTerminal()
        XCTAssertFalse(completed)
        XCTAssertNil(consumer.terminalStatus)
        XCTAssertEqual(store.commands, 0)
    }

    func testEncryptedResponseFromAnotherSourceIsRejectedBeforeCoreWrite() async throws {
        let store = JournalCommandProbe()
        let consumer = try IOSAssistantJournalConsumer(provider: provider(source: "foreign"), store: store,
            conversationID: "conversation", runID: "run", assistantID: "assistant")
        do { _ = try await consumer.recoverTerminal(); XCTFail("Expected exact paired source gate") }
        catch { XCTAssertEqual(store.commands, 0) }
    }

    func testNotificationCannotForgeCompletionOrCrossRunOwnership() async throws {
        let store = JournalCommandProbe()
        let consumer = try IOSAssistantJournalConsumer(provider: provider(), store: store,
            conversationID: "conversation", runID: "run", assistantID: "assistant")
        let hint: AgentCoreJSONValue = .object([
            "session_id": .string("conversation"), "source_host_id": .string("paired"),
            "run_id": .string("foreign-run"), "seal_digest": .string(String(repeating: "a", count: 64)),
        ])
        do { try await consumer.receiveHint(hint, terminal: true); XCTFail("Expected exact Run gate") }
        catch { XCTAssertEqual(store.commands, 0); XCTAssertNil(consumer.terminalStatus) }
    }

    func testJournalRequestPreservesDecimalCursorAndBoundedReads() throws {
        let identity = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant")
        let cursor = IOSAssistantJournalCursor(identity: identity, sequence: "9007199254740993", record_digest: String(repeating: "a", count: 64))
        let data = try JSONEncoder().encode(IOSAssistantJournalReadRequest.records(after: cursor, through: cursor))
        let object = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        XCTAssertEqual((object["after"] as? [String: Any])?["sequence"] as? String, "9007199254740993")
        XCTAssertEqual(object["max_records"] as? Int, 32)
        XCTAssertEqual(object["max_encoded_bytes"] as? Int, 262144)
    }
}
