// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS debug logging configuration, diagnostics log viewer, chat localization/string catalog structure, visual acceptance, attachment pipeline, PDF viewport, seed overrides, and runtime acceptance fixtures suite.

import AppKit
import Foundation
import PDFKit
import SQLite3
import XCTest
@testable import Nola

@MainActor
final class MacDebugLoggingConfigurationTests: XCTestCase {
    func testDebugLoggingDefaultsPersistenceResetAndSanitization() throws {
        let suiteName = "nola.mac.debug-logging.tests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defer {
            defaults.removePersistentDomain(forName: suiteName)
            defaults.removeObject(forKey: MacDebugLoggingConfiguration.enabledKey)
            defaults.removeObject(forKey: MacDebugLoggingConfiguration.levelKey)
        }
        let logger = Logger()

        MacDebugLoggingConfiguration.apply(defaults: defaults, logger: logger)
        XCTAssertFalse(logger.runtimeLoggingState.isDetailedLoggingEnabled)
        XCTAssertEqual(logger.runtimeLoggingState.minimumLevel, .debug)
        XCTAssertTrue(logger.shouldRecord(.error) && logger.shouldRecord(.warn))
        XCTAssertFalse(logger.shouldRecord(.info) || logger.shouldRecord(.debug))

        defaults.set(true, forKey: MacDebugLoggingConfiguration.enabledKey)
        defaults.set("debug", forKey: MacDebugLoggingConfiguration.levelKey)
        MacDebugLoggingConfiguration.apply(defaults: defaults, logger: logger)
        XCTAssertTrue(logger.runtimeLoggingState.isDetailedLoggingEnabled)
        XCTAssertTrue(logger.shouldRecord(.info) && logger.shouldRecord(.debug))
        XCTAssertFalse(logger.shouldRecord(.trace))

        defaults.set("trace", forKey: MacDebugLoggingConfiguration.levelKey)
        MacDebugLoggingConfiguration.apply(defaults: defaults, logger: logger)
        XCTAssertTrue(logger.shouldRecord(.trace))

        MacDebugLoggingConfiguration.reset(defaults: defaults, logger: logger)
        XCTAssertNil(defaults.object(forKey: MacDebugLoggingConfiguration.enabledKey))
        XCTAssertNil(defaults.object(forKey: MacDebugLoggingConfiguration.levelKey))
        XCTAssertFalse(logger.runtimeLoggingState.isDetailedLoggingEnabled)
        XCTAssertEqual(logger.runtimeLoggingState.minimumLevel, .debug)

        let source = "prompt=private request, token=secret-token, url=https://example.com/private?q=secret, path=/Users/person/Documents/report.pdf"
        let sanitized = Logger.sanitizeForDiagnostics(source)
        XCTAssertFalse(sanitized.contains("private request") || sanitized.contains("secret-token") || sanitized.contains("/private?q=secret") || sanitized.contains("/Users/person"))
        XCTAssertTrue(sanitized.contains("https://example.com") && sanitized.contains("<redacted>") && sanitized.contains("<path>"))
    }
}

final class MacDiagnosticsLogViewerTests: XCTestCase {
    func testParserPreservesOrderAndRecognizesEveryLevel() {
        let text = """
        [2026-07-15 10:00:00.001][ERROR][Chat] failed
        [2026-07-15 10:00:00.002][WARN][Search] slow
        [2026-07-15 10:00:00.003][INFO][Diagnostics] enabled
        [2026-07-15 10:00:00.004][DEBUG][Chat] started
        [2026-07-15 10:00:00.005][TRACE][Network] event
        malformed line
        """

        let entries = MacDiagnosticLogParser.parse(text)

        XCTAssertEqual(entries.map(\.level), [.error, .warning, .info, .debug, .trace, .other])
        XCTAssertEqual(entries.map(\.id), Array(0..<6))
        XCTAssertEqual(entries[1].module, "Search")
        XCTAssertEqual(entries[2].message, "enabled")
        XCTAssertEqual(entries[5].rawLine, "malformed line")
    }

