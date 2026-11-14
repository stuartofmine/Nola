// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacProcessingMotionTests: XCTestCase {
    func testHighlightMovesWithoutMovingTextAndWrapsOutsideGlyphs() async throws {
        for scheme in [ColorScheme.light, .dark] {
            let start = try await frame(phase: 0, highlight: true, scheme: scheme)
            let middle = try await frame(phase: 0.5, highlight: true, scheme: scheme)
            let end = try await frame(phase: 1, highlight: true, scheme: scheme)
            let staticFrame = try await frame(phase: 0.5, highlight: false, scheme: scheme)
            XCTAssertEqual(start.pixels, end.pixels, "The loop must wrap while the highlight is outside the glyphs")
            XCTAssertEqual(start.pixels, staticFrame.pixels, "Readable base text remains beneath the moving highlight")
            XCTAssertNotEqual(middle.pixels, staticFrame.pixels)
            XCTAssertEqual(start.size, middle.size, "Animation must never resize the status row")
        }
    }

    func testActualTimelineFlowsOnlyWhileActiveAndMotionIsAllowed() async throws {
        for (active, reduced, running) in [(true, false, true), (true, true, true), (false, false, true), (true, false, false)] {
            let host = NSHostingView(rootView:
                MacStatusTimelineText(text: Text("正在处理中"), isRunning: running,
                    isActive: active, reduceMotion: reduced)
                    .font(.system(size: 18)).padding(20).frame(width: 260, height: 80))
            let window = mount(host)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await Task.sleep(for: .milliseconds(120))
            let first = try pixels(host)
            var changed = false
            for _ in 0..<5 {
                try await Task.sleep(for: .milliseconds(220))
                if try pixels(host) != first { changed = true }
            }
            XCTAssertEqual(changed, active && !reduced && running,
                "Reduced motion, background and settled status must remain static")
        }
    }

    private func frame(phase: Double, highlight: Bool, scheme: ColorScheme) async throws -> (pixels: Data, size: NSSize) {
        let host = NSHostingView(rootView: MacStatusTextFrame(text: Text("正在处理中"), phase: phase, showsHighlight: highlight)
            .font(.system(size: 18)).padding(20).background(scheme == .dark ? Color.black : .white)
            .environment(\.colorScheme, scheme).frame(width: 260, height: 80))
        let window = mount(host)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await Task.sleep(for: .milliseconds(60))
        let data = try pixels(host)
        if phase == 0.5 && highlight {
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to:
                URL(fileURLWithPath: "/tmp/nola-processing-\(scheme == .dark ? "dark" : "light").png"))
        }
        return (data, host.fittingSize)
    }

    private func mount(_ host: NSView) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 260, height: 80),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        return window
    }

    private func pixels(_ host: NSView) throws -> Data {
        host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        return Data(bytes: try XCTUnwrap(bitmap.bitmapData), count: bitmap.bytesPerRow * bitmap.pixelsHigh)
    }
}
