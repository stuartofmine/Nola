import XCTest
import UIKit
@testable import Nola

final class ChatComposerPasteTests: XCTestCase {
    func testCopiedFinderURLBecomesAttachmentWithoutPathText() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        let source = URL(fileURLWithPath: "/tmp/copied-image.png")
        pasteboard.urls = [source]

        let urls = ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard) { _ in nil }

        XCTAssertEqual(urls, [source])
    }

    func testCopiedBitmapBecomesPNGAttachment() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        pasteboard.image = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).image { context in
            UIColor.systemBlue.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        }
        let expected = URL(fileURLWithPath: "/tmp/pasted.png")
        var received = Data()

        let urls = ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard) { data in
            received = data
            return expected
        }

        XCTAssertEqual(urls, [expected])
        XCTAssertFalse(received.isEmpty)
        XCTAssertEqual(received.prefix(8), Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]))
    }
}
