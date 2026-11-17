// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS chat tool/thinking/error/Mermaid presentation, composer layout/paste/commands, navigation gestures, knowledge, message edit drafts, draft stores/aggregates, and chat preferences suite.

import BeautifulMermaid
import Foundation
import SwiftUI
import Testing
import UIKit
import XCTest
@testable import Nola

@MainActor final class ChatComposerLayoutTests: XCTestCase {
    func testPhoneComposerKeepsWritingAreaAboveToolbarInBothAppearances() async throws {
        for width in [CGFloat(320), CGFloat(390)] {
            for dark in [false, true] {
                let root = PhoneComposerFixture().frame(width: width)
                    .environment(\.colorScheme, dark ? .dark : .light).ignoresSafeArea()
                let controller = UIHostingController(rootView: root)
                let size = controller.sizeThatFits(in: CGSize(width: width, height: 500))
                XCTAssertLessThanOrEqual(size.height, 116, "An empty composer should leave space for the conversation")
                let window = UIWindow(frame: CGRect(origin: .zero, size: size))
                window.rootViewController = controller
                window.makeKeyAndVisible()
                controller.view.frame = window.bounds
                try await Task.sleep(for: .milliseconds(100))
                controller.view.layoutIfNeeded()
                let editor = try XCTUnwrap(descendants(controller.view).first { $0 is ChatPasteAwareTextView })
                let bounds = editor.convert(editor.bounds, to: controller.view)
                XCTAssertGreaterThan(bounds.width, width - 64, "The toolbar must not squeeze the writing area")
                XCTAssertGreaterThanOrEqual(size.height - bounds.maxY, 44, "The toolbar belongs below the writing area")
                let image = UIGraphicsImageRenderer(size: size).image { _ in
                    controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                }
                let attachment = XCTAttachment(image: image)
                attachment.name = "composer-\(Int(width))-\(dark ? "dark" : "light")"
                attachment.lifetime = .keepAlways
                add(attachment)
                window.isHidden = true
            }
        }
    }

    func testMobilePaletteSupportsReadingAndActionsInBothAppearances() {
        for style in [UIUserInterfaceStyle.light, .dark] {
            let traits = UITraitCollection(userInterfaceStyle: style)
            for (ink, surface) in [
                (DesignTokens.contentColor, DesignTokens.chatBackgroundColor),
                (DesignTokens.secondaryContentColor, DesignTokens.chatBackgroundColor),
                (DesignTokens.secondaryContentColor, DesignTokens.rowFillColor),
                (DesignTokens.contentColor, DesignTokens.accentSubtleColor),
                (DesignTokens.onActionColor, DesignTokens.actionColor)
            ] {
                func luminance(_ color: Color) -> Double {
                    var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
                    UIColor(color).resolvedColor(with: traits).getRed(&red, green: &green, blue: &blue, alpha: &alpha)
                    func linear(_ value: CGFloat) -> Double {
                        let v = Double(value)
                        return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
                    }
                    return linear(red) * 0.2126 + linear(green) * 0.7152 + linear(blue) * 0.0722
                }
                let a = luminance(ink), b = luminance(surface)
                XCTAssertGreaterThanOrEqual((max(a, b) + 0.05) / (min(a, b) + 0.05), 4.5)
            }
        }
    }

    func testNativeMobileConversationAppearanceAtNarrowWidthsAndLargeType() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let container = ServiceContainer(applicationRuntime: try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("fixture.sqlite")))
        for width in [CGFloat(320), CGFloat(390)] {
            for dark in [false, true] {
                for large in [false, true] {
                    let root = NativeMobileConversationFixture()
                        .environmentObject(container)
                        .environment(\.colorScheme, dark ? .dark : .light)
                        .environment(\.dynamicTypeSize, large ? .accessibility5 : .large)
                    let controller = UIHostingController(rootView: root)
                    controller.overrideUserInterfaceStyle = dark ? .dark : .light
                    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: width, height: 844))
                    window.rootViewController = controller
                    window.makeKeyAndVisible()
                    controller.view.frame = window.bounds
                    try await Task.sleep(for: .milliseconds(200))
                    controller.view.layoutIfNeeded()
                    let editor = try XCTUnwrap(descendants(controller.view).first { $0 is ChatPasteAwareTextView })
                    let bounds = editor.convert(editor.bounds, to: controller.view)
                    XCTAssertGreaterThan(bounds.width, width - 64)
                    XCTAssertLessThanOrEqual(bounds.maxX, width)
                    let image = UIGraphicsImageRenderer(size: window.bounds.size).image { _ in
                        controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                    }
                    let attachment = XCTAttachment(image: image)
                    attachment.name = "mobile-chat-\(Int(width))-\(dark ? "dark" : "light")-\(large ? "large-type" : "standard")"
                    attachment.lifetime = .keepAlways
                    add(attachment)
                    window.isHidden = true
                }
            }
        }
    }

    private func descendants(_ view: UIView) -> [UIView] {
        view.subviews.flatMap { [$0] + descendants($0) }
    }
}

private struct PhoneComposerFixture: View {
    @State private var text = ""
    @FocusState private var focused: Bool
    var body: some View {
        ChatComposer(inputText: $text, attachments: [], attachmentMessage: nil,
            isImportingAttachments: false, isEditable: true, isGenerating: false, focus: $focused,
            onBeginAttachFiles: { { _ in } }, onRemoveAttachment: { _ in }, onSend: {}, onStop: {},
            showsModelControl: true, modelControl: HStack(spacing: 2) {
                Menu {} label: {
                Label("A long model name", systemImage: "chevron.down")
                    .font(.caption).lineLimit(1).frame(minHeight: 44)
                }
                ReasoningEffortMenu(efforts: ["none", "low", "medium", "high", "xhigh"], preference: "medium", select: { _ in })
            })
            .background(Color(uiColor: .systemBackground))
    }
}

private struct NativeMobileConversationFixture: View {
    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    message(.user, "周末想找个地方走走，不想把行程排得太满。")
                    message(.assistant, "那就把目标定小一点：一段山路，一顿好吃的，再留一点空白。\n\n上午慢慢出发，在山脚吃午饭，下午选一条能随时折返的路线。天气好的话，坐到太阳往下落再回来。")
                }.padding(16)
            }
            PhoneComposerFixture()
        }.background(DesignTokens.chatBackgroundColor)
    }
    private func message(_ role: ChatMessage.Role, _ text: String) -> some View {
        ChatBubble(conversationID: nil, message: ChatMessage(role: role, content: text),
            isLatestAssistant: role == .assistant, isStreaming: false, isRetrieving: false,
            pendingHostPermissionRequest: nil, onHostPermissionDecision: { _, _ in })
    }
}

final class ChatComposerPasteTests: XCTestCase {
    func testCopiedFinderURLBecomesAttachmentWithoutPathText() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        let source = URL(fileURLWithPath: "/tmp/copied-image.png")
        pasteboard.urls = [source]

        let urls = ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard) { _ in nil }

        XCTAssertEqual(urls, [source])
    }

    func testCopiedBitmapBecomesPNGAttachment() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        pasteboard.image = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).image { context in
            UIColor.systemBlue.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        }
        let expected = URL(fileURLWithPath: "/tmp/pasted.png")
        var received = Data()

        let urls = ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard) { data in
            received = data
            return expected
        }

        XCTAssertEqual(urls, [expected])
        XCTAssertFalse(received.isEmpty)
        XCTAssertEqual(received.prefix(8), Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]))
    }
    func testCopiedWebLinkRemainsTextInsteadOfBecomingAnAttachment() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        let link = try XCTUnwrap(URL(string: "https://example.invalid/article"))
        pasteboard.urls = [link]
        XCTAssertNil(ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard) { _ in
            XCTFail("A web link must not create an image file"); return nil
        })
        XCTAssertEqual(pasteboard.url, link)
    }

    func testImageWriteFailureReportsErrorAndPreventsSilentFallback() throws {
        let pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true))
        defer { UIPasteboard.remove(withName: pasteboard.name) }
        pasteboard.image = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).image { context in
            UIColor.systemBlue.setFill(); context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        }
        var failure: Error?
        let urls = ChatPasteboardAttachmentResolver.attachmentURLs(from: pasteboard,
            onFailure: { failure = $0 }, writeImage: { _ in nil })
        XCTAssertEqual(urls, [])
        XCTAssertEqual((failure as NSError?)?.code, CocoaError.fileWriteUnknown.rawValue)
    }

    func testContextTextImportPreservesWhitespaceAndRejectsOversizedOrInvalidText() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("skill.md")
        let exact = "  # Skill\n\n    indented workflow\n\n"
        try Data(exact.utf8).write(to: file)
        XCTAssertEqual(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 40_000), exact)
        try Data(repeating: 0x61, count: 40_001).write(to: file)
        XCTAssertThrowsError(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 40_000))
        try Data(repeating: 0x61, count: 192_001).write(to: file)
        XCTAssertThrowsError(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 200_000))
        try Data([0xFF, 0xFE, 0xFF]).write(to: file)
        XCTAssertThrowsError(try IOSContextFileImportPolicy.readText(from: file, maximumCharacters: 40_000))
        XCTAssertTrue(IOSContextFileImportPolicy.isCancellation(CocoaError(.userCancelled)))
        XCTAssertFalse(IOSContextFileImportPolicy.isCancellation(CocoaError(.fileReadNoPermission)))
    }

}

