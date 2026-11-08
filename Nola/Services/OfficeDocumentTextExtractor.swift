// SPDX-License-Identifier: Apache-2.0
// Local Office Open XML text extraction without a host dependency.

import Compression
import Foundation

enum OfficeDocumentTextExtractor {
    static func extractDOCX(from data: Data) throws -> String {
        let archive = try OpenXMLArchive(data: data)
        let preferredParts = [
            "word/document.xml",
            "word/footnotes.xml",
            "word/endnotes.xml",
            "word/comments.xml"
        ]
        let supplementalParts = archive.entryNames
            .filter {
                ($0.hasPrefix("word/header") || $0.hasPrefix("word/footer")) && $0.hasSuffix(".xml")
            }
            .sorted()
        let parts = preferredParts + supplementalParts
        var sections: [String] = []

        for part in parts where archive.contains(part) {
            let xml = try archive.data(for: part)
            let parser = WordprocessingMLTextParser(data: xml)
            let text = try parser.parse()
            if !text.isEmpty {
                sections.append(text)
            }
        }

        let result = sections.joined(separator: "\n\n")
            .replacingOccurrences(of: "\n{3,}", with: "\n\n", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !result.isEmpty else { throw OfficeExtractionError.noReadableText }
        return result
    }
}

private struct OpenXMLArchive {
    private struct Entry {
        let compressionMethod: UInt16
        let compressedSize: Int
        let uncompressedSize: Int
        let localHeaderOffset: Int
        let encrypted: Bool
    }

    private let archiveData: Data
    private let entries: [String: Entry]

    init(data: Data) throws {
        archiveData = data
        guard let eocdOffset = data.lastIndex(ofLittleEndianUInt32: 0x0605_4B50) else {
            throw OfficeExtractionError.invalidArchive
        }
        let entryCount = Int(try data.littleEndianUInt16(at: eocdOffset + 10))
        var offset = Int(try data.littleEndianUInt32(at: eocdOffset + 16))
        var parsed: [String: Entry] = [:]

        for _ in 0..<entryCount {
            guard try data.littleEndianUInt32(at: offset) == 0x0201_4B50 else {
                throw OfficeExtractionError.invalidArchive
            }
            let flags = try data.littleEndianUInt16(at: offset + 8)
            let method = try data.littleEndianUInt16(at: offset + 10)
            let compressedSize = Int(try data.littleEndianUInt32(at: offset + 20))
            let uncompressedSize = Int(try data.littleEndianUInt32(at: offset + 24))
            let nameLength = Int(try data.littleEndianUInt16(at: offset + 28))
            let extraLength = Int(try data.littleEndianUInt16(at: offset + 30))
            let commentLength = Int(try data.littleEndianUInt16(at: offset + 32))
            let localOffset = Int(try data.littleEndianUInt32(at: offset + 42))
            let nameData = try data.checkedSubdata(in: (offset + 46)..<(offset + 46 + nameLength))
            guard let name = String(data: nameData, encoding: .utf8) else {
                throw OfficeExtractionError.invalidArchive
            }
            parsed[name] = Entry(
                compressionMethod: method,
                compressedSize: compressedSize,
                uncompressedSize: uncompressedSize,
                localHeaderOffset: localOffset,
                encrypted: flags & 0x0001 != 0
            )
            offset += 46 + nameLength + extraLength + commentLength
        }
        entries = parsed
    }

    var entryNames: [String] { Array(entries.keys) }

    func contains(_ name: String) -> Bool { entries[name] != nil }

    func data(for name: String) throws -> Data {
        guard let entry = entries[name] else { throw OfficeExtractionError.missingDocumentPart }
        guard !entry.encrypted else { throw OfficeExtractionError.encryptedDocument }
        let localOffset = entry.localHeaderOffset
        guard try archiveData.littleEndianUInt32(at: localOffset) == 0x0403_4B50 else {
            throw OfficeExtractionError.invalidArchive
        }
        let nameLength = Int(try archiveData.littleEndianUInt16(at: localOffset + 26))
        let extraLength = Int(try archiveData.littleEndianUInt16(at: localOffset + 28))
        let payloadOffset = localOffset + 30 + nameLength + extraLength
        let compressed = try archiveData.checkedSubdata(
            in: payloadOffset..<(payloadOffset + entry.compressedSize)
        )

        switch entry.compressionMethod {
        case 0:
            return compressed
        case 8:
            return try compressed.inflate(expectedSize: entry.uncompressedSize)
        default:
            throw OfficeExtractionError.unsupportedCompression
        }
    }
}

private final class WordprocessingMLTextParser: NSObject, XMLParserDelegate {
    private let data: Data
    private var output = ""
    private var isReadingText = false
    private var parseError: Error?

    init(data: Data) {
        self.data = data
    }

