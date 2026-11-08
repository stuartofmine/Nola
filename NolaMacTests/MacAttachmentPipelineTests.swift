import AppKit
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacAttachmentPipelineTests: XCTestCase {
    func testExtractsDocxBodyAndSupplementalContent() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-docx-test-\(UUID().uuidString)", isDirectory: true)
        let package = root.appendingPathComponent("package", isDirectory: true)
        let word = package.appendingPathComponent("word", isDirectory: true)
        let archive = root.appendingPathComponent("report.docx")
        try FileManager.default.createDirectory(at: word, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        try writeXML("Header context", to: word.appendingPathComponent("header1.xml"))
        try writeXML("Body report", to: word.appendingPathComponent("document.xml"))
        try writeXML("Footnote detail", to: word.appendingPathComponent("footnotes.xml"))
        try writeXML("Endnote detail", to: word.appendingPathComponent("endnotes.xml"))
        try writeXML("Footer context", to: word.appendingPathComponent("footer1.xml"))
        try createZip(from: package, at: archive)

        let extraction = try await MacAttachmentPipeline().extract(archive)

        XCTAssertEqual(extraction.method, "Office Open XML")
        XCTAssertGreaterThan(extraction.byteCount, 0)
        XCTAssertTrue(extraction.text.contains("Header context"))
        XCTAssertTrue(extraction.text.contains("Body report"))
        XCTAssertTrue(extraction.text.contains("Footnote detail"))
        XCTAssertTrue(extraction.text.contains("Endnote detail"))
        XCTAssertTrue(extraction.text.contains("Footer context"))
    }

    func testNormalizesPlainTextWithoutDiscardingParagraphs() async throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-text-test-\(UUID().uuidString).txt")
        try "first   line\n\n\n\nsecond\tline".write(to: file, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: file) }

        let extraction = try await MacAttachmentPipeline().extract(file)

        XCTAssertEqual(extraction.method, "Text")
        XCTAssertEqual(extraction.text, "first line\n\nsecond line")
    }

    func testPreservesImageAboveLegacyEightMegabyteLimit() async throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-large-image-\(UUID().uuidString).png")
        let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: 1,
            pixelsHigh: 1,
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: 0,
            bitsPerPixel: 0
        )!
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: file)
        let handle = try FileHandle(forWritingTo: file)
        try handle.truncate(atOffset: 9 * 1_024 * 1_024)
        try handle.close()
        defer { try? FileManager.default.removeItem(at: file) }

        let extraction = try await MacAttachmentPipeline().extract(file)

        XCTAssertEqual(extraction.method, "Image")
        XCTAssertEqual(extraction.byteCount, 9 * 1_024 * 1_024)
    }

    func testRejectsBinaryDisguisedAsImage() async throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-invalid-image-\(UUID().uuidString).png")
        try Data("not an image".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }

        do {
            _ = try await MacAttachmentPipeline().extract(file)
            XCTFail("Expected invalid image content to be rejected")
        } catch MacAttachmentPipelineError.unreadable {
            // Expected: extension alone must not promote arbitrary bytes to model media.
        }
    }

    func testRejectsAttachmentAboveSharedThirtyMegabyteLimit() async throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-oversized-attachment-\(UUID().uuidString).png")
        try Data().write(to: file)
        let handle = try FileHandle(forWritingTo: file)
        try handle.truncate(atOffset: 30 * 1_024 * 1_024 + 1)
        try handle.close()
        defer { try? FileManager.default.removeItem(at: file) }

        do {
            _ = try await MacAttachmentPipeline().extract(file)
            XCTFail("Expected an attachment above 30 MB to be rejected")
        } catch MacAttachmentPipelineError.tooLarge(let byteCount) {
            XCTAssertEqual(byteCount, 30 * 1_024 * 1_024 + 1)
        }
    }

    private func writeXML(_ text: String, to url: URL) throws {
        let xml = "<w:document xmlns:w=\"urn:test\"><w:p><w:r><w:t>\(text)</w:t></w:r></w:p></w:document>"
        try xml.write(to: url, atomically: true, encoding: .utf8)
    }

    private func createZip(from directory: URL, at archive: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        process.arguments = ["-c", "-k", directory.path, archive.path]
        let errorPipe = Pipe()
        process.standardOutput = FileHandle.nullDevice
        process.standardError = errorPipe
        try process.run()
        process.waitUntilExit()
        if process.terminationStatus != 0 {
            let detail = String(decoding: errorPipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            XCTFail("Failed to create DOCX fixture: \(detail)")
        }
    }
}