@MainActor
final class ChatErrorPresentationTests: XCTestCase {
    func testFailedReloadKeepsOnlyConfirmedSameConversationPresentationAndReportsError() {
        let owner = UUID(), other = UUID()
        let confirmed = [ChatMessage(role: .assistant, content: "Already displayed")]
        var recovery = ChatConversationLoadRecovery()
        recovery.didLoad(owner)
        let retained = recovery.didFail(owner, message: "temporary read failure", existing: confirmed, submitted: [])
        XCTAssertEqual(retained.map(\.id), confirmed.map(\.id))
        XCTAssertEqual(recovery.failure?.message, "temporary read failure")
        XCTAssertEqual(recovery.failure?.conversationID, owner)
        let otherDraft = ChatMessage(role: .user, content: "Other conversation draft")
        let switched = recovery.didFail(other, message: "unavailable", existing: retained, submitted: [otherDraft])
        XCTAssertEqual(switched.map(\.id), [otherDraft.id])
        XCTAssertNil(recovery.confirmedConversationID)
        XCTAssertTrue(recovery.didFail(owner, message: "still unavailable", existing: switched, submitted: []).isEmpty)
    }

    func testSuccessfulRetryClearsFailureAndRetirementCannotResurrectConfirmedHistory() {
        let owner = UUID()
        let confirmed = [ChatMessage(role: .assistant, content: "History")]
        var recovery = ChatConversationLoadRecovery()
        recovery.didLoad(owner)
        _ = recovery.didFail(owner, message: "retry", existing: confirmed, submitted: [])
        recovery.didLoad(owner)
        XCTAssertNil(recovery.failure)
        recovery.retire()
        XCTAssertNil(recovery.confirmedConversationID)
        XCTAssertTrue(recovery.didFail(owner, message: "missing", existing: confirmed, submitted: []).isEmpty)
    }

    func testDelayedNewConversationSelectionPreservesSubmittedTurnAndResponseOwner() {
        let user = ChatMessage(role: .user, content: "Hello")
        let assistant = ChatMessage(role: .assistant, content: "")
        let submitted = [user, assistant]
        let loaded = ChatSubmissionPresentationPolicy.merging(persisted: [], submitted: submitted)
        XCTAssertEqual(loaded.map(\.id), [user.id, assistant.id])
        XCTAssertEqual(ChatSubmissionPresentationPolicy.activeAssistant(
            submitted: assistant.id, persisted: []), assistant.id)
    }

    func testSubmissionRetiresOnlyAfterAllMessageIDsArePersisted() {
        let user = ChatMessage(role: .user, content: "Hello")
        let assistant = ChatMessage(role: .assistant, content: "draft")
        let submitted = [user, assistant]
        XCTAssertEqual(ChatSubmissionPresentationPolicy.pendingSubmission(
            submitted, persisted: [user])?.map(\.id), submitted.map(\.id))
        XCTAssertNil(ChatSubmissionPresentationPolicy.pendingSubmission(submitted, persisted: submitted))
        XCTAssertNil(ChatSubmissionPresentationPolicy.pendingSubmission(nil, persisted: []))
    }

    func testExplicitConversationDeletionRetiresEvenUnpersistedSubmission() {
        let submitted = [ChatMessage(role: .user, content: "Hello"),
                         ChatMessage(role: .assistant, content: "draft")]
        XCTAssertNil(ChatSubmissionPresentationPolicy.pendingSubmission(
            submitted, persisted: [], conversationExists: false))
        XCTAssertEqual(ChatSubmissionPresentationPolicy.pendingSubmission(
            submitted, persisted: [])?.count, 2)
    }

    func testPartialPersistenceDoesNotDuplicateSubmissionAndCanonicalContentWins() {
        let user = ChatMessage(role: .user, content: "Hello")
        let draft = ChatMessage(role: .assistant, content: "stream")
        let partial = ChatSubmissionPresentationPolicy.merging(persisted: [user], submitted: [user, draft])
        XCTAssertEqual(partial.map(\.id), [user.id, draft.id])
        let committed = ChatMessage(id: draft.id, role: .assistant, content: "committed")
        let loaded = ChatSubmissionPresentationPolicy.merging(
            persisted: [user, committed], submitted: [user, draft])
        XCTAssertEqual(loaded.count, 2)
        XCTAssertEqual(loaded.last?.content, "committed")
    }

    func testConversationOwnedDraftSurvivesSwitchWithoutLeakingToOtherConversation() {
        let owner = UUID(), other = UUID()
        let user = ChatMessage(role: .user, content: "Hello")
        let assistant = ChatMessage(role: .assistant, content: "")
        var turns = [owner: [user, assistant]]
        XCTAssertTrue(ChatSubmissionPresentationPolicy.merging(
            persisted: [], submitted: turns[other] ?? []).isEmpty)
        ChatMessageDraftReducer.upsertAssistant(
            in: &turns[owner]!, assistantID: assistant.id, afterUserID: user.id,
            content: "completed while hidden")
        let restored = ChatSubmissionPresentationPolicy.merging(persisted: [], submitted: turns[owner] ?? [])
        XCTAssertEqual(restored.map(\.id), [user.id, assistant.id])
        XCTAssertEqual(restored.last?.content, "completed while hidden")
    }

    func testDismissAcknowledgesOccurrenceWithoutChangingRuntimeState() throws {
        let owner = UUID()
        let runtime = ChatProcessingState.error("connection lost")
        var presentation = ChatErrorPresentation()
        presentation.observe(runtime, conversationID: owner)
        let occurrence = try XCTUnwrap(presentation.occurrenceID)
        XCTAssertTrue(presentation.dismiss(occurrenceID: occurrence, conversationID: owner))
        XCTAssertNil(presentation.message)
        XCTAssertEqual(runtime, .error("connection lost"))
        presentation.observe(runtime, conversationID: owner)
        XCTAssertEqual(presentation.message, "connection lost")
        XCTAssertNotEqual(presentation.occurrenceID, occurrence)
        XCTAssertFalse(presentation.dismiss(occurrenceID: occurrence, conversationID: owner))
    }

    func testOldConversationAndOldErrorCannotDismissCurrentPresentation() throws {
        let first = UUID(), second = UUID()
        var presentation = ChatErrorPresentation()
        presentation.observe(.error("same text"), conversationID: first)
        let previous = try XCTUnwrap(presentation.occurrenceID)
        XCTAssertFalse(presentation.dismiss(occurrenceID: previous, conversationID: second))
        presentation.observe(.error("same text"), conversationID: second)
        XCTAssertFalse(presentation.dismiss(occurrenceID: previous, conversationID: second))
        XCTAssertEqual(presentation.message, "same text")
        presentation.observe(.processingWithCloud, conversationID: second)
        XCTAssertNil(presentation.message)
        XCTAssertNil(presentation.occurrenceID)
    }
}

