// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS assistant journal consumer/reader, canonical chat journal fixtures, message identity, run feedback, conversation projection contracts, historical part details, and web summary journal suite.

import Foundation
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

private actor JournalReadScript {
    struct Step: Sendable {
        let command: String
        let result: AgentCoreJSONValue
        var offset: String? = nil
        var base: AgentCoreJSONValue? = nil
        var afterPartID: AgentCoreJSONValue? = nil
    }
    private var steps: [Step]
    private var calls: [String] = []
    init(_ steps: [Step]) { self.steps = steps }
    func execute(_ request: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        guard case .object(let fields) = request, case .string(let type)? = fields["type"],
              !steps.isEmpty else { throw IOSApplicationRuntimeError.rejected("Unexpected read") }
        let step = steps.removeFirst()
        if type == "read_assistant_journal_range", fields["limit_bytes"] != .unsignedInteger(32_768) {
            throw IOSApplicationRuntimeError.rejected("Range exceeds the Store raw-byte contract")
        }
        guard type == step.command,
              step.offset == nil || fields["offset_bytes"] == .string(step.offset!),
              step.base == nil || fields["base"] == step.base,
              step.afterPartID == nil || fields["after_part_id"] == step.afterPartID else {
            throw IOSApplicationRuntimeError.rejected("Wrong read command or frontier: \(type)")
        }
        calls.append(type)
        return step.result
    }
    func remaining() -> Int { steps.count }
    func recorded() -> [String] { calls }
}

