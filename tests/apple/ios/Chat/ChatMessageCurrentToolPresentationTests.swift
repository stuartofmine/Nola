// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class ChatMessageCurrentToolPresentationTests: XCTestCase {
    func testFailedReceiptRemainsAvailableDuringNextToolWithoutDuplicatingCurrentTool() {
        let failed = HostAgentToolEvent(id: "failed", name: "bash", status: .failed, text: "Missing dependency")
        let running = HostAgentToolEvent(id: "current", name: "bash", status: .running, text: "Installing")
        let message = ChatMessage(role: .assistant, content: "", hostToolEvents: [failed, running])
        XCTAssertEqual(message.failedHistoricalToolEvents.map(\.id), ["failed"])
        XCTAssertEqual(message.liveHostToolEvents.map(\.id), ["current"])
        let currentFailure = ChatMessage(role: .assistant, content: "", hostToolEvents: [failed],
            metadata: [ChatConversationSurfaceMeta.currentToolCallIDKey: "failed"])
        XCTAssertTrue(currentFailure.failedHistoricalToolEvents.isEmpty)
    }

    func testHistoricalReasoningRemainsReadableWithoutAnActivePlaceholder() {
        let history = ChatMessage(role: .assistant, content: "Answer", thinking: "Actual reasoning")
        let runtime = ChatAssistantRuntimeState(message: history, isActive: false, isRetrieving: false)
        XCTAssertTrue(runtime.showsThinking)
        XCTAssertFalse(runtime.showsWaiting)
        XCTAssertFalse(runtime.showsPhase)
        let empty = ChatMessage(role: .assistant, content: "Answer", thinking: "  ")
        XCTAssertFalse(ChatAssistantRuntimeState(message: empty, isActive: false, isRetrieving: false).showsThinking)
    }

    func testLiveToolFocusKeepsHistoryAndReturnsOnlyLatestActiveTool() {
        let completed = HostAgentToolEvent(
            id: "completed",
            name: "read_file",
            status: .succeeded,
            text: "done"
        )
        let running = HostAgentToolEvent(
            id: "running",
            name: "code_search",
            status: .running,
            text: "indexing"
        )
        let message = ChatMessage(
            role: .assistant,
            content: "",
            hostToolEvents: [completed, running]
        )

        XCTAssertEqual(message.hostToolEvents.count, 2)
        XCTAssertEqual(message.liveHostToolEvents.map(\.id), ["running"])
    }

    func testCompletedHistoryDoesNotRemainInLiveBubble() {
        let message = ChatMessage(
            role: .assistant,
            content: "Result",
            hostToolEvents: [
                HostAgentToolEvent(
                    id: "completed",
                    name: "read_file",
                    status: .succeeded,
                    text: "done"
                ),
            ]
        )

        XCTAssertTrue(message.liveHostToolEvents.isEmpty)
        XCTAssertEqual(message.hostToolEvents.count, 1)
    }
}
