// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS activity read ownership, historical part details, journal browser reader, original text read model, projection measurement/observation coordinator, transcript uploader, and task guidance outbox suite.

import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

final class MacProjectionMeasurementAndTranscriptUploaderTests: XCTestCase {
    private func ticket(generation: UInt64 = 7) -> MacProjectionApplyMeasurement.Ticket {
        .init(id: "observation", session: MacProjectionApplyMeasurement.sessionDigest("abc"),
            stage: .windowRefresh, generation: generation, startedNS: 100)
    }

    func testProjectionApplyMeasurementSessionHashStaleCursorAndOutcomes() throws {
        XCTAssertEqual(MacProjectionApplyMeasurement.sessionDigest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        let sample = try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: .applied,
            cursor: .init(generation: 7, eventSequence: 42, projectionRevision: 12), nowNS: 145, selected: false))
        XCTAssertEqual(sample.stage, "window_refresh_proxy")
        XCTAssertEqual(sample.elapsedNS, 45)
        XCTAssertEqual(sample.monotonicNS, 145)
        XCTAssertEqual(sample.eventSequence, 42)
        XCTAssertEqual(sample.projectionRevision, 12)
        XCTAssertFalse(sample.selected)
        XCTAssertNotEqual(sample.session, "abc")

        for outcome in [MacProjectionApplyMeasurement.Outcome.applied, .unchanged] {
            let stale = try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: outcome,
                cursor: .init(generation: 8, eventSequence: 99, projectionRevision: 88), nowNS: 145, selected: false))
            XCTAssertEqual(stale.outcome, "discarded")
            XCTAssertNil(stale.eventSequence)
            XCTAssertNil(stale.projectionRevision)
            XCTAssertEqual(stale.generation, 7)
        }
        for outcome in [MacProjectionApplyMeasurement.Outcome.failed, .cancelled, .discarded] {
            let unapplied = try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: outcome,
                cursor: .init(generation: 7, eventSequence: 42, projectionRevision: 12), nowNS: 145, selected: false))
            XCTAssertNil(unapplied.eventSequence)
            XCTAssertNil(unapplied.projectionRevision)
            XCTAssertEqual(unapplied.outcome, outcome.rawValue)
        }
        XCTAssertNil(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: .applied, cursor: nil, nowNS: 99, selected: false))
        XCTAssertNil(try XCTUnwrap(MacProjectionApplyMeasurement.sample(ticket: ticket(), outcome: .unchanged, cursor: nil, nowNS: 145, selected: false)).eventSequence)
    }

    @MainActor
    func testTranscriptBodyUTF8ChunkingSealedUploadReplayAndFieldIdentity() throws {
        let text = String(repeating: "x", count: 32_767) + "😀" + String(repeating: "\u{0301}", count: 40_000) + "\"\n\\中"
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

        let root = FileManager.default.temporaryDirectory.appendingPathComponent("NolaTranscriptUploader-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("nola.db")
        let owner = MacContentUploadOwner(conversationID: "c", importID: "i", partID: "p")
        let original = String(repeating: "a", count: 32_767) + "😀" + String(repeating: "中", count: 4_000)
        let first = try MacApplicationPort(databaseURL: database)
        _ = try first.createConversation(MacCanonicalCreateConversationInput(id: "c", title: "Upload", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1))
        let uploaded = try MacTranscriptBodyUploader(port: first).upload(original, owner: owner, messageID: "m")
        let reopened = try MacApplicationPort(databaseURL: database)
        let uploader = MacTranscriptBodyUploader(port: reopened)
        XCTAssertEqual(try uploader.upload(original, owner: owner, messageID: "m"), uploaded)
        XCTAssertThrowsError(try uploader.upload("b" + original.dropFirst(), owner: owner, messageID: "m"))
        XCTAssertTrue(try XCTUnwrap(reopened.loadConversation("c")).messages.isEmpty)
        XCTAssertTrue(try reopened.readContentUploadChunk(id: uploaded.uploadID, owner: owner, offset: 0).text.hasPrefix("aaa"))

        let fieldOwner = MacContentUploadOwner(conversationID: "c", importID: "i-fields", partID: "p-fields")
        let contentText = try uploader.upload("{}", owner: fieldOwner, messageID: "m", field: .contentText)
        let contentJSON = try uploader.upload("{}", owner: fieldOwner, messageID: "m", field: .contentJSON)
        XCTAssertNotEqual(contentText.uploadID, contentJSON.uploadID)
        XCTAssertEqual(try uploader.upload("{}", owner: fieldOwner, messageID: "m", field: .contentJSON), contentJSON)
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: encoder.encode(MacArchivePartUpload(contentJSON, field: .contentJSON))) as? [String: Any])
        XCTAssertEqual(wire["field"] as? String, "content_json")
    }
}

