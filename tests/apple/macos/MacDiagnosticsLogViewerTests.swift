// SPDX-License-Identifier: MIT

import Foundation
import XCTest
@testable import Nola

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
