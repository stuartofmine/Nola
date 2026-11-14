// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacActivityReadOwnershipTests: XCTestCase {
    func testFilterAndManualRefreshNeverReuseAnotherQueriesCursorOrDoubleLoad() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = ActivityFixture()
        let host = NSHostingView(rootView: AppleActivityView(load: { try await fixture.load($0) },
            detail: { _ in nil }, openSource: { _ in }).environment(\.accessibilityEnabled, true)
            .environment(\.colorScheme, .light)
            .background(Color(nsColor: .windowBackgroundColor))
            .frame(width: 600, height: 560))
        let window = makeWindow(host)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await wait { fixture.queries.count == 1 && self.element(host, "activity.loadMore") != nil }
        let oldLoadMore = try XCTUnwrap(element(host, "activity.loadMore"))
        let agent = try XCTUnwrap(element(host, "activity.source.agent"))
        let refresh = try XCTUnwrap(element(host, "activity.refresh"))
        XCTAssertTrue(agent.accessibilityPerformPress?() ?? false)
        // These controls were captured from the previous frame. Their queued
        // actions still have to respect the current query, not the old cursor.
        _ = oldLoadMore.accessibilityPerformPress?()
        XCTAssertTrue(refresh.accessibilityPerformPress?() ?? false)
        try await wait { fixture.queries.count >= 2 }
        try await Task.sleep(for: .milliseconds(350))
        XCTAssertEqual(fixture.queries.count, 2)
        XCTAssertEqual(fixture.queries.last?.source, "agent")
        XCTAssertNil(fixture.queries.last?.cursor)
        XCTAssertNotNil(element(host, "activity.run.agent-result"))
        XCTAssertNil(element(host, "activity.run.all-result"))
        for scheme: ColorScheme in [.light, .dark] {
            host.rootView = AppleActivityView(load: { try await fixture.load($0) },
                detail: { _ in nil }, openSource: { _ in }).environment(\.accessibilityEnabled, true)
                .environment(\.colorScheme, scheme)
                .background(Color(nsColor: .windowBackgroundColor))
                .frame(width: 600, height: 560)
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            try await Task.sleep(for: .milliseconds(150))
            host.layoutSubtreeIfNeeded()
            XCTAssertNotNil(element(host, "activity.search"))
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let image = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])),
                uniformTypeIdentifier: "public.png")
            image.name = "activity-search-prompt-\(scheme == .dark ? "dark" : "light")"
            image.lifetime = .keepAlways
            add(image)
        }
    }

    func testRefreshFailureRetainsTheLastSuccessfulRowsAndCanRetry() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = ActivityFixture()
        let host = NSHostingView(rootView: AppleActivityView(load: { try await fixture.load($0) },
            detail: { _ in nil }, openSource: { _ in }).environment(\.accessibilityEnabled, true)
            .frame(width: 600, height: 560))
        let window = makeWindow(host)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await wait { self.element(host, "activity.run.all-result") != nil }
        fixture.failNext = true
        XCTAssertTrue(try XCTUnwrap(element(host, "activity.refresh")).accessibilityPerformPress?() ?? false)
        try await wait { fixture.queries.count == 2 }
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertNotNil(element(host, "activity.run.all-result"), "A read failure is not an empty activity history")
        XCTAssertTrue(try XCTUnwrap(element(host, "activity.refresh")).accessibilityPerformPress?() ?? false)
        try await wait { fixture.queries.count == 3 }
        XCTAssertNotNil(element(host, "activity.run.all-result"))
    }

    private func makeWindow<Content: View>(_ host: NSHostingView<Content>) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 560),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.orderFrontRegardless()
        return window
    }
    private func wait(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(4)
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertTrue(condition())
    }
    private func element(_ root: AnyObject, _ id: String) -> AnyObject? {
        var queue = [root], seen = Set<ObjectIdentifier>(), index = 0
        while index < queue.count && index < 4000 {
            let object = queue[index]; index += 1
            guard seen.insert(ObjectIdentifier(object)).inserted else { continue }
            if (object.accessibilityIdentifier?() ?? nil) == id { return object }
            let children = object.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = object as? NSView { queue += view.subviews }
        }
        return nil
    }
}

@MainActor private final class ActivityFixture {
    var queries: [AppleActivityQuery] = []
    var failNext = false
    private enum Failure: Error { case unavailable }
    func load(_ query: AppleActivityQuery) async throws -> AppleActivityPage {
        queries.append(query)
        if failNext { failNext = false; throw Failure.unavailable }
        let id = query.source == "agent" ? "agent-result" : "all-result"
        let cursor: Any = query.source == "agent" ? NSNull() : ["created_at_ms": 1, "run_id": id]
        return try JSONDecoder().decode(AppleActivityPage.self, from: JSONSerialization.data(withJSONObject: [
            "entries": [["run_id": id, "conversation_id": "conversation-" + id, "assistant_message_id": "assistant-" + id,
                "source": "agent", "title": id, "created_at_ms": 1, "completed_at_ms": 2,
                "status": "answered", "needs_response": false, "is_current": true]], "next_cursor": cursor]))
    }
}