final class ChatMemoryProfileTests: XCTestCase {
    func testCanonicalSensitivitySurvivesProfileRoundTripAndDefaultPolicyRejectsIt() throws {
        let canonical = IOSCanonicalMemoryRecord(
            record: IOSCanonicalSemanticMemoryRecord(
                id: "sensitive-memory",
                owner: .localProfile,
                namespace: IOSCanonicalMemoryNamespace(kind: "custom", name: "long_term"),
                scope: "global",
                kind: "reference",
                content: "Private health detail",
                summary: nil,
                tags: [],
                confidence: 0.95,
                importance: 0.8,
                sensitivity: "health.private",
                retention: IOSCanonicalMemoryRetention(mode: "permanent", expiresAt: nil),
                supersedes: [],
                provenance: IOSCanonicalMemoryProvenance(
                    kind: "explicit_user",
                    source: "nola://memory/sensitive-memory",
                    sessionID: nil,
                    turnID: nil,
                    itemIDs: [],
                    observedAt: "2026-09-10T00:00:00Z"
                )
            ),
            createdAtMs: 1,
            updatedAtMs: 1,
            lastUsedAtMs: nil,
            deletedAtMs: nil
        )

        var unbound = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(canonical.record)) as? [String: Any])
        unbound.removeValue(forKey: "owner")
        XCTAssertThrowsError(try JSONDecoder().decode(IOSCanonicalSemanticMemoryRecord.self,
            from: JSONSerialization.data(withJSONObject: unbound)))
        let profileRecord = try XCTUnwrap(canonical.chatMemoryRecord)
        XCTAssertEqual(profileRecord.canonicalRecord?.record.owner, .localProfile)
        XCTAssertEqual(profileRecord.sensitivity, "health.private")
        XCTAssertEqual(profileRecord.canonicalRecord?.record.sensitivity, "health.private")

        let selectionRecord = CoreMemoryRecord(
            id: profileRecord.id,
            owner: .localProfile,
            namespace: .personal,
            scope: .global,
            kind: .reference,
            content: profileRecord.text,
            confidence: profileRecord.confidence ?? 0.6,
            importance: 0.8,
            sensitivity: profileRecord.sensitivity,
            retention: .permanent,
            provenance: CoreMemoryProvenance(
                kind: .explicitUser,
                source: "nola://memory/sensitive-memory"
            )
        )
        let policy = CoreMemoryPolicy(
            longTermEnabled: true,
            automaticExtractionEnabled: true,
            searchLimit: 8,
            minimumConfidence: 0.5
        )
        let encodedPolicy = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(policy)
        ) as? [String: Any]
        XCTAssertEqual(encodedPolicy?["allowed_sensitivities"] as? [String], [])
        let unlabeledPolicy = try JSONDecoder().decode(
            CoreMemoryPolicy.self,
            from: Data(
                """
                {"long_term_enabled":true,"automatic_extraction_enabled":true,"search_limit":8,"minimum_confidence":0.5,"maximum_age_days":null}
                """.utf8
            )
        )
        XCTAssertEqual(unlabeledPolicy.allowedSensitivities, [])

        let plan = try XCTUnwrap(PersonalIntelligenceCore.selectMemories(
            CoreMemorySelectionInput(
                policy: policy,
                records: [selectionRecord],
                allowedNamespaces: [.personal],
                now: "2026-09-10T00:00:01Z",
                maximumInputTokens: 1_024,
                tokenAccounting: .coreEstimate
            )
        ))
        XCTAssertTrue(plan.selected.isEmpty)
        XCTAssertEqual(plan.rejected.first?.memoryID, "sensitive-memory")
        XCTAssertEqual(plan.rejected.first?.reason, .sensitivityNotAllowed)
    }

    func testStructuredWriteIsIdempotentAndPreservesRealProvenance() throws {
        let conversationID = UUID()
        let observedAt = Date(timeIntervalSince1970: 1_700_000_000)
        let provenance = ChatMemoryProvenance(
            source: "assistant_memory_tool",
            sourceID: "event-1",
            conversationID: conversationID,
            model: "test-model",
            route: "cloud"
        )
        let existing = ChatMemoryRecord(
            id: "memory:existing",
            namespace: .longTerm,
            kind: .semantic,
            text: "Existing fact",
            provenance: provenance,
            observedAt: observedAt
        )
        let profile = ChatMemoryProfile(
            longTermMemory: "Existing fact",
            memoryRecords: [existing]
        )
        let first = profile.addingMemory(
            longTerm: ["Existing fact", "New fact"],
            preferences: ["Prefer tables"],
            provenance: provenance,
            observedAt: observedAt
        )
        let second = first.addingMemory(
            longTerm: ["New fact"],
            preferences: ["Prefer tables"],
            provenance: provenance,
            observedAt: observedAt
        )

        XCTAssertEqual(second.memoryRecords, first.memoryRecords)
        XCTAssertEqual(second.memoryRecords.count, 3)
        let newFact = try XCTUnwrap(second.memoryRecords.first { $0.text == "New fact" })
        XCTAssertEqual(newFact.namespace, .longTerm)
        XCTAssertEqual(newFact.kind, .semantic)
        XCTAssertEqual(newFact.confidence, 0.76)
        XCTAssertEqual(newFact.provenance, provenance)
        XCTAssertEqual(newFact.observedAt, observedAt)
        XCTAssertEqual(newFact.retention, .durable)
        XCTAssertTrue(newFact.supersedes.isEmpty)
        XCTAssertEqual(second.memoryRecords.first { $0.text == "Existing fact" }?.observedAt, observedAt)
    }
}

extension ChatMemoryProfileTests {
    func testCoreRejectedMemoryNeverReturnsThroughClientFallback() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("memory-policy-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "Policy isolation", language: "en")
        try store.importMessages(conversationId: conversation.id, messages: [
            ConversationMessage(conversationId: conversation.id, role: .user, text: "Fixture history")
        ], source: .externalArchive)
        let profileStore = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let manager = DefaultContextManager(store: store, profileStore: profileStore)
        let sensitive = ChatMemoryRecord(
            id: "private-policy-marker", namespace: .longTerm, kind: .semantic,
            text: "private marker that the default policy excludes", confidence: 0.95,
            provenance: ChatMemoryProvenance(source: "manual_profile_edit"),
            observedAt: Date(), sensitivity: "health.private"
        )
        try runtime.upsertMemory(XCTUnwrap(sensitive.canonicalRecord))
        let emptySelection = try await manager.prepareOutboundPayload(for: conversation.id,
            maxTokens: 8_000, additionalComponents: [], tools: [])
        XCTAssertFalse(emptySelection.contextComponents.contains { $0.kind == .memory })

        let ordinary = ChatMemoryRecord(
            id: "ordinary-policy-marker", namespace: .longTerm, kind: .semantic,
            text: "A normal durable project preference", confidence: 0.95,
            provenance: ChatMemoryProvenance(source: "manual_profile_edit"), observedAt: Date()
        )
        try runtime.upsertMemory(XCTUnwrap(ordinary.canonicalRecord))
        let selected = try await manager.prepareOutboundPayload(for: conversation.id,
            maxTokens: 8_000, additionalComponents: [], tools: [])
        let ids = Set(selected.contextComponents.filter { $0.kind == .memory }.map(\.id))
        XCTAssertEqual(ids, ["profile:memory:ordinary-policy-marker"])
    }
}

