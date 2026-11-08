// SPDX-License-Identifier: Apache-2.0
import AppKit
import PDFKit
import XCTest
@testable import Nola

@MainActor final class MacPDFViewportTests: XCTestCase {
    func testNativePDFRestoresPageScaleAndDestinationForExactOwner() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let document = PDFDocument()
        for index in 0..<6 {
            let image = NSImage(size: NSSize(width: 400, height: 700), flipped: false) { rect in
                NSColor.white.setFill(); NSBezierPath(rect: rect).fill()
                ("Page \(index + 1)" as NSString).draw(at: NSPoint(x: 30, y: 620),
                    withAttributes: [.font: NSFont.systemFont(ofSize: 30), .foregroundColor: NSColor.black])
                return true
            }
            document.insert(try XCTUnwrap(PDFPage(image: image)), at: index)
        }
        let url = directory.appendingPathComponent("reading.pdf")
        XCTAssertTrue(document.write(to: url))
        let positions = MacDetailScrollPositions()
        let key = "conversation-a/file-a"
        let original = try XCTUnwrap(MacNativeDocumentPreview.makePreview(for: url) as? PDFView)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 460),
            styleMask: [.titled], backing: .buffered, defer: false)
        defer { window.orderOut(nil); window.close() }
        window.isReleasedWhenClosed = false
        window.contentView = original; window.orderFront(nil)
        let first = MacNativeDocumentPreview.Coordinator()
        first.attach(original, key: key, positions: positions, request: first.requestAttachment(original, key: key, replacingDocument: false))
        original.autoScales = false; original.scaleFactor = 1.2
        original.go(to: PDFDestination(page: try XCTUnwrap(original.document?.page(at: 3)), at: NSPoint(x: 20, y: 570)))
        try await Task.sleep(for: .milliseconds(100))
        first.capture()
        let saved = try XCTUnwrap(positions.pdfPosition(for: key))
        XCTAssertEqual(saved.pageIndex, 3)
        XCTAssertFalse(saved.autoScales)
        XCTAssertEqual(saved.scaleFactor, 1.2, accuracy: 0.01)
        first.detach()
        let restored = try XCTUnwrap(MacNativeDocumentPreview.makePreview(for: url) as? PDFView)
        window.contentView = restored; restored.layoutSubtreeIfNeeded()
        let second = MacNativeDocumentPreview.Coordinator()
        second.attach(restored, key: key, positions: positions, request: second.requestAttachment(restored, key: key, replacingDocument: false))
        defer { second.detach() }
        try await Task.sleep(for: .milliseconds(100))
        let destination = try XCTUnwrap(restored.currentDestination)
        XCTAssertEqual(restored.document?.index(for: try XCTUnwrap(destination.page)), saved.pageIndex)
        XCTAssertEqual(restored.scaleFactor, saved.scaleFactor, accuracy: 0.01)
        XCTAssertEqual(destination.point.y, saved.point.y, accuracy: 3)
        XCTAssertNil(positions.pdfPosition(for: "conversation-b/file-a"))
        XCTAssertNil(positions.pdfPosition(for: "conversation-a/file-b"))
        // Two scope updates can queue before PDFKit's layout callback. Only
        // the last request may bind; changing the document must first capture
        // the old document into its own key.
        let staleRequest = second.requestAttachment(restored, key: "owner-b", replacingDocument: true)
        let replacement = PDFDocument()
        replacement.insert(try XCTUnwrap(document.page(at: 0)?.copy() as? PDFPage), at: 0)
        restored.document = replacement
        let latestRequest = second.requestAttachment(restored, key: "owner-c", replacingDocument: false)
        second.attach(restored, key: "owner-b", positions: positions, request: staleRequest)
        second.attach(restored, key: "owner-c", positions: positions, request: latestRequest)
        restored.layoutSubtreeIfNeeded()
        second.capture()
        XCTAssertEqual(positions.pdfPosition(for: key)?.pageIndex, saved.pageIndex)
        XCTAssertNil(positions.pdfPosition(for: "owner-b"))
        XCTAssertEqual(positions.pdfPosition(for: "owner-c")?.pageIndex, 0)
    }

    func testPDFViewportRejectsInvalidValuesWithoutReplacingReadingState() {
        let positions = MacDetailScrollPositions()
        let valid = MacDetailScrollPositions.PDFViewport(pageIndex: 4, point: NSPoint(x: 1, y: 250), scaleFactor: 1.5, autoScales: false)
        positions.savePDF(valid, for: "owner/file")
        positions.savePDF(.init(pageIndex: -1, point: .zero, scaleFactor: 1, autoScales: false), for: "owner/file")
        positions.savePDF(.init(pageIndex: 1, point: NSPoint(x: CGFloat.nan, y: 0), scaleFactor: 1, autoScales: false), for: "owner/file")
        positions.savePDF(.init(pageIndex: 1, point: .zero, scaleFactor: 0, autoScales: false), for: "owner/file")
        XCTAssertEqual(positions.pdfPosition(for: "owner/file"), valid)
    }
}
