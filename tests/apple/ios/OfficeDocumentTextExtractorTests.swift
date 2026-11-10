import XCTest
import UIKit
@testable import Nola

@MainActor
final class OfficeDocumentTextExtractorTests: XCTestCase {
    func testAttachmentMessagePayloadPreservesSpecialFilenamesAndItemAlignment() throws {
        let first = ChatAttachmentDraft(
            name: "roadmap\n\"final\".png",
            typeLabel: "PNG",
            sizeBytes: 9_437_184,
            extractedContent: "",
            isImage: true,
            contentDigest: "missing-first"
        )
        let second = ChatAttachmentDraft(
            name: "预算 2026.xlsx",
            typeLabel: "XLSX",
            sizeBytes: 81_920,
            extractedContent: "sheet",
            contentDigest: "missing-second"
        )

        let encoded = try XCTUnwrap(ChatAttachmentMessagePayload.encode([first, second]))
        let decoded = ChatAttachmentMessagePayload.decode(from: [
            ChatAttachmentMessagePayload.metadataKey: encoded
        ])

        XCTAssertEqual(decoded.map(\.name), [first.name, second.name])
        XCTAssertEqual(decoded.map(\.sizeBytes), [Int64(first.sizeBytes), Int64(second.sizeBytes)])
        XCTAssertEqual(decoded.map(\.url), [nil, nil])
    }

    func testAttachmentMessagePayloadRejectsMalformedOrUnknownVersions() throws {
        XCTAssertTrue(ChatAttachmentMessagePayload.decode(from: [
            ChatAttachmentMessagePayload.metadataKey: "not-json"
        ]).isEmpty)

        let futurePayload = "{\"version\":2,\"items\":[{\"name\":\"a.png\",\"url\":null,\"sizeBytes\":1}]}"
        XCTAssertTrue(ChatAttachmentMessagePayload.decode(from: [
            ChatAttachmentMessagePayload.metadataKey: futurePayload
        ]).isEmpty)
    }

    func testExtractsBodyTableAndHeaderFromDOCXArchive() throws {
        let documentXML = """
        <?xml version="1.0" encoding="UTF-8"?>
        <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:body>
            <w:p><w:r><w:t>First paragraph</w:t></w:r></w:p>
            <w:tbl><w:tr>
              <w:tc><w:p><w:r><w:t>Cell A</w:t></w:r></w:p></w:tc>
              <w:tc><w:p><w:r><w:t>Cell B</w:t></w:r></w:p></w:tc>
            </w:tr></w:tbl>
          </w:body>
        </w:document>
        """
        let headerXML = """
        <?xml version="1.0" encoding="UTF-8"?>
        <w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:p><w:r><w:t>Report header</w:t></w:r></w:p>
        </w:hdr>
        """
        let archive = makeStoredZIP(entries: [
            ("word/document.xml", Data(documentXML.utf8)),
            ("word/header1.xml", Data(headerXML.utf8))
        ])

        let text = try OfficeDocumentTextExtractor.extractDOCX(from: archive)

        XCTAssertTrue(text.contains("First paragraph"))
        XCTAssertTrue(text.contains("Cell A"))
        XCTAssertTrue(text.contains("Cell B"))
        XCTAssertTrue(text.contains("Report header"))
    }

    func testRejectsArchiveWithoutDocumentBody() {
        let archive = makeStoredZIP(entries: [("word/styles.xml", Data("<styles/>".utf8))])
        XCTAssertThrowsError(try OfficeDocumentTextExtractor.extractDOCX(from: archive))
    }

    func testChatAttachmentPipelineImportsDOCXThroughTheUserUploadPath() async throws {
        let documentXML = """
        <?xml version="1.0" encoding="UTF-8"?>
        <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:body><w:p><w:r><w:t>Quarterly product roadmap</w:t></w:r></w:p></w:body>
        </w:document>
        """
        let archive = makeStoredZIP(entries: [("word/document.xml", Data(documentXML.utf8))])
        let url = try temporaryFile(named: "roadmap.docx", contents: archive)
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let result = try await ChatAttachmentPipeline.importFiles(from: .success([url]), parserHost: nil)
        let attachment = try XCTUnwrap(result.attachments.first)

        XCTAssertEqual(result.failedCount, 0)
        XCTAssertEqual(result.attachments.count, 1)
        XCTAssertTrue(attachment.extractedContent.contains("Quarterly product roadmap"))
        XCTAssertNil(attachment.importWarning)
        let storedURL = try XCTUnwrap(attachment.localURL)
        defer { try? FileManager.default.removeItem(at: storedURL.deletingLastPathComponent()) }
        XCTAssertEqual(try Data(contentsOf: storedURL), archive)
    }

