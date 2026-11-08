import Foundation
import XCTest
@testable import Nola

final class MacConversationPresentationOverlayTests: XCTestCase {
    func testHistoryScopeKeepsPageMembershipWithoutDroppingOffPageLiveState() {
        let historical = MacChatMessage(role: .assistant, text: "history", createdAt: .distantPast)
        let live = MacChatMessage(role: .assistant, text: "live", createdAt: .distantPast)
        var overlay = MacConversationPresentationOverlay()
        XCTAssertTrue(overlay.mutateMessage(canonical: [historical, live], messageID: live.id,
            createAssistantIfMissing: false) { $0.text = "live update" })
        XCTAssertTrue(overlay.mutateMessage(canonical: [historical], messageID: historical.id,
            createAssistantIfMissing: false) { $0.text = "history correction" })
        let history = overlay.renderedMessages(canonical: [historical], scope: .canonicalPage)
        XCTAssertEqual(history.map(\.id), [historical.id])
        XCTAssertEqual(history.first?.text, "history correction")
        let latest = overlay.renderedMessages(canonical: [live], scope: .canonicalPage)
        XCTAssertEqual(latest.first?.text, "live update", "Hiding an off-page row must not discard its live state")
        XCTAssertEqual(Set(overlay.renderedMessages(canonical: [historical]).map(\.id)),
            Set([historical.id, live.id]))
    }

    func testPendingSubmissionDoesNotAppendToHistoryPage() {
        let old = MacChatMessage(role: .user, text: "history", createdAt: .distantPast)
        let user = MacChatMessage(role: .user, text: "new question", createdAt: .distantPast)
        let assistant = MacChatMessage(role: .assistant, text: "", createdAt: .distantPast)
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(.init(submissionID: "pending", userMessage: user, assistantMessage: assistant))
        XCTAssertEqual(overlay.renderedMessages(canonical: [old], scope: .canonicalPage), [old])
        XCTAssertEqual(overlay.renderedMessages(canonical: [], scope: .latest).map(\.id), [user.id, assistant.id])
    }

    func testLargeCanonicalRefreshUsesIdentityIndexAndPreservesOrder() {
        let messageCount = 10_000
        let canonical = (0..<messageCount).map { index in
            MacChatMessage(
                id: UUID(),
                role: .assistant,
                text: "message \(index)",
                createdAt: .distantPast
            )
        }
        let previous = canonical.reversed()

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: canonical,
            previous: Array(previous)
        )

