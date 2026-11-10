// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacReplyDisclosureMotionTests: XCTestCase {
    func testProcessBodyHasIntermediateFramesAndRapidReversalRemainsUsable() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = VStack(alignment: .leading, spacing: 0) {
            MacJournalProcessView(partIDs: ["fixture-reasoning"], delivery: .completed,
                requiresAttention: false, toolCount: 0, attentionState: nil,
                hasLiveReasoning: false, reasoningOnly: true) {
                Rectangle().fill(.white).frame(width: 180, height: 100)
            }
            Spacer(minLength: 0)
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(.black).environment(\.colorScheme, .dark).environment(\.accessibilityEnabled, true)
        let host = NSHostingView(rootView: root)
        host.frame = NSRect(x: 0, y: 0, width: 480, height: 240)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(host, 250)
        let closed = try energy(host)
        func toggle() throws {
            let button = try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == "chat.journal.process.toggle" })
            XCTAssertTrue(button.accessibilityPerformPress?() ?? false)
        }
        try toggle(); try await settle(host, 45)
        let active = try energy(host)
        try await settle(host, 260)
        let opened = try energy(host)
        XCTAssertGreaterThan(opened, closed + 1_000_000)
        if !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            XCTAssertGreaterThan(active, closed + 1_000)
            XCTAssertLessThan(active, opened - 1_000, "The actual process body must have an intermediate drawing frame")
        }
        try toggle(); try await settle(host, 45); try toggle(); try await settle(host, 45)
        try toggle(); try await settle(host, 260)
        XCTAssertLessThan(try energy(host), closed + 100_000, "Rapid reversal must finish collapsed without residual content")
        try toggle(); try await settle(host, 260)
        XCTAssertGreaterThan(try energy(host), closed + 1_000_000, "The control must still reopen")
    }

    private func settle(_ view: NSView, _ milliseconds: Int) async throws {
        try await Task.sleep(for: .milliseconds(milliseconds))
        view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
    }
    private func energy(_ view: NSView) throws -> Int {
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        let bytes = try XCTUnwrap(bitmap.bitmapData)
        var total = 0
        for y in 0..<bitmap.pixelsHigh { for x in 0..<bitmap.pixelsWide {
            let offset = y * bitmap.bytesPerRow + x * bitmap.samplesPerPixel
            total += Int(bytes[offset]) + Int(bytes[offset + 1]) + Int(bytes[offset + 2])
        } }
        return total
    }
    private func objects(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], visited = Set<ObjectIdentifier>(), result: [AnyObject] = []
        var index = 0
        while index < queue.count && result.count < 4_000 {
            let item = queue[index]; index += 1
            guard visited.insert(ObjectIdentifier(item)).inserted else { continue }
            result.append(item)
            let children = item.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = item as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }
}