@MainActor
final class MacActivityReadOwnershipTests: XCTestCase {
    func testFilterAndManualRefreshNeverReuseAnotherQueriesCursorOrDoubleLoad() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = ActivityFixture()
        let host = NSHostingView(rootView: AppleActivityView(load: { try await fixture.load($0) },
            detail: { _ in nil }, openSource: { _ in }).environment(\.accessibilityEnabled, true)
            .environment(\.colorScheme, .light)
            .background(Color(nsColor: .windowBackgroundColor))
            .frame(width: 600, height: 560))
        let window = makeWindow(host)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await wait { fixture.queries.count == 1 && self.element(host, "activity.loadMore") != nil }
        let oldLoadMore = try XCTUnwrap(element(host, "activity.loadMore"))
        let agent = try XCTUnwrap(element(host, "activity.source.agent"))
        let refresh = try XCTUnwrap(element(host, "activity.refresh"))
        XCTAssertTrue(agent.accessibilityPerformPress?() ?? false)
        // These controls were captured from the previous frame. Their queued
        // actions still have to respect the current query, not the old cursor.
        _ = oldLoadMore.accessibilityPerformPress?()
        XCTAssertTrue(refresh.accessibilityPerformPress?() ?? false)
        try await wait { fixture.queries.count >= 2 }
        try await Task.sleep(for: .milliseconds(350))
        XCTAssertEqual(fixture.queries.count, 2)
        XCTAssertEqual(fixture.queries.last?.source, "agent")
        XCTAssertNil(fixture.queries.last?.cursor)
        XCTAssertNotNil(element(host, "activity.run.agent-result"))
        XCTAssertNil(element(host, "activity.run.all-result"))
        for scheme: ColorScheme in [.light, .dark] {
            host.rootView = AppleActivityView(load: { try await fixture.load($0) },
                detail: { _ in nil }, openSource: { _ in }).environment(\.accessibilityEnabled, true)
                .environment(\.colorScheme, scheme)
                .background(Color(nsColor: .windowBackgroundColor))
                .frame(width: 600, height: 560)
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            try await Task.sleep(for: .milliseconds(150))
            host.layoutSubtreeIfNeeded()
            XCTAssertNotNil(element(host, "activity.search"))
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let image = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])),
                uniformTypeIdentifier: "public.png")
            image.name = "activity-search-prompt-\(scheme == .dark ? "dark" : "light")"
            image.lifetime = .keepAlways
            add(image)
        }
    }

    func testRefreshFailureRetainsTheLastSuccessfulRowsAndCanRetry() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let fixture = ActivityFixture()
        let host = NSHostingView(rootView: AppleActivityView(load: { try await fixture.load($0) },
            detail: { _ in nil }, openSource: { _ in }).environment(\.accessibilityEnabled, true)
            .frame(width: 600, height: 560))
        let window = makeWindow(host)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await wait { self.element(host, "activity.run.all-result") != nil }
        fixture.failNext = true
        XCTAssertTrue(try XCTUnwrap(element(host, "activity.refresh")).accessibilityPerformPress?() ?? false)
        try await wait { fixture.queries.count == 2 }
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertNotNil(element(host, "activity.run.all-result"), "A read failure is not an empty activity history")
        XCTAssertTrue(try XCTUnwrap(element(host, "activity.refresh")).accessibilityPerformPress?() ?? false)
        try await wait { fixture.queries.count == 3 }
        XCTAssertNotNil(element(host, "activity.run.all-result"))
    }

    private func makeWindow<Content: View>(_ host: NSHostingView<Content>) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 560),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.orderFrontRegardless()
        return window
    }
    private func wait(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(4)
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertTrue(condition())
    }
    private func element(_ root: AnyObject, _ id: String) -> AnyObject? {
        var queue = [root], seen = Set<ObjectIdentifier>(), index = 0
        while index < queue.count && index < 4000 {
            let object = queue[index]; index += 1
            guard seen.insert(ObjectIdentifier(object)).inserted else { continue }
            if (object.accessibilityIdentifier?() ?? nil) == id { return object }
            let children = object.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = object as? NSView { queue += view.subviews }
        }
        return nil
    }
}

@MainActor private final class ActivityFixture {
    var queries: [AppleActivityQuery] = []
    var failNext = false
    private enum Failure: Error { case unavailable }
    func load(_ query: AppleActivityQuery) async throws -> AppleActivityPage {
        queries.append(query)
        if failNext { failNext = false; throw Failure.unavailable }
        let id = query.source == "agent" ? "agent-result" : "all-result"
        let cursor: Any = query.source == "agent" ? NSNull() : ["created_at_ms": 1, "run_id": id]
        return try JSONDecoder().decode(AppleActivityPage.self, from: JSONSerialization.data(withJSONObject: [
            "entries": [["run_id": id, "conversation_id": "conversation-" + id, "assistant_message_id": "assistant-" + id,
                "source": "agent", "title": id, "created_at_ms": 1, "completed_at_ms": 2,
                "status": "answered", "needs_response": false, "is_current": true]], "next_cursor": cursor]))
    }
}