    func testFiltersUseExactLevelModuleAndCaseInsensitiveText() {
        let entries = MacDiagnosticLogParser.parse("""
        [t1][DEBUG][Chat] Stream Started
        [t2][TRACE][Chat] trace only
        [t3][DEBUG][Search] search started
        """)

        XCTAssertEqual(
            MacDiagnosticLogParser.filter(entries, level: .debug, module: "Chat", query: "stream").map(\.id),
            [0]
        )
        XCTAssertEqual(
            MacDiagnosticLogParser.filter(entries, level: .trace, module: "", query: "").map(\.id),
            [1]
        )
        XCTAssertTrue(
            MacDiagnosticLogParser.filter(entries, level: .error, module: "", query: "").isEmpty
        )
    }

    func testLogSourceReportsReadableWritableFileAndCanClearIt() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = MacDiagnosticLogSource(baseDirectory: root)
        try FileManager.default.createDirectory(
            at: source.logFileURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try "[t][INFO][Diagnostics] ready\n".write(
            to: source.logFileURL,
            atomically: true,
            encoding: .utf8
        )

        let snapshot = source.load()

        XCTAssertTrue(snapshot.status.fileExists)
        XCTAssertTrue(snapshot.status.isWritable)
        XCTAssertGreaterThan(snapshot.status.sizeBytes, 0)
        XCTAssertEqual(snapshot.entries.first?.module, "Diagnostics")

        try source.clear()
        XCTAssertFalse(source.load().status.fileExists)
    }

    func testLogSourceMakesWriteFailureVisibleWithoutExposingAbsolutePath() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try "not a directory".write(to: root, atomically: true, encoding: .utf8)

        let snapshot = MacDiagnosticLogSource(baseDirectory: root).load()

        XCTAssertFalse(snapshot.status.isWritable)
        XCTAssertNotNil(snapshot.status.errorDescription)
        XCTAssertFalse(snapshot.status.errorDescription?.contains(root.path) == true)
    }

    func testSanitizerRedactsSecretsContentURLsAndAbsolutePaths() {
        let raw = """
        Authorization: Bearer secret-token
        preview: private response text
        endpoint=https://example.com/v1/chat?token=secret
        file=/Users/person/Documents/private.txt
        """

        let sanitized = MacDiagnosticLogSanitizer.sanitize(raw)

        XCTAssertFalse(sanitized.contains("secret-token"))
        XCTAssertFalse(sanitized.contains("private response text"))
        XCTAssertFalse(sanitized.contains("/v1/chat"))
        XCTAssertFalse(sanitized.contains("/Users/person"))
        XCTAssertTrue(sanitized.contains("<redacted>"))
        XCTAssertTrue(sanitized.contains("<path-redacted>"))
    }
}

final class MacChatLocalizationContractTests: XCTestCase {
    func testLocalExecutionRecoveryDoesNotClaimCancellationOrSuccess() {
        let chinese = Locale(identifier: "zh-Hans")
        XCTAssertEqual(String(localized: "Local execution connection interrupted. The result is not yet confirmed.",
                              table: "Chat", bundle: .main, locale: chinese),
                       "本地执行连接已中断，结果尚未确认。")
        XCTAssertEqual(String(localized: "Check execution status", table: "Chat", bundle: .main, locale: chinese),
                       "检查执行状态")
    }

