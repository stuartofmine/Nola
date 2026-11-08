import XCTest
@testable import Nola

@MainActor
final class MacAssistantPresentationSessionTests: XCTestCase {
    func testImmediateAppendDrainsOwnedPendingTextWithoutRewindingAcceptedCursor() {
        let session = MacAssistantPresentationSession(identitySeed: "message")
        let accepted = "# Heading\n\nA paragraph accepted before the synchronous adapter runs."
        _ = session.accept(authoritative: accepted)
        let old = session.beginReveal()
        _ = session.nextFrame(lease: old)
        XCTAssertNotEqual(session.visibleText, accepted)
        let suffix = "\n\nThe appended final paragraph."
        let frame = session.appendImmediately(suffix)
        XCTAssertEqual(frame.text, accepted + suffix)
        XCTAssertEqual(session.visibleText, session.acceptedText)
        XCTAssertEqual(session.assembler.rawText, accepted + suffix)
        XCTAssertEqual(session.pendingSegmentCount, 0)
        XCTAssertNil(session.nextFrame(lease: old))
        // Re-observing the same canonical value must not enqueue old segments.
        XCTAssertNil(session.accept(authoritative: accepted + suffix))
        session.flushPending()
        XCTAssertNil(session.nextFrame(lease: session.beginReveal()))
    }

    func testInterruptedRevealRetainsAllSegmentsForUnchangedSnapshot() {
        let text = (1...30).map { "Paragraph \($0) contains accepted but not yet visible text.\n" }.joined()
        let session = MacAssistantPresentationSession(identitySeed: "message")
        XCTAssertNil(session.accept(authoritative: text))
        session.flushPending()
        let old = session.beginReveal()
        XCTAssertNotNil(session.nextFrame(lease: old))
        let prefix = session.visibleText
        XCTAssertNotEqual(prefix, text)
        XCTAssertEqual(session.acceptedText, text)
        XCTAssertNil(session.accept(authoritative: text))
        let resumed = session.beginReveal()
        XCTAssertNil(session.nextFrame(lease: old))
        while session.nextFrame(lease: resumed) != nil {}
        XCTAssertEqual(session.visibleText, text)
        XCTAssertEqual(session.pendingSegmentCount, 0)
        XCTAssertTrue(session.visibleText.hasPrefix(prefix))
    }

    func testReplacementInvalidatesOldRevealWithoutAppendingItsSuffix() {
        let session = MacAssistantPresentationSession(identitySeed: "message")
        _ = session.accept(authoritative: "Original long answer with a queued suffix that must not return.")
        session.flushPending()
        let old = session.beginReveal()
        _ = session.nextFrame(lease: old)
        let replacement = session.accept(authoritative: "Corrected answer")
        XCTAssertEqual(replacement?.text, "Corrected answer")
        XCTAssertTrue(replacement?.replacesText == true)
        XCTAssertNil(session.nextFrame(lease: old))
        XCTAssertEqual(session.pendingSegmentCount, 0)
        XCTAssertEqual(session.visibleText, "Corrected answer")
        _ = session.accept(authoritative: "Corrected answer with continuation")
        session.flushPending()
        let next = session.beginReveal()
        while session.nextFrame(lease: next) != nil {}
        XCTAssertEqual(session.visibleText, "Corrected answer with continuation")
    }

    func testParserCompletionPreservesCommittedNodesAndCompletesOnlyOnce() {
        let session = MacAssistantPresentationSession(identitySeed: "message")
        _ = session.accept(authoritative: "# Heading\n\nFirst paragraph.\n\nUnfinished last line")
        session.flushPending()
        let lease = session.beginReveal()
        var committed: [MacAssistantTextNode] = []
        while let frame = session.nextFrame(lease: lease) { committed += frame.nodes }
        let tail = session.assembler.mutableTail
        XCTAssertFalse(committed.isEmpty)
        let completion = session.finishFrame(lease: lease)
        XCTAssertEqual(completion?.nodes.last?.id, tail?.id)
        XCTAssertTrue(completion?.isComplete == true)
        XCTAssertNil(session.finishFrame(lease: lease))
        XCTAssertEqual(session.assembler.rawText, session.visibleText)
    }

    func testCompletionCannotSkipPendingTextOrUseOldLease() {
        let session = MacAssistantPresentationSession(identitySeed: "message")
        _ = session.accept(authoritative: "A pending sentence that has not been revealed.")
        session.flushPending()
        let old = session.beginReveal()
        XCTAssertNil(session.finishFrame(lease: old))
        let current = session.beginReveal()
        while session.nextFrame(lease: current) != nil {}
        XCTAssertNil(session.finishFrame(lease: old))
        XCTAssertNotNil(session.finishFrame(lease: current))
    }
}
