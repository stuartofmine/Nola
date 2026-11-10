// SPDX-License-Identifier: Apache-2.0

import AppKit
import XCTest
@testable import Nola

final class MacCodeReferenceTests: XCTestCase {
    @MainActor
    func testPreparedPaletteMatchesPerRunAppearanceResolution() throws {
        let line = "let answer = 42; print(\"你好👩🏽‍💻\") // comment"
        let document = MacCodeReferenceDocument(url: URL(fileURLWithPath: "/inline.swift"),
            lines: Array(repeating: line, count: 250), byteCount: line.utf8.count * 250,
            encodingName: "UTF-8")
        for name in [NSAppearance.Name.aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua] {
            let appearance = try XCTUnwrap(NSAppearance(named: name))
            var expected = NSAttributedString(string: "")
            appearance.performAsCurrentDrawingAppearance {
                let base = MacCodeSyntaxHighlighter.attributedText(for: document, targetLine: 125)
                let resolved = NSMutableAttributedString(attributedString: base)
                base.enumerateAttributes(in: NSRange(location: 0, length: base.length)) { attributes, range, _ in
                    for key in [NSAttributedString.Key.foregroundColor, .backgroundColor] {
                        if let color = attributes[key] as? NSColor,
                           let rgb = color.usingColorSpace(.deviceRGB) {
                            resolved.addAttribute(key, value: rgb, range: range)
                        }
                    }
                }
                expected = resolved
            }
            let started = SuspendingClock.now
            let actual = MacCodeSyntaxHighlighter.attributedText(for: document, targetLine: 125, appearance: appearance)
            print("prepared_code_palette appearance=\(name.rawValue) lines=250 elapsed=\(started.duration(to: .now))")
            XCTAssertTrue(actual.isEqual(to: expected), "Palette preparation must preserve every font, token color, line number and target background")
        }
    }

    @MainActor
    func testInlineHighlightCacheReusesOnlyCurrentSourceLanguageAndAppearance() {
        let cache = MacInlineCodeHighlightCache()
        let source = "let answer = 42\r\n// 你好👩🏽‍💻"
        let first = cache.text(source: source, fileExtension: "swift", dark: true)
        for _ in 0..<20 {
            XCTAssertEqual(cache.text(source: source, fileExtension: "swift", dark: true), first)
        }
        XCTAssertEqual(cache.renderCount, 1, "Layout updates must reuse highlighted content")
        XCTAssertTrue(String(first.characters).contains("\n  2  // 你好👩🏽‍💻"))
        let light = cache.text(source: source, fileExtension: "swift", dark: false)
        XCTAssertNotEqual(first, light, "Appearance changes must resolve colors again")
        _ = cache.text(source: source, fileExtension: "py", dark: false)
        let appended = cache.text(source: source + "\nprint(answer)", fileExtension: "py", dark: false)
        XCTAssertTrue(String(appended.characters).contains("print(answer)"))
        XCTAssertEqual(cache.renderCount, 4)
        XCTAssertEqual(cache.text(source: source, fileExtension: "swift", dark: true), first)
        XCTAssertEqual(cache.renderCount, 5, "Old streamed prefixes must not accumulate in the cache")
    }

    @MainActor
    func testLongCodeHighlightingPreservesTokensUnicodeAndLineNumbers() throws {
        for (suffix, keyword, comment) in [("swift", "let", "//"), ("py", "def", "#"),
                                             ("ts", "const", "//"), ("cpp", "return", "//")] {
            let line = "\(keyword) value = 42; \"你好👩🏽‍💻\" \(comment) note"
            let document = MacCodeReferenceDocument(url: URL(fileURLWithPath: "/inline.\(suffix)"),
                lines: Array(repeating: line, count: 500), byteCount: line.utf8.count * 500,
                encodingName: "UTF-8")
            let started = SuspendingClock.now
            let result = MacCodeSyntaxHighlighter.attributedText(for: document, targetLine: 500)
            print("long_code_highlighting language=\(suffix) lines=500 elapsed=\(started.duration(to: .now))")
            let lines = result.string.components(separatedBy: "\n")
            XCTAssertEqual(lines.count, 500)
            XCTAssertEqual(lines.first, "  1  \(line)")
            XCTAssertEqual(lines.last, "500  \(line)")
            for (token, expected) in [(keyword, NSColor.systemPurple), ("42", .systemBlue),
                                       ("你好👩🏽‍💻", .systemRed), ("note", .systemGreen)] {
                let range = (result.string as NSString).range(of: token)
                XCTAssertNotEqual(range.location, NSNotFound)
                let actual = try XCTUnwrap(result.attribute(.foregroundColor,
                    at: range.location, effectiveRange: nil) as? NSColor)
                XCTAssertEqual(actual, expected)
            }
            let finalLine = (result.string as NSString).range(of: "500  ")
            XCTAssertNotNil(result.attribute(.backgroundColor, at: finalLine.location, effectiveRange: nil))
        }
    }