final class ConversationComposerDraftStoreTests: XCTestCase {
    func testOldExpertAcknowledgementCannotClearNewIdenticalTextAfterOwnerRoundTrip() {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a); store.current.text = "same message"
        let old = store.textSubmissionTicket()
        store.select(b); store.current.text = "B draft"
        store.select(a); store.current.text = ""; store.current.text = "same message"
        XCTAssertFalse(store.acknowledgeSubmittedText(old))
        XCTAssertEqual(store.current.text, "same message")
        store.select(b); XCTAssertEqual(store.current.text, "B draft")
    }

    func testAcknowledgementClearsOnlyItsExactUnchangedOwnerWhileAnotherDraftIsSelected() {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a); store.current.text = "submitted A"
        let ticket = store.textSubmissionTicket()
        store.select(b); store.current.text = "unsent B"
        XCTAssertTrue(store.acknowledgeSubmittedText(ticket))
        XCTAssertEqual(store.current.text, "unsent B")
        store.select(a); XCTAssertEqual(store.current.text, "")
        XCTAssertFalse(store.acknowledgeSubmittedText(ticket), "An old acknowledgement cannot consume a later draft revision")
    }

    func testPersistenceFailureFollowsItsOwnerNotSelection() {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a)
        store.current.text = "A"
        store.recordPersistenceFailure(owner: .conversation(a), detail: "A conflict")
        store.select(b)
        store.current.text = "B"
        XCTAssertNil(store.persistenceError)
        store.select(a)
        XCTAssertEqual(store.persistenceError, "A conflict")
        XCTAssertEqual(store.current.text, "A")
    }
    private func attachment(_ name: String) -> ChatAttachmentDraft {
        .init(name: name, typeLabel: "text", sizeBytes: 1, extractedContent: name)
    }

    func testPickerAndImportCompleteIntoOriginalConversationAfterSwitch() throws {
        var store = ConversationComposerDraftStore()
        let a = UUID(), b = UUID()
        store.select(a)
        store.current.text = "A draft"
        let operation = try XCTUnwrap(store.beginImport())
        store.select(b)
        store.current.text = "B draft"
        let bOperation = try XCTUnwrap(store.beginImport())
        XCTAssertTrue(store.finishImport(operation, attachments: [attachment("A.txt")], notice: "A imported"))
        XCTAssertEqual(store.current.text, "B draft")
        XCTAssertTrue(store.current.attachments.isEmpty)
        XCTAssertEqual(store.current.importID, bOperation.id)
        XCTAssertNil(store.current.notice)
        store.select(a)
        XCTAssertEqual(store.current.text, "A draft")
        XCTAssertEqual(store.current.attachments.map(\.name), ["A.txt"])
        XCTAssertEqual(store.current.notice, "A imported")
        XCTAssertNil(store.current.importID)
    }

    func testDuplicateAndDeletedOwnerCannotBeResurrected() throws {
        var store = ConversationComposerDraftStore()
        let id = UUID()
        store.select(id)
        let old = try XCTUnwrap(store.beginImport())
        XCTAssertTrue(store.finishImport(old, attachments: [attachment("once")], notice: nil))
        XCTAssertFalse(store.finishImport(old, attachments: [attachment("twice")], notice: "stale"))
        let pending = try XCTUnwrap(store.beginImport())
        store.remove(id)
        XCTAssertFalse(store.finishImport(pending, attachments: [attachment("deleted")], notice: nil))
        XCTAssertNil(store.draft(for: .conversation(id)))
    }

    func testNewDraftSubmissionRetiresOldOperationsAndDoesNotRestoreSentText() throws {
        var store = ConversationComposerDraftStore()
        let unsent = store.selected
        store.current.text = "submitted"
        let operation = try XCTUnwrap(store.beginImport())
        let id = UUID()
        store.submitted(to: id)
        store.current.text = "next draft"
        store.select(id) // delayed selection publication must not reset it
        XCTAssertEqual(store.current.text, "next draft")
        XCTAssertNil(store.draft(for: unsent))
        XCTAssertFalse(store.finishImport(operation, attachments: [attachment("late")], notice: nil))
        store.select(nil)
        XCTAssertNotEqual(store.selected, unsent)
        XCTAssertEqual(store.current.text, "")
    }

    func testCancellationOnlyRetiresMatchingOperation() throws {
        var store = ConversationComposerDraftStore()
        let cancelled = try XCTUnwrap(store.beginImport())
        XCTAssertTrue(store.finishImport(cancelled, attachments: [], notice: nil))
        let replacement = try XCTUnwrap(store.beginImport())
        XCTAssertFalse(store.finishImport(cancelled, attachments: [], notice: "old error"))
        XCTAssertEqual(store.current.importID, replacement.id)
        store.cancelImports()
        XCTAssertFalse(store.accepts(replacement))
    }
}

