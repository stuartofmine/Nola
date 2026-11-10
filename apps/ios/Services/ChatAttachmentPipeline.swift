// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Imports chat attachments through a local-first parsing pipeline.
// - Interfaces: ChatAttachmentPipeline

import CryptoKit
import Foundation
import PDFKit
import UIKit
import UniformTypeIdentifiers
import Vision

enum ChatAttachmentPipeline {
    // Keep the client import contract aligned with Agent Host and the model
    // media pipeline. Images between the legacy 8 MB boundary and 30 MB stay
    // intact; larger files fail before they can be presented as ready.
    private static let maximumFileBytes = 30 * 1024 * 1024
    private static let maximumWebHostFileBytes = 80 * 1024 * 1024
    private static let maximumExtractedCharacters = 80_000
    private static let maximumPDFPages = 500
    private static let maximumOCRPages = 12

    static func importFiles(
        from result: Result<[URL], Error>,
        parserHost: ChatAttachmentParserHost?
    ) async throws -> ChatAttachmentImportResult {
        let urls = try result.get()
        var attachments: [ChatAttachmentDraft] = []
        var failedCount = 0
        var seenDigests = Set<String>()

        for url in urls {
            do {
                let attachment = try await parse(url: url, parserHost: parserHost)
                guard seenDigests.insert(attachment.contentDigest).inserted else { continue }
                attachments.append(attachment)
                recordImportedAttachment(attachment)
            } catch {
                failedCount += 1
                recordAttachmentImportFailure(filename: url.lastPathComponent, error: error)
                Logger.shared.log(.warn, "ChatAttachmentPipeline", "Attachment import failed", metadata: [
                    "filename": url.lastPathComponent,
                    "error": error.localizedDescription
                ])
            }
        }

        return ChatAttachmentImportResult(attachments: attachments, failedCount: failedCount)
    }

    private static func recordImportedAttachment(_ attachment: ChatAttachmentDraft) {
        guard PersonalIntelligenceCore.isAvailable else { return }
        let eventID = UUID().uuidString
        let normalizedContent = attachment.extractedContent
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let excerpt = String(normalizedContent.prefix(1_600))
        let tags = [
            "attachment",
            "document",
            "文件",
            attachment.typeLabel.lowercased(),
            attachment.name.lowercased()
        ]
        let event = IntelligenceEvent(
            id: eventID,
            source: "attachment",
            kind: attachment.isImage ? "image.imported" : "document.imported",
            subject: attachment.contentDigest,
            summary: "Imported \(attachment.name) (\(attachment.typeLabel), \(attachment.extractionLabel), \(attachment.sizeBytes) bytes)",
            importance: normalizedContent.isEmpty ? 0.35 : 0.7,
            confidence: attachment.importWarning == nil ? 1.0 : 0.72,
            tags: tags,
            dedupeKey: "attachment.import.\(attachment.contentDigest)"
        )
        let candidates: [IntelligenceMemoryCandidate]
        if attachment.isImage || excerpt.isEmpty {
            candidates = []
        } else {
            candidates = [IntelligenceMemoryCandidate(
                semanticKey: "document.\(attachment.contentDigest)",
                kind: .semantic,
                statement: "File \(attachment.name) (\(attachment.typeLabel), \(attachment.extractionLabel)) contains: \(excerpt)",
                confidence: attachment.importWarning == nil ? 0.92 : 0.68,
                sourceEventIDs: [eventID],
                tags: tags
            )]
        }
        _ = PersonalIntelligenceCore.ingest(
            IntelligenceIngestRequest(events: [event], memoryCandidates: candidates)
        )
    }