    func testConversationInspectorUsesChatCatalogTranslations() throws {
        let simplifiedChinese = Locale(identifier: "zh-Hans")

        XCTAssertEqual(
            String(
                localized: "Conversation Details",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "对话详情"
        )
        XCTAssertEqual(
            String(
                localized: "Applies to this conversation",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "仅对当前对话有效"
        )
        XCTAssertEqual(
            String(
                localized: "Knowledge",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "知识"
        )
        XCTAssertEqual(
            String(
                format: String(
                    localized: "Context %@ / %@",
                    bundle: .main,
                    locale: simplifiedChinese
                ),
                "~5k",
                "128k"
            ),
            "上下文 ~5k / 128k"
        )
        XCTAssertEqual(
            String(localized: "Choose…", bundle: .main, locale: simplifiedChinese),
            "选择…"
        )
        XCTAssertEqual(
            String(
                localized: "Files",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "文件"
        )
        XCTAssertEqual(
            String(
                localized: "Browser",
                table: "Chat",
                bundle: .main,
                locale: simplifiedChinese
            ),
            "浏览器"
        )
        let germanResource = try XCTUnwrap(
            Bundle.main.url(
                forResource: "Chat",
                withExtension: "strings",
                subdirectory: nil,
                localization: "de"
            )
        )
        let germanData = try Data(contentsOf: germanResource)
        let germanStrings = try XCTUnwrap(
            PropertyListSerialization.propertyList(from: germanData, format: nil) as? [String: String]
        )

        XCTAssertEqual(germanStrings["Conversation Details"], "Gesprächsdetails")
        XCTAssertEqual(germanStrings["Applies to this conversation"], "Gilt für diese Unterhaltung")
    }
}

final class MacStringCatalogStructureTests: XCTestCase {
    func testLocalizableCatalogHasOnlyCanonicalTopLevelFields() throws {
        let catalogURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "Localizable.xcstrings",
                withExtension: "txt"
            )
        )
        let root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(contentsOf: catalogURL)) as? [String: Any]
        )

        XCTAssertEqual(Set(root.keys), ["sourceLanguage", "strings", "version"])
        let strings = try XCTUnwrap(root["strings"] as? [String: Any])
        let signingMessage = "This Nola build has an unstable signing identity. macOS may show an older Nola entry as enabled while denying the current build. Run a normally signed build, remove the stale entry, and grant the current Nola app access again."
        let entry = try XCTUnwrap(strings[signingMessage] as? [String: Any])
        let localizations = try XCTUnwrap(entry["localizations"] as? [String: Any])
        let simplifiedChinese = try XCTUnwrap(localizations["zh-Hans"] as? [String: Any])
        let unit = try XCTUnwrap(simplifiedChinese["stringUnit"] as? [String: Any])
        XCTAssertEqual(unit["state"] as? String, "translated")
        XCTAssertFalse((unit["value"] as? String)?.isEmpty ?? true)
    }
}

final class MacVisualAcceptanceContractTests: XCTestCase {
    func testOverallPassWithoutObservedLifecycleFramesIsRejected() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-visual-contract-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let artifact = root.appendingPathComponent("frames", isDirectory: true)
        try FileManager.default.createDirectory(at: artifact, withIntermediateDirectories: true)
        let receipt = root.appendingPathComponent("receipt.json")
        let payload: [String: Any] = [
            "schema_version": 1,
            "artifact_path": artifact.path,
            "artifact_kind": "application_window_sequence",
            "observer": "visual-reviewer",
            "observation_method": "image_capable_reviewer",
            "verdict": "pass",
            "observations": [],
            "motion_reviewed": false,
            "notes": "Generated but not inspected",
        ]
        try JSONSerialization.data(withJSONObject: payload).write(to: receipt)

        let result = MacVisualAcceptanceContract.validate(
            receiptURL: receipt,
            artifactPath: artifact.path,
            artifactKind: "application_window_sequence",
            requiredScope: "application_window_sequence"
        )