final class ConversationDraftRepositoryTests: XCTestCase {
    func testClosedPreAdmissionRejectionRestoresOnlyOriginalOwnerAndRetiresReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        var drafts = ConversationComposerDraftStore(repository: repository)
        drafts.current.text = "原始问题 @专家"
        let owner = UUID(), other = UUID()
        let submission = try drafts.prepareSubmission(to: owner,
            identity: IOSCanonicalAgentTurnIdentity(userMessageID: UUID()), configurationRevision: nil)
        drafts.select(other); drafts.current.text = "new work"
        XCTAssertTrue(try drafts.restoreRejectedSubmission(submission, conversationID: owner))
        XCTAssertEqual(drafts.selected, .conversation(other))
        XCTAssertEqual(drafts.current.text, "new work")
        XCTAssertTrue(try repository.pendingSubmissions().isEmpty)
        drafts.select(owner)
        XCTAssertEqual(drafts.current.text, "原始问题 @专家")
        XCTAssertTrue(drafts.current.restored)
    }

    func testRejectedSubmissionNeverOverwritesNewInputOrRetiresCommittedReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        var drafts = ConversationComposerDraftStore(repository: repository)
        drafts.current.text = "original"
        let owner = UUID()
        let submission = try drafts.prepareSubmission(to: owner,
            identity: IOSCanonicalAgentTurnIdentity(userMessageID: UUID()), configurationRevision: nil)
        drafts.current.text = "newer draft"
        XCTAssertFalse(try drafts.restoreRejectedSubmission(submission, conversationID: owner))
        XCTAssertEqual(drafts.current.text, "newer draft")
        XCTAssertEqual(try repository.pendingSubmissions().count, 1)
        try submission.committed()
        XCTAssertThrowsError(try repository.retireRejectedSubmission(XCTUnwrap(submission.ticket)))
        XCTAssertEqual(try repository.load(.conversation(owner))?.text, "newer draft")
    }

    func testRemoteSamplingRoundTripsWithoutLocalDefaults() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        let saved = try ConversationParameterSnapshot.updating(nil, modelKey: "remote", parameters: parameters)
        let restored = try ConversationParameterSnapshot.read(saved, modelKey: "remote")
        XCTAssertNil(restored.topK)
        XCTAssertNil(restored.repetitionPenalty)
        parameters.topK = 40
        parameters.repetitionPenalty = 1
        let explicit = try ConversationParameterSnapshot.updating(saved, modelKey: "remote", parameters: parameters)
        let explicitRestored = try ConversationParameterSnapshot.read(explicit, modelKey: "remote")
        XCTAssertEqual(explicitRestored.topK, 40)
        XCTAssertEqual(explicitRestored.repetitionPenalty, 1)
    }

    func testSubmissionIntentSurvivesRestartAndCommitPreservesNewInput() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        let conversation = UUID()
        let owner = ConversationComposerDraftStore.Owner.conversation(conversation)
        try repository.save(.init(text: "captured input"), owner: owner)
        _ = try repository.load(owner)
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let ticket = try repository.stageSubmission(owner: owner, conversationID: conversation,
            userMessageID: identity.visibleUserMessageID, configurationRevision: 7,
            draft: .init(text: "captured input"))
        let reopened = FileConversationDraftRepository(root: root)
        XCTAssertEqual(try reopened.pendingSubmissions(), [ticket])
        try repository.save(.init(text: "new input typed after dispatch"), owner: owner)
        XCTAssertEqual(try repository.load(owner)?.text, "new input typed after dispatch")
        try reopened.commitSubmission(ticket)
        try reopened.commitSubmission(ticket)
        XCTAssertTrue(try reopened.pendingSubmissions().isEmpty)
        XCTAssertEqual(try reopened.load(owner)?.text, "new input typed after dispatch")
    }

    func testLostAckReopensOriginalConversationAndRetiresOnlyCapturedUnsentVersion() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        var drafts = ConversationComposerDraftStore(repository: repository)
        drafts.current.text = "send exactly once"
        let original = drafts.selected
        let conversation = UUID()
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let submission = try drafts.prepareSubmission(to: conversation, identity: identity, configurationRevision: 2)
        XCTAssertEqual(try repository.pendingSubmissions().first?.text, "send exactly once")
        let restarted = ConversationComposerDraftStore(repository: FileConversationDraftRepository(root: root))
        XCTAssertEqual(restarted.selected, .conversation(conversation))
        XCTAssertEqual(try repository.load(original)?.text, "send exactly once")
        try submission.committed()
        XCTAssertTrue(try repository.pendingSubmissions().isEmpty)
        XCTAssertThrowsError(try repository.load(original))
    }

    func testPendingIndexPointingAtRetiredDraftRecoversAfterCrash() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let old = try repo.pendingOwner()
        try repo.remove(.unsent(old))
        // Simulate the crash boundary: durable tombstone, old pending index.
        try JSONEncoder().encode(old).write(to: root.appendingPathComponent("pending-owner.json"), options: .atomic)
        let reopened = FileConversationDraftRepository(root: root)
        let new = try reopened.pendingOwner()
        XCTAssertNotEqual(new, old)
        try reopened.save(.init(text: "new draft"), owner: .unsent(new))
        XCTAssertEqual(try reopened.load(.unsent(new))?.text, "new draft")
        XCTAssertThrowsError(try reopened.load(.unsent(old)))
    }
    func testSameAttachmentIdentityUsesImmutableVersionsAndFailedCommitKeepsOldSnapshot() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        let id = UUID()
        let first = ChatAttachmentDraft(id: id, name: "file", typeLabel: "text", sizeBytes: 3, extractedContent: "old")
        let second = ChatAttachmentDraft(id: id, name: "file", typeLabel: "text", sizeBytes: 3, extractedContent: "new")
        let repo = FileConversationDraftRepository(root: root)
        try repo.save(.init(text: "old snapshot", attachments: [first]), owner: owner)
        XCTAssertEqual(try repo.load(owner)?.attachments, [first])
        let failing = FileConversationDraftRepository(root: root, beforeManifestCommit: { throw CocoaError(.fileWriteUnknown) })
        _ = try failing.load(owner)
        let failure = expectation(forNotification: FileConversationDraftRepository.writeFailed, object: nil) {
            $0.userInfo?["repository"] as? UUID == failing.identity
        }
        try failing.save(.init(text: "uncommitted", attachments: [second]), owner: owner)
        wait(for: [failure], timeout: 2)
        XCTAssertEqual(try repo.load(owner)?.text, "old snapshot")
        XCTAssertEqual(try repo.load(owner)?.attachments, [first])
        try repo.save(.init(text: "committed", attachments: [second]), owner: owner)
        XCTAssertEqual(try repo.load(owner)?.attachments, [second])
        let blobs = try FileManager.default.subpathsOfDirectory(atPath: root.path).filter { $0.contains("\(id.uuidString)-") }
        XCTAssertEqual(blobs.count, 2)
        // Hash verification must reject corrupted content while keeping text.
        for path in blobs { try Data("corrupt".utf8).write(to: root.appendingPathComponent(path)) }
        let recovered = try XCTUnwrap(repo.load(owner))
        XCTAssertEqual(recovered.text, "committed")
        XCTAssertTrue(recovered.attachments.isEmpty)
        XCTAssertNotNil(recovered.notice)
    }
    func testUnsentConfigurationPersistsWithoutCredentialsAndMissingModelDoesNotFallback() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let owner = ConversationComposerDraftStore.Owner.unsent(try repo.pendingOwner())
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.23
        let configuration = ConversationComposerDraftStore.ModelConfiguration(modelID: "specific-profile", source: .configured, parameters: parameters)
        try repo.save(.init(text: "unsent", configuration: configuration), owner: owner)
        let restored = try XCTUnwrap(FileConversationDraftRepository(root: root).load(owner))
        XCTAssertEqual(restored.configuration, configuration)
        XCTAssertTrue(restored.restored)
        let differentModel = ModelInfo(id: "global-default", name: "Default", source: "api", isLocal: false)
        XCTAssertThrowsError(try IOSDraftModelResolver.resolve(configuration, catalog: [differentModel]))
        let sameIDLocal = ModelInfo(id: "specific-profile", name: "Local", source: "local", isLocal: true)
        XCTAssertThrowsError(try IOSDraftModelResolver.resolve(configuration, catalog: [sameIDLocal]))
        let matchingCloud = ModelInfo(id: "specific-profile", name: "Cloud", source: "api", isLocal: false)
        XCTAssertEqual(try IOSDraftModelResolver.resolve(configuration, catalog: [sameIDLocal, matchingCloud]), matchingCloud)
        let files = try FileManager.default.subpathsOfDirectory(atPath: root.path)
        let manifest = try XCTUnwrap(files.first { $0.hasSuffix("manifest.json") })
        let data = try Data(contentsOf: root.appendingPathComponent(manifest))
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let savedConfiguration = try XCTUnwrap(json["configuration"] as? [String: Any])
        XCTAssertEqual(Set(savedConfiguration.keys), ["modelID", "source", "parameters"])
    }

    func testTwoInstancesRejectStaleWriteAndDeletionCannotBeResurrected() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let a = FileConversationDraftRepository(root: root), b = FileConversationDraftRepository(root: root)
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        try a.save(.init(text: "original"), owner: owner)
        XCTAssertEqual(try b.load(owner)?.text, "original")
        try a.save(.init(text: "A committed"), owner: owner)
        let conflict = expectation(forNotification: FileConversationDraftRepository.writeFailed, object: nil) {
            $0.userInfo?["repository"] as? UUID == b.identity
        }
        try b.save(.init(text: "B must remain local"), owner: owner)
        wait(for: [conflict], timeout: 2)
        XCTAssertEqual(try a.load(owner)?.text, "A committed")
        try a.remove(owner)
        let deleted = expectation(forNotification: FileConversationDraftRepository.writeFailed, object: nil) {
            $0.userInfo?["repository"] as? UUID == b.identity
        }
        try b.save(.init(text: "late"), owner: owner)
        wait(for: [deleted], timeout: 2)
        XCTAssertThrowsError(try FileConversationDraftRepository(root: root).load(owner))
    }

    func testSendingExistingConversationClearsButDoesNotRetireItsComposer() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        var store = ConversationComposerDraftStore(repository: repo)
        let id = UUID()
        store.select(id)
        store.current.text = "first"
        store.submitted(to: id)
        store.current.text = "second"
        XCTAssertEqual(try FileConversationDraftRepository(root: root).load(.conversation(id))?.text, "second")
    }

    func testSameConversationInDifferentDatabaseScopesDoesNotShareOrDeleteDrafts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let first = FileConversationDraftRepository.scoped(to: root.appendingPathComponent("one.sqlite"))
        let second = FileConversationDraftRepository.scoped(to: root.appendingPathComponent("two.sqlite"))
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        try first.save(.init(text: "first"), owner: owner)
        try second.save(.init(text: "second"), owner: owner)
        XCTAssertEqual(try first.load(owner)?.text, "first")
        XCTAssertEqual(try second.load(owner)?.text, "second")
        try first.remove(owner)
        XCTAssertThrowsError(try first.load(owner))
        XCTAssertEqual(try second.load(owner)?.text, "second")
    }

    func testRestartRestoresTextAndAttachmentsAndMissingAttachmentKeepsText() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        let attachment = ChatAttachmentDraft(name: "notes.txt", typeLabel: "text", sizeBytes: 4, extractedContent: "body")
        try repo.save(.init(text: "draft", attachments: [attachment]), owner: owner)
        let reopened = FileConversationDraftRepository(root: root)
        XCTAssertEqual(try reopened.load(owner)?.text, "draft")
        XCTAssertEqual(try reopened.load(owner)?.attachments, [attachment])
        let resources = try FileManager.default.subpathsOfDirectory(atPath: root.path)
        let resource = try XCTUnwrap(resources.first { $0.contains("\(attachment.id.uuidString)-") && $0.hasSuffix(".json") })
        try FileManager.default.removeItem(at: root.appendingPathComponent(resource))
        let recovered = try XCTUnwrap(reopened.load(owner))
        XCTAssertEqual(recovered.text, "draft")
        XCTAssertTrue(recovered.attachments.isEmpty)
        XCTAssertNotNil(recovered.notice)
        try repo.remove(owner)
        XCTAssertThrowsError(try reopened.load(owner))
    }

    func testPendingIdentityAndSnapshotAreIndependentOfLiveDefaults() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let first = try repo.pendingOwner()
        XCTAssertEqual(try FileConversationDraftRepository(root: root).pendingOwner(), first)
        XCTAssertNotEqual(try repo.newPendingOwner(), first)
        let key = "configured:fixture"
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.25
        let json = try ConversationParameterSnapshot.updating("{\"foreign\":true}", modelKey: key, parameters: parameters)
        parameters.temperature = 1
        XCTAssertEqual(try ConversationParameterSnapshot.read(json, modelKey: key).temperature, 0.25)
        XCTAssertThrowsError(try ConversationParameterSnapshot.read(json, modelKey: "configured:missing"))
        XCTAssertEqual(try IOSConversationSettingsObject.decode(json)["foreign"] as? Bool, true)
    }
}