    private static func recordAttachmentImportFailure(filename: String, error: Error) {
        guard PersonalIntelligenceCore.isAvailable else { return }
        let reason = error.localizedDescription
        _ = PersonalIntelligenceCore.ingest(IntelligenceIngestRequest(events: [
            IntelligenceEvent(
                source: "attachment",
                kind: "document.import_failed",
                subject: filename,
                summary: "Failed to import \(filename): \(reason)",
                importance: 0.3,
                confidence: 1.0,
                tags: ["attachment", "failure", "文件", filename.lowercased()],
                dedupeKey: "attachment.failure.\(filename).\(reason)"
            )
        ]))
    }

    private static func parse(
        url: URL,
        parserHost: ChatAttachmentParserHost?
    ) async throws -> ChatAttachmentDraft {
        let didAccess = url.startAccessingSecurityScopedResource()
        defer {
            if didAccess { url.stopAccessingSecurityScopedResource() }
        }

        let values = try url.resourceValues(forKeys: [.fileSizeKey, .contentTypeKey, .isRegularFileKey])
        guard values.isRegularFile != false else {
            throw AttachmentPipelineError.notARegularFile
        }
        if let fileSize = values.fileSize, fileSize > maximumFileBytes {
            throw AttachmentPipelineError.fileTooLarge(maximumFileBytes)
        }

        let data = try await readData(from: url)
        guard data.count <= maximumFileBytes else {
            throw AttachmentPipelineError.fileTooLarge(maximumFileBytes)
        }

        let filename = url.lastPathComponent
        let contentType = values.contentType ?? UTType(filenameExtension: url.pathExtension)
        let mimeType = contentType?.preferredMIMEType ?? "application/octet-stream"
        let typeLabel = url.pathExtension.isEmpty
            ? (contentType?.localizedDescription ?? "File")
            : url.pathExtension.uppercased()
        let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        _ = try ChatAttachmentStorage.persist(data, digest: digest, filename: filename)

        if contentType?.conforms(to: .image) == true {
            let payload = try normalizedImagePayload(data: data, contentType: contentType)
            return ChatAttachmentDraft(
                name: filename,
                typeLabel: typeLabel,
                sizeBytes: data.count,
                extractedContent: localized("chat.attachment.image_reference", "Image attached for visual analysis."),
                isImage: true,
                imageBase64Data: payload.data.base64EncodedString(),
                imageMimeType: payload.mimeType,
                extractionLabel: localized("chat.attachment.extraction.image", "Image"),
                contentDigest: digest
            )
        }

        if contentType?.conforms(to: .pdf) == true || url.pathExtension.lowercased() == "pdf" {
            return try await parsePDF(
                data: data,
                filename: filename,
                typeLabel: typeLabel,
                mimeType: mimeType,
                digest: digest,
                parserHost: parserHost
            )
        }

        if url.pathExtension.lowercased() == "docx" {
            do {
                let text = try OfficeDocumentTextExtractor.extractDOCX(from: data)
                return ChatAttachmentDraft(
                    name: filename,
                    typeLabel: typeLabel,
                    sizeBytes: data.count,
                    extractedContent: limited(text),
                    isImage: false,
                    extractionLabel: localized("chat.attachment.extraction.docx", "DOCX text"),
                    contentDigest: digest,
                    importWarning: truncationWarning(for: text)
                )
            } catch {
                if parserHost == nil {
                    return unavailableDraft(
                        filename: filename,
                        typeLabel: typeLabel,
                        sizeBytes: data.count,
                        digest: digest,
                        warning: error.localizedDescription
                    )
                }
            }
        }

        if isLocallyReadableText(contentType: contentType, fileExtension: url.pathExtension) {
            let text = try extractText(data: data, contentType: contentType)
            return ChatAttachmentDraft(
                name: filename,
                typeLabel: typeLabel,
                sizeBytes: data.count,
                extractedContent: limited(text),
                isImage: false,
                extractionLabel: localized("chat.attachment.extraction.local_text", "Local text"),
                contentDigest: digest,
                importWarning: truncationWarning(for: text)
            )
        }

        if let parserHost {
            do {
                let parsed = try await parseWithWebHost(
                    data: data,
                    filename: filename,
                    mimeType: mimeType,
                    parserHost: parserHost
                )
                return ChatAttachmentDraft(
                    name: filename,
                    typeLabel: typeLabel,
                    sizeBytes: data.count,
                    extractedContent: limited(parsed),
                    isImage: false,
                    extractionLabel: localized("chat.attachment.extraction.web_host", "Host parser"),
                    contentDigest: digest,
                    importWarning: truncationWarning(for: parsed)
                )
            } catch {
                return unavailableDraft(
                    filename: filename,
                    typeLabel: typeLabel,
                    sizeBytes: data.count,
                    digest: digest,
                    warning: String(
                        format: localized("chat.attachment.warning.web_host_failed", "Host parsing failed: %@"),
                        error.localizedDescription
                    )
                )
            }
        }

        return unavailableDraft(
            filename: filename,
            typeLabel: typeLabel,
            sizeBytes: data.count,
            digest: digest,
            warning: localized(
                "chat.attachment.warning.web_host_required",
                "This format needs a connected host for text extraction."
            )
        )
    }

