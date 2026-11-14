// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Tests incremental Markdown buffering and stable iOS chat bubble state.
// - Interfaces:
//   - ChatStreamingRevealTokenizerTests
// - APIs:
//   - none
//
import Combine
import SwiftUI
import UIKit
import Vision
import XCTest
@testable import Nola

final class ChatStreamingRevealTokenizerTests: XCTestCase {
    @MainActor
    func testAsyncMarkdownInitializationAndSupersededSnapshotsRenderLatestText() async throws {
        let initial = "INITIAL VISIBLE ANSWER"
        let controller = UIHostingController(rootView: ChatSmoothStreamingMarkdownView(
            content: initial, isStreaming: true))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 300))
        window.rootViewController = controller
        window.makeKeyAndVisible()
        controller.view.frame = window.bounds
        defer { window.isHidden = true }
        func renderedText() throws -> String {
            controller.view.setNeedsLayout()
            controller.view.layoutIfNeeded()
            let image = UIGraphicsImageRenderer(bounds: controller.view.bounds).image { _ in
                controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
            }
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            request.recognitionLanguages = ["en-US"]
            try VNImageRequestHandler(cgImage: XCTUnwrap(image.cgImage)).perform([request])
            return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        }
        // Text is accepted immediately; its ink follows the shared reveal
        // reveal rather than being forced fully opaque on the first frame.
        await settleRuntimeUI(AppleStreamingRevealTiming.duration + 0.05)
        XCTAssertTrue(try renderedText().contains(initial))
        controller.rootView = ChatSmoothStreamingMarkdownView(content: "OBSOLETE INTERMEDIATE", isStreaming: true)
        controller.view.layoutIfNeeded()
        let final = "Clouds drift gently across the open sky. The wind carries them in every direction."
        controller.rootView = ChatSmoothStreamingMarkdownView(content: final, isStreaming: false)
        controller.view.layoutIfNeeded()
        // Hosting updates, the Markdown scheduler and ink rendering finish on
        // separate frames. Wait for the rendered result rather than assuming a
        // fixed wall-clock delay also covers them on a busy simulator.
        let deadline = Date().addingTimeInterval(5)
        var text = ""
        repeat {
            await settleRuntimeUI(0.05)
            text = try renderedText()
        } while text != final && Date() < deadline
        XCTAssertEqual(text, final, "OCR must retain sentence order across native line wrapping")
        XCTAssertFalse(text.contains("OBSOLETE"), text)
        XCTAssertFalse(text.contains("INITIAL"), text)
    }

    @MainActor
    func testMarkdownSchedulerCoalescesDeduplicatesAndCancelsPendingFrames() async throws {
        let scheduler = AppleMarkdownPresentationScheduler(.init(content: "initial", isStreaming: true))
        var snapshots: [AppleMarkdownRenderSnapshot] = []
        let subscription = scheduler.$snapshot.dropFirst().sink { snapshots.append($0) }
        scheduler.submit(.init(content: "first", isStreaming: true))
        scheduler.submit(.init(content: "latest", isStreaming: true))
        scheduler.submit(.init(content: "latest", isStreaming: true))
        XCTAssertTrue(snapshots.isEmpty)
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertEqual(snapshots.map(\.content), ["latest"])
        scheduler.submit(.init(content: "pending", isStreaming: true))
        scheduler.submit(.init(content: "terminal", isStreaming: false))
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertEqual(snapshots.map(\.content), ["latest", "terminal"])
        XCTAssertFalse(scheduler.buffer.renderBlocks(settleTail: true).contains(where: \.isMutableTail))
        // Restored presentation may become live again after Core resumes its Run.
        scheduler.submit(.init(content: "resumed", isStreaming: true))
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertEqual(scheduler.snapshot.content, "resumed")
        scheduler.submit(.init(content: "cancelled", isStreaming: false))
        scheduler.cancel()
        try await Task.sleep(nanoseconds: 40_000_000)
        XCTAssertEqual(scheduler.snapshot.content, "resumed")
        _ = subscription
    }

    @MainActor
    func testPresentationFrameCoalescesSynchronousBurstAndCancellationFencesTerminal() async throws {
        let subject = PassthroughSubject<Int, Never>()
        var received: [Int] = []
        let subscription = subject.coalescingChatFrames().sink { received.append($0) }
        subject.send(1); subject.send(2); subject.send(3)
        try await Task.sleep(nanoseconds: 60_000_000)
        XCTAssertEqual(received, [3])
        subject.send(4)
        subscription.cancel()
        received.append(99) // Completion applies its terminal snapshot directly.
        try await Task.sleep(nanoseconds: 40_000_000)
        XCTAssertEqual(received, [3, 99])
    }

    func testToolArrivalDoesNotMoveDisclosedTextIntoCollapsedProcess() throws {
        let text = IOSConversationProjectionV1.Part.text(.init(partID: "before", revision: 1,
            markdown: "I will inspect the file.", isFinal: true, review: nil))
        let tool = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: Data(#"{"kind":"tool","execution_source":{"kind":"model"},"part_id":"tool","revision":2,"tool_call_id":"call","capability_id":"file.read","state":"completed","title":"Read file","origin":{"kind":"builtin"},"artifacts":[],"input":{},"affected_resources":[]}"#.utf8))
        let result = IOSConversationProjectionV1.Part.text(.init(partID: "after", revision: 3,
            markdown: "The file is valid.", isFinal: false, review: nil))
        XCTAssertEqual(ChatExecutionProcessPolicy.segments([text, tool]).map { $0.parts.map(\.partID) }, [["before"], ["tool"]])
        XCTAssertEqual(ChatExecutionProcessPolicy.segments([text, tool, result]).map { $0.parts.map(\.partID) }, [["before"], ["tool"], ["after"]])
    }

    func testSharedAuthorityRuleAndTerminalReplacementApplyToIOSBuffer() {
        XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: "same", accepted: "same"), .unchanged)
        XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: "你好🌍", accepted: "你好"), .append("🌍"))
        XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: "short", accepted: "short provisional"), .replace("short"))
        XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: "", accepted: "retracted"), .replace(""))
        var buffer = AppleStreamingMarkdownBuffer()
        buffer.receive("Provisional answer", isStreaming: true, reduceMotion: false)
        buffer.prepareForCompletion("Corrected final answer", reduceMotion: false)
        XCTAssertEqual(buffer.source, "Corrected final answer")
        XCTAssertEqual(buffer.renderBlocks(settleTail: true).map(\.markdown).joined(), "Corrected final answer")
        XCTAssertFalse(buffer.renderBlocks(settleTail: true).contains(where: \.isMutableTail))
        buffer.finish(with: "")
        XCTAssertEqual(buffer.source, "")
        XCTAssertTrue(buffer.renderBlocks(settleTail: true).isEmpty)
    }

    func testDerivedAnswerReplacementAfterToolBoundaryDoesNotKeepStaleMarkdown() {
        var buffer = AppleStreamingMarkdownBuffer()
        buffer.receive("I will inspect the file.", isStreaming: true, reduceMotion: false)
        // A committed tool + text batch changes the displayed answer suffix;
        // it is not a provider delta and need not extend the prior suffix.
        buffer.receive("## Inspection result\n\nThe file is valid.", isStreaming: true, reduceMotion: false)
        XCTAssertEqual(buffer.visible, "## Inspection result\n\nThe file is valid.")
        XCTAssertEqual(buffer.source, buffer.visible)
        XCTAssertFalse(buffer.renderBlocks(settleTail: false).contains { $0.markdown.contains("I will inspect") })
        buffer.receive("## Inspection result\n\nThe file is valid. No changes needed.", isStreaming: true, reduceMotion: false)
        XCTAssertEqual(buffer.visible, "## Inspection result\n\nThe file is valid. No changes needed.")
    }

    func testIncrementalMarkdownUsesCoreSemanticBatchesWithoutASecondQueue() {
        let start = Date(timeIntervalSinceReferenceDate: 100)
        var buffer = AppleStreamingMarkdownBuffer()

        buffer.receive("第一段。", isStreaming: true, reduceMotion: false, now: start)
        XCTAssertEqual(buffer.visible, "第一段。")

        let extended = "第一段。\n\n## 小标题\n\n第二段包含 **加粗** 内容。"
        buffer.receive(extended, isStreaming: true, reduceMotion: false, now: start.addingTimeInterval(0.1))
        let committedPrefix = buffer.visible
        XCTAssertTrue(buffer.visible.hasPrefix(committedPrefix))
        XCTAssertEqual(buffer.visible, extended)
    }

    func testFinishingFlushesShortTailWithoutReplacingTheRenderSource() {
        let start = Date(timeIntervalSinceReferenceDate: 200)
        var buffer = AppleStreamingMarkdownBuffer()
        buffer.receive("短尾", isStreaming: true, reduceMotion: false, now: start)

        XCTAssertEqual(buffer.source, "短尾")
        XCTAssertEqual(buffer.visible, "短尾")
    }

    func testCompletionPreparationDoesNotCreateASecondRevealQueue() {
        let start = Date(timeIntervalSinceReferenceDate: 250)
        var buffer = AppleStreamingMarkdownBuffer()
        let complete = "第一段已经显示。第二段仍在等待舒缓显现，并且第三段会在完成事件之后继续按照语义节奏逐步出现，而不是突然全部跳到界面上。"
        buffer.receive(complete, isStreaming: true, reduceMotion: false, now: start)
        buffer.prepareForCompletion(complete, reduceMotion: false, now: start.addingTimeInterval(0.1))

        XCTAssertEqual(buffer.visible, complete)
        XCTAssertTrue(buffer.isSettled)
    }

    func testStreamingMarkdownCommitsStableBlocksAndKeepsOneMutableTail() {
        let start = Date(timeIntervalSinceReferenceDate: 275)
        var buffer = AppleStreamingMarkdownBuffer()
        buffer.receive(
            "## 已提交标题\n\n正在增长的正文",
            isStreaming: true,
            reduceMotion: false,
            now: start
        )
        let first = buffer.renderBlocks(settleTail: false)
        XCTAssertEqual(first.count, 2)
        XCTAssertFalse(first[0].isMutableTail)
        XCTAssertTrue(first[1].isMutableTail)
        XCTAssertEqual(first[0].id, "stream-markdown-0")

        buffer.receive(
            "## 已提交标题\n\n正在增长的正文继续出现。\n\n| 项目 | 状态 |\n|---|---|\n| 渲染 | 稳定 |",
            isStreaming: true,
            reduceMotion: false,
            now: start.addingTimeInterval(0.2)
        )
        let extended = buffer.renderBlocks(settleTail: false)

        XCTAssertEqual(extended[0], first[0], "Committed Markdown must keep identity and content.")
        XCTAssertEqual(extended.map(\.id), [
            "stream-markdown-0",
            "stream-markdown-1",
            "stream-markdown-2",
        ])
        XCTAssertEqual(extended.filter(\.isMutableTail).count, 1)

        buffer.prepareForCompletion(buffer.source, reduceMotion: false)
        let completed = buffer.renderBlocks(settleTail: true)
        XCTAssertFalse(completed.contains(where: \.isMutableTail))
        XCTAssertEqual(completed.map(\.id), extended.map(\.id))
    }

    func testTerminalPromotionParsesOnlyMutableMarkdownTail() {
        var buffer = AppleStreamingMarkdownBuffer()
        let stablePrefix = (0..<400).map { index in
            "## Section \(index)\n\nStable paragraph \(index).\n\n"
        }.joined()
        let streamed = stablePrefix + "Final paragraph is still growing"

        buffer.receive(streamed, isStreaming: true, reduceMotion: true)
        let beforeCompletion = buffer.renderBlocks(settleTail: false)
        XCTAssertGreaterThan(beforeCompletion.count, 400)

        buffer.finish(with: streamed)
        let completed = buffer.renderBlocks(settleTail: true)

        XCTAssertFalse(completed.contains(where: \.isMutableTail))
        XCTAssertEqual(completed.map(\.id), beforeCompletion.map(\.id))
        XCTAssertLessThan(buffer.lastParsedCharacterCount, 100)
        XCTAssertLessThan(buffer.lastParsedCharacterCount, streamed.count / 100)
    }

    func testTerminalPromotionCanAppendAndSettleTheFinalSuffix() {
        var buffer = AppleStreamingMarkdownBuffer()
        let streamed = "## Stable\n\nFirst paragraph.\n\nFinal para"
        let final = streamed + "graph completed."

        buffer.receive(streamed, isStreaming: true, reduceMotion: true)
        buffer.finish(with: final)

        XCTAssertEqual(buffer.source, final)
        XCTAssertEqual(buffer.visible, final)
        XCTAssertEqual(
            buffer.renderBlocks(settleTail: true),
            AppleStreamingMarkdownPartitioner.partition(final, settleTail: true)
        )
        XCTAssertLessThan(buffer.lastParsedCharacterCount, final.count)
    }

    func testIncrementalPartitionMatchesFullParseWhileOnlyTailChanges() {
        var buffer = AppleStreamingMarkdownBuffer()
        var source = ""
        let fragments = [
            "## 计划\n\n",
            "第一段包含 **稳定",
            "格式**，并继续增长。\n\n",
            "| 项目 | 状态 |\n| --- | --- |\n",
            "| 渲染 | 完成 |",
        ]

        for fragment in fragments {
            let previous = buffer.renderBlocks(settleTail: false)
            source += fragment
            buffer.receive(source, isStreaming: true, reduceMotion: false)
            let incremental = buffer.renderBlocks(settleTail: false)
            let full = AppleStreamingMarkdownPartitioner.partition(
                source,
                settleTail: false
            )
            XCTAssertEqual(incremental, full)
            let stablePrefix = previous.dropLast(previous.last?.isMutableTail == true ? 1 : 0)
            XCTAssertEqual(Array(incremental.prefix(stablePrefix.count)), Array(stablePrefix))
        }
    }

    func testTimelineFollowWindowIsBoundedAndHonorsReducedMotion() {
        XCTAssertGreaterThanOrEqual(ChatTimelineFollowPolicy.trailingPasses(reduceMotion: false), 10)
        XCTAssertLessThanOrEqual(ChatTimelineFollowPolicy.intervalNanoseconds(reduceMotion: false), 16_666_667)
        XCTAssertEqual(ChatTimelineFollowPolicy.trailingPasses(reduceMotion: true), 2)
        XCTAssertEqual(ChatTimelineFollowPolicy.intervalNanoseconds(reduceMotion: true), 16_000_000)
        XCTAssertLessThanOrEqual(DesignTokens.motionBase, 0.25)
        XCTAssertLessThanOrEqual(DesignTokens.streamRevealDuration, DesignTokens.motionBase)
        XCTAssertEqual(DesignTokens.maximumVisualBacklog, 0.32, accuracy: 0.001)
        XCTAssertLessThanOrEqual(
            Double(ChatTimelineFollowPolicy.trailingPasses(reduceMotion: false))
                * Double(ChatTimelineFollowPolicy.intervalNanoseconds(reduceMotion: false)) / 1_000_000_000,
            DesignTokens.maximumVisualBacklog
        )
    }

    func testTimelineMessageEntrancePolicyOnlyMarksAnAppendedSuffix() {
        let first = UUID(), second = UUID(), third = UUID()
        XCTAssertEqual(
            ChatTimelineMessageMotionPolicy.appendedIDs(previousConversationID: first, conversationID: first,
                previous: [first, second], current: [first, second, third]),
            [third]
        )
        XCTAssertTrue(ChatTimelineMessageMotionPolicy.appendedIDs(previousConversationID: first, conversationID: first,
            previous: nil, current: [first]).isEmpty)
        XCTAssertTrue(ChatTimelineMessageMotionPolicy.appendedIDs(previousConversationID: first, conversationID: second,
            previous: [first], current: [first, second]).isEmpty)
        XCTAssertTrue(ChatTimelineMessageMotionPolicy.appendedIDs(previousConversationID: first, conversationID: first,
            previous: [second], current: [first, second]).isEmpty)
        XCTAssertTrue(ChatTimelineMessageMotionPolicy.appendedIDs(previousConversationID: first, conversationID: first,
            previous: [first, second], current: [third]).isEmpty)
    }

    func testTimelineRejectsLateAsyncResultAfterConversationSwitchOrCancellation() {
        let original = UUID()
        let replacement = UUID()

        XCTAssertTrue(ChatTimelineFollowPolicy.acceptsAsyncResult(
            startedConversationID: original,
            currentConversationID: original,
            isCancelled: false
        ))
        XCTAssertFalse(ChatTimelineFollowPolicy.acceptsAsyncResult(
            startedConversationID: original,
            currentConversationID: replacement,
            isCancelled: false
        ))
        XCTAssertFalse(ChatTimelineFollowPolicy.acceptsAsyncResult(
            startedConversationID: original,
            currentConversationID: original,
            isCancelled: true
        ))
    }

    func testTimelineFollowMetricsAreProjectedOnceFromLongToolHistory() {
        var events = (0..<10_000).map { index in
            HostAgentToolEvent(
                id: "completed-\(index)",
                name: "tool",
                status: .succeeded,
                text: String(repeating: "x", count: 96)
            )
        }
        events.append(HostAgentToolEvent(
            id: "current",
            name: "browser",
            status: .permissionRequired,
            display: String(repeating: "界", count: 24),
            text: String(repeating: "y", count: 24)
        ))
        let message = ChatMessage(
            role: .assistant,
            content: String(repeating: "答", count: 240_000),
            thinking: String(repeating: "思", count: 2_400),
            hostToolEvents: events,
            metadata: [ChatConversationSurfaceMeta.currentToolCallIDKey: "current"]
        )

        XCTAssertEqual(message.timelineFollowMetrics.contentPhrase, 10_000)
        XCTAssertEqual(message.timelineFollowMetrics.thinkingPhrase, 100)
        XCTAssertEqual(message.timelineFollowMetrics.toolEventCount, 1)
        XCTAssertEqual(message.timelineFollowMetrics.toolContentPhrase, 2)
        XCTAssertEqual(message.liveHostToolEvents.map(\.id), ["current"])
    }

    func testSettlingTailKeepsTheSameRenderedMarkdownIdentity() {
        let markdown = "## 结论\n\n正文包含 **加粗** 和 [链接](https://example.com)。"
        let streaming = AppleStreamingMarkdownPartitioner.partition(
            markdown,
            settleTail: false
        )
        let completed = AppleStreamingMarkdownPartitioner.partition(
            markdown,
            settleTail: true
        )

        XCTAssertEqual(streaming.count, completed.count)
        XCTAssertTrue(streaming.last?.isMutableTail == true)
        XCTAssertTrue(completed.last?.isMutableTail == false)
        XCTAssertNotEqual(streaming.last, completed.last)
        XCTAssertTrue(streaming.last?.hasSameRenderedContent(as: completed.last!) == true)
    }

    func testContinuousParagraphKeepsOneNativeTextBlockAndStableIdentity() {
        let markdown = "Clouds drift gently across the open sky. The wind carries them in every direction."
        let live = AppleStreamingMarkdownPartitioner.partition(markdown, settleTail: false)
        let complete = AppleStreamingMarkdownPartitioner.partition(markdown, settleTail: true)
        XCTAssertEqual(live.count, 1)
        XCTAssertEqual(live.first?.markdown, markdown)
        XCTAssertEqual(live.first?.id, complete.first?.id)
        XCTAssertEqual(complete.first?.markdown, markdown)
    }

    func testNestedMarkdownFencesDoNotCommitCodeAsParagraphs() {
        let source = "````markdown\n```swift\nlet x = 1\n```\n\n~~~\n\n````\n\nAfter"
        let live = AppleStreamingMarkdownPartitioner.partition(source, settleTail: false)
        XCTAssertEqual(live.count, 2)
        XCTAssertTrue(live[0].markdown.hasSuffix("````\n\n"))
        XCTAssertFalse(live[0].isMutableTail)
        XCTAssertEqual(live[1].markdown, "After")
    }

    func testMutableMarkdownTailClosesInlineFormattingWithoutTouchingCodeFences() {
        XCTAssertEqual(
            AppleStreamingMarkdownTailNormalizer.stabilize("正文包含 **逐步出现"),
            "正文包含 **逐步出现**"
        )
        XCTAssertEqual(
            AppleStreamingMarkdownTailNormalizer.stabilize("正文包含 `inline"),
            "正文包含 `inline`"
        )

        let openFence = "```swift\nlet value = 42"
        XCTAssertEqual(
            AppleStreamingMarkdownTailNormalizer.stabilize(openFence),
            openFence
        )
    }

    func testStructuredToolDetailsPreferTraceArgumentsResourcesAndResult() {
        let event = HostAgentToolEvent(
            id: "write-1",
            name: "write_file",
            argsPreview: "tool args",
            status: .succeeded,
            text: "tool result"
        )
        let activity = AgentTraceActivity(
            spanId: "write-1",
            toolName: "write_file",
            status: .succeeded,
            arguments: .object([
                "path": .string("/tmp/report.md"),
                "mode": .string("replace"),
            ]),
            result: .object(["text": .string("wrote 42 bytes")]),
            resources: [
                AgentTraceResource(
                    uri: "file:///tmp/report.md",
                    operation: .update,
                    displayName: "report.md",
                    mimeType: "text/markdown"
                ),
            ],
            startedSequence: 2,
            finishedSequence: 3
        )

        let details = ChatToolDetailProjection(event: event, activity: activity)

        XCTAssertTrue(details.arguments.contains("path: /tmp/report.md"))
        XCTAssertEqual(details.resources.map(\.displayName), ["report.md"])
        XCTAssertEqual(details.resources.map(\.operation), [.update])
        XCTAssertEqual(details.result, "wrote 42 bytes")
    }

    func testTraceTransportProgressWithoutVisibleChangeIsUIEquivalent() {
        let activity = AgentTraceActivity(
            spanId: "search-1",
            toolName: "web_search",
            status: .running,
            summary: "Searching",
            startedSequence: 2
        )
        let earlier = AgentRunProjection(
            traceId: "trace",
            runId: "run",
            conversationId: "conversation",
            turnId: "turn",
            status: .running,
            activities: [activity],
            lastSequence: 2
        )
        let heartbeat = AgentRunProjection(
            traceId: "trace",
            runId: "run",
            conversationId: "conversation",
            turnId: "turn",
            status: .running,
            activities: [activity],
            lastSequence: 3
        )

        XCTAssertNotEqual(earlier, heartbeat)
        XCTAssertTrue(earlier.isVisuallyEquivalent(to: heartbeat))
    }

    func testRuntimeMarkdownRecognizesRichBlocksBeforeCompletion() throws {
        let markdown = """
        # 标题

        正文包含 **加粗** 和 [链接](https://example.com)。

        > 引用内容

        - 第一项
        - 第二项

        | 指标 | 数值 |
        | --- | ---: |
        | 收入 | 42 |

        ```swift
        let value = 42
        ```
        """

        XCTAssertEqual(
            ChatMarkdownRuntimeInspector.blockKinds(in: markdown),
            ["heading", "paragraph", "quote", "unordered-list", "table", "code"]
        )

        let inline = try AttributedString(
            markdown: "正文包含 **加粗** 和 [链接](https://example.com)。",
            options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        )
        XCTAssertTrue(inline.runs.contains { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true })
        XCTAssertTrue(inline.runs.contains { $0.link?.absoluteString == "https://example.com" })
    }

    func testAuthoritativePresentationReplacementUpdatesVisibleText() {
        let start = Date(timeIntervalSinceReferenceDate: 300)
        var buffer = AppleStreamingMarkdownBuffer()
        buffer.receive("旧答案。", isStreaming: true, reduceMotion: false, now: start)
        buffer.receive("更正后的答案。", isStreaming: true, reduceMotion: false, now: start.addingTimeInterval(0.1))

        XCTAssertEqual(buffer.source, "更正后的答案。")
        XCTAssertEqual(buffer.visible, "更正后的答案。")
    }

    func testAssistantDraftIdentitySurvivesThinkingToolAndAnswerUpdates() {
        let userID = UUID()
        let assistantID = UUID()
        var messages = [ChatMessage(id: userID, role: .user, content: "搜索最新消息")]

        ChatMessageDraftReducer.upsertAssistant(
            in: &messages,
            assistantID: assistantID,
            afterUserID: userID,
            tracePhase: "planning"
        )
        XCTAssertEqual(messages.map(\.id), [userID, assistantID])
        let initialRuntime = ChatAssistantRuntimeState(
            message: messages[1],
            isActive: true,
            isRetrieving: false
        )
        XCTAssertFalse(initialRuntime.showsWaiting)
        XCTAssertTrue(initialRuntime.showsPhase)
        XCTAssertEqual(messages[1].tracePhase, "planning")

        let tool = HostAgentToolEvent(
            id: "tool-1",
            name: "web_search",
            status: .running,
            display: "Searching"
        )
        ChatMessageDraftReducer.upsertAssistant(
            in: &messages,
            assistantID: assistantID,
            afterUserID: userID,
            thinking: "正在判断检索范围",
            hostToolEvents: [tool]
        )
        var runtime = ChatAssistantRuntimeState(
            message: messages[1],
            isActive: true,
            isRetrieving: false
        )
        XCTAssertEqual(messages.count, 2)
        XCTAssertEqual(messages[1].id, assistantID)
        XCTAssertTrue(runtime.hasThinking)
        XCTAssertTrue(runtime.hasTools)
        XCTAssertFalse(runtime.showsWaiting)
        XCTAssertEqual(messages[1].tracePhase, "planning")

        ChatMessageDraftReducer.upsertAssistant(
            in: &messages,
            assistantID: assistantID,
            afterUserID: userID,
            content: "这是检索结果。",
            thinkingFinished: true
        )
        runtime = ChatAssistantRuntimeState(
            message: messages[1],
            isActive: true,
            isRetrieving: false
        )
        XCTAssertEqual(messages.count, 2)
        XCTAssertEqual(messages[1].id, assistantID)
        XCTAssertEqual(messages[1].content, "这是检索结果。")
        XCTAssertTrue(runtime.hasContent)
        XCTAssertTrue(runtime.hasThinking)
        XCTAssertTrue(runtime.hasTools)
    }

    func testTracePhasePresentationNeverLeaksInternalAgentStatus() {
        XCTAssertEqual(
            ChatTracePhasePresentation.displayName(for: "tool_output_read"),
            NSLocalizedString(
                "chat.assistant.phase.tools",
                value: "Using tools",
                comment: ""
            )
        )
        XCTAssertEqual(
            ChatTracePhasePresentation.displayName(for: "agent running_turns=8"),
            NSLocalizedString(
                "chat.assistant.processing",
                value: "Nola is processing",
                comment: ""
            )
        )
    }

    @MainActor
    func testHostedBubbleUpdatesThroughWaitingThinkingToolsAndMarkdownInPlace() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("hosted-bubble-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let container = ServiceContainer(applicationRuntime: try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("fixture.sqlite")))
        let assistantID = UUID()
        let state = ChatBubbleRuntimeHarnessState(
            message: ChatMessage(id: assistantID, role: .assistant, content: "")
        )
        let controller = UIHostingController(rootView: ChatBubbleRuntimeHarness(state: state, container: container))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 720))
        window.rootViewController = controller
        window.makeKeyAndVisible()
        controller.view.frame = window.bounds
        await settleRuntimeUI()
        let hostedView = controller.view
        let waiting = snapshot(controller, name: "01-waiting")

        let tool = HostAgentToolEvent(
            id: "runtime-search",
            name: "web_search",
            status: .running,
            display: "Searching current sources"
        )
        state.message = ChatMessage(
            id: assistantID,
            role: .assistant,
            content: "",
            thinking: "正在确认来源并读取结果",
            thinkingFinished: false,
            hostToolEvents: [tool]
        )
        await settleRuntimeUI(0.25)
        let thinking = snapshot(controller, name: "02-thinking-and-tool")

        state.message = ChatMessage(
            id: assistantID,
            role: .assistant,
            content: "# 分析结果\n\n第一段包含 **实时 Markdown**。",
            thinking: "正在确认来源并读取结果",
            thinkingFinished: true,
            hostToolEvents: [tool]
        )
        await settleRuntimeUI(0.8)
        let partial = snapshot(controller, name: "03-streaming-markdown")

        state.message = ChatMessage(
            id: assistantID,
            role: .assistant,
            content: "# 分析结果\n\n第一段包含 **实时 Markdown**。\n\n| 来源 | 状态 |\n| --- | --- |\n| Web | 已读取 |",
            thinking: "正在确认来源并读取结果",
            thinkingFinished: true,
            hostToolEvents: [tool]
        )
        await settleRuntimeUI(1.1)
        state.isStreaming = false
        await settleRuntimeUI(1.8)
        let completed = snapshot(controller, name: "04-completed-markdown")

        XCTAssertTrue(controller.view === hostedView)
        XCTAssertEqual(state.message.id, assistantID)
        XCTAssertNotEqual(waiting.pngData(), thinking.pngData())
        XCTAssertNotEqual(thinking.pngData(), partial.pngData())
        XCTAssertNotEqual(partial.pngData(), completed.pngData())
        XCTAssertEqual(
            ChatMarkdownRuntimeInspector.blockKinds(in: state.message.content),
            ["heading", "paragraph", "table"]
        )
        _ = window
    }

    @MainActor
    func testResearchReplyRendersCompactBibliographyInLightAndDarkAppearance() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("research-reply-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let container = ServiceContainer(applicationRuntime: try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("fixture.sqlite")))
        let sources = try (1...31).map { index -> IOSConversationProjectionV1.Part in
            let data = try JSONSerialization.data(withJSONObject: ["kind": "source", "part_id": "source-\(index)", "revision": 1,
                "source_id": "s\(index)", "title": "跨平台交互与引用格式参考 \(index)", "uri": "https://example.com/\(index)"])
            return try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: data)
        }
        let parts: [IOSConversationProjectionV1.Part] = [
            .text(.init(partID: "answer", revision: 1, markdown: "正文里的[来源](https://example.com/inline)是内联引用。\n\n| 时间 | 经过和影响 |\n| --- | --- |\n| 阶段一 | 保留说明及[来源](https://example.com/table) |", isFinal: true, review: nil)),
            .skill(id: "skill", revision: 1, runID: "hidden-run", skillID: "conduct-deep-research", displayName: "conduct-deep-research", mode: "inline", state: .running, modelTurns: 8, toolCalls: 10, elapsedMillis: 116945, resultRef: nil, failure: nil)
        ] + sources
        let message = ChatMessage(role: .assistant, content: "", canonicalParts: parts)
        for style in [UIUserInterfaceStyle.light, .dark] {
            let state = ChatBubbleRuntimeHarnessState(message: message, isStreaming: false)
            let controller = UIHostingController(rootView: ChatBubbleRuntimeHarness(state: state, container: container))
            controller.overrideUserInterfaceStyle = style
            controller.view.frame = CGRect(x: 0, y: 0, width: 390, height: 640)
            await settleRuntimeUI(0.3)
            let image = snapshot(controller, name: "research-bibliography-\(style == .dark ? "dark" : "light")")
            XCTAssertGreaterThan(image.pngData()?.count ?? 0, 4000)
        }
    }

    @MainActor
    func testCompletedMarkdownRendersInLightAndDarkAppearance() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("hosted-bubble-appearance-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let container = ServiceContainer(applicationRuntime: try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("fixture.sqlite")))
        let message = ChatMessage(
            role: .assistant,
            content: "## 概览\n\n- **重点**：内容保持清晰\n- [来源链接](https://example.com)\n\n> 引用在深浅色下都应有足够对比度。"
        )

        for style in [UIUserInterfaceStyle.light, .dark] {
            let state = ChatBubbleRuntimeHarnessState(message: message, isStreaming: false)
            let controller = UIHostingController(rootView: ChatBubbleRuntimeHarness(state: state, container: container))
            controller.overrideUserInterfaceStyle = style
            controller.view.frame = CGRect(x: 0, y: 0, width: 390, height: 560)
            await settleRuntimeUI()
            let suffix = style == .dark ? "dark" : "light"
            let image = snapshot(controller, name: "markdown-\(suffix)")
            XCTAssertGreaterThan(image.pngData()?.count ?? 0, 4_000)
        }
    }
}