final class IOSDraftAggregateTests: XCTestCase {
    func testStandardCanonicalProfileOwnsProjectionAndSwitchBackPreservesOverrides() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let profile = IOSCanonicalModelProfile(id: "canonical-profile", displayName: "Canonical Name", provider: "fixture",
            modelName: "upstream-wire-id", runtime: "remote",
            sourceJSON: "{\"baseURL\":\"https://fixture.invalid/v1\",\"wireProtocol\":\"responses\"}",
            parametersJSON: "{}", selected: false, enabled: true, revision: 0,
            createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil)
        try runtime.upsertModelProfile(profile)
        let model = try XCTUnwrap(ModelSelectionManager.modelInfo(from: profile))
        XCTAssertEqual(model.id, "canonical-profile")
        XCTAssertEqual(model.effectiveUpstreamModelId, "upstream-wire-id")
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.23
        let conversation = try store.createConfiguredConversation(title: "Original", language: nil, model: model, parameters: parameters)
        try registerLocal("local-other", runtime: runtime)
        try store.selectConversationModel(conversationID: conversation.id,
            model: ModelInfo(id: "local-other", name: "Other", source: "local", isLocal: true), initialParameters: .webAlignedCloudDefaults)
        try store.selectConversationModel(conversationID: conversation.id, model: model, initialParameters: .webAlignedCloudDefaults)
        XCTAssertEqual(try store.conversationParameters(conversationId: conversation.id, modelKey: "configured:canonical-profile").temperature, 0.23)
        var displayed = try XCTUnwrap(store.getConversation(conversationId: conversation.id))
        displayed.selectedModelInfo = ModelInfo(id: "wrong", name: "Wrong", source: "local", isLocal: true)
        displayed.title = "Renamed"
        try store.upsertConversation(displayed)
        XCTAssertEqual(try store.getConversation(conversationId: conversation.id)?.selectedModelInfo?.id, model.id)
        let fields = try IOSConversationSettingsObject.decode(try runtime.loadConversation(id: conversation.id.uuidString)?.settingsJSON)
        XCTAssertNil(fields["selectedModelInfo"])
        XCTAssertEqual(fields["modelSelectionID"] as? String, "configured:canonical-profile")
    }

    func testHostIsConnectionBindingNotConfiguredModel() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let host = ModelInfo(id: "web-backend:local-host", name: "Host", source: "web", isLocal: false,
            baseURL: "https://host.invalid", apiKey: "must-not-persist")
        let conversation = try store.createConfiguredConversation(title: nil, language: nil, model: host, parameters: .webAlignedCloudDefaults)
        let stored = try XCTUnwrap(runtime.loadConversation(id: conversation.id.uuidString))
        let fields = try IOSConversationSettingsObject.decode(stored.settingsJSON)
        XCTAssertNil(fields["modelSelectionID"])
        XCTAssertNil(fields["modelOverrides"])
        XCTAssertNotNil(fields["executionHostBinding"])
        XCTAssertFalse(stored.settingsJSON?.contains("must-not-persist") ?? true)
        XCTAssertTrue(try XCTUnwrap(store.getConversation(conversationId: conversation.id)?.selectedModelInfo).isWebBackend)
        XCTAssertEqual(try store.conversationParameters(conversationId: conversation.id, modelKey: "execution-host"), .webAlignedCloudDefaults)
    }
    private func registerLocal(_ id: String, runtime: IOSApplicationRuntime) throws {
        try runtime.upsertModelProfile(.init(id: id, displayName: id, provider: "local", modelName: id,
            runtime: "mlx", sourceJSON: "{}", parametersJSON: "{}", selected: false,
            enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
    }
    @MainActor
    func testUnpublishedCreationAndUnreadableSelectedConversationNeverReplaceTheDraftSelection() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        try registerLocal("fixture-local", runtime: runtime)
        let container = ServiceContainer(applicationRuntime: runtime)
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = ModelInfo(id: "fixture-local", name: "Fixture", source: "local", isLocal: true)
        let id = try XCTUnwrap(manager.ensureCurrentConversation(initialParameters: .webAlignedCloudDefaults, selectOnCreate: false))
        XCTAssertNil(manager.currentConversationId)
        XCTAssertNotNil(manager.executionForConversation(id))
        manager.currentConversationId = id
        // A catalogue removal makes the selected identity unavailable without
        // bypassing the configuration command through a full settings write.
        XCTAssertTrue(try runtime.deleteModelProfile(id: "fixture-local"))
        XCTAssertEqual(manager.ensureCurrentConversation(), id)
        let unavailable = try XCTUnwrap(container.conversationStore?.getConversation(conversationId: id))
        XCTAssertNil(unavailable.selectedModelInfo)
        XCTAssertNotNil(unavailable.modelUnavailableReason)
        XCTAssertEqual(manager.currentConversationId, id)
        XCTAssertEqual(try runtime.listConversations().count, 1)
    }
    func testConfiguredCreationCommitsIdentityAndParametersTogetherAndRejectsInvalidInput() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        for id in ["fixture-local", "second-local", "third-local"] { try registerLocal(id, runtime: runtime) }
        let model = ModelInfo(id: "fixture-local", name: "Fixture", source: "local", isLocal: true)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.42
        let created = try store.createConfiguredConversation(title: "Owned", language: nil, model: model, parameters: parameters)
        let stored = try XCTUnwrap(runtime.loadConversation(id: created.id.uuidString))
        let json = try IOSConversationSettingsObject.decode(stored.settingsJSON)
        XCTAssertEqual(json["modelSelectionID"] as? String, "local:fixture-local")
        XCTAssertNil(json["selectedModelInfo"])
        XCTAssertEqual(try store.getConversation(conversationId: created.id)?.selectedModelInfo?.id, model.id)
        XCTAssertEqual(try store.conversationParameters(conversationId: created.id, modelKey: "local:fixture-local").temperature, 0.42)
        let second = ModelInfo(id: "second-local", name: "Second", source: "local", isLocal: true)
        try store.selectConversationModel(conversationID: created.id, model: second, initialParameters: parameters)
        let changed = try XCTUnwrap(runtime.loadConversation(id: created.id.uuidString))
        XCTAssertEqual(try IOSConversationSettingsObject.decode(changed.settingsJSON)["modelSelectionID"] as? String, "local:second-local")
        XCTAssertEqual(try store.getConversation(conversationId: created.id)?.selectedModelInfo?.id, second.id)
        XCTAssertEqual(try store.conversationParameters(conversationId: created.id, modelKey: "local:second-local").temperature, 0.42)
        parameters.maxOutputTokens = 0
        let third = ModelInfo(id: "third-local", name: "Third", source: "local", isLocal: true)
        XCTAssertThrowsError(try store.selectConversationModel(conversationID: created.id, model: third, initialParameters: parameters))
        XCTAssertEqual(try store.getConversation(conversationId: created.id)?.selectedModelInfo?.id, second.id)
        XCTAssertThrowsError(try store.createConfiguredConversation(title: "Invalid", language: nil, model: model, parameters: parameters))
        XCTAssertEqual(try runtime.listConversations().count, 1)
        parameters.maxOutputTokens = ModelParameters.webAlignedCloudDefaults.maxOutputTokens
        let validAfterRejection = try store.createConfiguredConversation(title: "Still writable", language: nil,
            model: model, parameters: parameters)
        XCTAssertNotEqual(validAfterRejection.id, created.id)
        XCTAssertEqual(try runtime.listConversations().count, 2,
            "Locally rejected editor input must not leave an unconfirmed mutation ahead of valid work")
    }
}

