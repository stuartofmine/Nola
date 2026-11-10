// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Extracts local document content for macOS chat using the shared 30 MB transport contract.

import AppKit
import Foundation
import PDFKit
import UniformTypeIdentifiers
import Vision

struct MacAttachmentExtraction: Sendable {
    let text: String
    let method: String
    let byteCount: Int64
}

struct MacPreparedAttachment: Sendable {
    let name: String
    let url: URL
    let extractedText: String
    let extractionMethod: String
}

enum MacAttachmentPipelineError: LocalizedError {
    case unavailable
    case tooLarge(Int64)
    case unreadable
    case unsupported(String)
    case extractionFailed(String)

    var errorDescription: String? {
        switch self {
        case .unavailable:
            return String(localized: "The selected file is no longer available.")
        case .tooLarge(let bytes):
            return String(
                format: String(localized: "The file is too large to import (%@)."),
                ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
            )
        case .unreadable:
            return String(localized: "The selected file could not be read.")
        case .unsupported(let type):
            return String(
                format: String(localized: "Text extraction is not available for %@."),
                type
            )
        case .extractionFailed(let detail): return detail
        }
    }
}

actor MacAttachmentPipeline {
    static let shared = MacAttachmentPipeline()

    private let maxFileBytes: Int64 = 30 * 1_024 * 1_024
    private let maxExtractedCharacters = 300_000
    private let maxPDFPages = 500
    private let maxPDFOCRPages = 12

    func extract(_ url: URL) async throws -> MacAttachmentExtraction {
        let accessing = url.startAccessingSecurityScopedResource()
        defer { if accessing { url.stopAccessingSecurityScopedResource() } }

        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard values.isRegularFile == true else { throw MacAttachmentPipelineError.unavailable }
        let byteCount = Int64(values.fileSize ?? 0)
        guard byteCount <= maxFileBytes else { throw MacAttachmentPipelineError.tooLarge(byteCount) }

        let ext = url.pathExtension.lowercased()
        let extracted: (text: String, method: String)
        switch ext {
        case "pdf": extracted = try await extractPDF(url)
        case "docx", "pptx", "xlsx": extracted = (try extractOfficeArchive(url, extension: ext), "Office Open XML")
        case "rtf", "rtfd", "html", "htm", "odt": extracted = (try await extractAttributedDocument(url), "Attributed document")
        case "txt", "md", "markdown", "csv", "tsv", "json", "jsonl", "xml", "yaml", "yml",
             "swift", "m", "mm", "h", "hpp", "c", "cc", "cpp", "rs", "go", "kt", "kts",
             "java", "js", "jsx", "ts", "tsx", "py", "rb", "php", "sh", "zsh", "fish",
             "css", "scss", "less", "sql", "toml", "ini", "log":
            extracted = (try extractPlainText(url), "Text")
        case "png", "jpg", "jpeg", "heic", "heif", "gif", "webp", "tiff", "bmp":
            guard NSImage(contentsOf: url) != nil else { throw MacAttachmentPipelineError.unreadable }
            return MacAttachmentExtraction(text: "", method: "Image", byteCount: byteCount)
        default:
            if let type = UTType(filenameExtension: ext), type.conforms(to: .text) {
                extracted = (try extractPlainText(url), "Text")
            } else {
                throw MacAttachmentPipelineError.unsupported(ext.isEmpty ? "this file type" : ext.uppercased())
            }
        }

        let normalized = normalize(extracted.text)
        guard !normalized.isEmpty else { throw MacAttachmentPipelineError.unreadable }
        return MacAttachmentExtraction(
            text: String(normalized.prefix(maxExtractedCharacters)),
            method: extracted.method,
            byteCount: byteCount
        )
    }

    private func extractPDF(_ url: URL) async throws -> (text: String, method: String) {
        guard let document = PDFDocument(url: url) else { throw MacAttachmentPipelineError.unreadable }
        var pages: [String] = []
        var characters = 0
        var ocrPages = 0
        let pageLimit = min(document.pageCount, maxPDFPages)
        for index in 0..<pageLimit {
            guard let pdfPage = document.page(at: index) else { continue }
            var text = pdfPage.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            var usedOCR = false
            if text.isEmpty, ocrPages < maxPDFOCRPages {
                ocrPages += 1
                text = (try? recognizeText(on: pdfPage)) ?? ""
                usedOCR = !text.isEmpty
            }
            guard !text.isEmpty else { continue }
            let page = "[Page \(index + 1)\(usedOCR ? ", OCR" : "")]\n\(text)"
            pages.append(page)
            characters += page.count
            if characters >= maxExtractedCharacters { break }
        }
        let wasPageLimited = document.pageCount > pageLimit
        let method = ocrPages > 0
            ? (wasPageLimited ? "PDF text + OCR (first \(pageLimit) pages)" : "PDF text + OCR")
            : (wasPageLimited ? "PDF text (first \(pageLimit) pages)" : "PDF text")
        return (pages.joined(separator: "\n\n"), method)
    }

    private func recognizeText(on page: PDFPage) throws -> String {
        let image = page.thumbnail(of: NSSize(width: 1_600, height: 2_200), for: .mediaBox)
        guard let data = image.tiffRepresentation,
              let bitmap = NSBitmapImageRep(data: data),
              let cgImage = bitmap.cgImage else { return "" }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        request.automaticallyDetectsLanguage = true
        let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
        try handler.perform([request])
        return request.results?
            .compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    private func extractAttributedDocument(_ url: URL) async throws -> String {
        try await MainActor.run {
            var attributes: NSDictionary?
            return try NSAttributedString(url: url, options: [:], documentAttributes: &attributes).string
        }
    }

    private func extractPlainText(_ url: URL) throws -> String {
        let data = try Data(contentsOf: url, options: [.mappedIfSafe])
        for encoding in [String.Encoding.utf8, .utf16, .utf16LittleEndian, .utf16BigEndian, .isoLatin1] {
            if let text = String(data: data, encoding: encoding), !text.isEmpty { return text }
        }
        throw MacAttachmentPipelineError.unreadable
    }

    private func extractOfficeArchive(_ url: URL, extension ext: String) throws -> String {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-office-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        process.arguments = ["-x", "-k", url.path, directory.path]
        let errorPipe = Pipe()
        process.standardOutput = FileHandle.nullDevice
        process.standardError = errorPipe
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let detail = String(decoding: errorPipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw MacAttachmentPipelineError.extractionFailed(
                detail.isEmpty ? "The Office document could not be expanded." : detail
            )
        }

        let files: [URL]
        switch ext {
        case "docx":
            let wordDirectory = directory.appendingPathComponent("word", isDirectory: true)
            let supplemental = ["footnotes.xml", "endnotes.xml"]
                .map { wordDirectory.appendingPathComponent($0) }
                .filter { FileManager.default.fileExists(atPath: $0.path) }
            files = officeFiles(in: wordDirectory, prefix: "header")
                + [wordDirectory.appendingPathComponent("document.xml")]
                + supplemental
                + officeFiles(in: wordDirectory, prefix: "footer")
        case "pptx": files = officeFiles(in: directory.appendingPathComponent("ppt/slides"), prefix: "slide")
        case "xlsx":
            files = [directory.appendingPathComponent("xl/sharedStrings.xml")]
                + officeFiles(in: directory.appendingPathComponent("xl/worksheets"), prefix: "sheet")
        default: files = []
        }

        var output: [String] = []
        var characters = 0
        for fileURL in files {
            guard let data = try? Data(contentsOf: fileURL),
                  let xml = String(data: data, encoding: .utf8) else { continue }
            let text = xmlText(xml)
            if !text.isEmpty {
                output.append(text)
                characters += text.count
            }
            if characters >= maxExtractedCharacters { break }
        }
        return output.joined(separator: "\n\n")
    }

    private func officeFiles(in directory: URL, prefix: String) -> [URL] {
        guard let files = try? FileManager.default.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        ) else { return [] }
        return files
            .filter { $0.lastPathComponent.hasPrefix(prefix) && $0.pathExtension == "xml" }
            .sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending }
    }

    private func xmlText(_ xml: String) -> String {
        xml
            .replacingOccurrences(of: #"<w:tab\s*/>"#, with: "\t", options: .regularExpression)
            .replacingOccurrences(of: #"</(?:w:p|a:p|row|si|c)>"#, with: "\n", options: .regularExpression)
            .replacingOccurrences(of: #"<[^>]+>"#, with: " ", options: .regularExpression)
            .replacingOccurrences(of: "&amp;", with: "&")
            .replacingOccurrences(of: "&lt;", with: "<")
            .replacingOccurrences(of: "&gt;", with: ">")
            .replacingOccurrences(of: "&quot;", with: "\"")
            .replacingOccurrences(of: "&apos;", with: "'")
    }

    private func normalize(_ text: String) -> String {
        text
            .replacingOccurrences(of: "\u{0000}", with: "")
            .replacingOccurrences(of: #"[ \t]+"#, with: " ", options: .regularExpression)
            .replacingOccurrences(of: #"\n{3,}"#, with: "\n\n", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
