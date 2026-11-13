// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatThinkingParserTests logic in the project module.
// - Interfaces:
//   - ChatThinkingParserTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

final class ChatThinkingParserTests: XCTestCase {
    @MainActor func testMessagePartDisclosurePreservesExplicitChoiceAndStableOwner() {
        XCTAssertTrue(ChatExecutionProcessPolicy.segmentExpanded(choices: [], liveReasoning: true))
        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: [], liveReasoning: false))
        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: [false, true], liveReasoning: true))
        XCTAssertTrue(ChatExecutionProcessPolicy.segmentExpanded(choices: [true], liveReasoning: false))
        let key = IOSMessagePartReadingKey(source: "store-a", conversation: "c", message: "m", part: "r")
        let choices = [key: false]
        XCTAssertNil(choices[.init(source: "store-b", conversation: "c", message: "m", part: "r")])
        XCTAssertNil(choices[.init(source: "store-a", conversation: "other", message: "m", part: "r")])
        XCTAssertNil(choices[.init(source: "store-a", conversation: "c", message: "other", part: "r")])
        // Inserting another Part changes segment ID, but not the original Part's choice.
        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: ["new", "r"].compactMap {
            choices[.init(source: "store-a", conversation: "c", message: "m", part: $0)]
        }, liveReasoning: true))
    }

    @MainActor func testSingleProcessPreservesExecutionOrderAndExternalContentAnchors() throws {
        let first = IOSConversationProjectionV1.Part.reasoning(.init(partID: "r1", revision: 1,
            state: .completed, title: "Thinking", summary: "inspect"))
        let second = IOSConversationProjectionV1.Part.reasoning(.init(partID: "r2", revision: 1,
            state: .running, title: "Thinking", summary: "verify"))
        let prose = IOSConversationProjectionV1.Part.text(.init(partID: "text", revision: 1,
            markdown: "Preparing a form", isFinal: true, review: nil))
        let tool = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: Data(#"{"kind":"tool","execution_source":{"kind":"model"},"part_id":"tool","revision":2,"tool_call_id":"call","capability_id":"render_ui","state":"completed","title":"Render","origin":{"kind":"local"},"artifacts":[],"input":{},"affected_resources":[]}"#.utf8))
        let surface = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: Data(#"{"kind":"ui_surface","part_id":"surface","revision":3,"surface":{"identity":{"owner":{"authority_id":"store","conversation_id":"c","participant_id":"agent","context_id":"ctx","originating_run_id":"run","message_id":"message","part_id":"tool"},"surface_id":"form","generation":1},"revision":1},"summary":"Form"}"#.utf8))
        let final = IOSConversationProjectionV1.Part.text(.init(partID: "final", revision: 1,
            markdown: "Finished", isFinal: true, review: nil))
        let ordered = [first, prose, second, tool, surface, final]
        let flow = ChatExecutionProcessPolicy.segments(ordered)
        XCTAssertEqual(flow.map { $0.parts.map(\.partID) }, [["r1", "r2", "tool"], ["text"], ["surface"], ["final"]])
        XCTAssertEqual(flow.filter { $0.kind == .process }.count, 1)
        XCTAssertEqual(flow.filter { $0.kind == .content }.flatMap(\.parts), [prose, surface, final])
        XCTAssertEqual(flow.first?.id, "execution-process")
        XCTAssertEqual(ChatExecutionProcessPolicy.liveReasoning(ordered, live: true).map(\.partID), ["r2"])
        XCTAssertTrue(ChatExecutionProcessPolicy.liveReasoning(ordered, live: false).isEmpty)
    }

    @MainActor func testEmptyReasoningDoesNotCreateAnEmptyExecutionDisclosure() {
        let hidden = IOSConversationProjectionV1.Part.reasoning(.init(partID: "hidden", revision: 1,
            state: .running, title: "Thinking", summary: nil))
        let empty = IOSConversationProjectionV1.Part.reasoning(.init(partID: "empty", revision: 1,
            state: .running, title: "Thinking", summary: " \n "))
        let answer = IOSConversationProjectionV1.Part.text(.init(partID: "answer", revision: 1,
            markdown: "Answer", isFinal: true, review: nil))
        XCTAssertTrue(ChatExecutionProcessPolicy.segments([hidden, empty]).isEmpty)
        XCTAssertEqual(ChatExecutionProcessPolicy.segments([hidden, answer, empty]).flatMap(\.parts), [answer])
        XCTAssertTrue(ChatExecutionProcessPolicy.liveReasoning([hidden, empty], live: true).isEmpty)
    }

    @MainActor func testProcessIdentitySurvivesAdditionalRoundsAndTextAnchors() {
        let reasoning = IOSConversationProjectionV1.Part.reasoning(.init(partID: "r1", revision: 1,
            state: .completed, title: "Thinking", summary: "Plan"))
        let answer = IOSConversationProjectionV1.Part.text(.init(partID: "answer", revision: 1,
            markdown: "Interim result", isFinal: true, review: nil))
        let next = IOSConversationProjectionV1.Part.reasoning(.init(partID: "r2", revision: 1,
            state: .running, title: "Thinking", summary: "Verify"))
        let before = ChatExecutionProcessPolicy.segments([answer, reasoning])
        let after = ChatExecutionProcessPolicy.segments([answer, reasoning, next])
        XCTAssertEqual(before.first?.parts, [answer])
        XCTAssertEqual(before.first { $0.kind == .process }?.id, after.first { $0.kind == .process }?.id)
        XCTAssertEqual(after.filter { $0.kind == .process }.count, 1)
        XCTAssertEqual(after.first { $0.kind == .process }?.parts.map(\.partID), ["r1", "r2"])
        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: [false], liveReasoning: true))
    }

    @MainActor func testHistoricalExecutionJoinsOneGroupButSurfaceStaysOutside() throws {
        func historical(_ kind: String, id: String) throws -> IOSConversationProjectionV1.Part {
            let projection = try JSONDecoder().decode(IOSConversationProjectionV1.HistoricalProjection.self,
                from: JSONSerialization.data(withJSONObject: ["owner": [:], "canonical_revision": 1,
                    "canonical_kind": kind, "status": ["state": "unsupported", "reason": "Details unavailable"], "fields": [:]]))
            return .historical(id: id, revision: 1, projection: projection)
        }
        let reasoning = try historical("reasoning", id: "old-reasoning")
        let surface = try historical("ui_surface", id: "old-surface")
        let tool = try historical("tool", id: "old-tool")
        let flow = ChatExecutionProcessPolicy.segments([reasoning, surface, tool])
        XCTAssertEqual(flow.filter { $0.kind == .process }.count, 1)
        XCTAssertEqual(flow.first?.parts.map(\.partID), ["old-reasoning", "old-tool"])
        XCTAssertEqual(flow.filter { $0.kind == .content }.flatMap(\.parts), [surface])
        XCTAssertFalse(ChatExecutionProcessPolicy.isExecution(surface))
    }

    @MainActor func testLegacyToolEventsKeepFirstCallOrderAndLatestReceiptWithoutDuplicates() {
        let first = HostAgentToolEvent(id: "a", name: "first", status: .running)
        let second = HostAgentToolEvent(id: "b", name: "second", status: .failed)
        let completed = HostAgentToolEvent(id: "a", name: "first", status: .succeeded)
        let result = ChatExecutionProcessPolicy.legacyEvents([first, second, completed])
        XCTAssertEqual(result.map(\.id), ["a", "b"])
        XCTAssertEqual(result.map(\.status), [.succeeded, .failed])
        XCTAssertTrue(ChatExecutionProcessPolicy.legacyEvents([]).isEmpty)
    }

    @MainActor func testUnknownErrorKeyUsesNamedFailureAndTaskControlsDoNotRepeatSummary() {
        let localized: [String: String] = ["known.failure": "An actual translated error", "chat.step.failed": "This step failed"]
        XCTAssertEqual(ChatExecutionProcessPolicy.errorTitle(displayKey: "known.failure", localize: { localized[$0] }), "An actual translated error")
        XCTAssertEqual(ChatExecutionProcessPolicy.errorTitle(displayKey: "protocol.unknown_error", localize: { localized[$0] }), "This step failed")
        let summary = IOSConversationProjectionV1.Part.task(id: "summary", revision: 1, taskID: "exact-task",
            displayKey: "Task", state: .running, progress: nil, currentStep: nil, resultRef: nil, cancelRequested: false)
        XCTAssertFalse(ChatExecutionProcessPolicy.taskControlShowsSummary(taskID: "exact-task", parts: [summary]))
        XCTAssertTrue(ChatExecutionProcessPolicy.taskControlShowsSummary(taskID: "other-task", parts: [summary]))
        XCTAssertTrue(ChatExecutionProcessPolicy.taskControlShowsSummary(taskID: "exact-task", parts: []))
    }

    @MainActor func testTaskAndSkillStatusesAreHumanLabelsAndSealedIsNotCompleted() {
        let states: [IOSConversationProjectionV1.ActivityState] = [.pending, .running, .waitingForPermission,
            .waitingForInput, .completed, .sealed, .cancelled, .failed]
        for state in states {
            let title = ChatExecutionProcessPolicy.activityTitle(state)
            XCTAssertFalse(title.isEmpty)
            XCTAssertNotEqual(title, state.rawValue)
            XCTAssertFalse(title.hasPrefix("chat."))
        }
        XCTAssertNotEqual(ChatExecutionProcessPolicy.activityTitle(.sealed), ChatExecutionProcessPolicy.activityTitle(.completed))
        for state in [IOSConversationProjectionV1.ActivityState.completed, .sealed, .cancelled, .failed] {
            XCTAssertTrue(ChatExecutionProcessPolicy.isTerminal(state), "Terminal tasks must not render an old step or progress")
        }
        XCTAssertFalse(ChatExecutionProcessPolicy.isTerminal(.running))
        XCTAssertEqual(ChatExecutionProcessPolicy.displayTitle("protocol.unknown.task", fallbackKey: "chat.part.task"),
            NSLocalizedString("chat.part.task", comment: "Task"))
    }

    func testSplitsClosedThinkingBlockFromResponse() {
        let result = ChatThinkingParser.splitThinkingBlocks(from: "<think>分析问题</think>\n最终回答")

        XCTAssertEqual(result.thinking, "分析问题")
        XCTAssertEqual(result.response, "\n最终回答")
        XCTAssertFalse(result.hasOpenThinkingBlock)
    }

    func testKeepsStreamingThinkingOutOfVisibleResponseBeforeCloseTag() {
        let result = ChatThinkingParser.splitThinkingBlocks(from: "<think>正在推理第一步")

        XCTAssertEqual(result.thinking, "正在推理第一步")
        XCTAssertEqual(result.response, "")
        XCTAssertTrue(result.hasOpenThinkingBlock)
    }

    func testMergesMultipleThinkingBlocksAndKeepsResponseOrder() {
        let result = ChatThinkingParser.splitThinkingBlocks(from: "前缀<think>A</think>正文<think>B</think>结尾")

        XCTAssertEqual(result.thinking, "AB")
        XCTAssertEqual(result.response, "前缀正文结尾")
        XCTAssertFalse(result.hasOpenThinkingBlock)
    }
}