final class IOSChatPresentationCommandsAndDraftContractsTests: XCTestCase {
    func testCurrentToolPresentationTraceMetaCommandsTitlePolicyKnowledgeAndEditDrafts() throws {
        // Current tool presentation & historical reasoning
        let failed = HostAgentToolEvent(id: "failed", name: "bash", status: .failed, text: "Missing dependency")
        let running = HostAgentToolEvent(id: "current", name: "bash", status: .running, text: "Installing")
        let msg1 = ChatMessage(role: .assistant, content: "", hostToolEvents: [failed, running])
        XCTAssertEqual(msg1.failedHistoricalToolEvents.map(\.id), ["failed"])
        XCTAssertEqual(msg1.liveHostToolEvents.map(\.id), ["current"])
        let currentFailure = ChatMessage(role: .assistant, content: "", hostToolEvents: [failed],
            metadata: [ChatConversationSurfaceMeta.currentToolCallIDKey: "failed"])
        XCTAssertTrue(currentFailure.failedHistoricalToolEvents.isEmpty)

        let history = ChatMessage(role: .assistant, content: "Answer", thinking: "Actual reasoning")
        let runtime = ChatAssistantRuntimeState(message: history, isActive: false, isRetrieving: false)
        XCTAssertTrue(runtime.showsThinking)
        XCTAssertFalse(runtime.showsWaiting || runtime.showsPhase)
        XCTAssertFalse(ChatAssistantRuntimeState(message: ChatMessage(role: .assistant, content: "Answer", thinking: "  "), isActive: false, isRetrieving: false).showsThinking)

        let completed = HostAgentToolEvent(id: "completed", name: "read_file", status: .succeeded, text: "done")
        let activeMsg = ChatMessage(role: .assistant, content: "", hostToolEvents: [completed, running])
        XCTAssertEqual(activeMsg.hostToolEvents.count, 2)
        XCTAssertEqual(activeMsg.liveHostToolEvents.map(\.id), ["current"])
        let doneMsg = ChatMessage(role: .assistant, content: "Result", hostToolEvents: [completed])
        XCTAssertTrue(doneMsg.liveHostToolEvents.isEmpty)
        XCTAssertEqual(doneMsg.hostToolEvents.count, 1)

        // Agent trace meta & composer slash commands
        let meta = ChatAgentTraceMeta.values(thinking: "  Inspecting the selected sources.  ", phase: "  searching  ")
        XCTAssertEqual(ChatAgentTraceMeta.thinking(from: meta), "Inspecting the selected sources.")
        XCTAssertEqual(ChatAgentTraceMeta.phase(from: meta), "searching")
        XCTAssertTrue(ChatAgentTraceMeta.values(thinking: " ", phase: nil).isEmpty)

        for text in ["搜索 current", "please /stop", "https://example.org", "模型权限设计"] {
            XCTAssertNil(ComposerCommand.parse(text))
        }
        XCTAssertEqual(ComposerCommand.query("/mo"), "mo")
        XCTAssertNil(ComposerCommand.query("/model "))
        XCTAssertEqual(ComposerCommand.parse("/search 最近变化")?.argument, "最近变化")
        XCTAssertNil(ComposerCommand.parse("/unknown")?.command)
        XCTAssertNil(ComposerCommand.parse("/stop extra")?.command)
        XCTAssertEqual(ComposerCommand.parse("/stop ")?.command, .stop)

        // Conversation title presentation policy
        let convID = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
        let generatedTitle = String(format: String(localized: "Conversation %@"), String(convID.prefix(8)))
        for rawTitle in [convID, "Session 01DAF2BB", generatedTitle] {
            XCTAssertEqual(IOSConversationTitlePresentationPolicy.displayTitle(rawTitle, conversationID: convID, fallback: "New conversation"), "New conversation")
        }
        XCTAssertEqual(IOSConversationTitlePresentationPolicy.displayTitle("Session planning", conversationID: convID, fallback: "New conversation"), "Session planning")
        XCTAssertEqual(IOSConversationTitlePresentationPolicy.displayTitle("Conversation design review", conversationID: convID, fallback: "New conversation"), "Conversation design review")

        // Knowledge snapshot & Conversation ISO8601 round-trip
        let kConvID = UUID()
        var kp = "Knowledge A", wiki = "Wiki A"
        let kSnap = ConversationKnowledgeSaveSnapshot(conversationID: kConvID, knowledgePoints: kp, wiki: wiki)
        kp = "Knowledge B"; wiki = "Wiki B"
        XCTAssertEqual(kSnap.conversationID, kConvID)
        XCTAssertEqual(kSnap.knowledgePoints, "Knowledge A")
        XCTAssertEqual(kSnap.wiki, "Wiki A")
        let conv = Conversation(title: "Knowledge", sessionMemory: "Current task", knowledgePoints: "- Preserve original evidence", wiki: "## Architecture\nLocal-first")
        let enc = JSONEncoder(); enc.dateEncodingStrategy = .iso8601
        let dec = JSONDecoder(); dec.dateDecodingStrategy = .iso8601
        let decodedConv = try dec.decode(Conversation.self, from: enc.encode(conv))
        XCTAssertEqual(decodedConv.knowledgePoints, "- Preserve original evidence")
        XCTAssertEqual(decodedConv.wiki, "## Architecture\nLocal-first")

        // AppleMessageEditDraft contracts
        var editDraft = AppleMessageEditDraft(sourceConversationID: "original", sourceMessageID: "user", expectedSourceRevision: 4, expectedMessageRevision: 7, text: "Original")
        editDraft.text = "Edited"
        let inPlace = try XCTUnwrap(editDraft.inPlacePreparation())
        XCTAssertEqual(inPlace.conversation_id, "original")
        XCTAssertEqual(inPlace.message_id, "user")
        XCTAssertEqual(inPlace.expected_revision, 4)
        XCTAssertEqual(inPlace, editDraft.inPlacePreparation())
        let editSub = AppleMessageEditSubmission(operationID: editDraft.operationID, targetConversationID: editDraft.targetConversationID,
            createdAtMs: editDraft.createdAtMs, sourceConversationID: "original", sourceMessageID: "user",
            expectedSourceRevision: 4, expectedMessageRevision: 7, text: "Edited")
        XCTAssertEqual(editSub.inPlacePreparation(), inPlace)
        let firstPrep = try XCTUnwrap(editDraft.preparation(title: "New"))
        let retryPrep = try XCTUnwrap(editDraft.preparation(title: "New"))
        XCTAssertEqual(editDraft.originalText, "Original")
        XCTAssertEqual(firstPrep.source_conversation_id, "original")
        XCTAssertEqual(firstPrep.expected_source_revision, 4)
        XCTAssertEqual(firstPrep.expected_source_message_revision, 7)
        XCTAssertEqual(firstPrep.edited_text, "Edited")
        XCTAssertEqual(firstPrep.created_at_ms, retryPrep.created_at_ms)
        XCTAssertEqual(firstPrep.operation_id, retryPrep.operation_id)
        XCTAssertEqual(firstPrep.target_conversation_id, retryPrep.target_conversation_id)
        XCTAssertNotEqual(firstPrep.target_conversation_id, firstPrep.source_conversation_id)

        var mutDraft = AppleMessageEditDraft(sourceConversationID: "source", sourceMessageID: "user", expectedSourceRevision: 4, expectedMessageRevision: 7, text: "First")
        let op1 = mutDraft.operationID, tgt1 = mutDraft.targetConversationID
        mutDraft.text = "Second"
        XCTAssertNotEqual(mutDraft.operationID, op1); XCTAssertNotEqual(mutDraft.targetConversationID, tgt1)
        XCTAssertEqual(mutDraft.sourceConversationID, "source"); XCTAssertEqual(mutDraft.expectedSourceRevision, 4)
        let op2 = mutDraft.operationID; mutDraft.text = "Second"; XCTAssertEqual(mutDraft.operationID, op2)

        let noWitness = AppleMessageEditDraft(sourceConversationID: "original", sourceMessageID: "user", expectedSourceRevision: nil, expectedMessageRevision: 7, text: "Edited")
        XCTAssertFalse(noWitness.hasSourceWitness)
        XCTAssertNil(noWitness.preparation(title: "New"))
    }