    func testChatAttachmentPipelineImportsSelectablePDFText() async throws {
        let renderer = UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 612, height: 792))
        let pdf = renderer.pdfData { context in
            context.beginPage()
            NSString(string: "Nola attachment pipeline PDF text").draw(
                at: CGPoint(x: 48, y: 64),
                withAttributes: [.font: UIFont.systemFont(ofSize: 18)]
            )
        }
        let url = try temporaryFile(named: "report.pdf", contents: pdf)
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let result = try await ChatAttachmentPipeline.importFiles(from: .success([url]), parserHost: nil)
        let attachment = try XCTUnwrap(result.attachments.first)

        XCTAssertEqual(result.failedCount, 0)
        XCTAssertTrue(attachment.extractedContent.contains("Nola attachment pipeline PDF text"))
        XCTAssertNil(attachment.importWarning)
        let storedURL = try XCTUnwrap(attachment.localURL)
        defer { try? FileManager.default.removeItem(at: storedURL.deletingLastPathComponent()) }
        XCTAssertEqual(try Data(contentsOf: storedURL), pdf)
    }

    func testChatAttachmentPipelineRejectsFilesAboveThe30MBLimitBeforeReading() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-attachment-limit-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("oversized.bin")
        FileManager.default.createFile(atPath: url.path, contents: Data())
        let handle = try FileHandle(forWritingTo: url)
        try handle.truncate(atOffset: UInt64(30 * 1024 * 1024 + 1))
        try handle.close()

        let result = try await ChatAttachmentPipeline.importFiles(from: .success([url]), parserHost: nil)

        XCTAssertTrue(result.attachments.isEmpty)
        XCTAssertEqual(result.failedCount, 1)
    }

    func testChatAttachmentPipelineRejectsInvalidSmallImagePayload() async throws {
        let url = try temporaryFile(
            named: "declared-image.png",
            contents: Data("this is not image data".utf8)
        )
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let result = try await ChatAttachmentPipeline.importFiles(
            from: .success([url]),
            parserHost: nil
        )

        XCTAssertTrue(result.attachments.isEmpty)
        XCTAssertEqual(result.failedCount, 1)
    }

    private func temporaryFile(named name: String, contents: Data) throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-attachment-test-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let url = root.appendingPathComponent(name)
        try contents.write(to: url, options: .atomic)
        return url
    }

    private func makeStoredZIP(entries: [(String, Data)]) -> Data {
        var archive = Data()
        var centralDirectory = Data()

        for (name, payload) in entries {
            let nameData = Data(name.utf8)
            let localOffset = UInt32(archive.count)

            archive.appendLE(UInt32(0x0403_4B50))
            archive.appendLE(UInt16(20))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt16(0))
            archive.appendLE(UInt32(0))
            archive.appendLE(UInt32(payload.count))
            archive.appendLE(UInt32(payload.count))
            archive.appendLE(UInt16(nameData.count))
            archive.appendLE(UInt16(0))
            archive.append(nameData)
            archive.append(payload)

            centralDirectory.appendLE(UInt32(0x0201_4B50))
            centralDirectory.appendLE(UInt16(20))
            centralDirectory.appendLE(UInt16(20))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt32(0))
            centralDirectory.appendLE(UInt32(payload.count))
            centralDirectory.appendLE(UInt32(payload.count))
            centralDirectory.appendLE(UInt16(nameData.count))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt16(0))
            centralDirectory.appendLE(UInt32(0))
            centralDirectory.appendLE(localOffset)
            centralDirectory.append(nameData)
        }

        let centralOffset = UInt32(archive.count)
        archive.append(centralDirectory)
        archive.appendLE(UInt32(0x0605_4B50))
        archive.appendLE(UInt16(0))
        archive.appendLE(UInt16(0))
        archive.appendLE(UInt16(entries.count))
        archive.appendLE(UInt16(entries.count))
        archive.appendLE(UInt32(centralDirectory.count))
        archive.appendLE(centralOffset)
        archive.appendLE(UInt16(0))
        return archive
    }
}

private extension Data {
    mutating func appendLE(_ value: UInt16) {
        append(UInt8(value & 0x00FF))
        append(UInt8((value >> 8) & 0x00FF))
    }

    mutating func appendLE(_ value: UInt32) {
        append(UInt8(value & 0x0000_00FF))
        append(UInt8((value >> 8) & 0x0000_00FF))
        append(UInt8((value >> 16) & 0x0000_00FF))
        append(UInt8((value >> 24) & 0x0000_00FF))
    }
}