@MainActor final class IOSAssistantJournalReaderTests: XCTestCase {
    private let identity = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant")
    private func cursor(_ sequence: Int) -> IOSAssistantJournalCursor {
        .init(identity: identity, sequence: String(sequence), record_digest: String(repeating: "a", count: 64))
    }
    private func wire<T: Encodable>(_ value: T) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value))
    }
    private func head(_ sequence: Int, text: String, generation: Int = 1, visibility: String = "public", owner: IOSAssistantJournalIdentity? = nil,
                      partID: String = "text", position: UInt64 = 0) throws -> AgentCoreJSONValue {
        .object([
            "identity": try wire(owner ?? identity), "part_id": .string(partID),
            "generation": .string(String(generation)), "revision": .string(String(sequence)), "metadata_revision": .string(String(sequence)),
            "order_key": .object(["section": .string("transcript"), "position": .string(String(position))]),
            "metadata": .object(["kind": .string("text"), "data": .object([
                "is_final": .bool(false), "visibility": .string(visibility), "review": visibility == "public" ? .null : .string("pending")])]),
            "bodies": .array([.object(["field": .string("markdown"), "content_generation": .string(String(generation)),
                "utf8_bytes": .string(String(text.utf8.count)), "chunk_count": .string(text.isEmpty ? "0" : "1"), "content_root": .string(String(repeating: "b", count: 64))])])
        ])
    }
    private func open(_ sequence: Int) throws -> JournalReadScript.Step {
        .init(command: "open_assistant_journal_snapshot", result: .object(["snapshot_id": .string("pin-\(sequence)"), "at": try wire(cursor(sequence))]))
    }
    private var release: JournalReadScript.Step { .init(command: "release_assistant_journal_snapshot", result: .null) }
    private func parts(_ head: AgentCoreJSONValue) throws -> JournalReadScript.Step {
        try partsPage([head])
    }
    private func partsPage(_ heads: [AgentCoreJSONValue], next: String? = nil,
                           after: AgentCoreJSONValue = .null, sequence: Int = 1) throws -> JournalReadScript.Step {
        .init(command: "read_assistant_journal_parts", result: .object([
            "at": try wire(cursor(sequence)), "parts": .array(heads), "next_part_id": next.map(AgentCoreJSONValue.string) ?? .null
        ]), afterPartID: after)
    }
    private func changes(_ sequence: Int, head: AgentCoreJSONValue) throws -> JournalReadScript.Step {
        let base = try wire(cursor(sequence - 1))
        return .init(command: "read_assistant_journal_part_changes", result: .object([
            "base": base, "at": try wire(cursor(sequence)), "changes": .array([.object(["part_id": .string("text"), "head": head])]), "next_part_id": .null
        ]), base: base)
    }
    private func range(_ suffix: String, offset: Int, total: Int) -> JournalReadScript.Step {
        .init(command: "read_assistant_journal_range", result: .object([
            "offset_bytes": .string(String(offset)), "next_offset_bytes": .string(String(offset + suffix.utf8.count)),
            "total_utf8_bytes": .string(String(total)), "text": .string(suffix), "end_of_content": .bool(offset + suffix.utf8.count == total)
        ]), offset: String(offset))
    }

    func testTaskControlPreservesSourceGuidanceCapability() async throws {
        for supported in [false, true] {
            guard case .object(var taskHead) = try head(1, text: "", partID: "task") else {
                return XCTFail("Invalid fixture")
            }
            taskHead["bodies"] = .array([])
            taskHead["metadata"] = .object(["kind": .string("task_control"), "data": .object([
                "supports_guidance": .bool(supported), "supports_cancellation": .bool(true),
                "task": .object(["task_id": .string("child"), "display_key": .string("Child"),
                    "state": .string("running"), "progress": .null, "cancel_requested": .bool(false)])])])
            let script = JournalReadScript(try [open(1), parts(.object(taskHead)), release])
            let reader = IOSAssistantJournalReader { try await script.execute($0) }
            let result = try await reader.read(at: cursor(1))
            guard case .taskControl(let control)? = result.parts.first else {
                return XCTFail("Source capability must not become a plain task")
            }
            XCTAssertEqual(control.supportsGuidance, supported)
            XCTAssertEqual(control.parentRunID, identity.run_id, "Task control retains journal owner, never currently selected Run")
            XCTAssertTrue(control.supportsCancellation)
            XCTAssertEqual(control.taskID, "child")
        }
    }

    func testColdPagesFollowDisplayOrderAcrossOpaqueRetiredContinuations() async throws {
        let script = JournalReadScript(try [
            open(1),
            partsPage([head(1, text: "first", partID: "z-first", position: 0)], next: "retired-9"),
            range("first", offset: 0, total: 5),
            partsPage([], next: "retired-1", after: .string("retired-9")),
            partsPage([head(1, text: "second", partID: "a-second", position: 1)], after: .string("retired-1")),
            range("second", offset: 0, total: 6), release,
        ])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        let result = try await reader.read(at: cursor(1))
        XCTAssertEqual(result.text, "firstsecond")
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }

    func testColdDirectoryRejectsCycleAndReleasesSnapshot() async throws {
        let script = JournalReadScript(try [open(1),
            partsPage([], next: "retired-a"),
            partsPage([], next: "retired-b", after: .string("retired-a")),
            partsPage([], next: "retired-a", after: .string("retired-b")), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        do { _ = try await reader.read(at: cursor(1)); XCTFail("Directory cursor cycle must fail") } catch {}
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }

    func testColdDirectoryRejectsRegressingDisplayPosition() async throws {
        let script = JournalReadScript(try [open(1),
            partsPage([head(1, text: "", partID: "a", position: 2), head(1, text: "", partID: "z", position: 1)]), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        do { _ = try await reader.read(at: cursor(1)); XCTFail("Display order must remain monotonic") } catch {}
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }

    func testColdPageMustMatchPinnedCursorEvenWhenEmpty() async throws {
        let script = JournalReadScript(try [open(1), partsPage([], sequence: 2), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        do { _ = try await reader.read(at: cursor(1)); XCTFail("Foreign page cursor must fail") } catch {}
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }

    func testCompleteUnicodeBodySpansBoundedPagesWithoutSplittingScalars() async throws {
        let first = String(repeating: "a", count: 32_766)
        let second = "🙂" + String(repeating: "中", count: 10_921)
        let last = "文🙂"
        let full = first + second + last
        let script = JournalReadScript(try [open(1), parts(head(1, text: full)),
            range(first, offset: 0, total: full.utf8.count),
            range(second, offset: first.utf8.count, total: full.utf8.count),
            range(last, offset: first.utf8.count + second.utf8.count, total: full.utf8.count), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        let content = try await reader.read(at: cursor(1))
        XCTAssertEqual(content.text, full)
        let remaining = await script.remaining()
        XCTAssertEqual(remaining, 0)
    }

    func testUnicodeSuffixReplacementAndRetirementUseFixedSnapshotAndExactByteOffsets() async throws {
        let script = JournalReadScript(try [
            open(1), parts(head(1, text: "你")), range("你", offset: 0, total: 3), release,
            open(2), changes(2, head: head(2, text: "你好🙂")), range("好🙂", offset: 3, total: 10), release,
            open(3), changes(3, head: head(3, text: "新", generation: 2)), range("新", offset: 0, total: 3), release,
            open(4), changes(4, head: .null), release
        ])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        let first = try await reader.read(at: cursor(1)); XCTAssertEqual(first.text, "你")
        let second = try await reader.read(at: cursor(2)); XCTAssertEqual(second.text, "你好🙂")
        let repeated = try await reader.read(at: cursor(2)); XCTAssertEqual(repeated, second)
        let stale = try await reader.read(at: cursor(1)); XCTAssertEqual(stale, second)
        let replacement = try await reader.read(at: cursor(3)); XCTAssertEqual(replacement.text, "新")
        let retired = try await reader.read(at: cursor(4)); XCTAssertTrue(retired.parts.isEmpty)
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }

    func testWithheldTextDoesNotIssueAnyBodyRead() async throws {
        let script = JournalReadScript(try [open(1), parts(head(1, text: "private draft", visibility: "withheld")), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        let result = try await reader.read(at: cursor(1))
        XCTAssertEqual(result.text, "")
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
        let commands = await script.recorded(); XCTAssertFalse(commands.contains("read_assistant_journal_range"))
    }

    func testApprovalReadsPreviouslyWithheldBodyFromItsOwnSnapshot() async throws {
        let script = JournalReadScript(try [
            open(1), parts(head(1, text: "Approved", visibility: "withheld")), release,
            open(2), changes(2, head: head(2, text: "Approved")), range("Approved", offset: 0, total: 8), release
        ])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        let withheld = try await reader.read(at: cursor(1)); XCTAssertTrue(withheld.text.isEmpty)
        let approved = try await reader.read(at: cursor(2)); XCTAssertEqual(approved.text, "Approved")
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }

    func testForeignPartOwnerIsRejectedAndPinnedSnapshotReleased() async throws {
        let foreign = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "other", assistant_message_id: "assistant")
        let script = JournalReadScript(try [open(1), parts(head(1, text: "wrong owner", owner: foreign)), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        do { _ = try await reader.read(at: cursor(1)); XCTFail("Foreign owner must not render") } catch {}
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
        let commands = await script.recorded(); XCTAssertEqual(commands.last, "release_assistant_journal_snapshot")
    }

    func testModelAttemptUsesFlatJournalFailureFieldsAndLosslessRevision() async throws {
        guard case .object(var value) = try head(1, text: "") else { return XCTFail("head") }
        value["revision"] = .string("9007199254740993")
        value["bodies"] = .array([])
        value["metadata"] = .object(["kind": .string("model_attempt"), "data": .object([
            "attempt_id": .string("attempt"), "state": .string("failed"),
            "provider": .string("provider"), "model": .string("model"),
            "failure_code": .string("rate_limited"), "retryable": .bool(true),
            "http_status": .string("429"), "fallback_attempt_id": .null,
            "fallback_provider": .null, "fallback_model": .null
        ])])
        let script = JournalReadScript(try [open(1), parts(.object(value)), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        let result = try await reader.read(at: cursor(1))
        guard case .modelAttempt(let attempt)? = result.parts.first else { return XCTFail("Missing typed attempt") }
        XCTAssertEqual(attempt.failure.code, "rate_limited")
        XCTAssertTrue(attempt.failure.retryable)
        XCTAssertEqual(attempt.revision, 9_007_199_254_740_993)
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }

    func testStepRemainsReadOnlyJournalDetailNotToolOrAssistantText() async throws {
        guard case .object(var value) = try head(1, text: "") else { return XCTFail("head") }
        value["bodies"] = .array([])
        value["metadata"] = .object(["kind": .string("step"), "data": .object([
            "title": .object(["kind": .string("inline"), "value": .string("Inspecting files")]), "state": .string("running")
        ])])
        let script = JournalReadScript(try [open(1), parts(.object(value)), release])
        let reader = IOSAssistantJournalReader { try await script.execute($0) }
        let result = try await reader.read(at: cursor(1))
        guard case .journalDetail(let id, _, let kind, let title, _)? = result.parts.first else { return XCTFail("Missing typed detail") }
        XCTAssertEqual(id, "text"); XCTAssertEqual(kind, "step"); XCTAssertEqual(title, "Inspecting files")
        XCTAssertTrue(result.text.isEmpty)
        let remaining = await script.remaining(); XCTAssertEqual(remaining, 0)
    }
}

#if DEBUG
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

@MainActor
final class IOSConversationProjectionContractTests: XCTestCase {
    func testUserMessageRevisionSurvivesProjectionForExactEditAdmission() throws {
        let json = #"{"message_id":"stable-user-id","role":"user","state":"completed","revision":17,"created_at_ms":1,"parts":[{"kind":"text","part_id":"body","revision":17,"markdown":"Original message","is_final":true}]}"#
        let source = try JSONDecoder().decode(IOSConversationProjectionV1.Message.self, from: Data(json.utf8))
        let presented = try XCTUnwrap(IOSConversationProjectionPresenter.message(from: source, activeRun: nil))
        XCTAssertEqual(presented.metadata["nola.canonicalMessageID"], "stable-user-id")
        XCTAssertEqual(presented.canonicalMessageRevision, 17)
        XCTAssertEqual(presented.content, "Original message")
    }

    func testAllPublicActivityStatesDecodeWithoutCollapsingWaitingOrSealed() throws {
        for state in ["pending", "running", "waiting_for_permission", "waiting_for_input", "completed", "sealed", "cancelled", "failed"] {
            let decoded = try JSONDecoder().decode(IOSConversationProjectionV1.ActivityState.self, from: Data("\"\(state)\"".utf8))
            XCTAssertEqual(decoded.rawValue, state)
        }
        XCTAssertThrowsError(try JSONDecoder().decode(IOSConversationProjectionV1.ActivityState.self, from: Data(#""foreign_state""#.utf8)))
    }

    func testCommittedCacheRatePreservesUnknownAndMeasuredZero() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":0,"write_tokens":1000,"hit_rate_bps":0,"is_compaction":true}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        XCTAssertEqual(usage.cacheUsage?.hit_rate_bps, 0)
        XCTAssertTrue(usage.cacheUsage?.is_compaction == true)
        let unknown = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: unknown).cacheUsage)
    }

    func testContextEvidenceUsesConversationTotalsNotLatestRequestRate() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":0,"write_tokens":null,"hit_rate_bps":0,"is_compaction":true},"conversation_cache_usage":{"input_tokens":10000,"read_tokens":7500,"write_tokens":null,"hit_rate_bps":7500,"is_compaction":false,"reported_requests":5,"unreported_requests":2}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        let evidence = try XCTUnwrap(IOSContextUsageEvidence.committed(usage))
        XCTAssertEqual(evidence.cacheHitRateBps, 7500)
        XCTAssertEqual(evidence.cachedInputTokens, 7500)
        XCTAssertEqual(evidence.cacheReportedRequests, 5)
        XCTAssertEqual(evidence.cacheUnreportedRequests, 2)
        XCTAssertEqual(evidence.usedTokens, 1000)
    }

    func testMissingConversationCacheTotalsDoNotFallBackToLatestRequest() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":1000,"write_tokens":null,"hit_rate_bps":10000,"is_compaction":false}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        let evidence = try XCTUnwrap(IOSContextUsageEvidence.committed(usage))
        XCTAssertNil(evidence.cacheHitRateBps)
        XCTAssertNil(evidence.cachedInputTokens)
    }

    func testNewRunWithoutOccupancyStillPresentsDurableConversationCacheTotals() throws {
        let data = Data(#"{"used_tokens":0,"context_window_tokens":0,"prompt_token_limit":0,"estimated":true,"measured_at_revision":0,"conversation_cache_usage":{"input_tokens":10000,"read_tokens":7500,"write_tokens":null,"hit_rate_bps":7500,"is_compaction":false,"reported_requests":5,"unreported_requests":2}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        let evidence = try XCTUnwrap(IOSContextUsageEvidence.committed(usage))
        XCTAssertFalse(evidence.hasCommittedTokenTotals)
        XCTAssertEqual(evidence.cacheHitRateBps, 7500)
        let presentation = ConversationContextUsage.withoutBreakdown().replacingTokenTotals(
            usedTokens: evidence.usedTokens, limitTokens: evidence.limitTokens,
            cacheHitRateBps: evidence.cacheHitRateBps, cacheReportedRequests: evidence.cacheReportedRequests,
            cacheUnreportedRequests: evidence.cacheUnreportedRequests, hasCommittedTokenTotals: evidence.hasCommittedTokenTotals)
        XCTAssertFalse(presentation.hasCommittedTokenTotals)
        XCTAssertFalse(presentation.hasEstimatedBreakdown)
        XCTAssertEqual(presentation.cacheReportedRequests, 5)
        XCTAssertEqual(presentation.cacheUnreportedRequests, 2)
        XCTAssertEqual(presentation.cacheHitRateBps, 7500)
    }

    func testStructuredNumberDisplayPreservesOrdinaryIntegerAndFractionFormatting() throws {
        for (json, expected) in [
            ("0", "0"), ("-0.0", "0"), ("42", "42"), ("-42", "-42"),
            ("42.0", "42"), ("0.125", "0.125"), ("-1.5", "-1.5"),
            ("9007199254740992", "9007199254740992"),
            ("-9223372036854775808", "-9223372036854775808"),
        ] {
            let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(json.utf8))
            XCTAssertEqual(value.displayText, expected, json)
        }
    }

    func testStructuredNumberDisplayDoesNotTrapOutsideInt64Range() throws {
        for json in ["-9223372036854777856", "1e100", "-1e100", "1e308", "-1e308"] {
            let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(json.utf8))
            let represented = try XCTUnwrap(Double(json))
            XCTAssertNil(Int64(exactly: represented), "Fixture must exercise the non-Int64 display path")
            XCTAssertEqual(value.displayText, String(represented), json)
        }
        let upperInside = Double(Int64.max).nextDown
        XCTAssertEqual(IOSConversationProjectionV1.StructuredValue.number(upperInside).displayText, "9223372036854774784")
        let lowerOutside = Double(Int64.min).nextDown
        XCTAssertEqual(IOSConversationProjectionV1.StructuredValue.number(lowerOutside).displayText, String(lowerOutside))
    }

    func testStructuredIntegerDecoderAndCoreBridgeRemainExactBeyondDoublePrecision() throws {
        for json in ["9007199254740991", "9007199254740992", "9007199254740993",
                     "-9007199254740993", "9223372036854775807", "9223372036854775808",
                     "18446744073709551615", "-9223372036854775808"] {
            let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(json.utf8))
            XCTAssertEqual(value.displayText, json)
            // StructuredValue is the Core value, not a lossy second bridge.
            let coreValue: AgentCoreJSONValue = value
            let encoded = try JSONEncoder().encode(coreValue)
            XCTAssertEqual(String(decoding: encoded, as: UTF8.self), json)
            XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: encoded), value)
        }
    }

    func testNestedHistoricalStructuredNumbersRenderWithoutNarrowing() throws {
        let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(#"{"count":18446744073709551615,"values":[1e308,-9223372036854775808,0.125],"label":"evidence"}"#.utf8))
        XCTAssertEqual(value.displayText, "{count: 18446744073709551615, label: evidence, values: [1e+308, -9223372036854775808, 0.125]}")
    }

    func testUnloadedHistoricalPartAndCanonicalPreviewRemainDescriptive() throws {
        let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("core/tests/fixtures/historical_unloaded_conversation_projection_v1.json")
        let whole = try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(contentsOf: fixture))
        guard case .historical(_, _, let projection) = whole.messages[0].parts[0],
              case .historical(_, _, let opaque) = whole.messages[0].parts[4] else { return XCTFail("Missing historical nodes") }
        XCTAssertEqual(projection.status.state, .notLoaded)
        XCTAssertNil(projection.source)
        XCTAssertNil(projection.binding)
        XCTAssertTrue(projection.fields.isEmpty)
        XCTAssertNil(projection.decodedText)
        XCTAssertNil(projection.timelineText)
        XCTAssertTrue(projection.needsHistoricalDetail)
        XCTAssertTrue(opaque.needsHistoricalDetail)
        XCTAssertEqual(opaque.timelineText, "opaque original evidence")
        XCTAssertTrue(projection.statusLabel.contains("selection_budget"))
        XCTAssertTrue(opaque.statusLabel.contains("opaque original evidence"))
    }

    func testCompleteCanonicalTextDoesNotRequireOpeningMetadataAsOriginal() throws {
        // Real user turns store a displayText metadata envelope rather than an
        // agent_part/message_part_view schema. Its canonical body is still text.
        let text = "完整原文🙂"
        let projection = try canonicalPreview(text: text, totalBytes: text.utf8.count)
        XCTAssertNil(projection.decodedText)
        XCTAssertEqual(projection.timelineText, text)
        XCTAssertFalse(projection.needsHistoricalDetail)
    }

    func testPartialOrInconsistentCanonicalPreviewStillOffersOriginal() throws {
        let text = "原文🙂"
        XCTAssertTrue(try canonicalPreview(text: text, totalBytes: text.utf8.count + 1).needsHistoricalDetail)
        XCTAssertTrue(try canonicalPreview(text: text, totalBytes: text.count).needsHistoricalDetail,
                      "Character count must never be mistaken for a complete UTF-8 body")
        XCTAssertTrue(try canonicalPreview(text: text, totalBytes: text.utf8.count, kind: "opaque_future_kind").needsHistoricalDetail)
    }

    private func canonicalPreview(text: String, totalBytes: Int, kind: String = "text") throws -> IOSConversationProjectionV1.HistoricalProjection {
        let object: [String: Any] = [
            "owner": ["conversation_id": "conversation", "message_id": "message", "part_id": "part"],
            "canonical_revision": 1, "canonical_kind": kind,
            "status": ["state": "unsupported", "code": "schema_missingkind"],
            "fields": [String: String](),
            "canonical_text": ["text": text, "total_utf8_bytes": totalBytes],
        ]
        return try JSONDecoder().decode(IOSConversationProjectionV1.HistoricalProjection.self,
                                        from: JSONSerialization.data(withJSONObject: object))
    }

    func testHistoricalProjectionRemainsDistinctAndNeverCreatesToolEvents() throws {
        let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("core/tests/fixtures/historical_conversation_projection_v1.json")
        let projection = try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(contentsOf: fixture))
        XCTAssertEqual(projection.messages[0].parts.count, 5)
        guard case .historical(_, _, let historical) = projection.messages[0].parts[0] else {
            return XCTFail("Historical text must retain its distinct Part type")
        }
        XCTAssertEqual(historical.decodedText, "  原文\\n🙂  ")
        XCTAssertFalse(historical.needsHistoricalDetail)
        guard case .historical(_, _, let deferred) = projection.messages[0].parts[3],
              case .deferred = deferred.fields["markdown"] else {
            return XCTFail("Deferred fields must retain the original locator")
        }
        XCTAssertNil(deferred.decodedText)
        XCTAssertTrue(deferred.needsHistoricalDetail)
        let message = try XCTUnwrap(IOSConversationProjectionPresenter.messages(from: projection).first)
        XCTAssertFalse(message.content.contains("supported"))
        XCTAssertFalse(message.content.contains("deferred"))
        XCTAssertFalse(message.content.contains("unknown_kind"))
        XCTAssertFalse(message.content.contains("must not run"))
        XCTAssertTrue(message.hostToolEvents.isEmpty)
        XCTAssertEqual(message.thinking, " reasoning 原文 ")
    }

    func testCoreJSONRoundTripPreservesCatalogRevisionBeyondDoublePrecision() throws {
        let expected: UInt64 = 675_868_731_199_239_600
        let decoded = try JSONDecoder().decode(
            AgentCoreJSONValue.self,
            from: Data(#"{"capability_catalog_revision":675868731199239600}"#.utf8)
        )
        guard case .object(let object) = decoded else {
            return XCTFail("An unsafe JSON integer must use the lossless UInt64 representation")
        }
        switch object["capability_catalog_revision"] {
        case .signedInteger(let revision):
            XCTAssertEqual(UInt64(revision), expected)
        case .unsignedInteger(let revision):
            XCTAssertEqual(revision, expected)
        default:
            XCTFail("An unsafe JSON integer must not be represented as Double")
        }

        let encoded = try JSONEncoder().encode(decoded)
        let roundTripped = try JSONDecoder().decode(
            [String: UInt64].self,
            from: encoded
        )
        XCTAssertEqual(roundTripped["capability_catalog_revision"], expected)
        XCTAssertTrue(String(decoding: encoded, as: UTF8.self).contains(String(expected)))
    }

    func testJournalNotificationsAreHintsWithoutLegacyProjectionAdapters() throws {
        let identity: [String: Any] = ["source_store_epoch": "epoch", "conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant"]
        let event = WebAgentProvider.parseSSEObject(["type": "assistant_journal_committed_v1",
            "session_id": "conversation", "source_host_id": "paired-host",
            "through": ["identity": identity, "sequence": "9007199254740993", "record_digest": String(repeating: "a", count: 64)]])
        guard case .assistantJournalCommitted(let raw) = event else { return XCTFail("Missing journal hint") }
        let hint = try JSONDecoder().decode(IOSAssistantJournalNotification.self, from: JSONEncoder().encode(raw))
        XCTAssertEqual(hint.through.sequence, "9007199254740993")
        let terminal = WebAgentProvider.parseSSEObject(["type": "assistant_run_terminal_committed_v1",
            "session_id": "conversation", "source_host_id": "paired-host", "run_id": "run",
            "seal_digest": String(repeating: "b", count: 64)])
        guard case .assistantRunTerminalSealed = terminal else { return XCTFail("Missing terminal hint") }
    }

    func testProjectionIsTheSingleInputForRestoredTextReasoningToolAndContext() throws {
        let json = #"""
        {
          "contract_version": 1, "execution_ownership": {"kind": "user"},
          "conversation_id": "conversation-1",
          "revision": 9,
          "title": "Projection contract",
          "messages": [{
            "message_id": "run-1:assistant",
            "role": "assistant",
            "state": "running",
            "revision": 8,
            "created_at_ms": 1787356800000,
            "parts": [
              {"kind":"reasoning","part_id":"reasoning-1","revision":1,"state":"completed","title":"Inspect source","summary":"Inspecting the source"},
              {"kind":"tool","execution_source":{"kind":"model"},"part_id":"tool-1","revision":2,"tool_call_id":"call-1","capability_id":"file.read","state":"completed","title":"Read source","origin":{"kind":"local"},"artifacts":[],"display_key":"tool.file.read","input":{"path":"/tmp/source.md"},"output":{"bytes":42},"affected_resources":[{"resource_type":"file","uri":"file:///tmp/source.md","operation":"read"}]},
              {"kind":"text","part_id":"text-1","revision":5,"markdown":"# Result\n\nBody","is_final":false}
            ]
          }],
          "active_run":{"run_id":"run-1","state":"running","active_part_id":"text-1","display_key":"agent.composing","progress":null},
          "context_usage":{"used_tokens":2048,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":9}
        }
        """#

        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertEqual(messages.count, 1)
        XCTAssertEqual(messages[0].content, "# Result\n\nBody")
        XCTAssertEqual(messages[0].thinking, "Inspecting the source")
        XCTAssertEqual(messages[0].hostToolEvents.first?.name, "file.read")
        XCTAssertEqual(messages[0].tracePhase, "agent.composing")
        XCTAssertTrue(messages[0].isCanonicalLive)
        XCTAssertEqual(messages[0].liveHostToolEvents.map(\.id), [])
        XCTAssertEqual(messages[0].canonicalParts.count, 3)
        XCTAssertEqual(projection.contextUsage.usedTokens, 2048)
        XCTAssertEqual(projection.contextUsage.promptTokenLimit, 57344)
    }

    func testModelAttemptFailureKeepsFailedAndFallbackIdentity() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"fallback","revision":3,"title":"Fallback",
          "messages":[{"message_id":"assistant","role":"assistant","state":"completed","revision":3,"created_at_ms":1,"parts":[
            {"kind":"model_attempt","part_id":"model-attempt:one","revision":3,"state":"failed","attempt_id":"one","provider":"OpenRouter","model":"free-model","failure":{"code":"rate_limited","message":"The provider rate limit was reached.","retryable":true},"fallback_attempt_id":"two","fallback_provider":"DeepSeek","fallback_model":"deepseek-chat"},
            {"kind":"text","part_id":"answer","revision":3,"markdown":"Recovered answer","is_final":true}
          ]}],
          "active_run":null,
          "surface":{"revision":3,"committed_message_ids":["assistant"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":["model-attempt:one"],"show_assistant_placeholder":false},
          "context_usage":{"used_tokens":0,"context_window_tokens":16384,"prompt_token_limit":12288,"estimated":false,"measured_at_revision":3}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        guard case let .modelAttempt(attempt) = projection.messages[0].parts[0] else {
            return XCTFail("Expected model attempt Part")
        }

        XCTAssertEqual(attempt.provider, "OpenRouter")
        XCTAssertEqual(attempt.model, "free-model")
        XCTAssertEqual(attempt.failure.code, "rate_limited")
        XCTAssertEqual(attempt.fallbackProvider, "DeepSeek")
        XCTAssertEqual(attempt.fallbackModel, "deepseek-chat")
    }

    func testSurfaceOrdersCommittedHistoryBeforeLivePlaceholder() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"surface","revision":3,"title":"Surface",
          "messages":[
            {"message_id":"live","role":"assistant","state":"running","revision":3,"created_at_ms":2,"parts":[]},
            {"message_id":"history","role":"assistant","state":"completed","revision":2,"created_at_ms":1,"parts":[{"kind":"text","part_id":"text","revision":2,"markdown":"Done","is_final":true}]}
          ],
          "active_run":{"run_id":"run","state":"running","active_part_id":null,"display_key":null,"progress":null},
          "surface":{"revision":3,"committed_message_ids":["history"],"live_message_id":"live","current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":true},
          "context_usage":{"used_tokens":1,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":true,"measured_at_revision":3}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertEqual(messages.map { $0.metadata["nola.canonicalMessageID"] }, ["history", "live"])
        XCTAssertFalse(messages[0].isCanonicalLive)
        XCTAssertTrue(messages[1].isCanonicalLive)
        XCTAssertEqual(messages[1].metadata[ChatConversationSurfaceMeta.placeholderKey], "true")
    }

    func testCompactionSourceIdentityDecodesAndLocatesCanonicalMessage() throws {
        let fingerprint = String(repeating: "a", count: 64)
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"compaction-source","revision":4,"title":"Compaction",
          "messages":[
            {"message_id":"source-message","role":"user","state":"completed","revision":1,"created_at_ms":1,"parts":[{"kind":"text","part_id":"source-part","revision":1,"markdown":"Canonical original","is_final":true}]},
            {"message_id":"summary-message","role":"assistant","state":"completed","revision":4,"created_at_ms":2,"parts":[{"kind":"compaction","part_id":"compaction-part","revision":4,"summary":"Bounded summary","retained_item_ids":["source-part"],"source_fingerprint":"\#(fingerprint)","original_token_count":120,"compacted_token_count":30}]}
          ],
          "active_run":null,
          "surface":{"revision":4,"committed_message_ids":["source-message","summary-message"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":false},
          "context_usage":{"used_tokens":30,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":4}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        guard case let .compaction(_, _, _, sourceIDs, decodedFingerprint, _, _) =
            try XCTUnwrap(messages.last?.canonicalParts.first) else {
            return XCTFail("Expected a compaction Part")
        }
        XCTAssertEqual(sourceIDs, ["source-part"])
        XCTAssertEqual(decodedFingerprint, fingerprint)
        XCTAssertEqual(
            ChatCompactionSourceNavigator.targetMessageID(
                sourceItemIDs: sourceIDs,
                messages: messages
            ),
            messages.first?.id
        )
        let conversationID = UUID(uuidString: "11111111-2222-3333-4444-555555555555")!
        let request = IOSCompactionSourceRequest(
            conversationID: conversationID,
            compactionPartID: "compaction-part",
            sourceItemIDs: sourceIDs,
            sourceFingerprint: fingerprint
        )
        XCTAssertTrue(ChatCompactionSourceNavigator.accepts(
            request,
            conversationID: conversationID,
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            request,
            conversationID: UUID(uuidString: "22222222-3333-4444-5555-666666666666"),
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            IOSCompactionSourceRequest(
                conversationID: conversationID,
                compactionPartID: "compaction-part",
                sourceItemIDs: sourceIDs,
                sourceFingerprint: String(repeating: "b", count: 64)
            ),
            conversationID: conversationID,
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            IOSCompactionSourceRequest(
                conversationID: conversationID,
                compactionPartID: "different-compaction-part",
                sourceItemIDs: sourceIDs,
                sourceFingerprint: fingerprint
            ),
            conversationID: conversationID,
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            IOSCompactionSourceRequest(
                conversationID: conversationID,
                compactionPartID: "compaction-part",
                sourceItemIDs: ["different-source-part"],
                sourceFingerprint: fingerprint
            ),
            conversationID: conversationID,
            messages: messages
        ))
    }

    func testCompactionSourceWindowPrependsCanonicalPageAndStopsAtHistoryEnd() throws {
        let json = #"""
        {
          "projection":{
            "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"paged-source","revision":5,"title":"Paged",
            "messages":[
              {"message_id":"older-source","role":"user","state":"completed","revision":1,"created_at_ms":1,"parts":[{"kind":"text","part_id":"canonical-source-part","revision":1,"markdown":"Canonical original","is_final":true}]},
              {"message_id":"older-neighbor","role":"assistant","state":"completed","revision":2,"created_at_ms":2,"parts":[{"kind":"text","part_id":"neighbor-part","revision":1,"markdown":"Neighbor","is_final":true}]}
            ],
            "active_run":null,
            "surface":{"revision":5,"committed_message_ids":["older-source","older-neighbor"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":false},
            "context_usage":{"used_tokens":20,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":5}
          },
          "event_sequence":9,"total_message_count":4,"before_ordinal":null,"has_more":false,"assistant_journal_heads":[]
        }
        """#
        let window = try JSONDecoder().decode(
            IOSConversationProjectionWindowV1.self,
            from: Data(json.utf8)
        )
        let older = IOSConversationProjectionPresenter.messages(from: window.projection)
        XCTAssertEqual(window.eventSequence, 9)
        XCTAssertTrue(ChatCompactionSourceNavigator.belongsToSnapshot(
            window,
            revision: 5,
            eventSequence: 9
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.belongsToSnapshot(
            window,
            revision: 6,
            eventSequence: 9
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.belongsToSnapshot(
            window,
            revision: 5,
            eventSequence: 10
        ))
        let alreadyLoaded = [
            ChatMessage(
                role: .assistant,
                content: "Summary",
                metadata: ["nola.canonicalMessageID": "summary-message"]
            )
        ]
        let merged = ChatCompactionSourceNavigator.mergingOlder(older, into: alreadyLoaded)

        XCTAssertEqual(
            merged.compactMap { $0.metadata["nola.canonicalMessageID"] },
            ["older-source", "older-neighbor", "summary-message"]
        )
        XCTAssertEqual(
            ChatCompactionSourceNavigator.targetMessageID(
                sourceItemIDs: ["canonical-source-part"],
                messages: merged
            ),
            older.first?.id
        )
        XCTAssertFalse(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: window.beforeOrdinal,
                hasMore: window.hasMore
            )
        )
        XCTAssertFalse(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: 3,
                hasMore: true
            ),
            "A stale cursor must terminate instead of repeatedly loading the same page"
        )
        XCTAssertFalse(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: 4,
                hasMore: true
            ),
            "A cursor moving toward newer history must not drive another older-page request"
        )
        XCTAssertTrue(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: 2,
                hasMore: true
            )
        )
    }

    func testCompactionSourcePagesCommitOldestFirstAndPreserveConcurrentSuffix() {
        func message(_ canonicalID: String) -> ChatMessage {
            ChatMessage(
                role: .assistant,
                content: canonicalID,
                metadata: ["nola.canonicalMessageID": canonicalID]
            )
        }

        let merged = ChatCompactionSourceNavigator.mergingFetchedOlderPages(
            [
                [message("nearer-page")],
                [message("oldest-page")],
            ],
            into: [message("initial-window"), message("concurrent-streaming-suffix")]
        )

        XCTAssertEqual(
            merged.compactMap { $0.metadata["nola.canonicalMessageID"] },
            ["oldest-page", "nearer-page", "initial-window", "concurrent-streaming-suffix"]
        )
    }

    func testUnknownPartKindFailsInsteadOfFallingBackToText() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"c","revision":1,"title":"",
          "messages":[{"message_id":"m","role":"assistant","state":"running","revision":1,"created_at_ms":1,"parts":[{"kind":"provider_delta","part_id":"p","revision":1}]}],
          "active_run":null,
          "context_usage":{"used_tokens":0,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":true,"measured_at_revision":1}
        }
        """#
        XCTAssertThrowsError(
            try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(json.utf8))
        )
    }

    func testColdProjectionRestorePreservesExactAppendOrderAndPendingAssistant() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"cold","revision":7,"title":"Cold restore",
          "messages":[
            {"message_id":"user","role":"user","state":"completed","revision":1,"created_at_ms":1,"parts":[{"kind":"text","part_id":"u","revision":1,"markdown":"Question","is_final":true}]},
            {"message_id":"assistant","role":"assistant","state":"running","revision":6,"created_at_ms":2,"parts":[
              {"kind":"text","part_id":"a1","revision":1,"markdown":"## Result\n\n","is_final":false},
              {"kind":"text","part_id":"a2","revision":2,"markdown":"First paragraph.","is_final":false},
              {"kind":"text","part_id":"a3","revision":3,"markdown":"\n\n```swift\nlet value = 42","is_final":false}
            ]}
          ],
          "active_run":{"run_id":"run","state":"running","active_part_id":"a3","display_key":"agent.composing","progress":null},
          "context_usage":{"used_tokens":42,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":7}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertEqual(messages.map(\.role), [.user, .assistant])
        XCTAssertEqual(
            messages[1].content,
            "## Result\n\nFirst paragraph.\n\n```swift\nlet value = 42"
        )
        XCTAssertEqual(messages[1].tracePhase, "agent.composing")
        XCTAssertFalse(messages[1].thinkingFinished)
    }

    func testCompletedGoalProjectionCommitsOneAnswerAndReleasesComposer() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"goal-completed","revision":12,"title":"Completed goal",
          "messages":[{
            "message_id":"goal-turn:assistant","role":"assistant","state":"completed","revision":11,"created_at_ms":1,
            "parts":[
              {"kind":"tool","execution_source":{"kind":"model"},"part_id":"goal-tool","revision":1,"tool_call_id":"goal-call","capability_id":"goal_complete","state":"completed","title":"Complete goal","origin":{"kind":"local"},"artifacts":[],"display_key":"tool.goal_complete","input":{"status":"completed"},"output":{"goal_status":"completed"},"affected_resources":[]},
              {"kind":"text","part_id":"goal-answer","revision":2,"markdown":"Goal completed exactly once.","is_final":true}
            ]
          }],
          "active_run":null,
          "context_usage":{"used_tokens":128,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":12}
        }
        """#

        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertNil(projection.activeRun)
        XCTAssertEqual(messages.count, 1)
        XCTAssertEqual(messages[0].content, "Goal completed exactly once.")
        XCTAssertEqual(messages[0].hostToolEvents.map(\.name), ["goal_complete"])
        XCTAssertFalse(ChatProcessingState.completed.blocksInput)
    }
}

@MainActor
final class IOSHistoricalPartDetailTests: XCTestCase {
    func testHistoricalActionUsesContentNamesInsteadOfProtocolKindOrFileExtension() {
        XCTAssertEqual(IOSHistoricalDetailPresentation.detailTitle(for: "text"), NSLocalizedString("history.detail.open_text", comment: ""))
        XCTAssertEqual(IOSHistoricalDetailPresentation.detailTitle(for: "reasoning"), NSLocalizedString("history.detail.open_reasoning", comment: ""))
        XCTAssertEqual(IOSHistoricalDetailPresentation.detailTitle(for: "future_kind.txt"), NSLocalizedString("history.detail.open_record", comment: ""))
        XCTAssertFalse(IOSHistoricalDetailPresentation.detailTitle(for: "future_kind.txt").contains("future_kind"))
    }

    private func projection() throws -> IOSConversationProjectionV1.HistoricalProjection {
        let file = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("core/tests/fixtures/historical_unloaded_conversation_projection_v1.json")
        let window = try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(contentsOf: file))
        guard case .historical(_, _, let value) = window.messages[0].parts[0] else { throw CocoaError(.coderReadCorrupt) }
        return value
    }

    func testUnavailableCanRetryAndNotLoadedRemainsExplicit() async throws {
        let value = try projection()
        let request = try XCTUnwrap(IOSHistoricalPartRequest(value))
        let state = IOSHistoricalPartDetailState()
        await state.load(request) { _ in nil }
        guard case .failed = state.phase else { return XCTFail("Missing unavailable state") }
        await state.load(request) { _ in value }
        XCTAssertEqual(state.phase, .loaded(value))
        XCTAssertEqual(value.status.state, .notLoaded)
    }

    func testDismissedLateResponseCannotPublish() async throws {
        let value = try projection()
        let request = try XCTUnwrap(IOSHistoricalPartRequest(value))
        let state = IOSHistoricalPartDetailState()
        let gate = HistoricalReadGate()
        let task = Task { await state.load(request) { _ in await gate.wait() } }
        await gate.waitUntilStarted()
        state.invalidate()
        await gate.finish(value)
        await task.value
        XCTAssertEqual(state.phase, .loading)
    }

    func testReloadFailureKeepsPreviousDetails() async throws {
        let value = try projection()
        let request = try XCTUnwrap(IOSHistoricalPartRequest(value))
        let state = IOSHistoricalPartDetailState()
        await state.load(request) { _ in value }
        await state.load(request) { _ in nil }
        XCTAssertEqual(state.previous, value)
        guard case .failed = state.phase else { return XCTFail("Expected recoverable error") }
    }

    func testStructuredContentIsValidJSONAndPreservesIntegerPrecision() throws {
        let value: AgentCoreJSONValue = .object(["count": .unsignedInteger(UInt64.max), "text": .string("quote\"\n中")])
        let output = IOSHistoricalDetailPresentation.formatted(value)
        XCTAssertTrue(output.contains("18446744073709551615"))
        XCTAssertTrue(output.contains("\n"))
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(output.utf8)), value)
    }

    private func page(_ request: IOSHistoricalPartRequest, offset: UInt64 = 0,
                      text: String = "中", next: UInt64 = 3, total: UInt64 = 6,
                      revision: Int64 = 1, end: Bool = false) -> IOSHistoricalOriginalPage {
        IOSHistoricalOriginalPage(conversationID: request.owner.conversationID,
            messageID: request.owner.messageID, partID: request.owner.partID, field: "content_json",
            revision: revision, offsetBytes: offset, nextOffsetBytes: next,
            totalUTF8Bytes: total, text: text, endOfContent: end)
    }

    func testOriginalRangeRejectsStaleRevisionMalformedOffsetsAndOversizedPage() throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let query = IOSHistoricalOriginalQuery(request, offset: 0)
        XCTAssertTrue(page(request).matches(query))
        XCTAssertFalse(page(request, revision: 2).matches(query))
        XCTAssertFalse(page(request, next: 1).matches(query)) // UTF-8 bytes, not characters.
        XCTAssertFalse(page(request, total: 2).matches(query))
        XCTAssertFalse(page(request, end: true).matches(query))
        XCTAssertFalse(page(request, text: "", next: 0).matches(query))
        XCTAssertFalse(page(request, text: String(repeating: "a", count: 8193), next: 8193, total: 8193, end: true).matches(query))
        XCTAssertFalse(page(request, offset: UInt64.max, next: 2).matches(IOSHistoricalOriginalQuery(request, offset: UInt64.max)))
    }

    func testOriginalPagingFailurePreservesVisiblePageAndCanRetry() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let first = page(request)
        let second = page(request, offset: 3, next: 6, end: true)
        let state = IOSHistoricalOriginalState()
        await state.load(request, offset: 0) { _ in first }
        await state.load(request, offset: 3) { _ in throw CocoaError(.fileReadUnknown) }
        XCTAssertEqual(state.page, first)
        XCTAssertNotNil(state.error)
        XCTAssertTrue(state.previousOffsets.isEmpty)
        await state.load(request, offset: 3) { _ in second }
        XCTAssertEqual(state.page, second)
        XCTAssertEqual(state.previousOffsets, [0])
        XCTAssertNil(state.error)
        await state.load(request, offset: 0) { _ in first }
        XCTAssertEqual(state.page, first)
        XCTAssertTrue(state.previousOffsets.isEmpty)
    }

    func testOriginalReadRejectsChangedRevisionWithoutReplacingVisiblePage() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let first = page(request)
        let stale = page(request, offset: 3, next: 6, revision: 2, end: true)
        let state = IOSHistoricalOriginalState()
        await state.load(request, offset: 0) { _ in first }
        await state.load(request, offset: 3) { _ in stale }
        XCTAssertEqual(state.page, first)
        XCTAssertNotNil(state.error)
        XCTAssertTrue(state.previousOffsets.isEmpty)
    }

    func testOriginalQueryUsesRevisionPinnedBoundedPublicContract() throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let data = try JSONEncoder().encode(IOSHistoricalOriginalQuery(request, offset: 3))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(object["type"] as? String, "read_conversation_export_range")
        XCTAssertEqual(object["field"] as? String, "content_json")
        XCTAssertEqual(object["expected_revision"] as? Int, 1)
        XCTAssertEqual(object["limit_bytes"] as? Int, 8192)
        XCTAssertEqual(object["offset_bytes"] as? Int, 3)
        XCTAssertNil(object["path"])
    }

    func testOriginalCancelledReadCannotPublish() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let value = page(request)
        let gate = OriginalReadGate()
        let state = IOSHistoricalOriginalState()
        let task = Task { await state.load(request, offset: 0) { _ in await gate.wait() } }
        await gate.waitUntilStarted()
        task.cancel()
        await gate.finish(value)
        await task.value
        XCTAssertNil(state.page)
        XCTAssertNil(state.error)
        XCTAssertFalse(state.loading)
    }

    func testOriginalLateDifferentRequestCannotReplaceNewResult() async throws {
        let request = try XCTUnwrap(IOSHistoricalPartRequest(projection()))
        let differentProjection = try JSONDecoder().decode(IOSConversationProjectionV1.HistoricalProjection.self,
            from: Data(#"{"owner":{"conversation_id":"other","message_id":"m","part_id":"p"},"canonical_revision":1,"canonical_kind":"text","status":{"state":"not_loaded"},"fields":{}}"#.utf8))
        let different = try XCTUnwrap(IOSHistoricalPartRequest(differentProjection))
        let first = page(request)
        let current = page(different)
        let gate = OriginalReadGate()
        let state = IOSHistoricalOriginalState()
        let task = Task { await state.load(request, offset: 0) { _ in await gate.wait() } }
        await gate.waitUntilStarted()
        await state.load(different, offset: 0) { _ in current }
        await gate.finish(first)
        await task.value
        XCTAssertEqual(state.page, current)
        XCTAssertNil(state.error)
    }
}

private actor OriginalReadGate {
    private var continuation: CheckedContinuation<IOSHistoricalOriginalPage, Never>?
    func wait() async -> IOSHistoricalOriginalPage {
        await withCheckedContinuation { continuation = $0 }
    }
    func waitUntilStarted() async {
        while continuation == nil { await Task.yield() }
    }
    func finish(_ value: IOSHistoricalOriginalPage) {
        continuation?.resume(returning: value)
        continuation = nil
    }
}

private actor HistoricalReadGate {
    private var continuation: CheckedContinuation<IOSConversationProjectionV1.HistoricalProjection?, Never>?
    func wait() async -> IOSConversationProjectionV1.HistoricalProjection? {
        await withCheckedContinuation { continuation = $0 }
    }
    func waitUntilStarted() async {
        while continuation == nil { await Task.yield() }
    }
    func finish(_ value: IOSConversationProjectionV1.HistoricalProjection) {
        continuation?.resume(returning: value)
        continuation = nil
    }
}

#if DEBUG
final class IOSWebSummaryJournalTests: XCTestCase {
    @MainActor
    func testSealedSummaryIncludesEverySegmentInCanonicalMessageOrder() throws {
        let root = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "root")
        let second = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "segment")
        let firstCursor = IOSAssistantJournalCursor(identity: root, sequence: "3", record_digest: String(repeating: "a", count: 64))
        let secondCursor = IOSAssistantJournalCursor(identity: second, sequence: "2", record_digest: String(repeating: "b", count: 64))
        XCTAssertEqual(try WebAgentProvider.summaryAssistantHeads([(3, secondCursor), (1, firstCursor)],
            root: root, expectedCount: "2"), [firstCursor, secondCursor])
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(1, firstCursor), (3, secondCursor)],
            root: root, expectedCount: "1"))
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(1, firstCursor), (3, firstCursor)],
            root: root, expectedCount: "2"))
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(3, secondCursor)], root: root, expectedCount: "1"))
        let foreign = IOSAssistantJournalCursor(identity: .init(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "foreign", assistant_message_id: "segment"), sequence: "2", record_digest: secondCursor.record_digest)
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(1, firstCursor), (3, foreign)],
            root: root, expectedCount: "2"))
    }
    func testSummaryEmitsOnlyOneSealedPublicSourceResult() async throws {
        try await exercise(.sealed)
    }
    func testMissingAdmissionFrameRecoversOriginalReceiptWithoutResubmission() async throws {
        try await exercise(.lostReceipt)
    }
    func testNonterminalEOFNeverReturnsPartialSuccess() async throws {
        try await exercise(.unsealed)
    }
    func testForeignReceiptNeverBecomesSummaryContent() async throws {
        try await exercise(.foreignReceipt)
    }
    private func exercise(_ mode: SummarySource.Mode) async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = try SummarySource(directory: directory, mode: mode)
        let provider = await WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "fixture", expectedHostId: IOSCanonicalChatJournalFixture.sourceHostID, clientId: "summary-client")) {
            try await source.response($0)
        }
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let input = ModelInput(dateRange: .init(start: "2026-01-01", end: "2026-01-02"), candidates: [],
            userPrefs: ["message":"Exercise the canonical Host transcript", "enable_thinking":"true",
                "host_conversation_id":UUID().uuidString, "submission_id":identity.submissionID,
                "user_message_id":identity.userMessageID, "assistant_message_id":identity.assistantMessageID])
        var chunks: [String] = []
        do {
            let stream = await provider.generateSummaryStream(input: input)
            for try await chunk in stream { chunks.append(chunk) }
            XCTAssertTrue(mode == .sealed || mode == .lostReceipt)
            XCTAssertEqual(chunks.count, 1)
            XCTAssertTrue(chunks.first?.contains("canonical Host stream") == true)
            XCTAssertTrue(chunks.first?.contains("summary-source-marker") == true)
            XCTAssertFalse(chunks.first?.contains("<think>") == true)
        } catch {
            XCTAssertTrue(mode == .unsealed || mode == .foreignReceipt, "Unexpected error: \(error)")
            XCTAssertTrue(chunks.isEmpty, "No partial text is a completed String summary")
        }
        let counts = await source.counts()
        XCTAssertEqual(counts.streams, 1, "Recovery must never resubmit")
        if mode == .sealed || mode == .lostReceipt { XCTAssertGreaterThan(counts.ranges, 0) }
        if mode == .foreignReceipt { XCTAssertEqual(counts.ranges, 0) }
    }
}
private actor SummarySource {
    enum Mode: Sendable { case sealed, lostReceipt, unsealed, foreignReceipt }
    private let source: IOSCanonicalChatJournalFixture
    private let intentStore: IOSApplicationRuntime
    private let mode: Mode
    private var run: IOSCanonicalChatJournalFixture.Run?
    private var streams = 0
    private var ranges = 0
    init(directory: URL, mode: Mode) throws {
        source = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"),
            orderingMarker: "summary-source-marker")
        intentStore = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("hash-owner.db"))
        self.mode = mode
    }
    func counts() -> (streams: Int, ranges: Int) { (streams, ranges) }
    private func value(_ object: Any) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: object, options: [.fragmentsAllowed]))
    }
    private func object(_ value: AgentCoreJSONValue) throws -> Any {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value), options: [.fragmentsAllowed])
    }
    func response(_ request: URLRequest) throws -> WebAgentTransportResponse {
        let path = request.url!.path
        if path.hasSuffix("/submission-frontier") || path.hasSuffix("/agent/settings")
            || path.hasSuffix("/agent/tool-plan/prepare") {
            return try source.response(for: request, token: "fixture")
        }
        let envelope = try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as! [String: Any]
        let payload = try WebAgentProvider.decryptMobileJSON(envelope, token: "fixture")
        if request.url!.path.hasSuffix("/submission"), payload["operation"] as? String == "prepare" {
            let identity = payload["identity"] as! [String: Any]
            let semantic = payload["request"] as! [String: Any]
            var input = identity
            input["conversation_title"] = "Summary hash"
            input["user_part_id"] = "\(identity["user_message_id"]!):text"
            input["assistant_part_id"] = "\(identity["assistant_message_id"]!):text"
            input["user_text"] = "Exercise the canonical Host transcript"
            input["created_at_ms"] = 1
            _ = try intentStore.executeAgentCommand(value(["type":"prepare_remote_submission", "input":input]))
            let bound = try intentStore.executeAgentCommand(value(["type":"bind_remote_submission_request",
                "identity":identity, "request":semantic]))
            let fields = try object(bound) as! [String: Any]
            return try encrypted(request, result: ["identity":identity, "request_digest":fields["request_digest"]!])
        }
        if request.url!.path.hasSuffix("/stream") {
            streams += 1
            let active = try source.begin(payload); run = active
            var receipt = try object(source.submissionReceipt(active)) as! [String: Any]
            if mode == .foreignReceipt {
                var identity = receipt["identity"] as! [String: Any]
                identity["submission_id"] = "foreign-run"; receipt["identity"] = identity
            }
            var frames: [AgentCoreJSONValue] = mode == .lostReceipt ? [] : [try value(["type":"submission_receipt", "receipt":receipt])]
            for stage in IOSCanonicalChatJournalFixture.Stage.allCases {
                if mode == .unsealed && stage == .terminal { break }
                frames += try source.advance(active, stage: stage)
            }
            var bytes = Data()
            for frame in frames {
                bytes.append(Data("data: ".utf8)); bytes.append(try JSONEncoder().encode(frame)); bytes.append(Data("\n\n".utf8))
            }
            return transport(request, data: bytes, contentType: "text/event-stream")
        }
        if request.url!.path.hasSuffix("/submission") {
            return try encrypted(request, result: object(source.submissionReceipt(XCTUnwrap(run))))
        }
        XCTAssertTrue(request.url!.path.hasSuffix("/journal"), "Unexpected summary request: \(path)")
        if payload["operation"] as? String == "range" { ranges += 1 }
        return try encrypted(request, result: object(source.journalResult(payload)))
    }
    private func encrypted(_ request: URLRequest, result: Any) throws -> WebAgentTransportResponse {
        var envelope = try WebAgentProvider.encryptMobileJSON(["source_host_id":IOSCanonicalChatJournalFixture.sourceHostID,
            "result":result], token: "fixture")
        envelope["type"] = "encrypted"
        return transport(request, data: try JSONSerialization.data(withJSONObject: envelope), contentType: "application/json")
    }
    private func transport(_ request: URLRequest, data: Data, contentType: String) -> WebAgentTransportResponse {
        .init(bytes: AsyncThrowingStream { stream in data.forEach { stream.yield($0) }; stream.finish() },
            response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil,
                headerFields: ["Content-Type":contentType])!)
    }
}
#endif

private nonisolated struct CanonicalFeedbackTestStore: IOSAgentApplicationCommandStore {
    let response: AgentCoreJSONValue
    func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        XCTAssertEqual(command.objectFields?["type"], .string("canonical_run_terminal"))
        XCTAssertEqual(command.objectFields?["conversation_id"], .string("parent"))
        XCTAssertEqual(command.objectFields?["run_id"], .string("run"))
        return response
    }
}

final class IOSCanonicalMessageIdentityAndRunFeedbackTests: XCTestCase {
    func testCanonicalTurnIdentityDerivationValidationAndBlockedRunFeedback() throws {
        let recovered = try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: "ui-action:opaque", assistantMessageID: "admitted-assistant")
        XCTAssertEqual(recovered.submissionID, "ui-action:opaque")
        XCTAssertEqual(recovered.assistantMessageID, "admitted-assistant")
        XCTAssertEqual(recovered.userMessageID, "")
        XCTAssertEqual(recovered.visibleAssistantMessageID, IOSCanonicalMessageIdentity.stableUUID(for: "admitted-assistant"))
        for invalid in ["", " run", "run\n", String(repeating: "a", count: 513)] {
            XCTAssertThrowsError(try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: invalid, assistantMessageID: "assistant"))
        }

        let userID = UUID(uuidString: "11111111-2222-3333-4444-555555555555")!
        let derived = IOSCanonicalAgentTurnIdentity(userMessageID: userID)
        XCTAssertEqual(derived.submissionID, userID.uuidString.lowercased())
        XCTAssertEqual(derived.assistantMessageID, "\(userID.uuidString.lowercased()):assistant")
        XCTAssertEqual(derived.visibleAssistantMessageID, IOSCanonicalMessageIdentity.stableUUID(for: derived.assistantMessageID))
        XCTAssertThrowsError(try IOSCanonicalAgentTurnIdentity(userMessageID: UUID(), validatingVisibleAssistantMessageID: UUID()))

        let cursor = IOSAssistantJournalCursor(identity: .init(source_store_epoch: "store", conversation_id: "parent",
            run_id: "run", assistant_message_id: "assistant"), sequence: "2", record_digest: String(repeating: "a", count: 64))
        let result: [String: Any] = ["result_id": "result", "run_id": "run", "conversation_id": "parent",
            "originating_submission_id": "submission", "input": ["watermark": "0", "applications_digest": "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"],
            "decided_at_revision": "1", "decision_event_cursor": "2", "requirement_revision": "0", "evidence_frontier": "0",
            "value": ["kind": "blocked", "reason": ["code": "context.input_budget_exceeded", "detail": NSNull()],
                "resolution_condition": "opaque budget", "blocking_fact_refs": ["budget"]]]
        for assistant in ["assistant", "other-assistant"] {
            let response = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: [
                "conversation_id": "parent", "run_id": "run", "assistant_message_id": assistant, "status": "sealed", "result": result]))
            let store = CanonicalFeedbackTestStore(response: response)
            if assistant == "assistant" { XCTAssertNotNil(try IOSCanonicalRunFeedback.read(cursor: cursor, store: store)) }
            else { XCTAssertThrowsError(try IOSCanonicalRunFeedback.read(cursor: cursor, store: store)) }
        }
        XCTAssertNil(try IOSCanonicalRunFeedback.read(cursor: cursor, store: CanonicalFeedbackTestStore(response: .null)))
    }
}