@MainActor
final class MacHistoricalPartDetailTests: XCTestCase {
    private func selection(_ conversation: String = "c", sequence: UInt64 = 1) -> MacHistoricalPartSelection {
        .init(owner: .init(conversationID: conversation, messageID: "m", partID: "p"),
              canonicalRevision: 2, windowSequence: sequence)
    }
    private func data(_ selection: MacHistoricalPartSelection, state: String = "supported") throws -> Data {
        try JSONSerialization.data(withJSONObject: [
            "owner": ["conversation_id": selection.owner.conversationID,
                      "message_id": selection.owner.messageID, "part_id": selection.owner.partID],
            "canonical_revision": selection.canonicalRevision, "canonical_kind": "text",
            "schema": "message_part_view", "binding": "canonical",
            "status": ["state": state], "fields": ["markdown": ["availability": "complete", "value": "original 中😀"]]
        ])
    }
    func testPortRejectsWrongOwnerRevisionAndOversizedResponse() throws {
        let expected = selection()
        let raw = try data(expected)
        XCTAssertNotNil(try MacApplicationPort.decodeHistoricalPart(raw, owner: expected.owner, revision: 2))
        XCTAssertThrowsError(try MacApplicationPort.decodeHistoricalPart(raw, owner: selection("other").owner, revision: 2))
        XCTAssertThrowsError(try MacApplicationPort.decodeHistoricalPart(raw, owner: expected.owner, revision: 3))
        XCTAssertThrowsError(try MacApplicationPort.decodeHistoricalPart(Data(repeating: 32, count: 32769), owner: expected.owner, revision: 2))
    }
    func testLateDifferentConversationOrWindowResponseNeverOverwritesSelection() async throws {
        let gate = DetailGate()
        let model = MacHistoricalPartDetailModel(reader: { await gate.read($0) })
        let first = selection()
        let second = selection("other", sequence: 5)
        let firstValue = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(first))
        let secondValue = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(second))
        let old = Task { await model.load(first) }
        await gate.waitForCount(1)
        model.invalidate()
        let new = Task { await model.load(second) }
        await gate.waitForCount(2)
        await gate.complete(1, value: secondValue)
        await new.value
        await gate.complete(0, value: firstValue)
        await old.value
        XCTAssertEqual(model.selection, second)
        XCTAssertEqual(model.state, .loaded(secondValue))
        // Even a same-owner window cursor update invalidates an outstanding read.
        let pending = Task { await model.load(second) }
        await gate.waitForCount(3)
        model.invalidate()
        await gate.complete(2, value: secondValue)
        await pending.value
        XCTAssertNil(model.selection)
        XCTAssertEqual(model.state, .idle)
    }
    func testDeferredAndNotLoadedStayExplicitAndWrongOwnerFails() async throws {
        let expected = selection()
        for state in ["deferred", "not_loaded"] {
            let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(expected, state: state))
            let model = MacHistoricalPartDetailModel(reader: { _ in projection })
            await model.load(expected)
            XCTAssertEqual(model.state, .loaded(projection))
        }
        let wrong = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.HistoricalProjection.self, from: data(selection("other")))
        let model = MacHistoricalPartDetailModel(reader: { _ in wrong })
        await model.load(expected)
        XCTAssertEqual(model.state, .failed)
    }

    func testUnsupportedLegacyTextUsesCanonicalPreviewForChatDisplay() throws {
        let expected = selection()
        var object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data(expected)) as? [String: Any]
        )
        object["status"] = [
            "state": "unsupported",
            "code": "schema_missingfield",
        ]
        object["canonical_text"] = [
            "text": "背诵出师表",
            "total_utf8_bytes": 15,
        ]
        let projection = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.HistoricalProjection.self,
            from: JSONSerialization.data(withJSONObject: object)
        )

        XCTAssertNil(projection.decodedText)
        XCTAssertEqual(projection.visibleText, "背诵出师表")
        XCTAssertTrue(projection.statusLabel.contains("schema_missingfield"))
    }
    func testRealWindowSelectionQueriesUnloadedPartWithoutChangingStream() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("history.db"))
        _ = try port.createConversation(.init(id: "c", title: "History", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1))
        let parts = try (0..<64).map { index -> MacCanonicalMessagePartInput in
            let id = "p\(index)"
            let raw = try JSONSerialization.data(withJSONObject: ["kind": "text", "part_id": id,
                "revision": 1, "markdown": String(repeating: "原", count: 80), "is_final": true])
            return .init(id: id, kind: "text", ordinal: Int64(index + 1), state: "completed",
                contentText: "", contentJSON: String(decoding: raw, as: UTF8.self), contentDigest: nil,
                createdAtMs: 1, updatedAtMs: 1)
        }
        _ = try port.importConversationTranscript(.init(importID: "import", source: "external_archive", conversationID: "c",
            expectedMessageCount: 0, messages: [.init(id: "m", runID: nil, role: "assistant", status: "completed",
                ordinal: 1, createdAtMs: 1, completedAtMs: 1, parts: parts)], importedAtMs: 2))
        let before = try port.loadConversationWindow("c")
        let projected = try MacCanonicalConversationRepository.projectMessages(before)
        let descriptor = try XCTUnwrap(projected.first?.historicalDetails?.last)
        XCTAssertEqual(descriptor.windowSequence, before.eventSequence)
        guard case .historical(let last) = before.projection.messages[0].parts.last else {
            return XCTFail("missing historical Part")
        }
        XCTAssertEqual(last.projection.status.state, .notLoaded)
        let model = MacHistoricalPartDetailModel(reader: { selection in
            try port.readHistoricalPart(selection.owner, expectedConversationRevision: selection.canonicalRevision)
        })
        await model.load(descriptor)
        guard case .loaded(let value) = model.state else { return XCTFail("detail unavailable") }
        XCTAssertEqual(value.status.state, .supported)
        XCTAssertEqual(value.decodedText, String(repeating: "原", count: 80))
        let after = try port.loadConversationWindow("c")
        XCTAssertEqual(after.eventSequence, before.eventSequence)
        XCTAssertEqual(after.projection, before.projection)
        XCTAssertEqual(try MacCanonicalConversationRepository.projectMessages(after), projected)
    }
    func testActualStoreHistoricalPublicReaderNeverRevealsPendingOrRejectedTextFallback() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("historical-review-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("review.db"))
        _ = try port.createConversation(.init(id: "review", title: "Review", settingsJSON: nil, createdAtMs: 1, updatedAtMs: 1))
        let reviews = ["pending", "rejected", "approved"]
        let parts = try reviews.enumerated().map { index, review -> MacCanonicalMessagePartInput in
            let marker = "HISTORICAL-\(review)-PRIVATE-OR-PUBLIC"
            let raw = try JSONSerialization.data(withJSONObject: ["kind":"text", "part_id":review,
                "revision":1, "markdown":marker, "is_final":true, "review":review])
            return .init(id:review, kind:"text", ordinal:Int64(index + 1), state:"completed",
                contentText:marker, contentJSON:String(decoding:raw,as:UTF8.self), contentDigest:nil,
                createdAtMs:1, updatedAtMs:1)
        }
        _ = try port.importConversationTranscript(.init(importID:"review-import", source:"external_archive", conversationID:"review",
            expectedMessageCount:0, messages:[.init(id:"message", runID:nil, role:"assistant", status:"completed",
                ordinal:1, createdAtMs:1, completedAtMs:1, parts:parts)], importedAtMs:2))
        let before = try port.loadConversationWindow("review")
        let revision = try XCTUnwrap(Int64(exactly:before.projection.revision))
        for review in reviews {
            let owner = MacCanonicalPartOwner(conversationID:"review", messageID:"message", partID:review)
            let projection = try XCTUnwrap(port.readHistoricalPart(owner, expectedConversationRevision:revision))
            if review == "approved" {
                XCTAssertEqual(projection.visibleText, "HISTORICAL-approved-PRIVATE-OR-PUBLIC")
            } else {
                XCTAssertNil(projection.decodedText)
                XCTAssertNil(projection.canonicalText)
                XCTAssertNil(projection.visibleText, "Neither structured text nor canonical fallback may disclose an unapproved Part")
            }
            XCTAssertThrowsError(try port.readHistoricalPart(owner, expectedConversationRevision:revision + 1))
        }
        let after = try port.loadConversationWindow("review")
        XCTAssertEqual(after.projection,before.projection)
        XCTAssertEqual(after.eventSequence,before.eventSequence)
    }

    func testActualStoreHistoricalTaskPreservesSealedAsDistinctFromCompleted() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("sealed-history-\(UUID())")
        try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        defer { try? FileManager.default.removeItem(at:directory) }
        let port=try MacApplicationPort(databaseURL:directory.appendingPathComponent("sealed.db"))
        _=try port.createConversation(.init(id:"sealed",title:"Sealed",settingsJSON:nil,createdAtMs:1,updatedAtMs:1))
        let raw=try JSONSerialization.data(withJSONObject:["kind":"task","part_id":"task-part","revision":1,
            "task_id":"task","display_key":"Task","state":"sealed","cancel_requested":false])
        _=try port.importConversationTranscript(.init(importID:"sealed-import",source:"external_archive",conversationID:"sealed",
            expectedMessageCount:0,messages:[.init(id:"message",runID:nil,role:"assistant",status:"completed",ordinal:1,
                createdAtMs:1,completedAtMs:1,parts:[.init(id:"task-part",kind:"task",ordinal:1,state:"completed",
                    contentText:"",contentJSON:String(decoding:raw,as:UTF8.self),contentDigest:nil,createdAtMs:1,updatedAtMs:1)])],importedAtMs:2))
        let window=try port.loadConversationWindow("sealed")
        let owner=MacCanonicalPartOwner(conversationID:"sealed",messageID:"message",partID:"task-part")
        let projection=try XCTUnwrap(port.readHistoricalPart(owner,expectedConversationRevision:try XCTUnwrap(Int64(exactly:window.projection.revision))))
        guard case .complete(.string(let state))?=projection.fields["state"] else { return XCTFail("Actual Store task state") }
        XCTAssertEqual(state,"sealed")
        XCTAssertNotEqual(state,"completed")
    }

}