    @MainActor
    func testExecutionProcessPolicyThinkingParserMermaidAndNavigationGestures() throws {
        let originalAttr = try AttributedString(markdown: "正文 [来源](https://example.com/report) [Source](https://example.com/other)")
        let styledAttr = AppleInlineReferenceStyle.applying(to: originalAttr, color: DesignTokens.secondaryContentColor)
        XCTAssertEqual(styledAttr.runs.compactMap(\.link), originalAttr.runs.compactMap(\.link))
        for run in styledAttr.runs where run.link != nil {
            XCTAssertEqual(run.font, .caption)
            XCTAssertEqual(run.foregroundColor, DesignTokens.secondaryContentColor)
        }

        let processing = NSLocalizedString("chat.processing", value: "Processing", comment: "Run remains active")
        XCTAssertEqual(ChatExecutionProcessPolicy.heading(live: true, currentTitle: nil, containsTools: true), processing)
        XCTAssertEqual(ChatExecutionProcessPolicy.heading(live: true, currentTitle: "Reading", containsTools: true), "Reading")
        XCTAssertEqual(ChatExecutionProcessPolicy.heading(live: false, currentTitle: "Stale title", containsTools: true),
            NSLocalizedString("chat.execution_process", value: "Execution process", comment: "Ordered reasoning and tools"))

        let src = ChatSourceReference(id: "source", sourceID: "s", title: "Source", uri: "https://example.com")
        let alias = ChatSourceReference(id: "alias", sourceID: "s", title: "Citation", uri: "https://example.com")
        let pinpoint = ChatSourceReference(id: "pinpoint", sourceID: "s", title: "Page two", uri: "https://example.com", location: .init(page: 2))
        XCTAssertEqual(ChatSourceReference.bibliography([src, alias, pinpoint]).map(\.id), ["source", "pinpoint"])
        XCTAssertEqual(ChatSourceReference.bibliography([ChatSourceReference(id: "one", sourceID: "one", title: "One", uri: ""), ChatSourceReference(id: "two", sourceID: "two", title: "Two", uri: "")]).count, 2)

        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: [], liveReasoning: true))
        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: [], liveReasoning: false))
        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: [false, true], liveReasoning: true))
        XCTAssertTrue(ChatExecutionProcessPolicy.segmentExpanded(choices: [true], liveReasoning: false))
        let rKey = IOSMessagePartReadingKey(source: "store-a", conversation: "c", message: "m", part: "r")
        let choices = [rKey: false]
        XCTAssertNil(choices[.init(source: "store-b", conversation: "c", message: "m", part: "r")])
        XCTAssertNil(choices[.init(source: "store-a", conversation: "other", message: "m", part: "r")])
        XCTAssertNil(choices[.init(source: "store-a", conversation: "c", message: "other", part: "r")])
        XCTAssertFalse(ChatExecutionProcessPolicy.segmentExpanded(choices: ["new", "r"].compactMap { choices[.init(source: "store-a", conversation: "c", message: "m", part: $0)] }, liveReasoning: true))

        let r1 = IOSConversationProjectionV1.Part.reasoning(.init(partID: "r1", revision: 1, state: .completed, title: "Thinking", summary: "inspect"))
        let r2 = IOSConversationProjectionV1.Part.reasoning(.init(partID: "r2", revision: 1, state: .running, title: "Thinking", summary: "verify"))
        let prose = IOSConversationProjectionV1.Part.text(.init(partID: "text", revision: 1, markdown: "Preparing a form", isFinal: true, review: nil))
        let tool = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: Data(#"{"kind":"tool","execution_source":{"kind":"model"},"part_id":"tool","revision":2,"tool_call_id":"call","capability_id":"render_ui","state":"completed","title":"Render","origin":{"kind":"local"},"artifacts":[],"input":{},"affected_resources":[]}"#.utf8))
        let surface = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: Data(#"{"kind":"ui_surface","part_id":"surface","revision":3,"surface":{"identity":{"owner":{"authority_id":"store","conversation_id":"c","participant_id":"agent","context_id":"ctx","originating_run_id":"run","message_id":"message","part_id":"tool"},"surface_id":"form","generation":1},"revision":1},"summary":"Form"}"#.utf8))
        let finalPart = IOSConversationProjectionV1.Part.text(.init(partID: "final", revision: 1, markdown: "Finished", isFinal: true, review: nil))
        let ordered = [r1, prose, r2, tool, surface, finalPart]
        let flow = ChatExecutionProcessPolicy.segments(ordered)
        XCTAssertEqual(flow.map { $0.parts.map(\.partID) }, [["r1"], ["text"], ["r2", "tool"], ["surface"], ["final"]])
        XCTAssertEqual(flow.flatMap(\.parts), ordered)
        XCTAssertEqual(ChatExecutionProcessPolicy.liveReasoning(ordered, live: true).map(\.partID), ["r2"])
        XCTAssertTrue(ChatExecutionProcessPolicy.liveReasoning(ordered, live: false).isEmpty)

        let localized: [String: String] = ["known.failure": "An actual translated error", "chat.step.failed": "This step failed"]
        XCTAssertEqual(ChatExecutionProcessPolicy.errorTitle(displayKey: "known.failure", localize: { localized[$0] }), "An actual translated error")
        XCTAssertEqual(ChatExecutionProcessPolicy.errorTitle(displayKey: "protocol.unknown_error", localize: { localized[$0] }), "This step failed")
        let summaryPart = IOSConversationProjectionV1.Part.task(id: "summary", revision: 1, taskID: "exact-task", displayKey: "Task", state: .running, progress: nil, currentStep: nil, resultRef: nil, cancelRequested: false)
        XCTAssertFalse(ChatExecutionProcessPolicy.taskControlShowsSummary(taskID: "exact-task", parts: [summaryPart]))
        XCTAssertTrue(ChatExecutionProcessPolicy.taskControlShowsSummary(taskID: "other-task", parts: [summaryPart]))
        XCTAssertTrue(ChatExecutionProcessPolicy.taskControlShowsSummary(taskID: "exact-task", parts: []))

        for state in [IOSConversationProjectionV1.ActivityState.pending, .running, .waitingForPermission, .waitingForInput, .completed, .sealed, .cancelled, .failed] {
            let title = ChatExecutionProcessPolicy.activityTitle(state)
            XCTAssertFalse(title.isEmpty || title == state.rawValue || title.hasPrefix("chat."))
        }
        XCTAssertNotEqual(ChatExecutionProcessPolicy.activityTitle(.sealed), ChatExecutionProcessPolicy.activityTitle(.completed))
        for state in [IOSConversationProjectionV1.ActivityState.completed, .sealed, .cancelled, .failed] {
            XCTAssertTrue(ChatExecutionProcessPolicy.isTerminal(state))
        }
        XCTAssertFalse(ChatExecutionProcessPolicy.isTerminal(.running))
        XCTAssertEqual(ChatExecutionProcessPolicy.displayTitle("protocol.unknown.task", fallbackKey: "chat.part.task"), NSLocalizedString("chat.part.task", comment: "Task"))

        let closedSplit = ChatThinkingParser.splitThinkingBlocks(from: "<think>分析问题</think>\n最终回答")
        XCTAssertEqual(closedSplit.thinking, "分析问题"); XCTAssertEqual(closedSplit.response, "\n最终回答"); XCTAssertFalse(closedSplit.hasOpenThinkingBlock)
        let openSplit = ChatThinkingParser.splitThinkingBlocks(from: "<think>正在推理第一步")
        XCTAssertEqual(openSplit.thinking, "正在推理第一步"); XCTAssertEqual(openSplit.response, ""); XCTAssertTrue(openSplit.hasOpenThinkingBlock)
        let multiSplit = ChatThinkingParser.splitThinkingBlocks(from: "前缀<think>A</think>正文<think>B</think>结尾")
        XCTAssertEqual(multiSplit.thinking, "AB"); XCTAssertEqual(multiSplit.response, "前缀正文结尾"); XCTAssertFalse(multiSplit.hasOpenThinkingBlock)

        let mermaidSource = "graph TD\n  A[开始] --> B{是否可用}\n  B -->|是| C[显示 Mermaid 图]\n  B -->|否| D[显示源码 fallback]"
        XCTAssertEqual(try MermaidRenderer.parse(mermaidSource).type, .flowchart)
        let mermaidImg = try XCTUnwrap(MermaidRenderer.renderImage(source: mermaidSource, theme: .zincLight.withTransparent(true)))
        XCTAssertGreaterThan(mermaidImg.size.width, 0); XCTAssertGreaterThan(mermaidImg.size.height, 0)

        let nav = UINavigationController(rootViewController: UIViewController())
        nav.loadViewIfNeeded()
        let gesture = try XCTUnwrap(nav.interactivePopGestureRecognizer)
        let origDelegate = gesture.delegate
        let origEnabled = gesture.isEnabled
        let chat = ChatNavigationGestureBridge.Controller()
        nav.pushViewController(chat, animated: false)
        chat.viewDidAppear(false)
        XCTAssertTrue(gesture.delegate === chat && gesture.isEnabled && chat.gestureRecognizerShouldBegin(gesture))
        chat.viewWillDisappear(false); XCTAssertTrue(gesture.delegate === chat)
        chat.viewDidAppear(false); XCTAssertTrue(gesture.delegate === chat)
        chat.viewDidDisappear(false)
        XCTAssertTrue(gesture.delegate === origDelegate); XCTAssertEqual(gesture.isEnabled, origEnabled)
        chat.viewDidAppear(false); XCTAssertTrue(gesture.delegate === chat); chat.viewDidDisappear(false)

        let rootChat = ChatNavigationGestureBridge.Controller()
        let rootNav = UINavigationController(rootViewController: rootChat)
        rootNav.loadViewIfNeeded()
        XCTAssertFalse(rootChat.gestureRecognizerShouldBegin(try XCTUnwrap(rootNav.interactivePopGestureRecognizer)))
    }
}

@Suite("iOS canonical chat preferences E2E")
struct IOSChatPreferencesEndToEndTests {
    @Test
    func preferencesSurviveColdReopenAndRejectObsoleteSchema() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-chat-preferences-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let removedKey = "chat.auto_compact.trigger_percent"
        UserDefaults.standard.set(55, forKey: removedKey)
        defer { UserDefaults.standard.removeObject(forKey: removedKey) }

        let first = try IOSApplicationRuntime(databaseURL: database)
        var preferences = IOSChatPreferences()
        preferences.autoCompactTriggerPercent = 90
        try preferences.save(store: first)

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        #expect(try IOSChatPreferences.load(store: reopened).autoCompactTriggerPercent == 90)

        _ = try reopened.putProductSetting(
            key: IOSChatPreferences.productSettingKey,
            valueJSON: #"{"schemaVersion":0,"autoCompactTriggerPercent":70}"#,
            updatedAtMs: 2
        )
        #expect(throws: IOSChatPreferencesError.unsupportedSchema(0)) {
            _ = try IOSChatPreferences.load(store: reopened)
        }
    }
}