        XCTAssertFalse(result.passed)
        XCTAssertEqual(Set(result.missingCheckpoints), MacVisualAcceptanceContract.conversationCheckpoints)
        XCTAssertFalse(result.motionReviewed)
    }

    func testPassingNotesWithoutOpenedEvidenceFilesAreRejected() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-visual-contract-\(UUID().uuidString)", isDirectory: true)
        let artifact = root.appendingPathComponent("frames", isDirectory: true)
        try FileManager.default.createDirectory(at: artifact, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let observations = MacVisualAcceptanceContract.conversationCheckpoints.map { checkpoint in
            [
                "checkpoint": checkpoint,
                "verdict": "pass",
                "notes": "Claimed as observed",
                "evidence_path": artifact.appendingPathComponent("missing-\(checkpoint).png").path,
                "evidence_kind": "screenshot",
            ]
        }
        let receipt = root.appendingPathComponent("receipt.json")
        let payload: [String: Any] = [
            "schema_version": 1,
            "artifact_path": artifact.path,
            "artifact_kind": "application_window_sequence",
            "observer": "visual-reviewer",
            "observation_method": "image_capable_reviewer",
            "verdict": "pass",
            "observations": observations,
            "motion_reviewed": true,
            "notes": "Evidence paths were never generated or opened",
        ]
        try JSONSerialization.data(withJSONObject: payload).write(to: receipt)

        let result = MacVisualAcceptanceContract.validate(
            receiptURL: receipt,
            artifactPath: artifact.path,
            artifactKind: "application_window_sequence",
            requiredScope: "application_window_sequence"
        )
        XCTAssertFalse(result.passed)
        XCTAssertEqual(Set(result.missingCheckpoints), MacVisualAcceptanceContract.conversationCheckpoints)
    }
}

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