    func parse() throws -> String {
        let parser = XMLParser(data: data)
        parser.delegate = self
        parser.shouldProcessNamespaces = false
        guard parser.parse() else {
            throw parseError ?? parser.parserError ?? OfficeExtractionError.invalidXML
        }
        return output
            .replacingOccurrences(of: "[ \\t]+\n", with: "\n", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func parser(
        _ parser: XMLParser,
        didStartElement elementName: String,
        namespaceURI: String?,
        qualifiedName qName: String?,
        attributes attributeDict: [String: String] = [:]
    ) {
        let name = elementName.split(separator: ":").last.map(String.init) ?? elementName
        switch name {
        case "t", "delText", "instrText":
            isReadingText = true
        case "tab":
            output.append("\t")
        case "br", "cr":
            appendNewline()
        default:
            break
        }
    }

    func parser(
        _ parser: XMLParser,
        didEndElement elementName: String,
        namespaceURI: String?,
        qualifiedName qName: String?
    ) {
        let name = elementName.split(separator: ":").last.map(String.init) ?? elementName
        if ["t", "delText", "instrText"].contains(name) {
            isReadingText = false
        } else if name == "p" {
            appendNewline()
        } else if name == "tc", !output.hasSuffix("\t") {
            output.append("\t")
        }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
        if isReadingText { output.append(string) }
    }

    func parser(_ parser: XMLParser, parseErrorOccurred parseError: Error) {
        self.parseError = parseError
    }

    private func appendNewline() {
        if !output.hasSuffix("\n") { output.append("\n") }
    }
}

private extension Data {
    func littleEndianUInt16(at offset: Int) throws -> UInt16 {
        guard offset >= 0, offset + 2 <= count else { throw OfficeExtractionError.invalidArchive }
        return UInt16(self[index(startIndex, offsetBy: offset)])
            | UInt16(self[index(startIndex, offsetBy: offset + 1)]) << 8
    }

    func littleEndianUInt32(at offset: Int) throws -> UInt32 {
        guard offset >= 0, offset + 4 <= count else { throw OfficeExtractionError.invalidArchive }
        return UInt32(self[index(startIndex, offsetBy: offset)])
            | UInt32(self[index(startIndex, offsetBy: offset + 1)]) << 8
            | UInt32(self[index(startIndex, offsetBy: offset + 2)]) << 16
            | UInt32(self[index(startIndex, offsetBy: offset + 3)]) << 24
    }

    func checkedSubdata(in range: Range<Int>) throws -> Data {
        guard range.lowerBound >= 0, range.upperBound <= count else {
            throw OfficeExtractionError.invalidArchive
        }
        return subdata(in: range)
    }

    func lastIndex(ofLittleEndianUInt32 signature: UInt32) -> Int? {
        guard count >= 22 else { return nil }
        let lowerBound = Swift.max(0, count - 65_557)
        for offset in stride(from: count - 22, through: lowerBound, by: -1) {
            if (try? littleEndianUInt32(at: offset)) == signature { return offset }
        }
        return nil
    }

    func inflate(expectedSize: Int) throws -> Data {
        guard expectedSize >= 0, expectedSize <= 128 * 1024 * 1024 else {
            throw OfficeExtractionError.documentPartTooLarge
        }
        if expectedSize == 0 { return Data() }
        var destination = [UInt8](repeating: 0, count: expectedSize)
        let decodedCount = withUnsafeBytes { sourceBuffer in
            destination.withUnsafeMutableBytes { destinationBuffer in
                compression_decode_buffer(
                    destinationBuffer.bindMemory(to: UInt8.self).baseAddress!,
                    expectedSize,
                    sourceBuffer.bindMemory(to: UInt8.self).baseAddress!,
                    count,
                    nil,
                    COMPRESSION_ZLIB
                )
            }
        }
        guard decodedCount > 0 else { throw OfficeExtractionError.decompressionFailed }
        return Data(destination.prefix(decodedCount))
    }
}

private enum OfficeExtractionError: LocalizedError {
    case invalidArchive
    case missingDocumentPart
    case encryptedDocument
    case unsupportedCompression
    case decompressionFailed
    case documentPartTooLarge
    case invalidXML
    case noReadableText

    var errorDescription: String? {
        switch self {
        case .invalidArchive: return tr("office.docx.error.invalid_archive", "The DOCX archive is invalid.")
        case .missingDocumentPart: return tr("office.docx.error.missing_body", "The DOCX document body is missing.")
        case .encryptedDocument: return tr("office.docx.error.encrypted", "Encrypted DOCX files cannot be parsed locally.")
        case .unsupportedCompression: return tr("office.docx.error.unsupported_compression", "The DOCX uses an unsupported compression format.")
        case .decompressionFailed: return tr("office.docx.error.decompression_failed", "The DOCX document body could not be decompressed.")
        case .documentPartTooLarge: return tr("office.docx.error.part_too_large", "A DOCX document part is too large to parse.")
        case .invalidXML: return tr("office.docx.error.invalid_xml", "The DOCX document XML is invalid.")
        case .noReadableText: return tr("office.docx.error.no_text", "No readable text was found in this DOCX file.")
        }
    }

    private func tr(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(key, value: fallback, comment: "")
    }
}
