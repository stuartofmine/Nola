import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacTranscriptBodyUploaderTests: XCTestCase {
    func testUTF8ChunksPreserveScalarsAndEscapingAcrossBoundaries() throws {
        // A long grapheme must not force one unbounded chunk.
        let text = String(repeating: "x", count: 32_767)
            + "😀" + String(repeating: "\u{0301}", count: 40_000) + "\"\n\\中"
        var restored = ""
        var offset: UInt64 = 0
        for chunk in MacTranscriptTextChunks(text: text) {
            XCTAssertEqual(chunk.offset, offset)
            XCTAssertFalse(chunk.text.isEmpty)
            XCTAssertLessThanOrEqual(chunk.text.utf8.count, 32_768)
            offset += UInt64(chunk.text.utf8.count)
            restored += chunk.text
        }
        XCTAssertEqual(Array(restored.utf8), Array(text.utf8))
        XCTAssertTrue(Array(MacTranscriptTextChunks(text: "")).isEmpty)
    }

    func testSealedUploadReplaysAfterReopenAndRejectsChangedPrefix() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaTranscriptUploader-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("nola.db")
        let owner = MacContentUploadOwner(conversationID: "c", importID: "i", partID: "p")
        let original = String(repeating: "a", count: 32_767) + "😀" + String(repeating: "中", count: 4_000)
        let first = try MacApplicationPort(databaseURL: database)
        _ = try first.createConversation(MacCanonicalCreateConversationInput(
            id: "c", title: "Upload", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1
        ))
        let uploaded = try MacTranscriptBodyUploader(port: first).upload(original, owner: owner, messageID: "m")
        let reopened = try MacApplicationPort(databaseURL: database)
        let uploader = MacTranscriptBodyUploader(port: reopened)
        XCTAssertEqual(try uploader.upload(original, owner: owner, messageID: "m"), uploaded)
        let changed = "b" + original.dropFirst()
        XCTAssertThrowsError(try uploader.upload(changed, owner: owner, messageID: "m"))
        XCTAssertTrue(try XCTUnwrap(reopened.loadConversation("c")).messages.isEmpty)
        let firstChunk = try reopened.readContentUploadChunk(id: uploaded.uploadID, owner: owner, offset: 0)
        XCTAssertTrue(firstChunk.text.hasPrefix("aaa"))
    }

    func testTwoFieldsOnTheSamePartHaveIndependentUploadIdentities() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("NolaFieldUploader-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        _ = try port.createConversation(MacCanonicalCreateConversationInput(
            id: "c", title: "Fields", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1
        ))
        let owner = MacContentUploadOwner(conversationID: "c", importID: "i", partID: "p")
        let uploader = MacTranscriptBodyUploader(port: port)
        let text = try uploader.upload("{}", owner: owner, messageID: "m", field: .contentText)
        let json = try uploader.upload("{}", owner: owner, messageID: "m", field: .contentJSON)
        XCTAssertNotEqual(text.uploadID, json.uploadID)
        XCTAssertEqual(try uploader.upload("{}", owner: owner, messageID: "m", field: .contentJSON), json)
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        let encoded = try encoder.encode(MacArchivePartUpload(json, field: .contentJSON))
        let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        XCTAssertEqual(wire["field"] as? String, "content_json")
    }
}
