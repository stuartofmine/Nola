// SPDX-License-Identifier: Apache-2.0

import XCTest
import AppKit
import SwiftUI
import Observation
import os
@testable import Nola

@MainActor
final class MacConversationLazyLoadingTests: XCTestCase {
    func testCanonicalMessagePublicationDoesNotInvalidateAnIdenticalPage() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("page.db"))
        let model = MacWorkspaceModel(
            canonicalRepository: MacCanonicalConversationRepository(store: store),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost()
        )
        defer { model.presentationDidDetach() }
        let id = try XCTUnwrap(model.selectedConversationID)
        let page = [MacChatMessage(role: .user, text: "Page content", createdAt: .now)]
        model.publishCanonicalMessages(page, conversationID: id)
        let changes = OSAllocatedUnfairLock(initialState: 0)
        withObservationTracking {
            _ = model.selectedMessages
        } onChange: {
            changes.withLock { $0 += 1 }
        }
        model.publishCanonicalMessages(page, conversationID: id)
        XCTAssertEqual(changes.withLock { $0 }, 0)
        var updated = page
        updated[0].text = "Changed content"
        model.publishCanonicalMessages(updated, conversationID: id)
        XCTAssertEqual(changes.withLock { $0 }, 1)
        XCTAssertEqual(model.selectedMessages.first?.text, "Changed content")
    }

    func testDeferredReaderRequiresExactFinalPartNotCompletedMessage() throws {
        for messageState in ["running", "completed"] {
            let json = """
            {"projection":{"contract_version":1,"conversation_id":"reader-policy","revision":9,"title":"fixture",
              "messages":[{"message_id":"assistant","role":"assistant","state":"\(messageState)","revision":9,"created_at_ms":1,
                "parts":[{"kind":"text","part_id":"old-final","revision":8,"markdown":"old preview","is_final":true},
                         {"kind":"text","part_id":"new-open","revision":9,"markdown":"live preview","is_final":false}]}],
              "active_run":null,"surface":{"revision":9,"committed_message_ids":["assistant"],"live_message_id":null,
                "current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":false},
              "context_usage":{"used_tokens":0,"context_window_tokens":1000,"prompt_token_limit":900,"estimated":true,"measured_at_revision":9}},
             "assistant_journal_heads":[],"stream_owners":null,"event_sequence":9,"total_message_count":1,"before_ordinal":null,"has_more":false,
             "deferred_text_parts":[{"message_id":"assistant","part_id":"old-final","is_final":true,"can_read_original":true,"total_utf8_bytes":100,"preview_utf8_bytes":11},
                                    {"message_id":"assistant","part_id":"new-open","is_final":false,"can_read_original":false,"total_utf8_bytes":200,"preview_utf8_bytes":12},
                                    {"message_id":"assistant","part_id":"missing","is_final":true,"can_read_original":true,"total_utf8_bytes":300,"preview_utf8_bytes":0}]}
            """
            let window = try JSONDecoder().decode(MacCanonicalConversationWindow.self, from: Data(json.utf8))
            let message = try XCTUnwrap(MacCanonicalConversationRepository.projectMessages(window).first)
            XCTAssertEqual(message.deferredContent?.parts.map(\.partID), ["old-final"])
            let openOnly = json.replacingOccurrences(of: "\"is_final\":true", with: "\"is_final\":false")
                .replacingOccurrences(of: "\"can_read_original\":true", with: "\"can_read_original\":false")
            let openWindow = try JSONDecoder().decode(MacCanonicalConversationWindow.self, from: Data(openOnly.utf8))
            let openMessage = try XCTUnwrap(MacCanonicalConversationRepository.projectMessages(openWindow).first)
            XCTAssertNil(openMessage.deferredContent)
            XCTAssertTrue(openMessage.text.contains("live preview"), "Hiding the reader must not drop live preview content")
            let unreadable = json.replacingOccurrences(of: "\"can_read_original\":true", with: "\"can_read_original\":false")
            let unreadableWindow = try JSONDecoder().decode(MacCanonicalConversationWindow.self, from: Data(unreadable.utf8))
            let unreadableMessage = try XCTUnwrap(MacCanonicalConversationRepository.projectMessages(unreadableWindow).first)
            XCTAssertNil(unreadableMessage.deferredContent, "Finality must not override the Store's read eligibility")
            XCTAssertTrue(unreadableMessage.text.contains("old preview"))
            let historical = json.replacingOccurrences(
                of: #"{"kind":"text","part_id":"old-final","revision":8,"markdown":"old preview","is_final":true}"#,
                with: #"{"kind":"historical","part_id":"old-final","revision":8,"projection":{"owner":{},"canonical_revision":8,"canonical_kind":"text","status":{"state":"supported"},"fields":{},"canonical_text":{"text":"old preview","total_utf8_bytes":100}}}"#)
            let historicalWindow = try JSONDecoder().decode(MacCanonicalConversationWindow.self, from: Data(historical.utf8))
            let historicalMessage = try XCTUnwrap(MacCanonicalConversationRepository.projectMessages(historicalWindow).first)
            XCTAssertEqual(historicalMessage.deferredContent?.parts.map(\.partID), ["old-final"])
            let historicalOpen = historical.replacingOccurrences(of: "\"is_final\":true", with: "\"is_final\":false")
                .replacingOccurrences(of: "\"can_read_original\":true", with: "\"can_read_original\":false")
            let historicalOpenWindow = try JSONDecoder().decode(MacCanonicalConversationWindow.self, from: Data(historicalOpen.utf8))
            XCTAssertNil(try MacCanonicalConversationRepository.projectMessages(historicalOpenWindow).first?.deferredContent)
            let missingLifecycle = json.replacingOccurrences(of: "\"is_final\":true,\"can_read_original\"", with: "\"can_read_original\"")
            XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalConversationWindow.self, from: Data(missingLifecycle.utf8)))
            let missingEligibility = json.replacingOccurrences(of: "\"can_read_original\":true,", with: "")
            XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalConversationWindow.self, from: Data(missingEligibility.utf8)))
        }
    }

    func testDeferredTextMetadataRejectsInvalidIdentityLengthsAndEligibility() throws {
        let valid: [String: Any] = [
            "message_id": "message", "part_id": "part", "is_final": true,
            "can_read_original": true, "total_utf8_bytes": 100, "preview_utf8_bytes": 11
        ]
        let decoder = JSONDecoder()
        func decode(_ value: [String: Any]) throws -> MacCanonicalConversationWindow.DeferredTextPart {
            try decoder.decode(MacCanonicalConversationWindow.DeferredTextPart.self,
                from: JSONSerialization.data(withJSONObject: value))
        }
        let original = try decode(valid)
        XCTAssertTrue(original.isFinal)
        XCTAssertTrue(original.canReadOriginal)
        XCTAssertEqual(try decoder.decode(MacCanonicalConversationWindow.DeferredTextPart.self,
            from: JSONEncoder().encode(original)), original)
        for (key, value) in [
            ("message_id", " \n" as Any), ("part_id", "" as Any),
            ("total_utf8_bytes", 0 as Any), ("total_utf8_bytes", 11 as Any),
            ("preview_utf8_bytes", 101 as Any), ("preview_utf8_bytes", -1 as Any),
            ("is_final", false as Any), ("can_read_original", NSNull() as Any),
            ("can_read_original", 1 as Any), ("can_read_original", "true" as Any),
            ("path", "/tmp/body" as Any), ("read_grant", true as Any)
        ] {
            var invalid = valid
            invalid[key] = value
            XCTAssertThrowsError(try decode(invalid), key)
        }
        for key in ["message_id", "part_id", "total_utf8_bytes", "preview_utf8_bytes", "is_final", "can_read_original"] {
            var invalid = valid
            invalid.removeValue(forKey: key)
            XCTAssertThrowsError(try decode(invalid), key)
        }
        var finalButUnreadable = valid
        finalButUnreadable["can_read_original"] = false
        let unreadable = try decode(finalButUnreadable)
        XCTAssertTrue(unreadable.isFinal)
        XCTAssertFalse(unreadable.canReadOriginal)
        var open = finalButUnreadable
        open["is_final"] = false
        XCTAssertFalse(try decode(open).isFinal)
    }

    func testCanonicalBackupRejectsUnavailableConversationInsteadOfSkippingIt() async throws {
        var appended = 0
        do {
            try await MacCanonicalBackupStream.appendConversations(["unavailable"],
                load: { _ in nil }, append: { _ in appended += 1 })
            XCTFail("A missing canonical conversation must fail the export")
        } catch MacDataManagementError.conversationUnavailable {
            // The outer exporter discards its temporary file, preserving an existing backup.
        }
        XCTAssertEqual(appended, 0)
    }

    func testCancelledCanonicalBackupDoesNotLoadOrAppendConversations() async throws {
        var loads = 0
        var appends = 0
        let task = Task { @MainActor in
            try await MacCanonicalBackupStream.appendConversations(["retained"],
                load: { _ in loads += 1; return nil }, append: { _ in appends += 1 })
        }
        task.cancel()
        do {
            try await task.value
            XCTFail("Cancellation must abort the backup")
        } catch is CancellationError {
            // Cancellation must not be misreported as a missing conversation.
        }
        XCTAssertEqual(loads, 0)
        XCTAssertEqual(appends, 0)
    }







    func testDeferredCanonicalTextPreservesIdentityAndReadsLosslessRanges() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-text-range-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        let original = String(repeating: "ab中😀", count: 100)
        try repository.restoreBackupSnapshot([storedConversation(id: "ranges", text: original)])
        let window = try store.loadConversationWindow("ranges", contentByteLimit: 32)
        let reference = try XCTUnwrap(window.deferredTextParts.first)
        XCTAssertEqual(reference.totalUTF8Bytes, UInt64(original.utf8.count))
        XCTAssertLessThanOrEqual(reference.previewUTF8Bytes, 32)
        let messages = try MacCanonicalConversationRepository.projectMessages(window)
        XCTAssertEqual(messages.first?.deferredContent?.parts, [reference])
        var offset: UInt64 = 0
        var restored = ""
        repeat {
            let range = try store.readConversationTextRange("ranges", messageID: reference.messageID,
                partID: reference.partID, offsetBytes: offset, limitBytes: 31)
            XCTAssertLessThanOrEqual(range.text.utf8.count, 31)
            restored += range.text
            offset = range.nextOffsetBytes
            if range.endOfContent { break }
        } while offset < reference.totalUTF8Bytes
        XCTAssertEqual(restored, original)
        XCTAssertThrowsError(try store.readConversationTextRange("foreign", messageID: reference.messageID,
            partID: reference.partID, offsetBytes: 0))
    }

    func testCancelledWindowWaitStopsPromptlyAndLeavesCommandPortUsable() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-window-cancel-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        try repository.restoreBackupSnapshot([storedConversation(id: "cancel-wait", text: "Retained")])
        let model = MacConversationWindowModel(store: store)
        try await model.select(conversationID: "cancel-wait")
        let originalSequence = try XCTUnwrap(model.window).eventSequence
        let waiting = Task { @MainActor in try await model.refreshIfChanged(waitMilliseconds: 5_000) }
        try await Task.sleep(for: .milliseconds(100))
        let start = ContinuousClock.now
        waiting.cancel()
        do {
            _ = try await waiting.value
            XCTFail("Cancelled wait must not report an ordinary unchanged window.")
        } catch is CancellationError {
            // Expected: no receipt lookup delay and no five-second socket wait.
        }
        XCTAssertLessThan(start.duration(to: .now), .seconds(1))
        XCTAssertEqual(model.window?.eventSequence, originalSequence)
        XCTAssertTrue(try store.renameConversation("cancel-wait", title: "Still usable", updatedAtMs: 9_000))
        let refreshed = try await model.refreshIfChanged()
        XCTAssertTrue(refreshed)
        XCTAssertEqual(model.window?.projection.title, "Still usable")
    }

    func testWindowWaitDoesNotBlockRenameAndRefreshesCommittedWindow() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-window-wait-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        try repository.restoreBackupSnapshot([storedConversation(id: "waiting", text: "Retained")])
        let model = MacConversationWindowModel(store: store)
        try await model.select(conversationID: "waiting")
        let originalSequence = try XCTUnwrap(model.window).eventSequence
        let waiting = Task { @MainActor in try await model.refreshIfChanged(waitMilliseconds: 1_500) }
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertTrue(try store.renameConversation("waiting", title: "Wake window", updatedAtMs: 9_000))
        let changed = try await waiting.value
        XCTAssertTrue(changed)
        XCTAssertEqual(model.window?.projection.title, "Wake window")
        XCTAssertEqual(model.messages.first?.text, "Retained")
        XCTAssertGreaterThan(try XCTUnwrap(model.window).eventSequence, originalSequence)
        let unchanged = try await model.refreshIfChanged(waitMilliseconds: 50)
        XCTAssertFalse(unchanged)
    }

    func testWindowModelPagesWithoutAccumulatingHistoryAndRefreshesFromApplication() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-window-model-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        let seed = storedConversation(id: "paged", text: "Seed")
        try repository.restoreBackupSnapshot([MacStoredConversation(
            conversation: seed.conversation,
            messages: (0..<130).map { index in
                MacChatMessage(role: .user, text: "Message \(index)",
                               createdAt: Date(timeIntervalSince1970: Double(index + 1)))
            }
        )])
        let model = MacConversationWindowModel(store: store)
        let firstLoad = Task { @MainActor in
            try await model.select(conversationID: "paged")
            XCTAssertEqual(model.messages.count, 64, "Every successful waiter must observe a loaded window.")
        }
        let secondLoad = Task { @MainActor in
            try await model.select(conversationID: "paged")
            XCTAssertEqual(model.messages.count, 64, "A coalesced load must not return an empty placeholder.")
        }
        try await firstLoad.value
        try await secondLoad.value
        XCTAssertEqual(model.messages.count, 64)
        XCTAssertEqual(model.messages.first?.text, "Message 66")
        XCTAssertFalse(model.isViewingHistory)
        XCTAssertTrue(model.hasEarlierMessages)
        let navigationChanges = OSAllocatedUnfairLock(initialState: 0)
        withObservationTracking {
            _ = model.hasEarlierMessages
            _ = model.oldestMessageOrdinal
            _ = model.isViewingHistory
        } onChange: {
            navigationChanges.withLock { $0 += 1 }
        }
        XCTAssertTrue(try store.renameConversation("paged", title: "Before paging", updatedAtMs: 8_000))
        let metadataChanged = try await model.refreshIfChanged()
        XCTAssertTrue(metadataChanged)
        XCTAssertEqual(model.window?.projection.title, "Before paging")
        XCTAssertEqual(navigationChanges.withLock { $0 }, 0,
            "Canonical metadata updates must not invalidate unchanged page navigation")
        try await model.loadEarlier()
        XCTAssertEqual(navigationChanges.withLock { $0 }, 1,
            "Actual page movement must still invalidate navigation")
        XCTAssertEqual(model.messages.count, 64)
        XCTAssertEqual(model.messages.first?.text, "Message 2")
        XCTAssertTrue(model.isViewingHistory)
        try await model.loadEarlier()
        XCTAssertEqual(model.messages.map(\.text), ["Message 0", "Message 1"])
        XCTAssertFalse(model.hasEarlierMessages)
        try await model.returnToLatest()
        XCTAssertEqual(model.messages.count, 64)
        XCTAssertFalse(model.isViewingHistory)
        XCTAssertTrue(try store.renameConversation("paged", title: "Renamed", updatedAtMs: 9_000))
        let changed = try await model.refreshIfChanged()
        XCTAssertTrue(changed)
        XCTAssertEqual(model.window?.projection.title, "Renamed")
        XCTAssertEqual(model.messages.count, 64)
        let unchanged = try await model.refreshIfChanged()
        XCTAssertFalse(unchanged)
        XCTAssertFalse(model.isLoading)
    }

    func testIndependentWindowsDoNotAdvanceEachOthersSubscriptionCursor() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-independent-subscriptions-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        try repository.restoreBackupSnapshot([storedConversation(id: "shared", text: "Transcript")])
        let first = MacConversationWindowModel(store: store)
        let second = MacConversationWindowModel(store: store)
        try await first.select(conversationID: "shared")
        try await second.select(conversationID: "shared")
        let originalSequence = try XCTUnwrap(second.window).eventSequence
        XCTAssertTrue(try store.renameConversation("shared", title: "Updated", updatedAtMs: 5_000))
        let window = try store.loadConversationWindow("shared")
        XCTAssertEqual(window.projection.title, "Updated")
        XCTAssertGreaterThan(window.eventSequence, 0)
        let update = try await first.refreshIfChanged()
        XCTAssertTrue(update)
        XCTAssertEqual(first.window?.projection.title, "Updated")
        XCTAssertEqual(second.window?.eventSequence, originalSequence)
        let secondUpdate = try await second.refreshIfChanged()
        XCTAssertTrue(secondUpdate)
        XCTAssertEqual(second.window?.projection.title, "Updated")
        let firstReplay = try await first.refreshIfChanged()
        let secondReplay = try await second.refreshIfChanged()
        XCTAssertFalse(firstReplay)
        XCTAssertFalse(secondReplay)
    }

    func testWindowCoalescesBacklogWithoutReplayingObsoleteIntermediateTitles() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-paged-subscription-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        try repository.restoreBackupSnapshot([
            storedConversation(id: "paged", text: "Original transcript")
        ])
        let model = MacConversationWindowModel(store: store)
        try await model.select(conversationID: "paged")
        let originalSequence = try XCTUnwrap(model.window).eventSequence

        for index in 1...300 {
            XCTAssertTrue(try store.renameConversation(
                "paged",
                title: "Revision \(index)",
                updatedAtMs: Int64(3_000 + index)
            ))
        }
        let updated = try await model.refreshIfChanged()
        XCTAssertTrue(updated)
        let latest = try XCTUnwrap(model.window)
        XCTAssertGreaterThan(latest.eventSequence, originalSequence)
        XCTAssertEqual(latest.projection.title, "Revision 300")
        XCTAssertEqual(model.messages.map(\.text), ["Original transcript"])
        let replay = try await model.refreshIfChanged()
        XCTAssertFalse(replay)
    }

    func testSidebarRestoreKeepsOnlySelectedTranscriptResident() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-lazy-conversations-\(UUID().uuidString)")
        try FileManager.default.createDirectory(
            at: root,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: root) }

        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(
                databaseURL: root.appendingPathComponent("nola.db")
            )
        )
        let first = storedConversation(id: "first", text: "first transcript")
        let selected = storedConversation(id: "selected", text: "selected transcript")
        try repository.restoreBackupSnapshot([first, selected])

        let sidebar = try repository.loadSidebar(eagerConversationID: selected.conversation.id)
        let byID = Dictionary(uniqueKeysWithValues: sidebar.map { ($0.conversation.id, $0) })

        XCTAssertEqual(byID[selected.conversation.id]?.messages.map(\.text), ["selected transcript"])
        XCTAssertEqual(byID[first.conversation.id]?.messages, [])
        XCTAssertEqual(
            try repository.load(conversationID: first.conversation.id)?.messages.map(\.text),
            ["first transcript"]
        )
    }

    private func storedConversation(id: String, text: String) -> MacStoredConversation {
        let timestamp = Date(timeIntervalSince1970: id == "first" ? 1_000 : 2_000)
        return MacStoredConversation(
            conversation: MacConversation(
                id: id,
                title: id,
                preview: text,
                updatedAt: timestamp,
                isPinned: false,
                isPersisted: true
            ),
            messages: [
                MacChatMessage(role: .user, text: text, createdAt: timestamp)
            ]
        )
    }
}