@MainActor
final class MacSeedOverrideTests: XCTestCase {
    func testMalformedPersistedSeedCannotFallBackToDefaults() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("invalid-seed.db")
        let repository = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: database))
        try repository.restoreBackupSnapshot([MacStoredConversation(conversation:
            MacConversation(id: "invalid", title: "Fixture", preview: "", updatedAt: .now,
                isPinned: false, isPersisted: true), messages: [])])
        // Simulate an old/corrupt database without using the validated write API.
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &connection), SQLITE_OK)
        defer { sqlite3_close(connection) }
        XCTAssertEqual(sqlite3_exec(connection,
            "UPDATE conversation_settings SET settings_json='{\"modelOverrides\":{\"configured:model\":{\"seed\":0}}}' WHERE conversation_id='invalid'",
            nil, nil, nil), SQLITE_OK)
        XCTAssertThrowsError(try repository.load(conversationID: "invalid"))
        XCTAssertEqual(sqlite3_exec(connection,
            "UPDATE conversation_settings SET settings_json='{\"modelOverrides\":{\"configured:model\":{\"seed\":{\"kind\":\"clear\"}}}}' WHERE conversation_id='invalid'",
            nil, nil, nil), SQLITE_OK)
        XCTAssertEqual(try repository.load(conversationID: "invalid")?.conversation
            .modelOverrides?["configured:model"]?.seed, .clear)
    }

    func testCanonicalCodecAndResolvedValues() throws {
        var defaults = ModelParameters.webAlignedCloudDefaults
        defaults.seed = 42
        for intent: MacSeedOverride? in [nil, .clear, .value(0), .value(-7), .value(Int64.min), .value(Int64.max)] {
            let override = MacConversationModelOverride(seed: intent)
            let data = try JSONEncoder().encode(override)
            let decoded = try JSONDecoder().decode(MacConversationModelOverride.self, from: data)
            XCTAssertEqual(decoded, override)
            switch intent {
            case .none: XCTAssertEqual(decoded.applying(to: defaults).seed, 42)
            case .clear: XCTAssertNil(decoded.applying(to: defaults).seed)
            case .value(let value): XCTAssertEqual(decoded.applying(to: defaults).seed, Int(value))
            }
            if case .value(let value) = intent {
                let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                XCTAssertEqual((object["seed"] as? [String: Any])?["value"] as? String, String(value))
            }
        }
    }

    func testInvalidAndLegacyShapesAreRejected() throws {
        for seed in ["null", "0", "{\"kind\":\"clear\",\"value\":null}",
                     "{\"kind\":\"value\",\"value\":0}",
                     "{\"kind\":\"clear\",\"other\":true}", "{}"] {
            XCTAssertThrowsError(try JSONDecoder().decode(MacConversationModelOverride.self,
                from: Data("{\"seed\":\(seed)}".utf8)))
        }
        for invalid in ["+1", "-0", "01", "-01", " 1", "1 ", "1.0", "1e3", "", "9223372036854775808", "-9223372036854775809"] {
            let object: [String: Any] = ["seed": ["kind": "value", "value": invalid]]
            XCTAssertThrowsError(try JSONDecoder().decode(MacConversationModelOverride.self,
                from: JSONSerialization.data(withJSONObject: object)))
        }
    }

    func testRemoteIntentDistinguishesOmissionClearAndExactInteger() throws {
        let profile = MacRemoteModelProfile(hostID: "fixture", profileID: "model", revision: 1,
            displayName: "Fixture", modelName: "model")
        for intent: MacSeedOverride? in [nil, .clear, .value(0), .value(Int64.min), .value(Int64.max)] {
            let payload = try MacRemoteModelIntent(profile: profile, tier: "balanced",
                overrides: .init(seed: intent)).payload()
            let overrides = try XCTUnwrap(payload["overrides"] as? [String: Any])
            // Exercise the actual Foundation HTTP serialization too, not just
            // an intermediate Swift integer or a floating-point comparison.
            let bytes = try JSONSerialization.data(withJSONObject: overrides)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
            let options = object["generation_options"] as? [String: Any]
            switch intent {
            case .none: XCTAssertNil(options)
            case .clear: XCTAssertTrue(options?["seed"] is NSNull)
            case .value(let value):
                XCTAssertEqual((options?["seed"] as? NSNumber)?.stringValue, String(value))
            }
        }
    }

    func testIsolatedCanonicalPersistenceReopensEveryIntent() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("seed.db")
        let intents: [MacSeedOverride?] = [nil, .clear, .value(0), .value(-7), .value(Int64.min), .value(Int64.max)]
        let repository = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: database))
        try repository.restoreBackupSnapshot(intents.enumerated().map { index, intent in
            var conversation = MacConversation(id: "seed-\(index)", title: "Seed", preview: "",
                updatedAt: .now, isPinned: false, isPersisted: true)
            conversation.modelOverrides = ["configured:fixture": MacConversationModelOverride(seed: intent)]
            return MacStoredConversation(conversation: conversation, messages: [])
        })
        let reopened = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: database))
        for (index, intent) in intents.enumerated() {
            let conversation = try XCTUnwrap(reopened.load(conversationID: "seed-\(index)")?.conversation)
            XCTAssertEqual(conversation.modelOverrides?["configured:fixture"]?.seed, intent)
        }
    }

    func testLocalRequestConsumesOnlyResolvedSeed() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var defaults = ModelParameters.webAlignedCloudDefaults
        defaults.seed = 42
        for intent: MacSeedOverride? in [nil, .clear, .value(0), .value(Int64.max)] {
            let parameters = MacConversationModelOverride(seed: intent).applying(to: defaults)
            let request = try MacLocalAgentRunRequest(conversationID: "seed-local", prompt: "fixture",
                projectURL: root, attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                modelParameters: parameters, wireProtocol: .chatCompletions,
                appEnvironment: environment, applicationPort: port)
            let data = try await MacAgentHostRuntime().initialRequestData(for: request)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let options = try XCTUnwrap(payload["generation_options"] as? [String: Any])
            if let expected = parameters.seed {
                XCTAssertEqual((options["seed"] as? NSNumber)?.stringValue, String(expected))
            } else {
                XCTAssertNil(options["seed"])
            }
        }
    }
}

#if DEBUG
final class MacRuntimeAcceptanceModelFixtureTests: XCTestCase {
    private func environment(
        execution: [String: Any] = ["toolCalls": true, "imageInput": false, "reasoning": false,
                                    "samplingParameters": false, "samplingRequiresNoReasoning": false],
        model: String = "fixture-model", wireProtocol: String = "chat_completions"
    ) throws -> [String: String] {
        let data = try JSONSerialization.data(withJSONObject: [
            "model": model, "wireProtocol": wireProtocol, "executionCapabilities": execution,
        ])
        return [
            "NOLA_MAC_RUNTIME_ACCEPTANCE_ENDPOINT": "http://127.0.0.1:1234/v1",
            "NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL": "fixture-model",
            "NOLA_MAC_RUNTIME_ACCEPTANCE_API_KEY": "fixture-secret",
            "NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON": String(decoding: data, as: UTF8.self),
            "NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_TOOL": "1",
        ]
    }

