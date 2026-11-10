import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacCompactionRecallEndToEndTests: XCTestCase {
    func testNativeTenMillionByteUploadRemainsChunkedAndUnpublished() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaNativeUpload-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        _ = try port.createConversation(MacCanonicalCreateConversationInput(
            id: "upload", title: "Upload", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1
        ))
        let owner = MacContentUploadOwner(conversationID: "upload", importID: "import", partID: "part")
        let total: UInt64 = 10_000_000
        _ = try port.beginContentUpload(id: "content", owner: owner, byteCount: total)
        var offset: UInt64 = 0
        while offset < total {
            let count = Int(min(32_768, total - offset))
            let text = String(repeating: "x", count: count)
            let progress = try port.appendContentChunk(id: "content", owner: owner, offset: offset, text: text)
            offset += UInt64(count)
            XCTAssertEqual(progress.committedUTF8Bytes, offset)
        }
        XCTAssertEqual(try port.contentUploadProgress(id: "content", owner: owner).committedUTF8Bytes, total)
        XCTAssertEqual(try port.sealContentUpload(id: "content", owner: owner).state, .sealed)
        offset = 0
        while offset < total {
            let chunk = try port.readContentUploadChunk(id: "content", owner: owner, offset: offset)
            XCTAssertEqual(chunk.owner, owner)
            XCTAssertEqual(chunk.offsetBytes, offset)
            XCTAssertFalse(chunk.text.isEmpty)
            XCTAssertLessThanOrEqual(chunk.text.utf8.count, 32_768)
            XCTAssertTrue(chunk.text.utf8.allSatisfy { $0 == 120 })
            XCTAssertEqual(chunk.nextOffsetBytes, offset + UInt64(chunk.text.utf8.count))
            offset = chunk.nextOffsetBytes
            XCTAssertEqual(chunk.endOfContent, offset == total)
        }
        XCTAssertTrue(try XCTUnwrap(port.loadConversation("upload")).messages.isEmpty)
    }

    func testTenMillionCharacterRestoreCommitsAllConversationsWithoutSplittingMessage() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaOversizedImport-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let original = MacChatMessage(role: .user, text: "Keep my original history", createdAt: .now)
        try repository.restoreBackupSnapshot([MacStoredConversation(
            conversation: MacConversation(id: "existing", title: "Existing", preview: "", updatedAt: .now, isPinned: false),
            messages: [original]
        )])
        try repository.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(id: "valid-first", title: "Valid first", preview: "", updatedAt: .now, isPinned: false),
                messages: [MacChatMessage(role: .user, text: "Valid backup prefix", createdAt: .now)]
            ),
            MacStoredConversation(
                conversation: MacConversation(id: "oversized", title: "Oversized", preview: "", updatedAt: .now, isPinned: false),
                messages: [MacChatMessage(role: .user, text: String(repeating: "x", count: 10_000_000), createdAt: .now)]
            )
        ])
        XCTAssertNil(try port.loadConversation("existing"))
        XCTAssertEqual(try port.loadConversation("valid-first")?.messages.count, 1)
        let page = try XCTUnwrap(port.loadConversationExportPage("oversized"))
        XCTAssertEqual(page.messages.count, 1)
        XCTAssertFalse(page.hasMore)
        let message = try XCTUnwrap(page.messages.first)
        XCTAssertEqual(message.parts.count, 1)
        let part = try XCTUnwrap(message.parts.first)
        var offset: UInt64 = 0
        while offset < 10_000_000 {
            let range = try port.readConversationTextRange("oversized", messageID: message.id,
                partID: part.id, offsetBytes: offset)
            XCTAssertEqual(range.totalUTF8Bytes, 10_000_000)
            XCTAssertTrue(range.text.utf8.allSatisfy { $0 == 120 })
            offset = range.nextOffsetBytes
        }
        XCTAssertEqual(offset, 10_000_000)
    }

    func testInvalidLateArchiveEntryPreservesAllExistingHistory() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaAtomicRestore-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let original = MacChatMessage(role: .user, text: "Keep my original history", createdAt: .now)
        func snapshot(_ id: String, _ message: MacChatMessage) -> MacStoredConversation {
            MacStoredConversation(conversation: MacConversation(id: id, title: id, preview: "",
                updatedAt: .now, isPinned: false), messages: [message])
        }
        try repository.restoreBackupSnapshot([snapshot("existing", original)])
        let duplicate = MacChatMessage(role: .user, text: "same message identity", createdAt: .now)
        XCTAssertThrowsError(try repository.restoreBackupSnapshot([
            snapshot("valid-first", duplicate), snapshot("invalid-second", duplicate)
        ]))
        XCTAssertNil(try port.loadConversation("valid-first"))
        XCTAssertNil(try port.loadConversation("invalid-second"))
        let retained = try XCTUnwrap(port.loadConversation("existing"))
        XCTAssertEqual(retained.messages.first?.id, original.id.uuidString)
        XCTAssertEqual(retained.messages.first?.parts.first?.contentText, original.text)
    }

    func testCancelledArchiveRestoreLeavesExistingHistory() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaCancelledRestore-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let original = MacChatMessage(role: .user, text: "Retain on cancellation", createdAt: .now)
        try repository.restoreBackupSnapshot([MacStoredConversation(
            conversation: MacConversation(id: "existing", title: "Existing", preview: "", updatedAt: .now, isPinned: false),
            messages: [original]
        )])
        let operation = Task { try await repository.restoreCanonicalBackupSnapshotAsync([]) }
        operation.cancel()
        do {
            try await operation.value
            XCTFail("Cancelled restore must not commit an empty archive")
        } catch is CancellationError {
        }
        XCTAssertEqual(try port.loadConversation("existing")?.messages.first?.id, original.id.uuidString)
    }

    func testSwitchingSearchAcrossTenMillionCharacterConversationsUsesBoundedEvidence() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaLongInspectorSearch-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let snapshots = (0..<2).map { index in
            let marker = "TAIL-\(index)-\(UUID().uuidString)"
            let messages = (0..<100).map { ordinal in
                MacChatMessage(
                    role: .user,
                    text: ordinal == 99
                        ? String(repeating: "x", count: 100_000 - marker.count) + marker
                        : String(repeating: "x", count: 100_000),
                    createdAt: .now
                )
            }
            return (marker, MacStoredConversation(
                conversation: MacConversation(id: "long-\(index)", title: "Long", preview: "", updatedAt: .now, isPinned: false),
                messages: messages
            ))
        }
        try repository.restoreBackupSnapshot(snapshots.map { $0.1 })
        let started = ContinuousClock.now
        for iteration in 0..<12 {
            let index = iteration % snapshots.count
            let (marker, snapshot) = snapshots[index]
            let results = try await repository.searchConversationAsync(
                conversationID: snapshot.conversation.id, query: marker
            )
            XCTAssertEqual(results.count, 1)
            XCTAssertEqual(results.first?.messageID, snapshot.messages.last?.id.uuidString)
            XCTAssertEqual(results.first?.messageOrdinal, 100)
            XCTAssertTrue(results.first?.snippet.hasSuffix(marker) == true)
            XCTAssertLessThanOrEqual(results.first?.snippet.count ?? Int.max, 1_600)
            let foreign = try await repository.searchConversationAsync(
                conversationID: snapshots[1 - index].1.conversation.id, query: marker
            )
            XCTAssertTrue(foreign.isEmpty)
        }
        print("Alternating native 10M history queries: \(started.duration(to: .now))")
    }

    func testNativeRecallOfTenMillionCharactersReturnsBoundedTailEvidence() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaMacBoundedRecall-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        let marker = "HISTORY-TAIL-\(UUID().uuidString)"
        let text = String(repeating: "x", count: 10_000_000 - marker.count) + marker
        try repository.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(id: "large", title: "Large", preview: "", updatedAt: .now, isPinned: false),
                messages: [MacChatMessage(role: .user, text: text, createdAt: .now)]
            )
        ])
        let started = ContinuousClock.now
        let data = MacConversationRecallBridge.responseData(
            requestID: "bounded-recall", conversationID: "large",
            arguments: ["query": marker, "limit": 20], store: store
        )
        print("Native 10M recall: \(started.duration(to: .now)), response bytes: \(data.count)")
        XCTAssertLessThan(data.count, 8_000)
        let envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(envelope["is_error"] as? Bool, false)
        let output = try XCTUnwrap(envelope["output"] as? [String: Any])
        let matches = try XCTUnwrap(output["hits"] as? [[String: Any]])
        XCTAssertEqual(matches.count, 1)
        let match = try XCTUnwrap(matches.first)
        XCTAssertTrue((match["snippet"] as? String)?.hasSuffix(marker) == true)
        XCTAssertGreaterThan(try XCTUnwrap(match["start_character"] as? Int), 9_000_000)
        let source = try XCTUnwrap(match["source"] as? [String: Any])
        XCTAssertNotNil(source["message_id"] as? String)
        XCTAssertNotNil(source["part_id"] as? String)
    }

    func testCanonicalCompactionTracePersistsSummaryBesideOriginalTranscript() throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("NolaMacCompactionRecall-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        try FileManager.default.createDirectory(at: baseDirectory, withIntermediateDirectories: true)
        let store = try MacApplicationPort(
            databaseURL: baseDirectory.appendingPathComponent("nola.db")
        )
        let repository = MacCanonicalConversationRepository(store: store)
        let conversationID = "conversation-compaction-e2e"
        let originalText = "The launch constraint is to preserve the cobalt migration note."
        try repository.restoreBackupSnapshot([
            MacStoredConversation(
            conversation: MacConversation(
                id: conversationID,
                title: "Compaction E2E",
                preview: originalText,
                updatedAt: .now,
                isPinned: false
            ),
                messages: [MacChatMessage(role: .user, text: originalText, createdAt: .now)]
            )
        ])

        let trace: [String: Any] = [
            "schema_version": 1,
            "trace_id": "trace-compaction-e2e",
            "run_id": "run-compaction-e2e",
            "conversation_id": conversationID,
            "message_id": "assistant-compaction-e2e",
            "turn_id": "turn-compaction-e2e",
            "event_id": "event-compaction-e2e",
            "sequence": 4,
            "emitted_at": "2026-07-23T08:00:00.000Z",
            "kind": "context.compacted",
            "status": "succeeded",
            "visibility": "detail",
            "payload": [
                "phase": "compacted",
                "summary": "Keep the cobalt migration note as a durable launch constraint.",
                "detail": "Compacted 24 items; 3 remain",
            ],
        ]
        let data = try JSONSerialization.data(withJSONObject: ["trace": trace])
        let line = try XCTUnwrap(String(data: data, encoding: .utf8))
        let event = try XCTUnwrap(
            MacAgentHostRuntime.decode(line: line, conversationID: conversationID)
        )

        let saved = MacAgentHostRuntime.persistCompactionCheckpointIfNeeded(event, store: store)
        XCTAssertEqual(saved?.checkpoint.id, "event-compaction-e2e")

        let recallData = MacConversationRecallBridge.responseData(
            requestID: "recall-1",
            conversationID: conversationID,
            arguments: ["query": "cobalt migration note"],
            store: store
        )
        let recall = try XCTUnwrap(
            JSONSerialization.jsonObject(with: recallData) as? [String: Any]
        )
        let output = try XCTUnwrap(recall["output"] as? [String: Any])
        let matches = try XCTUnwrap(output["hits"] as? [[String: Any]])
        XCTAssertEqual(matches.count, 1)
        XCTAssertEqual(matches.first?["snippet"] as? String, originalText)
        let source = try XCTUnwrap(matches.first?["source"] as? [String: Any])
        XCTAssertNotNil(source["message_id"] as? String)
        XCTAssertNotNil(source["part_id"] as? String)

        _ = MacAgentHostRuntime.persistCompactionCheckpointIfNeeded(event, store: store)
        XCTAssertEqual(try store.countContextCheckpoints(conversationID), 1)
    }
}