private actor DetailGate {
    typealias Projection = MacCanonicalConversationProjectionV1.HistoricalProjection
    private var requests: [CheckedContinuation<Projection?, Never>] = []
    private var waiter: (Int, CheckedContinuation<Void, Never>)?
    func read(_ selection: MacHistoricalPartSelection) async -> Projection? {
        await withCheckedContinuation { continuation in
            requests.append(continuation)
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
    func complete(_ index: Int, value: Projection) { requests[index].resume(returning: value) }
}

@MainActor
final class MacJournalBrowserReaderTests: XCTestCase {
    func testActualCommittedToolDrivesBrowserChromeAndColdRead() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "browser-conversation",
            runID: "browser-run", assistantMessageID: "browser-assistant",
            tool: ScriptedCanonicalTool(callID: "navigate", capabilityID: "browser_navigate",
                title: "Open documentation", summary: "Page loaded",
                input: ["url": "https://example.com/requested"],
                output: #"{"url":"https://example.com/final","title":"Documentation"}"#,
                outputPreview: "Page loaded", resourceURI: "https://example.com/final"))
        _ = try fixture.finish("The page is ready.")
        let cursor = try XCTUnwrap(fixture.heads().first)
        let reference = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: cursor)
        let reader = MacJournalBrowserReader()
        let updates = try await reader.read(reference)
        let tool = try XCTUnwrap(updates.first)
        XCTAssertEqual(updates.count, 1)
        XCTAssertEqual(tool.toolName, "browser_navigate")
        XCTAssertEqual(tool.state, "completed")
        let snapshot = try XCTUnwrap(MacConversationBrowserSnapshot.reducingCoreTool(
            toolName: tool.toolName, state: tool.state, arguments: tool.arguments,
            structuredOutput: tool.output, outputPreview: nil, previous: nil))
        XCTAssertEqual(snapshot.currentURL, "https://example.com/final")
        XCTAssertEqual(snapshot.currentTitle, "Documentation")
        let cold = try await MacJournalBrowserReader().read(reference)
        XCTAssertEqual(cold.first?.arguments, tool.arguments)
        XCTAssertEqual(cold.first?.output, tool.output)
        let unchanged = try await reader.read(reference)
        XCTAssertEqual(unchanged.count, 1, "Repeated cursor does not duplicate browser operations")
    }

    func testNonBrowserToolsAndDifferentRunsCannotLeakIntoBrowserChrome() async throws {
        let reader = MacJournalBrowserReader()
        let browser = try MacAssistantJournalTestFixture(conversationID: "shared-conversation",
            runID: "old-run", assistantMessageID: "old-assistant",
            tool: ScriptedCanonicalTool(callID: "old-browser", capabilityID: "browser_navigate",
                title: "Open", summary: "Opened", input: ["url": "https://example.com/old"],
                output: #"{"url":"https://example.com/old"}"#, outputPreview: "Opened",
                resourceURI: "https://example.com/old"))
        _ = try browser.finish("Opened")
        let oldUpdates = try await reader.read(.init(databaseURL: browser.databaseURL,
            cursor: XCTUnwrap(browser.heads().first)))
        XCTAssertEqual(oldUpdates.count, 1)
        let next = try MacAssistantJournalTestFixture(conversationID: "shared-conversation",
            runID: "new-run", assistantMessageID: "new-assistant",
            tool: ScriptedCanonicalTool(callID: "read", capabilityID: "read_file", title: "Read",
                summary: "Read", input: ["path": "/tmp/read.txt"],
                output: #"{"url":"https://example.com/not-browser"}"#, outputPreview: "Read",
                resourceURI: "file:///tmp/read.txt"))
        _ = try next.finish("Read")
        let nextUpdates = try await reader.read(.init(databaseURL: next.databaseURL,
            cursor: XCTUnwrap(next.heads().first)))
        XCTAssertTrue(nextUpdates.isEmpty)
    }
}

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

