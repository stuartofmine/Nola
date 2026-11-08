// SPDX-License-Identifier: Apache-2.0
import XCTest
import CryptoKit
@testable import Nola

@MainActor
final class MacCanonicalExportTests: XCTestCase {
    func testPagedExportRetainsEmptyAndStructuredPartsAcrossMessages() async throws {
        let header = MacConversationExportPage.Header(id: "c", title: "Mixed", revision: 3,
            createdAtMs: 1, updatedAtMs: 4, settingsJSON: "{\"retained\":true}")
        let originalJSON = "{\"tool\":\"lookup\",\"result\":\"中😀\"}"
        let first = MacConversationExportPage(conversation: header, messages: [
            .init(id: "m1", runID: nil, role: "user", status: "completed", ordinal: 1,
                createdAtMs: 1, completedAtMs: 2, parts: [
                    .init(id: "empty", kind: "text", ordinal: 1, state: "completed",
                        contentDigest: "digest-original", createdAtMs: 11, updatedAtMs: 12,
                        contentTextUTF8Bytes: 0, contentJSONUTF8Bytes: nil),
                    .init(id: "structured", kind: "tool_result", ordinal: 2, state: "completed",
                        contentDigest: "digest-original", createdAtMs: 11, updatedAtMs: 12,
                        contentTextUTF8Bytes: 0, contentJSONUTF8Bytes: UInt64(originalJSON.utf8.count))
                ])
        ], nextOrdinal: 1, hasMore: true)
        let second = MacConversationExportPage(conversation: header, messages: [
            .init(id: "m2", runID: nil, role: "assistant", status: "completed", ordinal: 2,
                createdAtMs: 3, completedAtMs: 4, parts: [
                    .init(id: "answer", kind: "text", ordinal: 1, state: "completed",
                        contentDigest: "digest-original", createdAtMs: 11, updatedAtMs: 12,
                        contentTextUTF8Bytes: 4, contentJSONUTF8Bytes: nil)
                ])
        ], nextOrdinal: 2, hasMore: false)
        var output = Data("[".utf8)
        var bodyRequests = 0
        try await MacCanonicalBackupStream.appendPagedConversations(["c"],
            loadPage: { _, after, _ in after == nil ? first : second },
            readRange: { id, message, part, field, revision, offset in
                bodyRequests += 1
                XCTAssertEqual(offset, 0)
                let text = part == "structured" ? originalJSON : "done"
                return MacConversationExportRange(conversationID: id, messageID: message,
                    partID: part, field: field, revision: revision, offsetBytes: 0,
                    nextOffsetBytes: UInt64(text.utf8.count), totalUTF8Bytes: UInt64(text.utf8.count),
                    text: text, endOfContent: true)
            }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { output.append($0) })
        output.append(contentsOf: "]".utf8)
        let decoded = try JSONDecoder().decode([MacBackupConversation].self, from: output)[0].canonical
        XCTAssertEqual(decoded.settingsJSON, header.settingsJSON)
        XCTAssertEqual(decoded.messages.map(\.id), ["m1", "m2"])
        XCTAssertEqual(decoded.messages[0].parts[0].contentText, "")
        XCTAssertNil(decoded.messages[0].parts[0].contentJSON)
        XCTAssertEqual(decoded.messages[0].parts[1].contentJSON, originalJSON)
        XCTAssertEqual(decoded.messages[1].parts[0].contentText, "done")
        let restored = MacArchiveRestoreWriter.messageInput(decoded.messages[0])
        XCTAssertEqual(restored.parts[1].contentDigest, "digest-original")
        XCTAssertEqual(restored.parts[1].createdAtMs, 11)
        XCTAssertEqual(restored.parts[1].updatedAtMs, 12)
        XCTAssertNotEqual(restored.parts[1].createdAtMs, restored.createdAtMs)
        XCTAssertEqual(bodyRequests, 2)
    }

