// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacOriginalTextReadModelTests: XCTestCase {
    private func request(part: String = "p", offset: UInt64 = 0) throws -> MacOriginalTextReadModel.Request {
        let reference = try JSONDecoder().decode(MacCanonicalConversationWindow.DeferredTextPart.self,
            from: JSONSerialization.data(withJSONObject: [
                "message_id": "m", "part_id": part, "total_utf8_bytes": 100,
                "preview_utf8_bytes": 0, "is_final": true, "can_read_original": true
            ]))
        return .init(conversationID: "c", reference: reference, offsetBytes: offset)
    }

    private func assertPhysicalCancellation(
        _ cancellation: MacApplicationQueryCancellation, port: MacApplicationPort,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        // An absent conversation would be a Store rejection without forwarding
        // the token. Cancellation must reach ServiceClient before any query runs.
        XCTAssertThrowsError(try port.readConversationTextRange("c", messageID: "m",
            partID: "p", offsetBytes: 0, cancellation: cancellation), file: file, line: line) { error in
            XCTAssertTrue(error is CancellationError, "Expected physical query cancellation, got \(error)",
                file: file, line: line)
        }
    }

    func testPageAndPartReplacementCancelPhysicalQueryAndIgnoreLateReply() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("reader-replacement-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("fixture.db"))
        let first = try request()
        for next in [try request(offset: 4), try request(part: "other-part")] {
            let gate = OriginalTextReadGate()
            let model = MacOriginalTextReadModel()
            let old = Task { await model.load(first) { try await gate.read($0, cancellation: $1) } }
            await gate.waitForCount(1)
            let new = Task { await model.load(next) { try await gate.read($0, cancellation: $1) } }
            await gate.waitForCount(2)
            let oldCancellation = await gate.cancellation(at: 0)
            let newCancellation = await gate.cancellation(at: 1)
            XCTAssertFalse(oldCancellation === newCancellation)
            assertPhysicalCancellation(oldCancellation, port: port)
            await gate.complete(1, text: "next")
            let newResult = await new.value
            XCTAssertEqual(newResult?.text, "next")
            await gate.complete(0, text: "late")
            let oldResult = await old.value
            XCTAssertNil(oldResult)
            XCTAssertEqual(model.selection, next)
            XCTAssertEqual(model.page?.text, "next")
            XCTAssertEqual(model.page?.partID, next.reference.partID)
            XCTAssertEqual(model.page?.offsetBytes, next.offsetBytes)
            XCTAssertFalse(model.failed)
        }
    }

    func testClosingReaderCancelsPhysicalQueryAndLateReplyCannotReopenIt() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("reader-close-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("fixture.db"))
        let input = try request()
        let gate = OriginalTextReadGate()
        let model = MacOriginalTextReadModel()
        let pending = Task { await model.load(input) { try await gate.read($0, cancellation: $1) } }
        await gate.waitForCount(1)
        model.invalidate()
        assertPhysicalCancellation(await gate.cancellation(at: 0), port: port)
        await gate.complete(0, text: "late")
        let result = await pending.value
        XCTAssertNil(result)
        XCTAssertNil(model.page)
        XCTAssertNil(model.selection)
        XCTAssertFalse(model.failed)
    }

    func testTaskCancellationCancelsPhysicalQueryEvenWhenReaderReturnsLater() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("reader-task-cancel-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("fixture.db"))
        let input = try request()
        let gate = OriginalTextReadGate()
        let model = MacOriginalTextReadModel()
        let pending = Task { await model.load(input) { try await gate.read($0, cancellation: $1) } }
        await gate.waitForCount(1)
        pending.cancel()
        assertPhysicalCancellation(await gate.cancellation(at: 0), port: port)
        await gate.complete(0, text: "late")
        let result = await pending.value
        XCTAssertNil(result)
        XCTAssertNil(model.page)
        XCTAssertFalse(model.failed)
    }

    func testAlreadyCancelledTaskCannotReplaceCurrentPageOrStartQuery() async throws {
        let model = MacOriginalTextReadModel()
        let first = try request()
        let next = try request(offset: 4)
        let page = MacCanonicalTextRange(conversationID: "c", messageID: "m", partID: "p",
            offsetBytes: 0, nextOffsetBytes: 4, totalUTF8Bytes: 100, text: "kept", endOfContent: false)
        _ = await model.load(first) { _, _ in page }
        let gate = OriginalTextReadGate()
        let obsolete = Task {
            await model.load(next) { _, _ in
                await gate.recordUnexpectedRead()
                return page
            }
        }
        obsolete.cancel()
        let result = await obsolete.value
        let queryCount = await gate.unexpectedReadCount
        XCTAssertNil(result)
        XCTAssertEqual(queryCount, 0)
        XCTAssertEqual(model.selection, first)
        XCTAssertEqual(model.page?.text, "kept")
    }
}

private actor OriginalTextReadGate {
    private var requests: [MacOriginalTextReadModel.Request] = []
    private var cancellations: [MacApplicationQueryCancellation] = []
    private var replies: [CheckedContinuation<MacCanonicalTextRange, Error>] = []
    private var waiter: (Int, CheckedContinuation<Void, Never>)?
    private(set) var unexpectedReadCount = 0

    func recordUnexpectedRead() { unexpectedReadCount += 1 }

    func read(_ request: MacOriginalTextReadModel.Request,
              cancellation: MacApplicationQueryCancellation) async throws -> MacCanonicalTextRange {
        try await withCheckedThrowingContinuation { continuation in
            requests.append(request)
            cancellations.append(cancellation)
            replies.append(continuation)
            if let (count, continuation) = waiter, requests.count >= count {
                waiter = nil
                continuation.resume()
            }
        }
    }

    func waitForCount(_ count: Int) async {
        if requests.count >= count { return }
        await withCheckedContinuation { waiter = (count, $0) }
    }

    func cancellation(at index: Int) -> MacApplicationQueryCancellation { cancellations[index] }

    func complete(_ index: Int, text: String) {
        let request = requests[index]
        let next = request.offsetBytes + UInt64(text.utf8.count)
        replies[index].resume(returning: .init(conversationID: request.conversationID,
            messageID: request.reference.messageID, partID: request.reference.partID,
            offsetBytes: request.offsetBytes, nextOffsetBytes: next,
            totalUTF8Bytes: request.reference.totalUTF8Bytes, text: text,
            endOfContent: next == request.reference.totalUTF8Bytes))
    }
}