    @MainActor
    func testCodeForegroundResolvesAgainstExplicitAppearanceNotAmbientAppearance() throws {
        let document = MacCodeReferenceDocument(
            url: URL(fileURLWithPath: "/inline.swift"),
            lines: ["let answer = 42", "print(answer)"],
            byteCount: 28,
            encodingName: "UTF-8"
        )
        for dark in [false, true] {
            let requested = try XCTUnwrap(NSAppearance(named: dark ? .darkAqua : .aqua))
            let opposite = try XCTUnwrap(NSAppearance(named: dark ? .aqua : .darkAqua))
            var text = NSAttributedString(string: "")
            opposite.performAsCurrentDrawingAppearance {
                text = MacCodeSyntaxHighlighter.attributedText(
                    for: document, targetLine: nil, appearance: requested
                )
            }
            XCTAssertTrue(text.string.contains("print(answer)"))
            for token in ["answer", "print", "1"] {
                let range = (text.string as NSString).range(of: token)
                let color = try XCTUnwrap(text.attribute(.foregroundColor, at: range.location, effectiveRange: nil) as? NSColor)
                let rgb = try XCTUnwrap(color.usingColorSpace(.deviceRGB))
                let brightness = (rgb.redComponent + rgb.greenComponent + rgb.blueComponent) / 3
                if dark {
                    XCTAssertGreaterThan(brightness, 0.5, "\(token) must remain visible on dark code background")
                } else {
                    XCTAssertLessThan(brightness, 0.5, "\(token) must remain visible on light code background")
                }
            }
        }
    }

    func testParsesAbsoluteMarkdownPathWithLineAndColumn() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("Nola Code Reference \(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        let file = directory.appendingPathComponent("Example File.swift")
        try "one\ntwo\nthree\n".write(to: file, atomically: true, encoding: .utf8)
        let encoded = file.path.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed)!
        let target = try XCTUnwrap(URL(string: "\(encoded):2:4"))

        let reference = try XCTUnwrap(MacCodeReferenceParser.parse(target))
        XCTAssertEqual(reference.url, file.standardizedFileURL)
        XCTAssertEqual(reference.line, 2)
        XCTAssertEqual(reference.column, 4)
    }

    func testParsesFileURLGitHubFragmentAndRejectsWebURL() throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("reference-\(UUID().uuidString).py")
        try "alpha\nbeta\n".write(to: file, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: file) }

        var components = try XCTUnwrap(URLComponents(url: file, resolvingAgainstBaseURL: false))
        components.fragment = "L2C3"
        let reference = try XCTUnwrap(MacCodeReferenceParser.parse(try XCTUnwrap(components.url)))

        XCTAssertEqual(reference.url, file.standardizedFileURL)
        XCTAssertEqual(reference.line, 2)
        XCTAssertEqual(reference.column, 3)
        XCTAssertNil(MacCodeReferenceParser.parse(try XCTUnwrap(URL(string: "https://example.com/source.swift:42"))))
    }

    func testLoadsTextDocumentWithoutChangingLineIdentity() throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("reference-\(UUID().uuidString).swift")
        try "first\r\nsecond\r\nthird".write(to: file, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: file) }

        let reference = MacCodeReference(url: file, line: 2, column: nil)
        let document = try MacCodeReferenceLoader.load(reference)

        XCTAssertEqual(document.lines, ["first", "second", "third"])
        XCTAssertEqual(document.encodingName, "UTF-8")
        XCTAssertEqual(document.lineCount, 3)
    }

    func testSyntaxPreviewAddsLineNumbersAndTokenHighlighting() {
        let document = MacCodeReferenceDocument(
            url: URL(fileURLWithPath: "/tmp/example.py"),
            lines: [
                "def add(a, b):",
                "    # sum the values",
                "    return a + b",
            ],
            byteCount: 48,
            encodingName: "UTF-8"
        )

        let highlighted = MacCodeSyntaxHighlighter.attributedText(
            for: document,
            targetLine: 2
        )

        XCTAssertTrue(highlighted.string.contains("1  def add"))
        XCTAssertTrue(highlighted.string.contains("2      # sum the values"))
        let defRange = (highlighted.string as NSString).range(of: "def")
        XCTAssertNotNil(
            highlighted.attribute(
                .foregroundColor,
                at: defRange.location,
                effectiveRange: nil
            )
        )
        let targetRange = (highlighted.string as NSString).range(
            of: "2      # sum the values"
        )
        XCTAssertNotNil(
            highlighted.attribute(
                .backgroundColor,
                at: targetRange.location,
                effectiveRange: nil
            )
        )
    }

    func testMissingFileStillParsesAndFailsInsidePreviewLoader() throws {
        let missing = URL(fileURLWithPath: "/tmp/nola-missing-\(UUID().uuidString).swift")
        let target = try XCTUnwrap(URL(string: "\(missing.path):12"))
        let reference = try XCTUnwrap(MacCodeReferenceParser.parse(target))

        XCTAssertEqual(reference.line, 12)
        XCTAssertThrowsError(try MacCodeReferenceLoader.load(reference)) { error in
            guard case MacCodeReferenceLoadError.missing = error else {
                return XCTFail("Expected a missing-file error, got \(error)")
            }
        }
    }
}