@MainActor
final class MacProjectionObservationCoordinatorTests: XCTestCase {
    func testCancelledPresentationReadDoesNotStartWork() async {
        let read = Task { @MainActor in
            try await MacConversationReadTask.run {
                XCTFail("A cancelled page must not start storage work")
                return 1
            }
        }
        read.cancel()
        do {
            _ = try await read.value
            XCTFail("Cancelled read returned a value")
        } catch is CancellationError {
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testCancellingPresentationReadStopsFollowingStagesAndDrainsCurrentCall() async {
        let started = expectation(description: "First storage call entered")
        let release = DispatchSemaphore(value: 0)
        defer { release.signal() }
        let read = Task {
            try await MacConversationReadTask.run {
                XCTAssertFalse(Thread.isMainThread)
                started.fulfill()
                guard release.wait(timeout: .now() + 5) == .success else {
                    throw NSError(domain: "Read gate timed out", code: 1)
                }
                try Task.checkCancellation()
                XCTFail("Cancellation must skip subsequent pages and token estimation")
                return 1
            }
        }
        await fulfillment(of: [started], timeout: 5)
        read.cancel()
        release.signal()
        do {
            _ = try await read.value
            XCTFail("Cancelled read published a value")
        } catch is CancellationError {
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
        let current = try? await MacConversationReadTask.run { 2 }
        XCTAssertEqual(current, 2, "A cancelled page must not cancel a replacement read")
    }

    private final class Gate {
        private var continuation: CheckedContinuation<Void, Never>?
        private var opened = false

        func wait() async {
            if opened { return }
            await withCheckedContinuation { continuation = $0 }
        }

        func open() {
            opened = true
            continuation?.resume()
            continuation = nil
        }
    }

    func testReplacementDrainsOldObserverBeforeStartingNewOne() async {
        let coordinator = MacProjectionObservationCoordinator()
        let oldStarted = Gate()
        let oldRelease = Gate()
        let newStarted = Gate()
        let newRelease = Gate()
        var events: [String] = []
        let old = coordinator.start(conversationID: "conversation") { _ in
            events.append("old started")
            oldStarted.open()
            await oldRelease.wait()
            events.append("old finished")
        }
        await oldStarted.wait()
        let current = coordinator.start(conversationID: "conversation") { _ in
            events.append("new started")
            newStarted.open()
            await newRelease.wait()
        }
        XCTAssertFalse(coordinator.isCurrent(old))
        XCTAssertTrue(coordinator.isCurrent(current))
        await Task.yield()
        XCTAssertEqual(events, ["old started"])
        oldRelease.open()
        await newStarted.wait()
        XCTAssertEqual(events, ["old started", "old finished", "new started"])
        XCTAssertTrue(coordinator.isCurrent(current), "Old completion must not release its successor")
        newRelease.open()
        await coordinator.cancelAndDrain(conversationID: "conversation")
        XCTAssertFalse(coordinator.isCurrent(current))
    }

    func testCancelledObserverRemainsDrainableByReplacement() async {
        let coordinator = MacProjectionObservationCoordinator()
        let started = Gate()
        let release = Gate()
        let replacementStarted = Gate()
        let old = coordinator.start(conversationID: "conversation") { _ in
            started.open()
            await release.wait()
        }
        await started.wait()
        coordinator.cancel(conversationID: "conversation")
        XCTAssertFalse(coordinator.isCurrent(old))
        var replacementHasStarted = false
        coordinator.start(conversationID: "conversation") { _ in
            replacementHasStarted = true
            replacementStarted.open()
        }
        await Task.yield()
        XCTAssertFalse(replacementHasStarted)
        release.open()
        await replacementStarted.wait()
        await coordinator.cancelAndDrain(conversationID: "conversation")
    }

    func testCancellingQueuedReplacementDoesNotInvokeItsOperation() async {
        let coordinator = MacProjectionObservationCoordinator()
        let started = Gate()
        let release = Gate()
        coordinator.start(conversationID: "conversation") { _ in
            started.open()
            await release.wait()
        }
        await started.wait()
        var invoked = false
        coordinator.start(conversationID: "conversation") { _ in invoked = true }
        coordinator.cancel(conversationID: "conversation")
        release.open()
        await coordinator.cancelAndDrain(conversationID: "conversation")
        XCTAssertFalse(invoked)
    }

    func testDifferentConversationsDoNotWaitForEachOther() async {
        let coordinator = MacProjectionObservationCoordinator()
        let started = Gate()
        let release = Gate()
        let otherStarted = Gate()
        coordinator.start(conversationID: "first") { _ in
            started.open()
            await release.wait()
        }
        await started.wait()
        coordinator.start(conversationID: "second") { _ in otherStarted.open() }
        await otherStarted.wait()
        await coordinator.cancelAndDrain(conversationID: "second")
        release.open()
        await coordinator.cancelAndDrain(conversationID: "first")
    }

    func testActiveObservationRemainsReusableWhileOtherConversationIsCancelled() async {
        let coordinator = MacProjectionObservationCoordinator()
        let release = Gate()
        let active = coordinator.start(conversationID: "active") { _ in await release.wait() }
        let other = coordinator.start(conversationID: "other") { _ in await release.wait() }
        XCTAssertTrue(coordinator.isObserving(conversationID: "active"))
        coordinator.cancel(conversationID: "other")
        XCTAssertFalse(coordinator.isObserving(conversationID: "other"))
        XCTAssertTrue(coordinator.isObserving(conversationID: "active"))
        XCTAssertTrue(coordinator.isCurrent(active))
        XCTAssertFalse(coordinator.isCurrent(other))
        release.open()
        await coordinator.cancelAllAndDrain()
        XCTAssertFalse(coordinator.isObserving(conversationID: "active"))
    }

    func testDetachCanPreserveOnlyTheUnfinishedRunObserver() async {
        let coordinator = MacProjectionObservationCoordinator()
        let activeRelease = Gate()
        let idleRelease = Gate()
        let active = coordinator.start(conversationID: "active") { _ in await activeRelease.wait() }
        let idle = coordinator.start(conversationID: "idle") { _ in await idleRelease.wait() }
        coordinator.cancelAll(except: "active")
        XCTAssertTrue(coordinator.isCurrent(active))
        XCTAssertFalse(coordinator.isCurrent(idle))
        activeRelease.open()
        idleRelease.open()
        await coordinator.cancelAllAndDrain()
        XCTAssertFalse(coordinator.isCurrent(active))
    }

    func testDrainingDetachedLeasesCannotCancelReattachedObserver() async {
        let coordinator = MacProjectionObservationCoordinator()
        let oldStarted = Gate()
        let oldRelease = Gate()
        let newStarted = Gate()
        let newRelease = Gate()
        let old = coordinator.start(conversationID: "conversation") { _ in
            oldStarted.open()
            await oldRelease.wait()
        }
        await oldStarted.wait()
        let drain = Task { @MainActor in await coordinator.cancelAllAndDrain() }
        while coordinator.isCurrent(old) { await Task.yield() }
        let current = coordinator.start(conversationID: "conversation") { _ in
            newStarted.open()
            await newRelease.wait()
        }
        oldRelease.open()
        await drain.value
        await newStarted.wait()
        XCTAssertTrue(coordinator.isCurrent(current))
        newRelease.open()
        await coordinator.cancelAllAndDrain()
    }
}

private actor GuidanceReceiptTransport {
    enum Mode { case lostResponse, absentThenRetry, unavailable, confirmed, foreign, selectiveFailure }
    var mode: Mode
    var requests: [(String, Data)] = []
    init(_ mode: Mode) { self.mode = mode }
    func send(_ path: String, _ data: Data) throws -> Data {
        requests.append((path, data))
        let input = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        let taskID = input["taskId"] as? String
        if mode == .unavailable || (mode == .selectiveFailure && taskID == "task-a") { throw URLError(.networkConnectionLost) }
        if requests.count == 1 && (mode == .lostResponse || mode == .absentThenRetry) {
            throw URLError(.networkConnectionLost)
        }
        let guidance: Any = mode == .absentThenRetry && path.hasSuffix("/receipt") ? NSNull() : [
            "task_id": input["taskId"]!, "message_id": input["messageId"]!,
            "owner_session_id": input["conversationId"]!, "content": input["message"]!,
            "submitted_at_ms": input["submittedAtMs"]!, "status": "submitted"
        ]
        return try JSONSerialization.data(withJSONObject: [
            "hostId": mode == .foreign ? "other-host" : input["hostId"]!,
            "conversationId": input["conversationId"]!, "taskId": input["taskId"]!,
            "messageId": input["messageId"]!, "guidance": guidance
        ])
    }
    func paths() -> [String] { requests.map(\.0) }
    func bodies() -> [Data] { requests.map(\.1) }
}

final class AppleTaskGuidanceOutboxTests: XCTestCase {
    func testCoreTransportChecksExactRunBeforeSubmittingFixedIntent() throws {
        let payload = try JSONSerialization.data(withJSONObject: [
            "hostId": "host", "conversationId": "conversation", "parentRunId": "run",
            "taskId": "task", "messageId": "fixed-message", "submittedAtMs": 42,
            "message": "Continue carefully"
        ])
        var commands: [[String: Any]] = []
        _ = try AppleTaskGuidanceCoreTransport.exchange(path: "chat/agent/v2/task/message",
            payload: payload, conversationId: "conversation", parentRunId: "run", parentTurnId: "turn") { data in
                commands.append(try JSONSerialization.jsonObject(with: data) as! [String: Any])
                return Data("null".utf8)
            }
        XCTAssertEqual(commands.map { $0["type"] as? String }, ["load_task_guidance_receipt", "submit_task_guidance"])
        XCTAssertEqual(commands[0]["parent_run_id"] as? String, "run")
        XCTAssertEqual(commands[1]["parent_turn_id"] as? String, "turn")
        XCTAssertEqual(commands[1]["message_id"] as? String, "fixed-message")
        XCTAssertEqual(commands[1]["submitted_at_ms"] as? Int, 42)
        var failedCommands = 0
        XCTAssertThrowsError(try AppleTaskGuidanceCoreTransport.exchange(path: "chat/agent/v2/task/message",
            payload: payload, conversationId: "conversation", parentRunId: "run", parentTurnId: "turn") { _ in
                failedCommands += 1
                throw URLError(.cannotParseResponse)
            })
        XCTAssertEqual(failedCommands, 1, "Unconfirmed source authority must never reach submission")
        commands.removeAll()
        _ = try AppleTaskGuidanceCoreTransport.exchange(path: "chat/agent/v2/task/message",
            payload: payload, conversationId: "conversation", parentRunId: "run", parentTurnId: "turn") { data in
                commands.append(try JSONSerialization.jsonObject(with: data) as! [String: Any])
                return Data("{}".utf8)
            }
        XCTAssertEqual(commands.count, 1, "A recorded intent must be returned without another submission")
    }

    func testLostResponseQueriesReceiptWithoutSecondInstruction() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let transport = GuidanceReceiptTransport(.lostResponse)
        let outbox = AppleTaskGuidanceOutbox(directory: directory)
        let receipt = try await outbox.submit(hostId: "host", conversationId: "conversation",
            parentRunId: "source-run", taskId: "task", message: "  Continue carefully  ", transport: { path, data in try await transport.send(path, data) })
        XCTAssertEqual(receipt.content, "Continue carefully")
        let paths = await transport.paths()
        XCTAssertEqual(paths, ["chat/agent/v2/task/message", "chat/agent/v2/task/message/receipt"])
        let bodies = await transport.bodies()
        XCTAssertEqual(bodies.first, bodies.last)
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: directory.path).isEmpty)
    }

