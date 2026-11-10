// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

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