    private static func parsePDF(
        data: Data,
        filename: String,
        typeLabel: String,
        mimeType: String,
        digest: String,
        parserHost: ChatAttachmentParserHost?
    ) async throws -> ChatAttachmentDraft {
        guard let document = PDFDocument(data: data) else {
            throw AttachmentPipelineError.invalidPDF
        }

        var sections: [String] = []
        var usedOCR = false
        var skippedOCRPages = 0
        var ocrPageCount = 0
        var extractedCharacterCount = 0

        let pageLimit = min(document.pageCount, maximumPDFPages)
        for index in 0..<pageLimit {
            guard let page = document.page(at: index) else { continue }
            let selectableText = page.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !selectableText.isEmpty {
                sections.append("[Page \(index + 1)]\n\(selectableText)")
                extractedCharacterCount += selectableText.count
                if extractedCharacterCount >= maximumExtractedCharacters { break }
                continue
            }

            guard ocrPageCount < maximumOCRPages else {
                skippedOCRPages += 1
                continue
            }
            ocrPageCount += 1
            if let recognized = (try? recognizeText(on: page)) ?? nil, !recognized.isEmpty {
                usedOCR = true
                sections.append("[Page \(index + 1), OCR]\n\(recognized)")
                extractedCharacterCount += recognized.count
                if extractedCharacterCount >= maximumExtractedCharacters { break }
            }
        }

        var extracted = sections.joined(separator: "\n\n")
        if extracted.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, let parserHost {
            do {
                extracted = try await parseWithWebHost(
                    data: data,
                    filename: filename,
                    mimeType: mimeType,
                    parserHost: parserHost
                )
                return ChatAttachmentDraft(
                    name: filename,
                    typeLabel: typeLabel,
                    sizeBytes: data.count,
                    extractedContent: limited(extracted),
                    isImage: false,
                    extractionLabel: localized("chat.attachment.extraction.web_host", "Host parser"),
                    contentDigest: digest,
                    importWarning: truncationWarning(for: extracted)
                )
            } catch {
                return unavailableDraft(
                    filename: filename,
                    typeLabel: typeLabel,
                    sizeBytes: data.count,
                    digest: digest,
                    warning: String(
                        format: localized("chat.attachment.warning.web_host_failed", "Host parsing failed: %@"),
                        error.localizedDescription
                    )
                )
            }
        }

        let warning: String?
        let skippedPDFPages = max(0, document.pageCount - pageLimit)
        if skippedPDFPages > 0 {
            warning = String(
                format: localized("chat.attachment.warning.pdf_page_limit", "%d PDF pages were not processed."),
                skippedPDFPages
            )
        } else if skippedOCRPages > 0 {
            warning = String(
                format: localized("chat.attachment.warning.ocr_page_limit", "%d scanned pages were not OCR-processed."),
                skippedOCRPages
            )
        } else if extracted.isEmpty {
            warning = localized("chat.attachment.warning.no_pdf_text", "No readable text was found in this PDF.")
        } else {
            warning = truncationWarning(for: extracted)
        }

        return ChatAttachmentDraft(
            name: filename,
            typeLabel: typeLabel,
            sizeBytes: data.count,
            extractedContent: limited(extracted),
            isImage: false,
            extractionLabel: usedOCR
                ? localized("chat.attachment.extraction.pdf_ocr", "PDF text + OCR")
                : localized("chat.attachment.extraction.pdf_text", "PDF text"),
            contentDigest: digest,
            importWarning: warning
        )
    }