    func testExactDeclaredRouteAndDisabledCapabilitiesSurvive() throws {
        let provider = try MacRuntimeAcceptanceModelFixture.provider(environment: environment(wireProtocol: "responses"))
        XCTAssertEqual(provider.model, "fixture-model")
        XCTAssertEqual(provider.baseURL.absoluteString, "http://127.0.0.1:1234/v1")
        XCTAssertEqual(provider.wireProtocol, .responses)
        XCTAssertTrue(provider.executionCapabilities.toolCalls)
        XCTAssertFalse(provider.executionCapabilities.reasoning)
        XCTAssertFalse(provider.executionCapabilities.imageInput)
        XCTAssertFalse(provider.executionCapabilities.samplingParameters)
    }

    func testMissingDeclarationNeverCreatesAnAllFalseConfiguration() throws {
        var input = try environment()
        input.removeValue(forKey: "NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON")
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
    }

    func testDeclaredReasoningUsesProductionValidationAndProtocol() throws {
        var input = try environment(execution: ["toolCalls": true, "imageInput": false,
            "reasoning": true, "samplingParameters": false, "samplingRequiresNoReasoning": false],
            wireProtocol: "responses")
        var reasoning = MacReasoningConfiguration()
        reasoning.control = .effort
        reasoning.efforts = ["low", "medium", "high"]
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = String(decoding:
            try JSONEncoder().encode(reasoning), as: UTF8.self)
        let provider = try MacRuntimeAcceptanceModelFixture.provider(environment: input)
        XCTAssertEqual(provider.reasoning?.options.first, "low")
        XCTAssertEqual(provider.reasoning?.responses, true)
        reasoning.control = .budget
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = String(decoding:
            try JSONEncoder().encode(reasoning), as: UTF8.self)
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
    }

    func testReasoningOverrideCannotEnableDisabledCapability() throws {
        var input = try environment(wireProtocol: "responses")
        var reasoning = MacReasoningConfiguration()
        reasoning.control = .effort
        reasoning.efforts = ["low", "medium", "high"]
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = String(decoding:
            try JSONEncoder().encode(reasoning), as: UTF8.self)
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"] = "invalid-secret-fixture"
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input)) { error in
            XCTAssertFalse(error.localizedDescription.contains("invalid-secret-fixture"))
        }
    }

    func testFixtureCannotSwitchModelOrUnknownProtocol() throws {
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: environment(model: "other-model")))
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: environment(wireProtocol: "guess-from-url")))
    }

    func testFlagsRequireJSONBooleansAndKnownFields() throws {
        for flag: Any in [1, "true", NSNull()] {
            let input = try environment(execution: ["toolCalls": flag, "imageInput": false,
                "reasoning": false, "samplingParameters": false, "samplingRequiresNoReasoning": false])
            XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        }
        let alias = try environment(execution: ["toolCalls": true, "native_tool_calls": false,
            "imageInput": false, "reasoning": false, "samplingParameters": false, "samplingRequiresNoReasoning": false])
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: alias))
    }

    func testToolRequirementCannotEnableExplicitFalse() throws {
        var input = try environment(execution: ["toolCalls": false, "imageInput": false,
            "reasoning": false, "samplingParameters": false, "samplingRequiresNoReasoning": false])
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_TOOL"] = "0"
        let provider = try MacRuntimeAcceptanceModelFixture.provider(environment: input)
        XCTAssertFalse(provider.executionCapabilities.toolCalls)
    }

    func testLegacyVisionAndThinkingOptionsCannotEnableExplicitFalse() throws {
        for key in ["NOLA_MAC_RUNTIME_ACCEPTANCE_VISION_INPUT", "NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_THINKING"] {
            var input = try environment()
            input[key] = "1"
            XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input))
        }
    }

    func testInvalidFixtureDiagnosticDoesNotLeakCredentials() throws {
        var input = try environment()
        input["NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON"] = "fixture-secret-invalid-json"
        XCTAssertThrowsError(try MacRuntimeAcceptanceModelFixture.provider(environment: input)) { error in
            XCTAssertFalse(error.localizedDescription.contains("fixture-secret"))
        }
    }
}

