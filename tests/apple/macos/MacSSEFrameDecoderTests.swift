import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacSSEFrameDecoderTests: XCTestCase {
    private func decode(_ input: String, finish: Bool = true) -> [String] {
        var decoder = MacSSEFrameDecoder()
        var frames = input.utf8.compactMap { decoder.append($0) }
        if finish, let tail = decoder.finish() { frames.append(tail) }
        return frames
    }

    func testConsecutiveFramesAreEmittedAtEachBlankLineWithoutWaitingForEOF() {
        XCTAssertEqual(decode("data: first\n\ndata: second\n\ndata: third\n\n", finish: false),
                       ["first", "second", "third"])
    }

    func testMultilineDataRetainsNewlinesWithinOneFrame() {
        XCTAssertEqual(decode("data: {\ndata:   \"type\": \"delta\",\ndata:   \"text\": \"你好\"\ndata: }\n\n"),
                       ["{\n  \"type\": \"delta\",\n  \"text\": \"你好\"\n}"])
    }

    func testLFCRLFAndBareCRHaveTheSameFrameSemantics() {
        for newline in ["\n", "\r\n", "\r"] {
            let wire = ["data: alpha", "data: beta", "", "data: gamma", "", ""].joined(separator: newline)
            XCTAssertEqual(decode(wire), ["alpha\nbeta", "gamma"], "newline: \(newline.debugDescription)")
        }
    }

    func testSplitCRLFDoesNotInsertAnExtraDataLineOrDispatchTwice() {
        var decoder = MacSSEFrameDecoder()
        for byte in "data: one\r".utf8 { XCTAssertNil(decoder.append(byte)) }
        XCTAssertNil(decoder.append(10))
        XCTAssertEqual(decoder.append(13), "one")
        XCTAssertNil(decoder.append(10))
        XCTAssertNil(decoder.finish())
    }

    func testMixedNewlineFormsRemainIndependentOfByteBoundaries() {
        XCTAssertEqual(decode("data: a\r\ndata: b\r\rdata: c\n\ndata: d\r\n\r\n"),
                       ["a\nb", "c", "d"])
    }

    func testCommentsAndNonDataFieldsDoNotBecomePayload() {
        XCTAssertEqual(decode(": heartbeat\n\nevent: delta\nid: 4\nretry: 500\nunknown: value\ndata: payload\n: inline comment\n\n"),
                       ["payload"])
        XCTAssertEqual(decode(": heartbeat\nevent: ignored\nid: 4\n\n"), [])
    }

    func testOnlyOneOptionalSpaceAfterColonIsRemovedAndFieldNamesAreExact() {
        XCTAssertEqual(decode("Data: ignored\n data: ignored\ndata:  keep one space\ndata:\tkeep tab\ndata:trailing  \n\n"),
                       [" keep one space\n\tkeep tab\ntrailing  "])
    }

    func testEmptyDataIsAnEventButEmptyOrCommentOnlyBlocksAreNot() {
        XCTAssertEqual(decode("\n\n: comment\n\ndata:\n\ndata\n\n"), ["", ""])
        XCTAssertEqual(decode("data: one\ndata:\n\n"), ["one\n"])
    }

    func testInitialUTF8BOMIsIgnoredButBOMInsidePayloadIsPreserved() {
        XCTAssertEqual(decode("\u{FEFF}data: 你好 👋🏽 café\n\ndata: \u{FEFF}payload\n\n"),
                       ["你好 👋🏽 café", "\u{FEFF}payload"])
        // Only the very beginning of the stream has BOM semantics. A later
        // BOM-prefixed field is not the exact SSE field name `data`.
        XCTAssertEqual(decode("data: first\n\n\u{FEFF}data: ignored\n\n"), ["first"])
    }

    func testMultibyteUTF8IsNotDecodedPerByte() {
        var decoder = MacSSEFrameDecoder()
        for byte in "data: 中文🙂e\u{301}".utf8 { XCTAssertNil(decoder.append(byte)) }
        XCTAssertNil(decoder.append(10))
        XCTAssertEqual(decoder.append(10), "中文🙂e\u{301}")
        XCTAssertNil(decoder.finish())
    }

    func testEOFDrainsUnterminatedDataExactlyOnce() {
        for wire in ["data: tail", "data: tail\n", "data: tail\r", "data: tail\r\n"] {
            var decoder = MacSSEFrameDecoder()
            XCTAssertTrue(wire.utf8.compactMap { decoder.append($0) }.isEmpty)
            XCTAssertEqual(decoder.finish(), "tail", "wire: \(wire.debugDescription)")
            XCTAssertNil(decoder.finish(), "EOF must consume, not duplicate, its pending frame")
        }
    }

    func testEOFDoesNotRedispatchAnAlreadyDelimitedFrameOrCreateCommentEvent() {
        XCTAssertEqual(decode("data: done\n\n: final heartbeat"), ["done"])
        XCTAssertEqual(decode("data: done\n\n"), ["done"])
        XCTAssertEqual(decode("event: unfinished metadata"), [])
        XCTAssertEqual(decode(""), [])
    }
}