    private static func readData(from url: URL) async throws -> Data {
        try await Task.detached(priority: .userInitiated) {
            try Data(contentsOf: url, options: [.mappedIfSafe])
        }.value
    }

    private static func recognizeText(on page: PDFPage) throws -> String? {
        let image = page.thumbnail(of: CGSize(width: 1_600, height: 2_200), for: .mediaBox)
        guard let cgImage = image.cgImage else { return nil }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        request.automaticallyDetectsLanguage = true
        let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
        try handler.perform([request])
        return request.results?
            .compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func parseWithWebHost(
        data: Data,
        filename: String,
        mimeType: String,
        parserHost: ChatAttachmentParserHost
    ) async throws -> String {
        guard data.count <= maximumWebHostFileBytes else {
            throw AttachmentPipelineError.webHostFileTooLarge(maximumWebHostFileBytes)
        }
        return try await WebAgentProvider.parseDocumentAttachment(
            baseURL: parserHost.baseURL,
            token: parserHost.token,
            filename: filename,
            mimeType: mimeType,
            base64Data: data.base64EncodedString(),
            pinnedCertificateSHA256: parserHost.pinnedCertificateSHA256
        )
    }

    private static func extractText(data: Data, contentType: UTType?) throws -> String {
        if contentType?.conforms(to: .rtf) == true || contentType?.conforms(to: .html) == true {
            let documentType: NSAttributedString.DocumentType = contentType?.conforms(to: .rtf) == true ? .rtf : .html
            if let attributed = try? NSAttributedString(
                data: data,
                options: [.documentType: documentType],
                documentAttributes: nil
            ) {
                return attributed.string
            }
        }

        for encoding in [String.Encoding.utf8, .utf16, .unicode, .isoLatin1] {
            if let text = String(data: data, encoding: encoding) { return text }
        }
        throw AttachmentPipelineError.unreadableText
    }

    private static func isLocallyReadableText(contentType: UTType?, fileExtension: String) -> Bool {
        if contentType?.conforms(to: .text) == true { return true }
        return ["txt", "md", "markdown", "json", "xml", "csv", "html", "htm", "rtf", "yaml", "yml", "log"]
            .contains(fileExtension.lowercased())
    }

    private static func normalizedImagePayload(data: Data, contentType: UTType?) throws -> (data: Data, mimeType: String) {
        let originalMimeType = contentType?.preferredMIMEType ?? "image/jpeg"
        guard let image = UIImage(data: data) else {
            throw AttachmentPipelineError.imageNormalizationFailed
        }
        guard data.count > 4 * 1024 * 1024 else {
            return (data, originalMimeType)
        }

        let longestSide = max(image.size.width, image.size.height)
        let scale = min(1, 2_048 / max(longestSide, 1))
        let targetSize = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let renderer = UIGraphicsImageRenderer(size: targetSize)
        let normalized = renderer.image { _ in
            image.draw(in: CGRect(origin: .zero, size: targetSize))
        }
        guard let jpeg = normalized.jpegData(compressionQuality: 0.82) else {
            throw AttachmentPipelineError.imageNormalizationFailed
        }
        return (jpeg, "image/jpeg")
    }

    private static func unavailableDraft(
        filename: String,
        typeLabel: String,
        sizeBytes: Int,
        digest: String,
        warning: String
    ) -> ChatAttachmentDraft {
        ChatAttachmentDraft(
            name: filename,
            typeLabel: typeLabel,
            sizeBytes: sizeBytes,
            extractedContent: "",
            isImage: false,
            extractionLabel: localized("chat.attachment.extraction.unavailable", "Text unavailable"),
            contentDigest: digest,
            importWarning: warning
        )
    }

    private static func limited(_ text: String) -> String {
        String(text.prefix(maximumExtractedCharacters))
    }

    private static func truncationWarning(for text: String) -> String? {
        guard text.count > maximumExtractedCharacters else { return nil }
        return localized(
            "chat.attachment.warning.text_truncated",
            "The extracted text was shortened to fit the conversation context."
        )
    }

    private static func localized(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(key, value: fallback, comment: "")
    }
}

nonisolated enum ChatAttachmentStorage {
    static func persist(_ data: Data, digest: String, filename: String) throws -> URL {
        let destination = try destinationURL(
            digest: digest,
            filename: filename,
            createDirectory: true
        )
        if !FileManager.default.fileExists(atPath: destination.path) {
            try data.write(to: destination, options: .atomic)
        }
        return destination
    }

    static func existingURL(digest: String, filename: String) -> URL? {
        guard let destination = try? destinationURL(
            digest: digest,
            filename: filename,
            createDirectory: false
        ), FileManager.default.fileExists(atPath: destination.path) else {
            return nil
        }
        return destination
    }

    private static func destinationURL(
        digest: String,
        filename: String,
        createDirectory: Bool
    ) throws -> URL {
        let fileManager = FileManager.default
        let supportURL = try fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: createDirectory
        )
        let directory = supportURL
            .appendingPathComponent("ChatAttachments", isDirectory: true)
            .appendingPathComponent(digest, isDirectory: true)
        if createDirectory {
            try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        }
        let trimmed = filename.trimmingCharacters(in: .whitespacesAndNewlines)
        let safeFilename = trimmed.isEmpty
            ? "Attachment"
            : URL(fileURLWithPath: trimmed).lastPathComponent
        return directory.appendingPathComponent(safeFilename, isDirectory: false)
    }
}