    func testOnlyAuthoritativeNullPermitsSameIntentResubmission() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let transport = GuidanceReceiptTransport(.absentThenRetry)
        _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
            conversationId: "conversation", parentRunId: "run", taskId: "task", message: "指导", transport: { path, data in try await transport.send(path, data) })
        let bodies = await transport.bodies()
        XCTAssertEqual(bodies.count, 3)
        XCTAssertTrue(bodies.allSatisfy { $0 == bodies[0] })
    }

    func testColdRetryKeepsCompleteIntentAndRejectsForeignReceipt() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let unavailable = GuidanceReceiptTransport(.unavailable)
        do {
            _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
                conversationId: "conversation", parentRunId: "run", taskId: "task", message: "Keep scope", transport: { path, data in try await unavailable.send(path, data) })
            XCTFail("unknown outcome must remain pending")
        } catch {}
        let foreign = GuidanceReceiptTransport(.foreign)
        do {
            _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
                conversationId: "conversation", parentRunId: "run", taskId: "task", message: "Keep scope", transport: { path, data in try await foreign.send(path, data) })
            XCTFail("foreign owner must not clear intent")
        } catch {}
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path).count, 1)
        let confirmed = GuidanceReceiptTransport(.confirmed)
        try await AppleTaskGuidanceOutbox(directory: directory).recover(hostId: "host",
            conversationId: "conversation", parentRunId: "run", transport: { path, data in try await confirmed.send(path, data) })
        let original = await unavailable.bodies()
        let recovered = await confirmed.bodies()
        XCTAssertEqual(original.first, recovered.first)
        let paths = await confirmed.paths()
        XCTAssertEqual(paths, ["chat/agent/v2/task/message/receipt"])
    }
    func testOneUnresolvedOrMalformedIntentDoesNotStarveOtherTasks() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let unavailable = GuidanceReceiptTransport(.unavailable)
        for task in ["task-a", "task-b"] {
            do {
                _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
                    conversationId: "conversation", parentRunId: "run", taskId: task, message: task,
                    transport: { path, data in try await unavailable.send(path, data) })
                XCTFail("fixture should retain pending intent")
            } catch {}
        }
        try Data("invalid".utf8).write(to: directory.appendingPathComponent("000-corrupt.json"))
        let transport = GuidanceReceiptTransport(.selectiveFailure)
        do {
            try await AppleTaskGuidanceOutbox(directory: directory).recover(hostId: "host",
                conversationId: "conversation", parentRunId: "run",
                transport: { path, data in try await transport.send(path, data) })
            XCTFail("unresolved intents must still be reported")
        } catch AppleTaskGuidanceOutbox.Failure.recoveryIncomplete(let count) {
            XCTAssertEqual(count, 2)
        }
        let bodies = await transport.bodies()
        XCTAssertTrue(try bodies.contains { data in
            (try JSONSerialization.jsonObject(with: data) as? [String: Any])?["taskId"] as? String == "task-b"
        })
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path).count, 2)
    }

}