        XCTAssertEqual(refreshed.map(\.id), canonical.map(\.id))
        XCTAssertEqual(refreshed.map(\.text), canonical.map(\.text))
    }

    func testCanonicalRefreshKeepsCompletedNodesButUpdatesTraceMetadata() {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        let node = MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "Existing answer")
        live.append([node])
        live.finish()
        let old = MacChatMessage(id: id, role: .assistant, text: node.text, createdAt: .now, presentation: live)
        var canonical = old
        var fresh = MacAssistantPresentation.processing()
        fresh.append([MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: node.text)])
        fresh.upsertTool(externalID: "tool", title: "Read", detail: "Completed", state: .completed)
        fresh.finish()
        canonical.presentation = fresh
        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(in: [canonical], previous: [old])
        XCTAssertEqual(refreshed.first?.presentation?.textNodes, live.textNodes)
        XCTAssertEqual(refreshed.first?.presentation?.traceItems, fresh.traceItems)
        canonical.text = "Authoritative correction"
        XCTAssertEqual(
            MacConversationPresentationOverlay.preservingCompletedText(in: [canonical], previous: [old]),
            [canonical]
        )
    }

    func testStaleCanonicalRefreshCannotReopenCompletedMarkdownPresentation() throws {
        let id = UUID()
        let node = MacAssistantTextNode(
            blockID: UUID(),
            kind: .markdownBlock,
            text: "## Stable result"
        )
        var completed = MacAssistantPresentation.processing()
        completed.append([node])
        completed.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: node.text,
            createdAt: .now,
            presentation: completed
        )

        var stale = MacAssistantPresentation.processing()
        stale.append([MacAssistantTextNode(
            blockID: UUID(),
            kind: .markdownBlock,
            text: node.text
        )])
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: node.text,
            createdAt: previous.createdAt,
            presentation: stale
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )
        let presentation = try XCTUnwrap(refreshed.first?.presentation)

        XCTAssertTrue(presentation.isComplete)
        XCTAssertEqual(presentation.textNodes, [node])
    }

    func testTerminalTextOnlyRefreshKeepsCompletedLivePresentationTree() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        let nodes = [
            MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "First sentence."),
            MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: " Second sentence."),
        ]
        live.append(nodes)
        live.upsertTool(externalID: "transient-tool", title: "Read", detail: "Completed", state: .completed)
        live.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: nodes.map(\.text).joined(),
            createdAt: .now,
            presentation: live
        )
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: previous.text,
            createdAt: previous.createdAt
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )

        XCTAssertEqual(try XCTUnwrap(refreshed.first?.presentation).textNodes, nodes)
        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).traceItems.isEmpty)
        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).isComplete)
    }

    func testCanonicalRefreshDoesNotPreserveTransportOnlyModelFallbackDisclosure() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        live.append([MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "Recovered answer")])
        live.upsertStatus(
            externalID: "model-route:event-2",
            title: "Model fallback",
            detail: "Failed model: Primary · first\nFalling back to: Backup · second",
            state: .completed
        )
        live.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: "Recovered answer",
            createdAt: .now,
            presentation: live
        )
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: previous.text,
            createdAt: previous.createdAt
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )

        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).traceItems.isEmpty)
    }

    func testSuccessfulCanonicalRefreshDropsTransportOnlyModelProtocolFailure() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        live.append([MacAssistantTextNode(blockID: UUID(), kind: .phrase, text: "Verified answer")])
        live.upsertStatus(
            externalID: "model-protocol:event-1",
            title: "Response failed",
            detail: "The invalid tool request was rejected before retrying.",
            state: .failed
        )
        live.finish()
        let previous = MacChatMessage(
            id: id,
            role: .assistant,
            text: "Verified answer",
            createdAt: .now,
            presentation: live
        )
        let canonical = MacChatMessage(
            id: id,
            role: .assistant,
            text: previous.text,
            createdAt: previous.createdAt
        )

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(
            in: [canonical],
            previous: [previous]
        )

        XCTAssertTrue(try XCTUnwrap(refreshed.first?.presentation).traceItems.isEmpty)
    }

    func testFailedCanonicalRefreshReplacesProtocolPreviewsWithCommittedFailure() throws {
        let id = UUID()
        var live = MacAssistantPresentation.processing()
        for index in 1...3 {
            live.upsertStatus(externalID: "model-protocol:event-\(index)",
                             title: "Response failed", detail: "Retrying", state: .failed)
        }
        live.finish()
        let previous = MacChatMessage(id: id, role: .assistant, text: "", createdAt: .now, presentation: live)
        var committed = MacAssistantPresentation.processing()
        committed.upsertStatus(externalID: "model-attempt:terminal-attempt",
                               title: "Model request failed",
                               detail: "The model repeatedly emitted pseudo tool execution or unverified command output",
                               state: .failed)
        committed.finish()
        let canonical = MacChatMessage(id: id, role: .assistant, text: "", createdAt: previous.createdAt,
                                       presentation: committed, deliveryState: .failed)

        let refreshed = MacConversationPresentationOverlay.preservingCompletedText(in: [canonical], previous: [previous])
        let traces = try XCTUnwrap(refreshed.first?.presentation).traceItems
        XCTAssertEqual(traces, committed.traceItems)
        XCTAssertEqual(traces.count, 1)
        XCTAssertEqual(traces[0].state, .failed)
        XCTAssertTrue(traces[0].detail.contains("unverified command output"))
    }

    func testPendingSubmissionShadowsCanonicalMessagesWithoutChangingTheirOrder() {
        let userID = UUID()
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: userID, role: .user, text: "canonical user", createdAt: .distantPast),
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical partial", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(MacPendingSubmissionOverlay(
            submissionID: "submission-1",
            userMessage: MacChatMessage(id: userID, role: .user, text: "pending user", createdAt: .now),
            assistantMessage: MacChatMessage(id: assistantID, role: .assistant, text: "revealed text", createdAt: .now)
        ))

        let rendered = overlay.renderedMessages(canonical: canonical)

        XCTAssertEqual(rendered.map(\.id), [userID, assistantID])
        XCTAssertEqual(rendered.map(\.text), ["pending user", "revealed text"])
        XCTAssertEqual(canonical.map(\.text), ["canonical user", "canonical partial"])
    }

    func testCanonicalAcceptanceMovesOnlyAssistantPresentationIntoTransientState() {
        let userID = UUID()
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: userID, role: .user, text: "canonical user", createdAt: .distantPast),
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical final", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(MacPendingSubmissionOverlay(
            submissionID: "submission-2",
            userMessage: MacChatMessage(id: userID, role: .user, text: "pending user", createdAt: .now),
            assistantMessage: MacChatMessage(id: assistantID, role: .assistant, text: "paced partial", createdAt: .now)
        ))

        overlay.acceptCanonical(
            canonical,
            retainPendingAssistantPresentation: true
        )

        XCTAssertNil(overlay.pendingAssistantMessageID)
        XCTAssertEqual(
            overlay.renderedMessages(canonical: canonical).map(\.text),
            ["canonical user", "paced partial"]
        )
        XCTAssertEqual(canonical.last?.text, "canonical final")
    }

    func testCanonicalCompletionAfterRunFinishedCannotRestoreStalePresentationOverride() {
        let userID = UUID()
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: userID, role: .user, text: "canonical user", createdAt: .distantPast),
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical final", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        overlay.beginSubmission(MacPendingSubmissionOverlay(
            submissionID: "submission-finished-first",
            userMessage: MacChatMessage(id: userID, role: .user, text: "pending user", createdAt: .now),
            assistantMessage: MacChatMessage(id: assistantID, role: .assistant, text: "stale partial", createdAt: .now)
        ))

        overlay.acceptCanonical(
            canonical,
            retainPendingAssistantPresentation: false
        )

        XCTAssertTrue(overlay.isEmpty)
        XCTAssertEqual(overlay.renderedMessages(canonical: canonical), canonical)
    }

    func testDiscardingTransientMessageReturnsRenderingToCanonicalProjection() {
        let assistantID = UUID()
        let canonical = [
            MacChatMessage(id: assistantID, role: .assistant, text: "canonical final", createdAt: .distantPast),
        ]
        var overlay = MacConversationPresentationOverlay()
        XCTAssertTrue(overlay.mutateMessage(
            canonical: canonical,
            messageID: assistantID,
            createAssistantIfMissing: false
        ) { $0.text = "animated partial" })
        XCTAssertEqual(overlay.renderedMessages(canonical: canonical).first?.text, "animated partial")

        overlay.discardTransientMessage(assistantID)

        XCTAssertTrue(overlay.isEmpty)
        XCTAssertEqual(overlay.renderedMessages(canonical: canonical), canonical)
    }

    func testUnknownMessageCannotEnterPresentationWithoutExplicitAssistantShell() {
        var overlay = MacConversationPresentationOverlay()
        let unknownID = UUID()

        XCTAssertFalse(overlay.mutateMessage(
            canonical: [],
            messageID: unknownID,
            createAssistantIfMissing: false
        ) { $0.text = "forbidden" })
        XCTAssertTrue(overlay.isEmpty)

        XCTAssertTrue(overlay.mutateMessage(
            canonical: [],
            messageID: unknownID,
            createAssistantIfMissing: true
        ) { $0.text = "temporary shell" })
        XCTAssertEqual(
            overlay.renderedMessages(canonical: []).first?.role,
            .assistant
        )
    }
}