final class MacRuntimeAcceptancePermissionFixtureTests: XCTestCase {
    private func root() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("nola-mac-permission-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
        return url
    }

    private func pending(_ fixture: MacRuntimeAcceptancePermissionFixture, access: String = "write",
                         target: URL? = nil) throws -> MacCorePendingPermissionIdentity {
        let resource = try JSONSerialization.data(withJSONObject: [
            "execution_target": "fixture-host", "capability": "fixture.overwrite", "operation": "filesystem",
            "resources": [["kind": "file", "filesystem_id": "fixture-filesystem", "access": access,
                           "components": (target ?? fixture.url).pathComponents.filter { $0 != "/" }]],
        ])
        return .init(allowConversation: true, requestID: "canonical-request", turnID: "canonical-run", callID: "canonical-call",
                     capabilityID: "fixture.overwrite", resource: String(decoding: resource, as: UTF8.self),
                     permissionDomain: "filesystem", rationale: "fixture overwrite")
    }

    private func challenge(callID: String = "canonical-call") -> MacRemotePermissionRequest {
        .init(id: "canonical-request", allowConversation: true, protocolVersion: 1, nonce: "fixture-nonce", hostID: "fixture-host",
              clientID: "fixture-client", sessionID: "fixture-session", toolName: "fixture.overwrite",
              arguments: "{}", callID: callID, reason: "fixture", message: "fixture", expiresAt: nil)
    }

    func testAuditRequiresExactCanonicalIdentityAndUnchangedOriginalBytes() throws {
        let root = try root()
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("original\n".utf8).write(to: root.appendingPathComponent("existing.txt"))
        let fixture = try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "existing.txt")
        let identity = try pending(fixture)
        let audit = try fixture.audit(pending: identity, challenge: challenge(), runID: "canonical-run", currentBytes: fixture.originalBytes)
        XCTAssertEqual(audit["canonical_pending_verified"] as? Bool, true)
        XCTAssertEqual(audit["fixture_bytes_base64"] as? String, fixture.originalBytes.base64EncodedString())
        XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(callID: "other-call"),
                                              runID: "canonical-run", currentBytes: fixture.originalBytes))
        XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(),
                                              runID: "other-run", currentBytes: fixture.originalBytes))
        XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(),
                                              runID: "canonical-run", currentBytes: Data("changed\n".utf8)))
    }

    func testReadOrDifferentFileResourceCannotProvePendingOverwrite() throws {
        let root = try root()
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("original\n".utf8).write(to: root.appendingPathComponent("existing.txt"))
        let fixture = try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "existing.txt")
        for identity in [try pending(fixture, access: "read"), try pending(fixture, target: root.appendingPathComponent("different.txt"))] {
            XCTAssertThrowsError(try fixture.audit(pending: identity, challenge: challenge(),
                                                  runID: "canonical-run", currentBytes: fixture.originalBytes))
        }
    }

    func testFixtureMustAlreadyExistWithoutTraversalOrSymlink() throws {
        let root = try root()
        defer { try? FileManager.default.removeItem(at: root) }
        XCTAssertThrowsError(try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "missing.txt"))
        XCTAssertThrowsError(try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "../outside.txt"))
        let original = root.appendingPathComponent("existing.txt")
        try Data("original\n".utf8).write(to: original)
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("linked.txt"), withDestinationURL: original)
        XCTAssertThrowsError(try MacRuntimeAcceptancePermissionFixture(projectURL: root, relativePath: "linked.txt"))
        XCTAssertEqual(try Data(contentsOf: original), Data("original\n".utf8))
    }
}
#endif
