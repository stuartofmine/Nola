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
    func testCopiedWebLinkRemainsTextInsteadOfBecomingAnAttachment() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        let link = try XCTUnwrap(URL(string: "https://example.invalid/article"))
        pasteboard.urls = [link]
        XCTAssertNil(ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard) { _ in
            XCTFail("A web link must not create an image file"); return nil
        })
        XCTAssertEqual(pasteboard.url, link)
    }

    func testImageWriteFailureReportsErrorAndPreventsSilentFallback() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        pasteboard.image = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).image { context in
            UIColor.systemBlue.setFill(); context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        }
        var failure: Error?
        let urls = ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard,
            onFailure: { failure = $0 }, writeImage: { _ in nil })
        XCTAssertEqual(urls, [])
        XCTAssertEqual((failure as NSError?)?.code, CocoaError.fileWriteUnknown.rawValue)
    }

    func testContextTextImportPreservesWhitespaceAndRejectsOversizedOrInvalidText() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("skill.md")
        let exact = "  # Skill\n\n    indented workflow\n\n"
        try Data(exact.utf8).write(to: file)
        XCTAssertEqual(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 40_000), exact)
        try Data(repeating: 0x61, count: 40_001).write(to: file)
        XCTAssertThrowsError(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 40_000))
        try Data(repeating: 0x61, count: 192_001).write(to: file)
        XCTAssertThrowsError(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 200_000))
        try Data([0xFF, 0xFE, 0xFF]).write(to: file)
        XCTAssertThrowsError(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 40_000))
        XCTAssertTrue(IOSContextFileImportPolicy.isCancellation(CocoaError(.userCancelled)))
        XCTAssertFalse(IOSContextFileImportPolicy.isCancellation(CocoaError(.fileReadNoPermission)))
    }

}