    func testTenMillionByteSinglePartStreamsLosslessJSONWithoutWholeBodyWrites() async throws {
        let original = String(repeating: "中😀\n\\\"", count: 1_000_000)
        let bytes = Data(original.utf8)
        XCTAssertEqual(bytes.count, 10_000_000)
        let originalJSON = "{\"preserved\":\"原始 JSON\\n\\\"\"}"
        let jsonBytes = Data(originalJSON.utf8)
        let page = fixture(textBytes: UInt64(bytes.count), jsonBytes: UInt64(jsonBytes.count))
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: url) }
        try Data("[".utf8).write(to: url)
        let handle = try FileHandle(forWritingTo: url)
        try handle.seekToEnd()
        var maximumWrite = 0
        var requests = 0
        try await MacCanonicalBackupStream.appendPagedConversations(["c"],
            loadPage: { id, after, revision in
                XCTAssertEqual(id, "c")
                XCTAssertNil(after)
                XCTAssertTrue(revision == nil || revision == 1)
                return page
            },
            readRange: { id, message, part, field, revision, offset in
                requests += 1
                let source = field == .contentText ? bytes : jsonBytes
                var end = min(Int(offset) + 32 * 1024, source.count)
                while end < source.count && (source[end] & 0xC0) == 0x80 { end -= 1 }
                let text = try XCTUnwrap(String(data: source[Int(offset)..<end], encoding: .utf8))
                return MacConversationExportRange(conversationID: id, messageID: message,
                    partID: part, field: field, revision: revision, offsetBytes: offset,
                    nextOffsetBytes: UInt64(end), totalUTF8Bytes: UInt64(source.count), text: text,
                    endOfContent: end == source.count)
            }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { data in
                maximumWrite = max(maximumWrite, data.count)
                try handle.write(contentsOf: data)
            })
        try handle.write(contentsOf: Data("]".utf8))
        try handle.close()
        XCTAssertGreaterThan(requests, 300)
        XCTAssertLessThanOrEqual(maximumWrite, 6 * 32 * 1024)
        let decoded = try JSONDecoder().decode([MacBackupConversation].self, from: Data(contentsOf: url))
        XCTAssertEqual(decoded.count, 1)
        XCTAssertEqual(decoded[0].canonical.messages.count, 1)
        XCTAssertEqual(decoded[0].canonical.messages[0].id, "m")
        XCTAssertEqual(decoded[0].canonical.messages[0].parts[0].id, "p")
        XCTAssertEqual(decoded[0].canonical.messages[0].parts[0].contentText, original)
        XCTAssertEqual(decoded[0].canonical.messages[0].parts[0].contentJSON, originalJSON)
    }

    func testExportRefusesForeignOrShortOriginalRangeInsteadOfPublishingPreview() async throws {
        for badResponse in ["foreign", "short", "length", "revision"] {
            var completed = false
            do {
                try await MacCanonicalBackupStream.appendPagedConversations(["c"],
                    loadPage: { _, _, _ in self.fixture(textBytes: 100, jsonBytes: nil) },
                    readRange: { id, message, part, field, revision, offset in
                        MacConversationExportRange(conversationID: badResponse == "foreign" ? "other" : id,
                            messageID: message, partID: part, field: field,
                            revision: badResponse == "revision" ? revision + 1 : revision,
                            offsetBytes: offset, nextOffsetBytes: badResponse == "short" ? 3 : 7,
                            totalUTF8Bytes: badResponse == "length" ? 7 : 100,
                            text: "preview", endOfContent: true)
                    }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { data in
                        if data == Data("]}".utf8) { completed = true }
                    })
                XCTFail("Invalid range must fail: \(badResponse)")
            } catch MacApplicationServiceError.invalidResponse { }
            XCTAssertFalse(completed)
        }
    }

    func testMissingConversationAndCancellationProduceNoArchiveBytes() async throws {
        var writes = 0
        do {
            try await MacCanonicalBackupStream.appendPagedConversations(["missing"],
                loadPage: { _, _, _ in nil }, readRange: { _, _, _, _, _, _ in
                    throw MacApplicationServiceError.invalidResponse
                }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { _ in writes += 1 })
            XCTFail("Missing conversation must fail")
        } catch MacDataManagementError.conversationUnavailable { }
        XCTAssertEqual(writes, 0)
        let task = Task { @MainActor in
            try await MacCanonicalBackupStream.appendPagedConversations(["c"],
                loadPage: { _, _, _ in XCTFail("Cancelled export cannot read"); return nil },
                readRange: { _, _, _, _, _, _ in throw MacApplicationServiceError.invalidResponse },
                readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { _ in writes += 1 })
        }
        task.cancel()
        do { try await task.value; XCTFail("Expected cancellation") } catch is CancellationError { }
        XCTAssertEqual(writes, 0)
    }

    func testNativeExportAndAtomicRestorePreserveOriginalPartDigestAndTimestamps() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("NolaPartMetadata-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("nola.db")
        let port = try MacApplicationPort(databaseURL: database)
        let semanticJSON = " { \"source_uri\":\"file:///evidence\", \"integer\":9007199254740993, \"result\":\""
            + String(repeating: "中😀", count: 30_000) + "\" } "
        let original = MacCanonicalConversation(
            id: "metadata", title: "Original", revision: 1, createdAtMs: 100, updatedAtMs: 900,
            settingsJSON: nil, messages: [MacCanonicalConversationMessage(
                id: "message", runID: nil, role: "assistant", status: "completed", ordinal: 1,
                createdAtMs: 110, completedAtMs: 890, parts: [
                    MacCanonicalConversationPart(id: "text", kind: "text", ordinal: 1, state: "completed",
                        contentText: "Original text", contentJSON: nil, contentDigest: "text-digest",
                        createdAtMs: 120, updatedAtMs: 180),
                    MacCanonicalConversationPart(id: "semantic", kind: "tool_result", ordinal: 2, state: "completed",
                        contentText: "Source evidence", contentJSON: semanticJSON,
                        contentDigest: "semantic-digest", createdAtMs: 300, updatedAtMs: 800)
                ])])
        try MacArchiveRestoreWriter.restore([MacBackupConversation(original)], port: port)
        var bytes = Data("[".utf8)
        try await MacCanonicalBackupStream.appendPagedConversations([original.id],
            loadPage: { id, after, revision in
                try port.loadConversationExportPage(id, afterOrdinal: after, expectedRevision: revision)
            }, readRange: { id, message, part, field, revision, offset in
                try port.readConversationExportRange(id, messageID: message, partID: part, field: field,
                    expectedRevision: revision, offsetBytes: offset)
            }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { bytes.append($0) })
        bytes.append(contentsOf: "]".utf8)
        let backup = try JSONDecoder().decode([MacBackupConversation].self, from: bytes)[0]
        XCTAssertEqual(backup.canonical.messages, original.messages)
        let reopened = try MacApplicationPort(databaseURL: database)
        try MacArchiveRestoreWriter.restore([backup], port: reopened)
        let page = try XCTUnwrap(reopened.loadConversationExportPage(original.id))
        for (actual, expected) in zip(page.messages[0].parts, original.messages[0].parts) {
            XCTAssertEqual(actual.id, expected.id)
            XCTAssertEqual(actual.contentDigest, expected.contentDigest)
            XCTAssertEqual(actual.createdAtMs, expected.createdAtMs)
            XCTAssertEqual(actual.updatedAtMs, expected.updatedAtMs)
        }
        var restoredJSON = ""
        var offset: UInt64 = 0
        while true {
            let json = try reopened.readConversationExportRange(original.id, messageID: "message", partID: "semantic",
                field: .contentJSON, expectedRevision: page.conversation.revision, offsetBytes: offset)
            restoredJSON += json.text
            offset = json.nextOffsetBytes
            if json.endOfContent { break }
        }
        XCTAssertEqual(restoredJSON, semanticJSON)
    }

    func testPagedExportReadsDeclaredDerivationSeparatelyAndFailsIfMissing() async throws {
        let claim = derivationFixture(text: "")
        let page = MacConversationExportPage(conversation: .init(id: "c", title: "Claim", revision: 7,
            createdAtMs: 1, updatedAtMs: 2, settingsJSON: nil), messages: [
                .init(id: "m", runID: nil, role: "user", status: "completed", ordinal: 1,
                    createdAtMs: 1, completedAtMs: 2, parts: [
                        .init(id: "p", kind: "text", ordinal: 1, state: "completed", contentDigest: nil,
                            createdAtMs: 1, updatedAtMs: 2, contentTextUTF8Bytes: 0,
                            contentJSONUTF8Bytes: nil, hasDerivation: true)
                    ])
            ], nextOrdinal: 1, hasMore: false)
        var output = Data("[".utf8)
        var proofQueries = 0
        var pinnedRevisionChecks = 0
        try await MacCanonicalBackupStream.appendPagedConversations(["c"],
            loadPage: { _, _, revision in
                if revision == 7 { pinnedRevisionChecks += 1 }
                return page
            }, readRange: { _, _, _, _, _, _ in throw MacApplicationServiceError.invalidResponse },
            readDerivation: { owner, revision in
                proofQueries += 1
                XCTAssertEqual(owner, claim.target.owner)
                XCTAssertEqual(revision, 7)
                return MacStoredPartDerivation(derivation: claim, provenance: .localBranch)
            }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { output.append($0) })
        output.append(contentsOf: "]".utf8)
        let canonical = try JSONDecoder().decode([MacBackupConversation].self, from: output)[0].canonical
        XCTAssertEqual(canonical.messages[0].parts[0].derivation, claim)
        XCTAssertEqual(proofQueries, 1)
        // The tool-source pass and final export fence both pin the same revision.
        XCTAssertEqual(pinnedRevisionChecks, 2)
        XCTAssertFalse(String(decoding: output, as: UTF8.self).contains("provenance"))
        XCTAssertFalse(String(decoding: output, as: UTF8.self).contains("has_derivation"))
        do {
            try await MacCanonicalBackupStream.appendPagedConversations(["c"],
                loadPage: { _, _, _ in page },
                readRange: { _, _, _, _, _, _ in throw MacApplicationServiceError.invalidResponse },
                readDerivation: { _, _ in nil }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { _ in })
            XCTFail("A declared derivation must never be silently omitted")
        } catch {}
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        let manifest = try encoder.encode(MacArchiveManifestEntry.partDerivation(claim))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: manifest) as? [String: Any])
        XCTAssertEqual(object["kind"] as? String, "part_derivation")
        XCTAssertEqual(Set(object.keys), ["kind", "record"])
        XCTAssertFalse(String(decoding: manifest, as: UTF8.self).contains("provenance"))
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalConversationPart.self,
            from: Data(#"{"id":"old","kind":"text","ordinal":1,"state":"completed","content_text":"old"}"#.utf8)))
    }

    func testNativePagedBackupRestorePreservesDerivationClaimAndArchiveProvenance() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("NolaDerivation-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = try MacApplicationPort(databaseURL: root.appendingPathComponent("source.db"))
        let text = "Original branch bytes 中😀"
        let claim = derivationFixture(text: text)
        let original = MacCanonicalConversation(id: "c", title: "Claim", revision: 1,
            createdAtMs: 1, updatedAtMs: 2, settingsJSON: nil, messages: [
                .init(id: "m", runID: nil, role: "user", status: "completed", ordinal: 1,
                    createdAtMs: 1, completedAtMs: 2, parts: [
                        .init(id: "p", kind: "text", ordinal: 1, state: "completed", contentText: text,
                            contentJSON: nil, contentDigest: "original-digest", createdAtMs: 1,
                            updatedAtMs: 2, derivation: claim)
                    ])
            ])
        try MacArchiveRestoreWriter.restore([MacBackupConversation(original)], port: source)
        var bytes = Data("[".utf8)
        try await MacCanonicalBackupStream.appendPagedConversations(["c"],
            loadPage: { id, after, revision in
                try source.loadConversationExportPage(id, afterOrdinal: after, expectedRevision: revision)
            }, readRange: { id, message, part, field, revision, offset in
                try source.readConversationExportRange(id, messageID: message, partID: part,
                    field: field, expectedRevision: revision, offsetBytes: offset)
            }, readDerivation: { owner, revision in
                let record = try source.readConversationPartDerivation(owner, expectedConversationRevision: revision)
                XCTAssertEqual(record?.provenance, .archive)
                return record
            }, readToolSources: { _, _ in [] }, readToolSourceRange: { _, _, _, _ in throw MacApplicationServiceError.invalidResponse }, append: { bytes.append($0) })
        bytes.append(contentsOf: "]".utf8)
        let backup = try JSONDecoder().decode([MacBackupConversation].self, from: bytes)[0]
        XCTAssertEqual(backup.canonical.messages[0].parts[0].derivation, claim)
        XCTAssertEqual(backup.canonical.messages[0].parts[0].contentText, text)
        let targetPath = root.appendingPathComponent("target.db")
        let target = try MacApplicationPort(databaseURL: targetPath)
        try MacArchiveRestoreWriter.restore([backup], port: target)
        let reopened = try MacApplicationPort(databaseURL: targetPath)
        let page = try XCTUnwrap(reopened.loadConversationExportPage("c"))
        XCTAssertTrue(page.messages[0].parts[0].hasDerivation)
        let record = try XCTUnwrap(reopened.readConversationPartDerivation(claim.target.owner,
            expectedConversationRevision: page.conversation.revision))
        XCTAssertEqual(record.derivation, claim)
        XCTAssertEqual(record.provenance, .archive)
        XCTAssertEqual(try reopened.readConversationExportRange("c", messageID: "m", partID: "p",
            field: .contentText, expectedRevision: page.conversation.revision, offsetBytes: 0).text, text)
    }

    private func derivationFixture(text: String) -> MacPartDerivation {
        let fingerprint = MacPartFieldFingerprint(field: .contentText, totalUTF8Bytes: UInt64(text.utf8.count),
            sha256: SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined())
        let target = MacPartDerivationSource(owner: .init(conversationID: "c", messageID: "m", partID: "p"),
            runID: nil, kind: "text", state: "completed", fields: [fingerprint, nil])
        let source = MacPartDerivationSource(owner: .init(conversationID: "deleted-source", messageID: "source-m", partID: "source-p"),
            runID: nil, kind: "text", state: "completed", fields: [fingerprint, nil])
        return MacPartDerivation(version: 1, operationID: "original-branch", target: target,
            immediateSource: source, rootSource: nil)
    }

    private func fixture(textBytes: UInt64, jsonBytes: UInt64?) -> MacConversationExportPage {
        MacConversationExportPage(conversation: .init(id: "c", title: "Original", revision: 1,
            createdAtMs: 1, updatedAtMs: 2, settingsJSON: nil), messages: [
                .init(id: "m", runID: nil, role: "user", status: "completed", ordinal: 1,
                    createdAtMs: 1, completedAtMs: 2, parts: [
                        .init(id: "p", kind: "text", ordinal: 1, state: "completed",
                            contentDigest: "digest-original", createdAtMs: 11, updatedAtMs: 12,
                            contentTextUTF8Bytes: textBytes, contentJSONUTF8Bytes: jsonBytes)
                    ])
            ], nextOrdinal: 1, hasMore: false)
    }
}
