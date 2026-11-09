// SPDX-License-Identifier: Apache-2.0
import AppKit
import Combine
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacStreamingAppendRevealTests: XCTestCase {
    func testLiveMarkdownAccessibilityTracksEveryAppendAndFinalMarker() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let state = AppendRevealFixture(kind: .markdownBlock, text: "1. First", suppressed: false)
        let hosting = NSHostingView(rootView: AppendRevealRoot(state: state)
            .environment(\.accessibilityEnabled, true))
        hosting.frame = NSRect(x: 0, y: 0, width: 780, height: 360)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting, milliseconds: 100)
        XCTAssertTrue(accessibleText(hosting).contains("First"))
        state.text = "1. First item has appended words.\n2. Second"
        try await settle(hosting, milliseconds: 100)
        XCTAssertTrue(accessibleText(hosting).contains("First item has appended words."))
        XCTAssertTrue(accessibleText(hosting).contains("Second"))
        state.text += " item has its complete suffix.\n\nNOLA-STREAM-END"
        state.contentIsFinal = true
        try await settle(hosting, milliseconds: 450)
        let completed = accessibleText(hosting)
        XCTAssertTrue(completed.contains("Second item has its complete suffix."), completed)
        XCTAssertTrue(completed.contains("NOLA-STREAM-END"), completed)
    }

    func testCompletedSuffixStaysVisiblyCompleteAfterRevealSettles() async throws {
        let state = AppendRevealFixture(kind: .phrase, text: "REEDITED=32", suppressed: false)
        let hosting = NSHostingView(rootView: AppendRevealRoot(state: state))
        hosting.frame = NSRect(x: 0, y: 0, width: 500, height: 100)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = hosting; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting, milliseconds: 600)
        let before = try bitmap(hosting)
        state.text += "3"
        state.contentIsFinal = true
        try await settle(hosting, milliseconds: Int((AppleStreamingRevealTiming.duration + 0.2) * 1000))
        let completed = try bitmap(hosting)
        try await settle(hosting, milliseconds: 450)
        let later = try bitmap(hosting)
        XCTAssertGreaterThan(brightPixels(completed), brightPixels(before) + 5,
                             "The final glyph must actually be visible, not only accessible")
        XCTAssertEqual(changedPixels(completed, later), 0,
                       "A settled terminal response must not keep changing or lose its final glyph")
    }

    private func accessibleText(_ root: NSView) -> String {
        var queue: [AnyObject] = [root]
        var visited = Set<ObjectIdentifier>()
        var values: [String] = []
        var cursor = 0
        while cursor < queue.count && visited.count < 4_000 {
            let object = queue[cursor]
            cursor += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            if let label = object.accessibilityLabel?() ?? nil { values.append(label) }
            let rawValue: Any? = object.accessibilityValue?() ?? nil
            if let value = rawValue { values.append(String(describing: value)) }
            let raw: [Any] = (object.accessibilityChildren?() ?? nil) ?? []
            queue.append(contentsOf: (raw + NSAccessibility.unignoredChildren(from: raw)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return values.joined(separator: "\n")
    }

    func testStableMutableIdentityRevealsOnlyAppendedCharactersAndFinalSuffix() {
        let now = Date(timeIntervalSince1970: 100)
        var state = AppleInlineAppendRevealState(text: "Visible", streaming: true, visiblePrefixCount: 7, at: now)
        state = state.updating(text: "Visible first", streaming: true, reduceMotion: false, at: now)
        let first = state.segments[0]
        XCTAssertEqual(first.range, 7..<13)
        state = state.updating(text: "Visible first final", streaming: false, reduceMotion: false,
                               at: now.addingTimeInterval(0.05))
        XCTAssertEqual(state.segments[0].id, first.id)
        XCTAssertEqual(state.segments[0].startedAt, first.startedAt)
        XCTAssertEqual(state.segments[1].range, 13..<19)
        XCTAssertTrue(state.segments.allSatisfy { $0.range.lowerBound >= 7 })
        let settled = state.updating(text: state.text, streaming: false, reduceMotion: false,
                                     at: now.addingTimeInterval(1))
        XCTAssertTrue(settled.segments.isEmpty)
        let oldTail = AppleStreamingRevealTiming.remaining(elapsed: 0.25, position: 1)
        let newHead = AppleStreamingRevealTiming.remaining(elapsed: 0.20, position: 0, precedingElapsed: 0.25)
        XCTAssertGreaterThanOrEqual(newHead, oldTail, "A later batch cannot appear ahead of the preceding tail")
        XCTAssertEqual(AppleStreamingRevealTiming.remaining(elapsed: 0.53, position: 1, precedingElapsed: 0.58), 0)
        XCTAssertEqual(AppleStreamingRevealTiming.remaining(elapsed: 0.20, position: 1,
            completionElapsed: 0.16, precedingElapsed: 0.25), 0)
    }

    func testHistoryReplacementAndReducedMotionNeverReplayExistingText() {
        let now = Date(timeIntervalSince1970: 100)
        let history = AppleInlineAppendRevealState(text: "Past", streaming: false, at: now)
        XCTAssertTrue(history.segments.isEmpty)
        let replacement = history.updating(text: "Replacement", streaming: true, reduceMotion: false, at: now)
        XCTAssertTrue(replacement.segments.isEmpty)
        let motion = replacement.updating(text: "Replacement new", streaming: true, reduceMotion: true, at: now)
        XCTAssertTrue(motion.segments.isEmpty)
        XCTAssertTrue(motion.updating(text: motion.text, streaming: true, reduceMotion: false, at: now).segments.isEmpty)
    }

    func testHostedStructuredAppendAndTerminalFramesRevealWithoutMovingPrefix() async throws {
        try await checkFrames(kind: .markdownBlock, prefix: "- OLD PREFIX ", suffix: "new words rise into clarity", final: false)
        try await checkFrames(kind: .markdownBlock, prefix: "- OLD PREFIX ", suffix: "last words arrive at completion", final: true)
    }

    func testHostedSamePhraseWithMathStillRevealsAppendedProse() async throws {
        try await checkFrames(kind: .phrase, prefix: "OLD $x^2$ ", suffix: "new words rise after the equation", final: false)
    }

    func testHostedSuppressedRevealPaintsFinalGlyphsImmediately() async throws {
        try await checkFrames(kind: .markdownBlock, prefix: "- OLD PREFIX ", suffix: "immediate accessible text", final: false, suppressed: true)
    }

    func testLiveFinalUnmatchedMarkdownMatchesReopenedHistory() async throws {
        for kind in [MacAssistantTextNode.Kind.phrase, .markdownBlock] {
            let source = kind == .phrase ? "Prefix **unfinished" : "- Prefix **unfinished"
            let state = AppendRevealFixture(kind: kind, text: source, suppressed: false)
            let hosting = NSHostingView(rootView: AppendRevealRoot(state: state))
            hosting.frame = NSRect(x: 0, y: 0, width: 780, height: 180)
            let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await settle(hosting, milliseconds: 430)
            let provisional = try bitmap(hosting)
            // Match Journal: the owner has streamed, but syntax is now final.
            // No transport-animation flag is repurposed as parser finality.
            state.contentIsFinal = true
            try await settle(hosting, milliseconds: 430)
            let completed = try bitmap(hosting)
            let history = AppendRevealFixture(kind: kind, text: source, suppressed: false)
            history.streaming = false
            history.contentIsFinal = true
            hosting.rootView = AppendRevealRoot(state: history)
            try await settle(hosting, milliseconds: 430)
            let reopened = try bitmap(hosting)
            XCTAssertGreaterThan(brightPixels(completed), 80, "Completed text must remain visibly drawn.")
            XCTAssertGreaterThan(changedPixels(provisional, completed), 40,
                                 "Finality must stop repairing the unmatched emphasis marker.")
            XCTAssertEqual(changedPixels(completed, reopened), 0,
                           "A live-completed response must use the same Markdown interpretation as history.")
        }
    }

    private func checkFrames(kind: MacAssistantTextNode.Kind, prefix: String, suffix: String,
                             final: Bool, suppressed: Bool = false) async throws {
        let state = AppendRevealFixture(kind: kind, text: prefix, suppressed: suppressed)
        let hosting = NSHostingView(rootView: AppendRevealRoot(state: state))
        hosting.frame = NSRect(x: 0, y: 0, width: 780, height: 180)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        // The reference prefix must finish its initial 0.52 s ink reveal.
        // A 0.43 s reference still contains a fading final math glyph.
        try await settle(hosting, milliseconds: Int((AppleStreamingRevealTiming.duration + 0.2) * 1000))
        let before = try bitmap(hosting)
        let size = hosting.fittingSize
        state.text += suffix
        if final { state.contentIsFinal = true }
        try await settle(hosting, milliseconds: 35)
        let active = try bitmap(hosting)
        let activeSize = hosting.fittingSize
        try await settle(hosting, milliseconds: 400)
        let settled = try bitmap(hosting)
        XCTAssertEqual(activeSize, hosting.fittingSize, "Drawing-only reveal must not change the laid-out size.")
        XCTAssertEqual(size.height, hosting.fittingSize.height, "The one-line fixture must not reflow.")
        XCTAssertEqual(changedPixels(before, active, maxX: 90), 0, "Already-visible prefix must remain pixel stable.")
        XCTAssertEqual(changedPixels(before, settled, maxX: 90), 0, "Completion must not erase the visible prefix.")
        XCTAssertGreaterThan(brightPixels(settled), brightPixels(before) + 40,
                             "Completed suffix must add visible text, not merely differ by becoming blank.")
        let changed = changedPixels(active, settled)
        if suppressed { XCTAssertEqual(changed, 0) }
        else { XCTAssertGreaterThan(changed, 40, "New glyphs must visibly progress through reveal frames.") }
        for (name, image) in [("active", active), ("settled", settled)] {
            let data = try XCTUnwrap(image.representation(using: .png, properties: [:]))
            let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
            attachment.name = "append-reveal-\(kind)-\(final)-\(suppressed)-\(name)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }

    private func settle(_ view: NSView, milliseconds: Int) async throws {
        view.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(milliseconds))
        view.layoutSubtreeIfNeeded()
        view.displayIfNeeded()
    }

    private func bitmap(_ view: NSView) throws -> NSBitmapImageRep {
        let image = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: image)
        return image
    }

    private func changedPixels(_ lhs: NSBitmapImageRep, _ rhs: NSBitmapImageRep, maxX: Int? = nil) -> Int {
        var changed = 0
        let end = min(maxX ?? lhs.pixelsWide, lhs.pixelsWide)
        for y in 0..<min(lhs.pixelsHigh, rhs.pixelsHigh) {
            for x in 0..<end {
                guard let a = lhs.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB),
                      let b = rhs.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                if abs(a.redComponent - b.redComponent) + abs(a.greenComponent - b.greenComponent)
                    + abs(a.blueComponent - b.blueComponent) > 0.02 { changed += 1 }
            }
        }
        return changed
    }

    private func brightPixels(_ image: NSBitmapImageRep) -> Int {
        var count = 0
        for y in stride(from: 0, to: image.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: image.pixelsWide, by: 2) {
                guard let color = image.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                if min(color.redComponent, color.greenComponent, color.blueComponent) > 0.35 { count += 1 }
            }
        }
        return count
    }
}

@MainActor
private final class AppendRevealFixture: ObservableObject {
    let id = UUID()
    let blockID = UUID()
    let kind: MacAssistantTextNode.Kind
    let suppressed: Bool
    @Published var text: String
    @Published var streaming = true
    @Published var contentIsFinal = false
    init(kind: MacAssistantTextNode.Kind, text: String, suppressed: Bool) {
        self.kind = kind
        self.text = text
        self.suppressed = suppressed
    }
}

private struct AppendRevealRoot: View {
    @ObservedObject var state: AppendRevealFixture
    var body: some View {
        MacAssistantPresentationView(nodes: [MacAssistantTextNode(id: state.id, blockID: state.blockID,
                                                                 kind: state.kind, text: state.text)],
                                     animateNewNodes: state.streaming, contentIsFinal: state.contentIsFinal,
                                     suppressReveal: state.suppressed)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .foregroundStyle(.white).background(.black).preferredColorScheme(.dark)
    }
}
