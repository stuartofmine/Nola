import XCTest
@testable import Nola

final class MacAssistantPresentationVisibilityTests: XCTestCase {
    func testSourcePrimaryFieldsExcludeRawEvidenceAndResourceAddresses() {
        for field in ["title", "name", "citation_section", "source_snippet"] {
            XCTAssertTrue(MacJournalSourceFieldPresentation.isPrimary(field))
        }
        for field in ["uri", "source_evidence", "mime_type", "future_field", ""] {
            XCTAssertFalse(MacJournalSourceFieldPresentation.isPrimary(field))
        }
    }

    func testToolResultFieldsExcludeRawDataAndPermissionRecords() {
        for field in ["tool_summary", "tool_artifacts"] {
            XCTAssertTrue(MacJournalToolFieldPresentation.isResult(field))
        }
        for field in ["tool_input", "tool_output", "tool_output_text", "tool_resources",
                      "permission_decision", "permission_rationale", "future_field", ""] {
            XCTAssertFalse(MacJournalToolFieldPresentation.isResult(field))
        }
    }

    func testSettledEmptyThinkingDoesNotLeaveProcessRows() {
        for state in [MacAssistantTrace.State.completed, .cancelled] {
            let placeholder = MacAssistantTrace(kind: .thinking, title: "正在思考中", detail: "  ", state: state)
            XCTAssertFalse(MacAssistantArtifactPresentationPolicy.shouldShowInProcess(placeholder))
        }
        for state in [MacAssistantTrace.State.running, .failed, .incomplete] {
            let trace = MacAssistantTrace(kind: .thinking, title: "Thinking", detail: "", state: state)
            XCTAssertTrue(MacAssistantArtifactPresentationPolicy.shouldShowInProcess(trace))
        }
        let reasoning = MacAssistantTrace(kind: .thinking, title: "Thinking", detail: "Analysis", state: .completed)
        XCTAssertTrue(MacAssistantArtifactPresentationPolicy.shouldShowInProcess(reasoning))
    }

    func testActivePlaceholderIsVisibleButTerminalEmptyAssistantIsNot() {
        let message = MacChatMessage(
            role: .assistant,
            text: "",
            createdAt: Date(),
            presentation: .processing()
        )

        XCTAssertTrue(macChatMessageIsVisible(message, isActive: true))
        XCTAssertFalse(macChatMessageIsVisible(message, isActive: false))
    }

    func testTerminalAssistantWithThinkingTraceRemainsVisible() {
        var presentation = MacAssistantPresentation.processing()
        presentation.appendThinking("Inspecting the request", replacing: false)
        let message = MacChatMessage(
            role: .assistant,
            text: "",
            createdAt: Date(),
            presentation: presentation
        )

        XCTAssertTrue(macChatMessageIsVisible(message, isActive: false))
    }

    func testCanonicalTraceIdentityAndDatesRemainStableAcrossProjectionRefreshes() {
        func presentation() -> MacAssistantPresentation {
            var value = MacAssistantPresentation.processing()
            value.upsertCoreThinking(
                partID: "reasoning-1",
                title: "Thinking",
                detail: "Inspecting the runtime",
                isComplete: true
            )
            value.upsertTool(
                externalID: "tool-1",
                title: "read_file",
                detail: "Read runtime.swift",
                state: .completed
            )
            return value
        }

        let first = presentation().traceItems
        let second = presentation().traceItems

        XCTAssertEqual(first.map(\.id), second.map(\.id))
        XCTAssertEqual(first.map(\.startedAt), second.map(\.startedAt))
        XCTAssertEqual(first.map(\.finishedAt), second.map(\.finishedAt))
    }

    func testTimelineKeepsEveryReasoningAndToolItemInOriginalOrder() {
        var presentation = MacAssistantPresentation.processing()
        presentation.upsertCoreThinking(
            partID: "reasoning-1",
            title: "Thinking",
            detail: "Inspecting",
            isComplete: true
        )
        presentation.upsertTool(
            externalID: "tool-1",
            title: "read_file",
            detail: "One",
            state: .completed
        )
        presentation.upsertCoreThinking(
            partID: "reasoning-2",
            title: "Thinking",
            detail: "Comparing",
            isComplete: true
        )
        presentation.upsertTool(
            externalID: "tool-2",
            title: "grep",
            detail: "Two",
            state: .running
        )

        XCTAssertEqual(
            MacAssistantTraceTimelinePolicy.visibleDetailTraces(presentation.traceItems)
                .compactMap(\.externalID),
            ["reasoning-1", "tool-1", "reasoning-2", "tool-2"]
        )
    }

    func testTimelineGroupsRepeatedToolKindsWithoutDiscardingTheirAuditEntries() {
        let traces = [
            MacAssistantTrace(
                externalID: "tree-1",
                kind: .tool,
                title: "file_tree",
                detail: "First root",
                state: .completed
            ),
            MacAssistantTrace(
                externalID: "thinking-1",
                kind: .thinking,
                title: "Thinking",
                detail: "Comparing roots",
                state: .completed
            ),
            MacAssistantTrace(
                externalID: "tree-2",
                kind: .tool,
                title: "file_tree",
                detail: "Second root",
                state: .running
            ),
            MacAssistantTrace(
                externalID: "glob-1",
                kind: .tool,
                title: "glob",
                detail: "Swift files",
                state: .completed
            ),
        ]

        let groups = MacAssistantTraceTimelinePolicy.groupedDetailTraces(traces)

        XCTAssertEqual(groups.count, 4)
        XCTAssertEqual(groups.flatMap(\.traces).compactMap(\.externalID),
                       ["tree-1", "thinking-1", "tree-2", "glob-1"])
        XCTAssertEqual(groups[2].state, .running)
    }
}
