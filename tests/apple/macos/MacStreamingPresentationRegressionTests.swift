// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacStreamingPresentationRegressionTests: XCTestCase {
    func testConversationTraceSwitchRetiresOldStreamOnlyAfterAdmission() throws {
        func event(_ trace: String, _ sequence: Int, conversation: String = "conversation") throws -> MacAgentTraceEnvelope {
            try XCTUnwrap(MacAgentTraceEnvelope.decode([
                "schema_version": 1, "trace_id": trace, "run_id": "turn-\(trace)",
                "conversation_id": conversation, "turn_id": "turn-\(trace)",
                "event_id": "\(trace)-\(sequence)", "sequence": sequence,
                "emitted_at": "2026-10-12T00:00:00Z", "kind": "phase.changed",
                "status": "running", "visibility": "summary", "payload": ["phase": "compacting"],
            ]))
        }
        for projectsContent in [true, false] {
            var owner = MacConversationTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            XCTAssertEqual(owner.apply(try event("a", 1), to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            XCTAssertEqual(owner.apply(try event("b", 0), to: &presentation).disposition,
                .outOfOrder(last: 0, received: 0))
            XCTAssertEqual(owner.apply(try event("a", 1), to: &presentation).disposition, .duplicate)
            XCTAssertEqual(owner.apply(try event("b", 1, conversation: "foreign"),
                to: &presentation).disposition, .identityMismatch)
            XCTAssertEqual(owner.apply(try event("b", 1), to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            let before = presentation
            for sequence in [1, 2, 100] {
                XCTAssertEqual(owner.apply(try event("a", sequence), to: &presentation,
                    projectsContent: projectsContent).disposition, .retiredTrace)
                XCTAssertEqual(presentation, before)
            }
            XCTAssertEqual(owner.apply(try event("b", 2), to: &presentation).disposition, .applied)
        }
    }

    func testTraceConflictRejectsChangedPayloadWithoutConsumingSequence() throws {
        func event(_ sequence: Int, id: String, phase: String = "compacting",
                   trace: String = "trace") throws -> MacAgentTraceEnvelope {
            try XCTUnwrap(MacAgentTraceEnvelope.decode([
                "schema_version": 1, "trace_id": trace, "run_id": "run",
                "conversation_id": "conversation", "turn_id": "turn", "event_id": id,
                "sequence": sequence, "emitted_at": "2026-10-12T00:00:00Z",
                "kind": "phase.changed", "status": "running", "visibility": "summary",
                "payload": ["phase": phase],
            ]))
        }
        for projectsContent in [true, false] {
            var reducer = MacAgentTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            let original = try event(1, id: "first")
            XCTAssertEqual(reducer.apply(original, to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            let before = presentation
            for conflict in [try event(1, id: "first", phase: "running"),
                             try event(2, id: "first")] {
                XCTAssertEqual(reducer.apply(conflict, to: &presentation,
                    projectsContent: projectsContent).disposition, .eventConflict)
                XCTAssertEqual(presentation, before)
            }
            XCTAssertEqual(reducer.apply(try event(0, id: "bad", trace: "foreign"),
                to: &presentation).disposition, .identityMismatch)
            XCTAssertEqual(reducer.apply(original, to: &presentation).disposition, .duplicate,
                "A rejected new trace must not erase the old replay history")
            XCTAssertEqual(reducer.apply(try event(2, id: "second"), to: &presentation).disposition, .applied)
        }
    }

    func testTraceIdentityRejectionDoesNotResetCursorOrConsumeEventID() throws {
        func event(_ sequence: Int, trace: String = "trace", run: String = "run",
                   conversation: String = "conversation") throws -> MacAgentTraceEnvelope {
            let data = try JSONSerialization.data(withJSONObject: [
                "schema_version": 1, "trace_id": trace, "run_id": run,
                "conversation_id": conversation, "turn_id": "turn", "event_id": "event-\(sequence)",
                "sequence": sequence, "emitted_at": "2026-10-12T00:00:00Z",
                "kind": "phase.changed", "status": "running", "visibility": "summary",
                "payload": ["phase": "compacting"],
            ])
            return try JSONDecoder().decode(MacAgentTraceEnvelope.self, from: data)
        }
        for projectsContent in [true, false] {
            var reducer = MacAgentTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            XCTAssertEqual(reducer.apply(try event(1), to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            let before = presentation
            for foreign in [try event(1, run: "other"), try event(2, run: "other"),
                            try event(2, conversation: "other"),
                            try event(1, trace: "other", conversation: "other")] {
                let rejected = reducer.apply(foreign, to: &presentation, projectsContent: projectsContent)
                XCTAssertEqual(rejected.disposition, .identityMismatch)
                XCTAssertFalse(rejected.isTerminal)
                XCTAssertEqual(presentation, before)
            }
            XCTAssertEqual(reducer.apply(try event(1), to: &presentation).disposition, .duplicate)
            XCTAssertEqual(reducer.apply(try event(2), to: &presentation).disposition, .applied)
            XCTAssertEqual(reducer.apply(try event(1, trace: "next", run: "next-run"),
                to: &presentation).disposition, .identityMismatch)
        }
    }

    func testValidationDiagnosticsDoNotReplaceRunStateOrBecomeCompleted() throws {
        for projectsContent in [true, false] {
            for status in ["incomplete", "blocked", "evidence_insufficient", "failed", "error"] {
                var reducer = MacAgentTraceReducer()
                var presentation = MacAssistantPresentation.processing()
                presentation.upsertStatus(externalID: "run-status", title: "Run", detail: "Original failure", state: .failed)
                func event(_ sequence: Int, _ status: String) throws -> MacAgentTraceEnvelope {
                    let data = try JSONSerialization.data(withJSONObject: [
                        "schema_version": 1, "trace_id": "trace", "run_id": "run",
                        "conversation_id": "conversation", "turn_id": "turn", "event_id": "event-\(sequence)",
                        "sequence": sequence, "emitted_at": "2026-10-12T00:00:00Z",
                        "kind": "validation.updated", "status": "incomplete", "visibility": "summary",
                        "payload": ["validation": ["status": status, "summary": "Diagnostic"]],
                    ])
                    return try JSONDecoder().decode(MacAgentTraceEnvelope.self, from: data)
                }
                let outcome = reducer.apply(try event(1, status), to: &presentation, projectsContent: projectsContent)
                XCTAssertFalse(outcome.isTerminal)
                let diagnostic = try XCTUnwrap(presentation.traceItems.first { $0.externalID == "validation-diagnostic" })
                XCTAssertEqual(diagnostic.state, ["failed", "error"].contains(status) ? .failed : .incomplete)
                XCTAssertEqual(presentation.traceItems.first { $0.externalID == "run-status" }?.detail, "Original failure")
                XCTAssertTrue(MacAssistantPresentation.preservesAcrossCanonicalProjection(diagnostic))
                presentation.finish()
                XCTAssertEqual(presentation.traceItems.first { $0.externalID == "validation-diagnostic" }?.state, diagnostic.state)
                _ = reducer.apply(try event(2, "not_required"), to: &presentation, projectsContent: projectsContent)
                XCTAssertFalse(presentation.traceItems.contains { $0.externalID == "validation-diagnostic" })
                XCTAssertEqual(presentation.traceItems.first { $0.externalID == "run-status" }?.detail, "Original failure")
            }
        }
    }

    func testCoreOwnedCompactionStatusSettlesOnEveryTerminalPath() throws {
        for terminal in ["run.completed", "run.failed", "run.cancelled", "context.compacted"] {
            var reducer = MacAgentTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            func event(_ sequence: Int, _ kind: String, _ payload: [String: String]) throws -> MacAgentTraceEnvelope {
                let data = try JSONSerialization.data(withJSONObject: [
                    "schema_version": 1, "trace_id": "trace", "run_id": "run",
                    "conversation_id": "conversation", "turn_id": "turn", "event_id": "event-\(sequence)",
                    "sequence": sequence, "emitted_at": "2026-09-21T00:00:00Z",
                    "kind": kind, "status": "running", "visibility": "summary", "payload": payload,
                ])
                return try JSONDecoder().decode(MacAgentTraceEnvelope.self, from: data)
            }
            _ = reducer.apply(try event(1, "phase.changed", ["phase": "compacting"]), to: &presentation, projectsContent: false)
            XCTAssertTrue(presentation.traceItems.contains { $0.externalID == "context-compaction" })
            _ = reducer.apply(try event(2, "tool.progress", ["summary": "model_attempt_started"]), to: &presentation, projectsContent: true)
            XCTAssertFalse(presentation.traceItems.contains { $0.kind == .tool })
            XCTAssertTrue(presentation.traceItems.contains { $0.externalID == "context-compaction" })
            _ = reducer.apply(try event(3, terminal, [:]), to: &presentation, projectsContent: false)
            XCTAssertFalse(presentation.traceItems.contains { $0.externalID == "context-compaction" }, terminal)
        }
    }

    func testLargeProviderPayloadIsReleasedAsSeveralLosslessSemanticSegments() {
        let input = String(repeating: "这一小段内容应该平滑浮现，而不是一次跳到最终结果。", count: 80)
        var buffer = MacPacedStreamBuffer(minimumLength: 18, maximumLength: 56)

        let streamed = buffer.append(input)
        let completed = streamed + buffer.finish()

        XCTAssertGreaterThan(completed.count, 10)
        XCTAssertEqual(completed.joined(), input)
        XCTAssertTrue(completed.allSatisfy { $0.count <= 56 })
    }

    func testMarkdownMutableTailKeepsItsIdentityWhenFinalized() throws {
        let seed = UUID().uuidString
        var assembler = MacSemanticStreamAssembler(identitySeed: seed)

        _ = assembler.append("## 稳定标题\n\n正在流式显示的段落")
        let mutableID = try XCTUnwrap(assembler.mutableTail?.id)
        let finalized = assembler.finish()

        XCTAssertEqual(finalized.last?.id, mutableID)
        XCTAssertEqual(finalized.last?.text, "正在流式显示的段落")
    }

    func testIncompleteProseTailIsVisibleAndKeepsIdentityAtSemanticCommit() throws {
        var assembler = MacSemanticStreamAssembler(identitySeed: UUID().uuidString)
        var presentation = MacAssistantPresentation.processing()

        presentation.append(assembler.append("尚未形成完整语义边界的正文"))
        presentation.updateMutableTextNode(assembler.mutableTail)
        let liveNode = try XCTUnwrap(presentation.renderedTextNodes.last)
        XCTAssertEqual(liveNode.text, "尚未形成完整语义边界的正文")

        presentation.append(assembler.finish())
        presentation.updateMutableTextNode(nil)
        XCTAssertEqual(presentation.renderedTextNodes.map(\.text), ["尚未形成完整语义边界的正文"])
        XCTAssertEqual(presentation.renderedTextNodes.last?.id, liveNode.id)
    }

    func testDefaultPacingReleasesShortWordGroupsWithoutLosingText() {
        let input = "Streaming output should feel immediate while keeping every character intact."
        var buffer = MacPacedStreamBuffer()

        let streamed = buffer.append(input)
        let completed = streamed + buffer.finish()

        XCTAssertGreaterThan(streamed.count, 1)
        XCTAssertEqual(completed.joined(), input)
        XCTAssertTrue(completed.allSatisfy { $0.count <= 24 })
    }

    func testOpenCodeBlockRemainsVisibleForIncrementalUpdates() throws {
        var assembler = MacSemanticStreamAssembler(identitySeed: UUID().uuidString)
        var presentation = MacAssistantPresentation.processing()

        presentation.append(assembler.append("```swift\nlet value = 1"))
        presentation.updateMutableTextNode(assembler.mutableTail)
        let node = try XCTUnwrap(presentation.renderedTextNodes.last)

        XCTAssertEqual(node.kind, .code(language: "swift"))
        XCTAssertEqual(node.text, "let value = 1")
    }

    func testLiveStructuredBlockRendersIncompleteInlineMarkdownWithoutRawMarkers() throws {
        var assembler = MacSemanticStreamAssembler(identitySeed: UUID().uuidString)

        XCTAssertTrue(assembler.append("- **正在审").isEmpty)
        let node = try XCTUnwrap(assembler.mutableTail)
        XCTAssertEqual(node.kind, .markdownBlock)
        let item = try XCTUnwrap(MacMarkdownTextParser.parse(node.text).first)
        guard case .listItem(_, let children) = item,
              case .paragraph(let source) = children.first else {
            return XCTFail("Expected the mutable tail to remain a rendered list item")
        }
        let prepared = MacMarkdownInlinePreprocessor.prepare(
            source,
            repairingIncompleteSyntax: true
        )
        let attributed = try AttributedString(
            markdown: prepared,
            options: .init(
                interpretedSyntax: .inlineOnlyPreservingWhitespace,
                failurePolicy: .returnPartiallyParsedIfPossible
            )
        )

        XCTAssertEqual(String(attributed.characters), "正在审")
        XCTAssertFalse(String(attributed.characters).contains("**"))
    }
}
