import Foundation
import XCTest
@testable import Nola

final class MacMarkdownTailBoundaryScannerTests: XCTestCase {
    func testFenceScannerProcessesLongUTF8BodyOnceAcrossByteBoundaries() {
        var scanner = MacMarkdownTailBoundaryScanner(kind: .fence(marker: 96, minimumCount: 4))
        let body = String(repeating: "汉🙂body\r\n```\r\n    ````\r\n````not close\r\n", count: 1000)
        let source = body + "  `````\t\r\nignored"
        for byte in source.utf8 { scanner.append([byte]) }
        XCTAssertEqual(scanner.boundary, body.utf8.count..<(source.utf8.count - "ignored".utf8.count))
        XCTAssertEqual(scanner.scannedByteCount, source.utf8.count - "ignored".utf8.count)
    }

    func testFenceEOFAndCRLFSplitDoNotCommitPrematurely() {
        var scanner = MacMarkdownTailBoundaryScanner(kind: .fence(marker: 96, minimumCount: 3))
        scanner.append("body\n```".utf8)
        XCTAssertNil(scanner.closingBoundary(allowEndOfInput: false))
        XCTAssertEqual(scanner.closingBoundary(allowEndOfInput: true), 5..<8)
        scanner.append("\r".utf8)
        XCTAssertNil(scanner.closingBoundary(allowEndOfInput: false))
        XCTAssertNil(scanner.closingBoundary(allowEndOfInput: true))
        scanner.append("\n".utf8)
        XCTAssertEqual(scanner.boundary, 5..<10)
    }

    func testTableScannerRetainsBlankLineAcrossAppends() {
        var scanner = MacMarkdownTailBoundaryScanner(kind: .table)
        scanner.append("| A |\n| --- |\n".utf8)
        let rows = String(repeating: "| 汉🙂 |\n", count: 2000)
        scanner.append(rows.utf8)
        let before = scanner.scannedByteCount
        XCTAssertNil(scanner.boundary)
        scanner.append("\nnext".utf8)
        XCTAssertEqual(scanner.boundary, (before - 1)..<(before + 1))
        XCTAssertEqual(scanner.scannedByteCount, before + 1)
    }

    func testAssemblerLongOpenFenceKeepsStableTailAndLinearBoundaryWork() {
        var assembler = MacSemanticStreamAssembler(identitySeed: "fence")
        XCTAssertTrue(assembler.append("```swift\r\n").isEmpty)
        let id = assembler.mutableTail?.id
        let row = "let 中文 = \"🙂\"\n"
        for _ in 0..<1000 { XCTAssertTrue(assembler.append(row).isEmpty) }
        XCTAssertEqual(assembler.mutableTail?.id, id)
        XCTAssertEqual(assembler.mutableTail?.text, String(repeating: row, count: 1000))
        XCTAssertEqual(assembler.boundaryScannedByteCount, row.utf8.count * 1000)
        XCTAssertTrue(assembler.append("```").isEmpty)
        let nodes = assembler.finish()
        XCTAssertEqual(nodes.count, 1)
        XCTAssertEqual(nodes.first?.text, String(repeating: row, count: 1000).trimmingCharacters(in: .newlines))
        XCTAssertNil(assembler.mutableTail)
    }

    func testAssemblerLongTableAndFollowingParagraphResetBoundaryOwner() {
        var assembler = MacSemanticStreamAssembler(identitySeed: "table")
        let header = "| A |\n| --- |\n"
        XCTAssertTrue(assembler.append(header).isEmpty)
        let row = "| 汉🙂 |\n"
        for _ in 0..<1000 { XCTAssertTrue(assembler.append(row).isEmpty) }
        XCTAssertEqual(assembler.boundaryScannedByteCount, header.utf8.count + row.utf8.count * 1000)
        let nodes = assembler.append("\nAfter.\n") + assembler.finish()
        XCTAssertEqual(nodes.first?.text, (header + String(repeating: row, count: 1000)).dropLast().description)
        XCTAssertTrue(nodes.dropFirst().map(\.text).joined().contains("After."))
    }

    func testIncompleteTableDividerFallsBackToProseWithoutHidingText() {
        var assembler = MacSemanticStreamAssembler()
        var nodes = assembler.append("| A | B |\n| ---")
        XCTAssertNil(assembler.mutableTail)
        nodes += assembler.append(" |\nordinary prose\n")
        nodes += assembler.finish()
        XCTAssertEqual(nodes.map(\.text).joined(), "| A | B |\n| --- |\nordinary prose\n")
    }

    @MainActor
    func testAuthoritativeReplacementDoesNotReuseOldFenceBoundary() {
        let session = MacAssistantPresentationSession(identitySeed: "replacement")
        _ = session.appendImmediately("```swift\n" + String(repeating: "old\n", count: 100))
        let stale = session.beginReveal()
        let replacement = session.accept(authoritative: "| New |\n| --- |\n| value |\n\n")
        XCTAssertTrue(replacement?.replacesText == true)
        XCTAssertNil(session.nextFrame(lease: stale))
        XCTAssertFalse(replacement?.nodes.contains { $0.text.contains("old") } ?? true)
        XCTAssertEqual(session.acceptedText, "| New |\n| --- |\n| value |\n\n")
    }
}