private enum AttachmentPipelineError: LocalizedError {
    case notARegularFile
    case fileTooLarge(Int)
    case webHostFileTooLarge(Int)
    case invalidPDF
    case unreadableText
    case imageNormalizationFailed

    var errorDescription: String? {
        switch self {
        case .notARegularFile:
            return NSLocalizedString(
                "chat.attachment.error.not_regular",
                value: "The selected item is not a regular file.",
                comment: "Attachment is not a regular file"
            )
        case let .fileTooLarge(maximumBytes):
            let format = NSLocalizedString(
                "chat.attachment.error.file_too_large_format",
                value: "The file exceeds the %d MB attachment limit.",
                comment: "Attachment file size limit"
            )
            return String(format: format, locale: Locale.current, maximumBytes / 1024 / 1024)
        case let .webHostFileTooLarge(maximumBytes):
            let format = NSLocalizedString(
                "chat.attachment.error.web_host_file_too_large_format",
                value: "The file exceeds the %d MB host parsing limit, but local parsing may still be available.",
                comment: "Host attachment parser size limit"
            )
            return String(format: format, locale: Locale.current, maximumBytes / 1024 / 1024)
        case .invalidPDF:
            return NSLocalizedString("chat.attachment.error.invalid_pdf", value: "The PDF could not be opened.", comment: "Invalid PDF attachment")
        case .unreadableText:
            return NSLocalizedString("chat.attachment.error.unreadable_text", value: "The file does not contain readable text.", comment: "Unreadable text attachment")
        case .imageNormalizationFailed:
            return NSLocalizedString("chat.attachment.error.image_normalization_failed", value: "The image could not be prepared for upload.", comment: "Image attachment normalization failed")
        }
    }
}