@MainActor
private final class ChatBubbleRuntimeHarnessState: ObservableObject {
    @Published var message: ChatMessage
    @Published var isStreaming: Bool

    init(message: ChatMessage, isStreaming: Bool = true) {
        self.message = message
        self.isStreaming = isStreaming
    }
}

private struct ChatBubbleRuntimeHarness: View {
    @ObservedObject var state: ChatBubbleRuntimeHarnessState
    let container: ServiceContainer

    var body: some View {
        ScrollView {
            ChatBubble(
                conversationID: nil,
                message: state.message,
                isLatestAssistant: true,
                isStreaming: state.isStreaming,
                isRetrieving: false,
                pendingHostPermissionRequest: nil,
                onHostPermissionDecision: { _, _ in }
            )
            .padding(16)
        }
        .background(Color(uiColor: .systemBackground))
        .environmentObject(container)
    }
}

@MainActor
private func settleRuntimeUI(_ seconds: TimeInterval = 0.15) async {
    try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    await Task.yield()
}

@MainActor
private func snapshot(
    _ controller: UIHostingController<ChatBubbleRuntimeHarness>,
    name: String
) -> UIImage {
    controller.view.setNeedsLayout()
    controller.view.layoutIfNeeded()
    controller.view.backgroundColor = .systemBackground
    let format = UIGraphicsImageRendererFormat.preferred()
    format.opaque = true
    let renderer = UIGraphicsImageRenderer(bounds: controller.view.bounds, format: format)
    let image = renderer.image { _ in
        controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
    }
    let attachment = XCTAttachment(image: image)
    attachment.name = name
    attachment.lifetime = .keepAlways
    XCTContext.runActivity(named: name) { activity in
        activity.add(attachment)
    }
    return image
}
