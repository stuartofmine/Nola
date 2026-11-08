// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

@MainActor
final class MacCanonicalConversationRepositoryEndToEndTests: XCTestCase {
    func testConfigurationCommitSurvivesStaleMetadataAndParameterClearPreservesForeignFields() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try MacAppleChatConfigurationStore(applicationPort: port).replaceEndpoints([
            MacRemoteEndpointConfiguration(id: "endpoint", name: "Fixture", baseURL: "https://fixture.invalid/v1",
                models: [.init(id: "one", modelName: "one", executionCapabilities: .init()),
                         .init(id: "two", modelName: "two", executionCapabilities: .init())])
        ])
        var original = MacConversation(id: "selection", title: "Fixture", preview: "", updatedAt: .now,
            isPinned: false, isPersisted: true, modelSelectionID: "configured:one")
        try repository.ensureConversation(original)
        original = try XCTUnwrap(repository.load(conversationID: original.id)?.conversation)
        let receipt = try await withCheckedThrowingContinuation { continuation in
            repository.enqueueModelConfiguration(conversation: original, selectionID: "configured:two") {
                continuation.resume(with: $0)
            }
        }
        XCTAssertEqual(receipt.selectionID, "configured:two")
        XCTAssertThrowsError(try repository.saveMetadata(conversation: original, knowledge: nil))
        var current = try XCTUnwrap(repository.load(conversationID: original.id)?.conversation)
        XCTAssertEqual(current.modelSelectionID, "configured:two")
        // A -> B -> A is still a stale edit even though the identity matches.
        let stale = current
        _ = try await withCheckedThrowingContinuation { continuation in
            repository.enqueueModelConfiguration(conversation: current, selectionID: "configured:one", host: .remote) {
                continuation.resume(with: $0)
            }
        }
        current = try XCTUnwrap(repository.load(conversationID: original.id)?.conversation)
        XCTAssertEqual(current.executionHostRawValue, "remote")
        _ = try await withCheckedThrowingContinuation { continuation in
            repository.enqueueModelConfiguration(conversation: current, selectionID: "configured:two", host: .local) {
                continuation.resume(with: $0)
            }
        }
        do {
            _ = try await withCheckedThrowingContinuation { continuation in
                repository.enqueueModelConfiguration(conversation: stale, selectionID: "configured:one") {
                    continuation.resume(with: $0)
                }
            }
            XCTFail("ABA must reject the original revision")
        } catch {}
        current = try XCTUnwrap(repository.load(conversationID: original.id)?.conversation)
        XCTAssertEqual(current.executionHostRawValue, "local")
        let baseline = try XCTUnwrap(current.modelOverrides?["configured:two"])
        var cleared = baseline
        cleared.temperature = nil
        _ = try await withCheckedThrowingContinuation { continuation in
            repository.enqueueModelConfiguration(conversation: current, selectionID: current.modelSelectionID,
                overrideEdit: ("configured:two", baseline, cleared)) { continuation.resume(with: $0) }
        }
        let stored = try XCTUnwrap(port.loadConversationMetadata(original.id)?.settingsJSON)
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(stored.utf8)) as? [String: Any])
        let models = try XCTUnwrap(fields["modelOverrides"] as? [String: [String: Any]])
        XCTAssertNil(models["configured:two"]?["temperature"])
        XCTAssertNotNil(models["configured:two"]?["reasoningPreference"])
        current = try XCTUnwrap(repository.load(conversationID: original.id)?.conversation)
        do {
            _ = try await withCheckedThrowingContinuation { continuation in
                repository.enqueueModelConfiguration(conversation: original, selectionID: "configured:one") {
                    continuation.resume(with: $0)
                }
            }
            XCTFail("Stale identity must not overwrite another selection")
        } catch {}
        XCTAssertEqual(current.modelSelectionID, "configured:two")
    }
    func testMetadataPatchWireRejectsStaleRevisionWithoutOverwritingKnowledge() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-metadata-cas-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: application)
        let conversation = MacConversation(id: "metadata-cas", title: "Shared", preview: "",
            updatedAt: .now, isPinned: false, isPersisted: true)
        try repository.ensureConversation(conversation)
        let original = try XCTUnwrap(application.loadConversationMetadata(conversation.id))
        let knowledge = "{\"knowledge\":{\"keyPointsMarkdown\":\"new\",\"wikiMarkdown\":\"keep\"}}"
        XCTAssertThrowsError(try application.patchConversationMetadata(.init(
            conversationID: conversation.id, expectedRevision: original.revision,
            patchJSON: knowledge, updatedAtMs: 978_307_201_500)))
        _ = try application.patchConversationConfiguration(.init(
            conversationID: conversation.id, expectedRevision: XCTUnwrap(original.configurationRevision),
            patchJSON: knowledge, updatedAtMs: 978_307_201_500)).committedSnapshot()
        let stale = try application.patchConversationConfiguration(.init(
            conversationID: conversation.id, expectedRevision: XCTUnwrap(original.configurationRevision),
            patchJSON: "{\"isPinned\":true}", updatedAtMs: 978_307_202_000))
        XCTAssertEqual(stale.status, .conflict)
        let saved = try XCTUnwrap(repository.load(conversationID: conversation.id))
        XCTAssertFalse(saved.conversation.isPinned)
        XCTAssertEqual(saved.knowledge?.keyPointsMarkdown, "new")
        XCTAssertEqual(saved.knowledge?.wikiMarkdown, "keep")
    }

    func testQueuedMetadataOnlySavePreservesNewSharedKnowledgeAndExplicitEmptyClearsIt() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-shared-knowledge-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let queue = DispatchQueue(label: "knowledge-metadata-test")
        let repository = MacCanonicalConversationRepository(store: application, metadataQueue: queue)
        var conversation = MacConversation(id: "shared-knowledge", title: "Shared", preview: "",
            updatedAt: .now, isPinned: false, isPersisted: true)
        conversation.configurationRevision = try repository.ensureConversation(conversation)
        try repository.saveMetadata(conversation: conversation, knowledge: MacConversationKnowledge(
            keyPointsMarkdown: "Old facts", wikiMarkdown: "Old wiki", updatedAt: nil))

        conversation = try XCTUnwrap(repository.load(conversationID: conversation.id)?.conversation)
        queue.suspend()
        var suspended = true
        defer { if suspended { queue.resume() } }
        conversation.isPinned = true
        let finished = expectation(description: "Stale metadata save rejected")
        try repository.enqueueMetadataSave(conversation: conversation, knowledge: nil) { result in
            if case .success = result { XCTFail("Foreign configuration commit must fence stale metadata") }
            finished.fulfill()
        }
        // A different client commits knowledge after the Mac captured its
        // metadata edit, but before the queued write begins.
        var shared = try XCTUnwrap(JSONSerialization.jsonObject(with:
            Data(try XCTUnwrap(application.loadConversationSettings(conversation.id)).utf8)) as? [String: Any])
        shared["knowledge"] = ["keyPointsMarkdown": "  新事实\n", "wikiMarkdown": "# Shared wiki\n"]
        let settings = String(decoding: try JSONSerialization.data(withJSONObject: shared), as: UTF8.self)
        _ = try application.patchConversationConfiguration(.init(conversationID: conversation.id,
            expectedRevision: XCTUnwrap(application.loadConversationMetadata(conversation.id)?.configurationRevision),
            patchJSON: String(decoding: JSONSerialization.data(withJSONObject: ["knowledge": shared["knowledge"]!]), as: UTF8.self),
            updatedAtMs: 978_307_201_500)).committedSnapshot()
        queue.resume()
        suspended = false
        await fulfillment(of: [finished], timeout: 10)

        let saved = try XCTUnwrap(repository.load(conversationID: conversation.id))
        XCTAssertFalse(saved.conversation.isPinned)
        XCTAssertEqual(saved.knowledge?.keyPointsMarkdown, "  新事实\n")
        XCTAssertEqual(saved.knowledge?.wikiMarkdown, "# Shared wiki\n")
        XCTAssertEqual(saved.knowledge?.updatedAt, Date(timeIntervalSinceReferenceDate: 1.5))
        conversation = saved.conversation
        try repository.saveMetadata(conversation: conversation, knowledge: .empty)
        let cleared = try XCTUnwrap(repository.load(conversationID: conversation.id))
        XCTAssertEqual(cleared.knowledge?.keyPointsMarkdown, "")
        XCTAssertEqual(cleared.knowledge?.wikiMarkdown, "")
    }

    func testQueuedMetadataDoesNotBlockMainActorAndPreservesSubmissionOrder() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-metadata-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let queue = DispatchQueue(label: "metadata-test")
        let repository = MacCanonicalConversationRepository(store: application, metadataQueue: queue)
        var conversation = MacConversation(id: "metadata-order", title: "Order", preview: "",
            updatedAt: .now, isPinned: false, isPersisted: true)
        conversation.configurationRevision = try repository.ensureConversation(conversation)
        queue.suspend()
        var suspended = true
        defer { if suspended { queue.resume() } }
        var completions: [Int] = []
        let finished = expectation(description: "Both snapshots committed")
        finished.expectedFulfillmentCount = 2
        try repository.enqueueMetadataSave(conversation: conversation, knowledge: nil) { result in
            if case .failure(let error) = result { XCTFail("\(error)") }
            completions.append(1)
            finished.fulfill()
        }
        conversation.isPinned = true
        try repository.enqueueMetadataSave(conversation: conversation, knowledge: nil) { result in
            if case .success = result { XCTFail("A second stale snapshot must not overtake the committed configuration") }
            completions.append(2)
            finished.fulfill()
        }
        // Storage is deliberately unable to progress. The main actor must
        // still execute another task without either submission waiting for it.
        let responsive = await Task { @MainActor in true }.value
        XCTAssertTrue(responsive)
        XCTAssertTrue(completions.isEmpty)
        queue.resume()
        suspended = false
        await fulfillment(of: [finished], timeout: 10)
        XCTAssertEqual(Set(completions), [1, 2])
        XCTAssertFalse(try XCTUnwrap(repository.load(conversationID: conversation.id)).conversation.isPinned)
        conversation = try XCTUnwrap(repository.load(conversationID: conversation.id)?.conversation)
        try repository.enqueueMetadataSave(conversation: conversation, knowledge: nil) { result in
            if case .failure(let error) = result { XCTFail("\(error)") }
        }
        try repository.delete(conversationID: conversation.id)
        XCTAssertNil(try application.loadConversation(conversation.id),
            "Deletion must run after queued writes, never allow a late save to restore the row")
    }

    func testPlainUserTurnRemainsTextAfterColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-plain-user-turn-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let databaseURL = root.appendingPathComponent("nola.db")
        let conversation = MacConversation(
            id: "plain-user-turn",
            title: "Stable title",
            preview: "",
            updatedAt: .now,
            isPinned: false,
            isPersisted: true
        )
        let user = MacChatMessage(role: .user, text: "背诵出师表", createdAt: .now)
        let runID = "plain-user-run"
        let assistantID = try XCTUnwrap(
            MacCanonicalMessageIdentity.stableID("\(runID):assistant")
        )
        let assistant = MacChatMessage(
            id: assistantID,
            role: .assistant,
            text: "",
            createdAt: .now,
            presentation: .processing()
        )

        do {
            let application = try MacApplicationPort(databaseURL: databaseURL)
            let repository = MacCanonicalConversationRepository(store: application)
            try repository.ensureConversation(conversation)
            try repository.prepareRemoteSubmission(
                conversation: conversation,
                userMessage: user,
                assistantMessage: assistant,
                runID: runID,
                sourceHostID: "fixture-host", clientID: "fixture-client"
            )
            XCTAssertNil(try application.activeRunIdentity(conversationID: conversation.id))
            let source = try MacAssistantJournalTestFixture(conversationID: conversation.id, runID: runID,
                assistantMessageID: "\(runID):assistant", userText: user.text,
                userMessageID: user.id.uuidString, userPartID: "\(user.id.uuidString):body")
            try source.ingestAdmissions(into: application)
            let stored = try XCTUnwrap(application.loadConversation(conversation.id))
            XCTAssertNil(stored.messages.first?.parts.first?.contentJSON)
        }

        let reopened = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: databaseURL)
        )
        let restored = try XCTUnwrap(reopened.load(conversationID: conversation.id))
        let restoredUser = try XCTUnwrap(restored.messages.first { $0.role == .user })
        XCTAssertEqual(restoredUser.text, "背诵出师表")
        XCTAssertTrue(restoredUser.historicalDetails?.isEmpty ?? true)
    }

    func testBoundedHistoryWindowsWalkBackWithoutDuplicatesOrMissingMessages() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-window-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: application)
        let messages = (1...12).map { index in
            MacChatMessage(role: .user, text: "Message \(index)", createdAt: Date(timeIntervalSince1970: Double(index)))
        }
        try repository.restoreBackupSnapshot([MacStoredConversation(
            conversation: MacConversation(
                id: "window-conversation", title: "Window", preview: "",
                updatedAt: .now, isPinned: false, isPersisted: true
            ),
            messages: messages
        )])

        var beforeOrdinal: Int64?
        var restored: [MacChatMessage] = []
        var exhausted = false
        for _ in 0..<5 {
            let window = try application.loadConversationWindow(
                "window-conversation", beforeOrdinal: beforeOrdinal,
                limit: 3, contentByteLimit: 512_000
            )
            XCTAssertEqual(window.totalMessageCount, 12)
            XCTAssertEqual(
                try repository.conversationContextUsage(conversationID: "window-conversation"),
                window.projection.contextUsage
            )
            XCTAssertLessThanOrEqual(window.projection.messages.count, 3)
            let page = try MacCanonicalConversationRepository.projectMessages(window.projection)
            XCTAssertEqual(page.compactMap(\.canonicalMessageID), window.projection.messages.map(\.messageID),
                "The real Core Store producer supplies exact user message IDs to the current presentation join")
            XCTAssertTrue(page.allSatisfy { $0.role == .user && $0.canonicalMessageID != nil })
            XCTAssertEqual(page.compactMap { $0.canonicalPresentation?.message }, window.projection.messages,
                "The actual Core→repository producer preserves typed Part order before any rendering")
            restored.insert(contentsOf: page, at: 0)
            if !window.hasMore {
                exhausted = true
                break
            }
            let next = try XCTUnwrap(window.beforeOrdinal)
            if let beforeOrdinal { XCTAssertLessThan(next, beforeOrdinal) }
            beforeOrdinal = next
        }
        XCTAssertTrue(exhausted)
        XCTAssertEqual(restored.map(\.text), messages.map(\.text))
        XCTAssertEqual(Set(restored.map(\.id)).count, 12)
    }

    func testConversationMetadataStoresPreferencesWithoutDuplicatingCanonicalFacts() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-preferences-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let databaseURL = directory.appendingPathComponent("nola-product.db")
        let application = try MacApplicationPort(databaseURL: databaseURL)
        let repository = MacCanonicalConversationRepository(store: application)
        try MacAppleChatConfigurationStore(applicationPort: application).replaceEndpoints([
            MacRemoteEndpointConfiguration(id: "preferences-endpoint", name: "Fixture",
                baseURL: "https://fixture.invalid/v1", models: [
                    MacRemoteModelConfiguration(id: "model-profile-1", modelName: "fixture", executionCapabilities: .init())
                ])
        ])
        var conversation = MacConversation(
            id: "preference-conversation",
            title: "Canonical title",
            preview: "must not be stored",
            updatedAt: Date(timeIntervalSince1970: 123),
            isPinned: true,
            modelSelectionID: "configured:model-profile-1",
            responseStyleRawValue: "concise",
            memoryEnabledOverride: true,
            executionBackendRawValue: "codex",
            browserSnapshot: MacConversationBrowserSnapshot(
                target: "managed",
                sessionID: "nola-preference-conversation",
                pages: [MacConversationBrowserPage(
                    index: 0,
                    title: "Nola Docs",
                    url: "https://example.com/docs",
                    isActive: true
                )],
                currentTitle: "Nola Docs",
                currentURL: "https://example.com/docs",
                isConnected: true,
                updatedAt: Date(timeIntervalSince1970: 120),
                previewURL: "http://127.0.0.1:12345/view?session_id=nola-preference-conversation&token=test"
            )
        )
        conversation.configurationRevision = try repository.ensureConversation(conversation)
        conversation.preview = "still presentation only"
        try repository.saveMetadata(conversation: conversation, knowledge: .empty)

        let raw = try XCTUnwrap(application.loadConversation(conversation.id)?.settingsJSON)
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any]
        )
        XCTAssertEqual(object["version"] as? Int, 2)
        XCTAssertNil(object["conversation"])
        XCTAssertNil(object["id"])
        XCTAssertNil(object["title"])
        XCTAssertNil(object["preview"])
        XCTAssertNil(object["updatedAt"])
        XCTAssertNil(object["contextSnapshot"])

        let coldRepository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: databaseURL)
        )
        let restored = try XCTUnwrap(coldRepository.load(conversationID: conversation.id))
        XCTAssertEqual(restored.conversation.id, conversation.id)
        XCTAssertEqual(restored.conversation.title, "Canonical title")
        XCTAssertTrue(restored.conversation.isPinned)
        XCTAssertEqual(restored.conversation.modelSelectionID, "configured:model-profile-1")
        XCTAssertEqual(restored.conversation.responseStyleRawValue, "concise")
        XCTAssertEqual(restored.conversation.executionBackendRawValue, "codex")
        XCTAssertNil(restored.conversation.browserSnapshot, "Device browser presentation is not canonical conversation configuration")
        XCTAssertEqual(
            restored.contextUsage,
            try coldRepository.conversationContextUsage(conversationID: conversation.id)
        )
    }

    func testMalformedPermissionMetadataFailsWithoutChangingStoredPreferences() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-permission-migration-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let application = try MacApplicationPort(databaseURL: directory.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: application)
        try MacAppleChatConfigurationStore(applicationPort: application).replaceEndpoints([
            MacRemoteEndpointConfiguration(id: "permission-endpoint", name: "Fixture",
                baseURL: "https://fixture.invalid/v1", models: [
                    MacRemoteModelConfiguration(id: "keep-model", modelName: "fixture", executionCapabilities: .init())
                ])
        ])
        var conversation = MacConversation(
            id: "permission-migration",
            title: "Permission migration",
            preview: "",
            updatedAt: .now,
            isPinned: true,
            modelSelectionID: "configured:keep-model",
            responseStyleRawValue: "professional",
            permissionModeRawValue: MacPermissionMode.cautious.rawValue
        )
        conversation.configurationRevision = try repository.ensureConversation(conversation)
        try repository.saveMetadata(conversation: conversation, knowledge: .empty)
        let stored = try XCTUnwrap(application.loadConversation(conversation.id)?.settingsJSON)
        var object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(stored.utf8)) as? [String: Any]
        )

        let invalidProfiles: [Any] = [true, "allow-all", "moderate", "", "unknown"]
        for value in invalidProfiles {
            let before = try XCTUnwrap(application.loadConversationMetadata(conversation.id))
            let data = try JSONSerialization.data(withJSONObject: ["permissionProfile": value])
            XCTAssertThrowsError(try application.patchConversationConfiguration(.init(
                conversationID: conversation.id,
                expectedRevision: XCTUnwrap(before.configurationRevision),
                patchJSON: String(decoding: data, as: UTF8.self),
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
            )).committedSnapshot())
            XCTAssertEqual(try application.loadConversation(conversation.id)?.settingsJSON, before.settingsJSON)
        }
    }

    func testCanonicalUUIDIdentitySurvivesOptimisticToDurableProjection() throws {
        let submittedID = UUID()

        XCTAssertEqual(
            MacCanonicalMessageIdentity.stableID(submittedID.uuidString),
            submittedID
        )
        XCTAssertEqual(
            MacCanonicalMessageIdentity.stableID("remote-assistant:answer"),
            MacCanonicalMessageIdentity.stableID("remote-assistant:answer")
        )
        XCTAssertNotEqual(
            MacCanonicalMessageIdentity.stableID("remote-assistant:answer"),
            MacCanonicalMessageIdentity.stableID("remote-assistant:other")
        )
    }

    func testCompletedTextOnlyAssistantUsesDirectMarkdownAfterReload() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-text-presentation-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        let conversationID = "text-presentation-conversation"
        let assistantID = "text-presentation-assistant"
        let markdown = "## Result\n\nFirst sentence. Second sentence."
        let timestamp: Int64 = 1_700_000_000_000
        let store = try MacApplicationPort(databaseURL: directory.appendingPathComponent("nola.db"))
        XCTAssertTrue(try store.createConversation(
            MacCanonicalCreateConversationInput(
                id: conversationID,
                title: "Text presentation",
                settingsJSON: nil,
                createdAtMs: timestamp,
                updatedAtMs: timestamp + 1
            )
        ).created)
        XCTAssertEqual(try store.importConversationTranscript(
            MacCanonicalImportConversationTranscriptInput(
                importID: "text-presentation-import",
                source: "external_archive",
                conversationID: conversationID,
                expectedMessageCount: 0,
                messages: [
                    MacCanonicalMessageInput(
                        id: assistantID,
                        runID: "text-presentation-run",
                        role: "assistant",
                        status: "completed",
                        ordinal: 1,
                        createdAtMs: timestamp,
                        completedAtMs: timestamp + 1,
                        parts: [
                            MacCanonicalMessagePartInput(
                                id: "text-presentation-part",
                                kind: "text",
                                ordinal: 1,
                                state: "completed",
                                contentText: markdown,
                                contentJSON: nil,
                                contentDigest: nil,
                                createdAtMs: timestamp,
                                updatedAtMs: timestamp + 1
                            ),
                        ]
                    ),
                ],
                importedAtMs: timestamp + 1
            )
        ).messageCount, 1)

        let repository = MacCanonicalConversationRepository(store: store)
        let message = try XCTUnwrap(repository.load(conversationID: conversationID)?.messages.first)
        XCTAssertNil(message.presentation)
        XCTAssertEqual(message.text, markdown)
    }

    func testAssistantWithNoVisiblePartsIsNotProjectedAfterReload() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-empty-failure-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let timestamp: Int64 = 1_700_000_000_000
        let conversationID = "empty-failure-conversation"
        let store = try MacApplicationPort(databaseURL: directory.appendingPathComponent("nola.db"))
        _ = try store.createConversation(MacCanonicalCreateConversationInput(
            id: conversationID,
            title: "Empty failure",
            settingsJSON: nil,
            createdAtMs: timestamp,
            updatedAtMs: timestamp
        ))
        _ = try store.importConversationTranscript(MacCanonicalImportConversationTranscriptInput(
            importID: "empty-failure-import",
            source: "external_archive",
            conversationID: conversationID,
            expectedMessageCount: 0,
            messages: [MacCanonicalMessageInput(
                id: "empty-assistant",
                runID: "failed-run",
                role: "assistant",
                status: "completed",
                ordinal: 1,
                createdAtMs: timestamp,
                completedAtMs: timestamp,
                parts: [MacCanonicalMessagePartInput(
                    id: "empty-text",
                    kind: "text",
                    ordinal: 1,
                    state: "completed",
                    contentText: "",
                    contentJSON: nil,
                    contentDigest: nil,
                    createdAtMs: timestamp,
                    updatedAtMs: timestamp
                )]
            )],
            importedAtMs: timestamp
        ))

        let repository = MacCanonicalConversationRepository(store: store)
        XCTAssertEqual(try repository.load(conversationID: conversationID)?.messages, [])
    }

    func testTerminalJournalCommitsAfterRepositoryRestart() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-remote-restart-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let databaseURL = directory.appendingPathComponent("nola-product.db")
        let conversationID = "contract-conversation-1"
        let runID = "MixedCase-Remote-Run"
        let assistantID = "contract-assistant-1"
        let source: MacAssistantJournalTestFixture
        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            _ = try store.prepareFixtureRemoteSubmission(
                MacPrepareRemoteSubmissionInput(
                    conversationID: conversationID,
                    conversationTitle: "Host journal contract",
                    submissionID: runID,
                    sourceHostID: "contract-host-1", clientID: "fixture-client",
                    userMessageID: "contract-user-1",
                    userPartID: "contract-user-1:text",
                    userText: "Explain the contract.",
                    userContentJSON: nil,
                    assistantMessageID: assistantID,
                    assistantPartID: "contract-assistant-1:text",
                    contextCheckpointID: nil,
                    createdAtMs: 1_787_443_200_000
                )
            )
            source = try MacAssistantJournalTestFixture.forReceiver(receiver: store, conversationID: conversationID,
                runID: runID, assistantMessageID: assistantID, sourceHostID: "contract-host-1")
            _ = try source.finish("The journal is the durable UI source.")
            XCTAssertEqual(try store.activeRunIdentity(conversationID: conversationID)?.runID, runID)
        }

        // Reopen before receiving any source evidence. Mixed-case Run ownership
        // and the pending user/assistant binding must survive the cold reopen.
        let restartedStore = try MacApplicationPort(databaseURL: databaseURL)
        let restartedRepository = MacCanonicalConversationRepository(store: restartedStore)
        XCTAssertEqual(try restartedStore.activeRunIdentity(conversationID: conversationID)?.runID, runID)
        let observedWindow = restartedRepository.makeWindowModel()
        try await observedWindow.select(conversationID: conversationID)
        XCTAssertEqual(observedWindow.messages.map(\.role), [.user, .assistant])
        let seal = try source.commitTerminal(into: restartedStore)
        let changed = try await observedWindow.refreshIfChanged()
        XCTAssertTrue(changed)
        XCTAssertNotNil(observedWindow.messages.last?.journalReference)
        XCTAssertEqual(seal.identity.run_id, runID)
        XCTAssertNil(try restartedStore.activeRunIdentity(conversationID: conversationID))
        let restored = try XCTUnwrap(restartedRepository.load(conversationID: conversationID))
        let reference = try XCTUnwrap(restored.messages.last?.journalReference)
        let body = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
        XCTAssertEqual(body, "The journal is the durable UI source.")
        XCTAssertEqual(restored.messages.filter { $0.role == .assistant }.count, 1)
        let asynchronous = try await restartedRepository.loadAsync(conversationID: conversationID)
        XCTAssertEqual(asynchronous?.messages.last?.journalReference, reference)
        XCTAssertEqual(try restartedRepository.loadAll().first?.messages.last?.journalReference, reference)
        let window = restartedRepository.makeWindowModel()
        try await window.select(conversationID: conversationID)
        XCTAssertEqual(window.messages.last?.journalReference, reference)
        // Exact duplicate source recovery is idempotent after the durable commit.
        try source.commitTerminal(into: restartedStore)
        XCTAssertEqual(try restartedRepository.load(conversationID: conversationID)?.messages, restored.messages)
    }

    func testCanonicalCorePartsRestoreStructuredPresentationWithoutPollutingAnswerText() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-core-parts-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let databaseURL = root.appendingPathComponent("nola.db")
        let conversationID = "conversation-core-parts"
        let assistantID = "assistant-core-parts"
        let timestamp: Int64 = 1_700_000_000_000
        let store = try MacApplicationPort(databaseURL: databaseURL)
        let result = try store.createConversation(
            MacCanonicalCreateConversationInput(
                id: conversationID,
                title: "Canonical parts",
                settingsJSON: nil,
                createdAtMs: timestamp,
                updatedAtMs: timestamp + 3
            )
        )
        XCTAssertTrue(result.created)
        let appended = try store.importConversationTranscript(
            MacCanonicalImportConversationTranscriptInput(
                importID: "external-archive:canonical-parts",
                source: "external_archive",
                conversationID: conversationID,
                expectedMessageCount: 0,
                messages: [
                    MacCanonicalMessageInput(
                        id: assistantID,
                        runID: "turn-core-parts",
                        role: "assistant",
                        status: "completed",
                        ordinal: 1,
                        createdAtMs: timestamp,
                        completedAtMs: timestamp + 3,
                        parts: [
                            MacCanonicalMessagePartInput(
                                id: "reason-1",
                                kind: "reasoning",
                                ordinal: 1,
                                state: "completed",
                                contentText: "Inspecting",
                                contentJSON: #"{"kind":"reasoning","id":"reason-1","message_id":"assistant-core-parts","turn_id":"turn-core-parts","title":"Thinking","detail":"Inspecting input","is_complete":true}"#,
                                contentDigest: nil,
                                createdAtMs: timestamp,
                                updatedAtMs: timestamp + 1
                            ),
                            MacCanonicalMessagePartInput(
                                id: "tool-1",
                                kind: "tool",
                                ordinal: 2,
                                state: "completed",
                                contentText: "Read 20 lines",
                                contentJSON: #"{"kind":"tool","execution_source":{"kind":"model"},"id":"tool-1","message_id":"assistant-core-parts","turn_id":"turn-core-parts","call_id":"call-1","capability_id":"bash","title":"Create document","state":"completed","input":{"operation_kind":"mutate_and_validate"},"raw_input":"","output_preview":"Validated document","full_output_ref":null,"artifacts":[{"uri":"file:///tmp/Nola%20Report.docx","name":"Nola Report.docx","mime_type":"application/vnd.openxmlformats-officedocument.wordprocessingml.document","operation":"create"}],"started_at_ms":1,"finished_at_ms":2}"#,
                                contentDigest: nil,
                                createdAtMs: timestamp + 1,
                                updatedAtMs: timestamp + 2
                            ),
                            MacCanonicalMessagePartInput(
                                id: "tool-2",
                                kind: "tool",
                                ordinal: 3,
                                state: "completed",
                                contentText: "Use the structured file tool",
                                contentJSON: #"{"kind":"tool","execution_source":{"kind":"model"},"id":"tool-2","message_id":"assistant-core-parts","turn_id":"turn-core-parts","call_id":"call-2","capability_id":"bash","title":"Inspect","state":"failed","input":{"operation_kind":"inspect"},"raw_input":"","output_preview":"tool_execution_skipped: true\nnext_action: call_file_tree_with_equivalent_structured_arguments","full_output_ref":null,"artifacts":[],"started_at_ms":2,"finished_at_ms":3}"#,
                                contentDigest: nil,
                                createdAtMs: timestamp + 2,
                                updatedAtMs: timestamp + 3
                            ),
                            MacCanonicalMessagePartInput(
                                id: "text-1",
                                kind: "text",
                                ordinal: 4,
                                state: "completed",
                                contentText: "## Result\n\nCanonical answer.",
                                contentJSON: ###"{"kind":"text","id":"text-1","message_id":"assistant-core-parts","turn_id":"turn-core-parts","text":"## Result\n\nCanonical answer.","is_complete":true}"###,
                                contentDigest: nil,
                                createdAtMs: timestamp + 2,
                                updatedAtMs: timestamp + 3
                            ),
                        ]
                    ),
                ],
                importedAtMs: timestamp + 3
            )
        )
        XCTAssertEqual(appended.messageCount, 1)
        let committedContext = try store.commitContextCheckpoint(
            MacCanonicalContextCommit(
                checkpoint: MacCanonicalContextCheckpoint(
                id: "snapshot-core-parts",
                conversationID: conversationID,
                runID: "turn-core-parts",
                modelProfileID: "test-model",
                policyVersion: "v1",
                coveredThroughMessageID: assistantID,
                coveredThroughSequence: 1,
                summaryPartID: "snapshot-core-parts:summary",
                retainedTailStartMessageID: nil,
                tokenUsage: MacCanonicalContextTokenUsage(
                    estimated: 8,
                    measured: 7,
                    window: 65_536,
                    promptLimit: 57_344
                ),
                payloadDigest: "digest-core-parts",
                createdAtMs: timestamp + 4
                ),
                components: [
                    MacCanonicalContextComponent(
                        id: "snapshot-core-parts:summary",
                        checkpointID: "snapshot-core-parts",
                        kind: "conversation_summary",
                        sourceID: nil,
                        sourceURI: nil,
                        title: nil,
                        mimeType: "text/plain",
                        authorization: "not_required",
                        observedAtMs: timestamp + 4,
                        maximumAgeMinutes: nil,
                        freshness: "fresh",
                        contentReference: MacCanonicalContextContentReference(
                            type: "inline_preview",
                            text: "Canonical summary",
                            truncated: false,
                            artifactID: nil,
                            blobID: nil
                        ),
                        contentDigest: "digest-core-parts",
                        estimatedTokens: 8,
                        provenance: MacCanonicalContextProvenance(
                            providerID: "test",
                            collectionMethod: "compaction",
                            sourceRevision: nil,
                            disclosedToModel: true
                        )
                    )
                ]
            )
        )
        XCTAssertEqual(committedContext.checkpoint.id, "snapshot-core-parts")
        _ = try store.patchConversationConfiguration(
            MacCanonicalPatchConversationMetadataInput(
                conversationID: conversationID,
                expectedRevision: XCTUnwrap(store.loadConversationMetadata(conversationID)?.configurationRevision),
                patchJSON: #"{"memoryEnabledOverride":true}"#,
                updatedAtMs: timestamp + 5
            )
        ).committedSnapshot()

        let reopenedStore = try MacApplicationPort(databaseURL: databaseURL)
        XCTAssertEqual(try reopenedStore.countContextCheckpoints(conversationID), 1)
        let snapshot = try XCTUnwrap(reopenedStore.loadContextCheckpoint("snapshot-core-parts"))
        XCTAssertEqual(snapshot.checkpoint.id, "snapshot-core-parts")
        XCTAssertEqual(snapshot.components.first?.contentReference.text, "Canonical summary")
        let settingsObject = try JSONSerialization.jsonObject(with: Data(XCTUnwrap(reopenedStore.loadConversation(conversationID)?.settingsJSON).utf8)) as? [String: Any]
        XCTAssertEqual(settingsObject?["memoryEnabledOverride"] as? Bool, true)
        let repository = MacCanonicalConversationRepository(
            store: reopenedStore,
        )
        let message = try XCTUnwrap(repository.load(conversationID: conversationID)?.messages.first)

        XCTAssertEqual(message.id, MacCanonicalMessageIdentity.stableID(assistantID))
        XCTAssertEqual(message.text, "## Result\n\nCanonical answer.")
        let presentation = try XCTUnwrap(message.presentation)
        let reasoning = try XCTUnwrap(presentation.traceItems.first { $0.externalID == "reason-1" })
        XCTAssertEqual(reasoning.detail, "Inspecting input")
        XCTAssertEqual(reasoning.state, .completed)
        let tool = try XCTUnwrap(presentation.traceItems.first { $0.externalID == "tool-1" })
        XCTAssertEqual(tool.result, "Validated document")
        XCTAssertEqual(tool.state, .completed)
        XCTAssertEqual(tool.affectedFiles, ["/tmp/Nola Report.docx"])
        let correctedTool = try XCTUnwrap(
            presentation.traceItems.first { $0.externalID == "tool-2" }
        )
        XCTAssertEqual(correctedTool.state, .completed)
    }


    func testConversationPermissionGrantSurvivesStoreReopen() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-permission-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let databaseURL = directory.appendingPathComponent("nola-product.db")
        let conversationID = "permission-conversation"
        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            _ = try store.prepareFixtureRemoteSubmission(MacPrepareRemoteSubmissionInput(
                conversationID: conversationID,
                conversationTitle: "Permission",
                submissionID: "permission-run",
                sourceHostID: "fixture-host", clientID: "fixture-client",
                userMessageID: "permission-user",
                userPartID: "permission-user:body",
                userText: "Edit the file",
                userContentJSON: nil,
                assistantMessageID: "permission-run:assistant",
            assistantPartID: "permission-run:assistant:text",
                contextCheckpointID: nil,
                createdAtMs: 1
            ))
            let grantInput = MacCanonicalPermissionGrantInput(
                id: "permission-conversation-write",
                target: .conversation(id: conversationID),
                capabilityPattern: "write_file",
                resourcePattern: "*",
                decision: .allow,
                createdAtMs: 2,
                updatedAtMs: 2
            )
            let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(grantInput)) as? [String: Any]
            let target = try XCTUnwrap(encoded?["target"] as? [String: Any])
            XCTAssertEqual(target["type"] as? String, "conversation")
            XCTAssertEqual(target["conversation_id"] as? String, conversationID)
            XCTAssertNil(encoded?["scope"])
            XCTAssertNil(encoded?["conversation_id"])
            XCTAssertNil(encoded?["host_id"])
            _ = try store.upsertPermissionGrant(grantInput)
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let rules = try reopened.effectivePermissionRules(
            conversationID: conversationID,
            hostID: "mac-local"
        )
        XCTAssertEqual(rules.rules.count, 1)
        XCTAssertEqual(rules.rules[0].capabilityPattern, "write_file")
        XCTAssertEqual(rules.rules[0].source, "conversation")
        XCTAssertEqual(rules.rules[0].decision, "allow")
        XCTAssertTrue(try reopened.revokePermissionGrant(
            "permission-conversation-write",
            revokedAtMs: 3
        ))
        XCTAssertTrue(try reopened.effectivePermissionRules(
            conversationID: conversationID,
            hostID: "mac-local"
        ).rules.isEmpty)
    }

    func testCanonicalSkillAvailabilitySurvivesStoreReopen() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-skill-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let databaseURL = directory.appendingPathComponent("nola-product.db")
        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            let stored = try XCTUnwrap(store.synchronizeBuiltInSkills([MacCanonicalSkillInput(
                id: "word",
                source: .builtIn,
                version: "1",
                displayName: "Word",
                summary: "Create a document",
                category: "documents",
                instructionsRef: "nola-skill://word/1",
                instructionsDigest: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
                instructionsText: "hello",
                requiredTools: [],
                defaultMode: .inline,
                enabled: true,
                createdAtMs: 1,
                updatedAtMs: 1
            )]).first)
            XCTAssertEqual(stored.record.id, "word")
            let expected = try XCTUnwrap(store.skillEditor("word"))
            XCTAssertFalse(try store.setSkillEnabled(expected, enabled: false, updatedAtMs: 2).record.enabled)
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let skills = try reopened.listSkills()
        XCTAssertEqual(skills.map(\.id), ["word"])
        XCTAssertFalse(try XCTUnwrap(skills.first).enabled)
        let editor = try XCTUnwrap(reopened.skillEditor("word"))
        XCTAssertThrowsError(try reopened.deleteUserSkill(editor, deletedAtMs: 3))
    }

    func testBuiltInCatalogSyncPreservesActivationAndRollsBackInvalidBatch() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-catalog-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let databaseURL = directory.appendingPathComponent("nola.db")
        func package(_ id: String, version: String, valid: Bool = true) -> MacCanonicalSkillInput {
            MacCanonicalSkillInput(
                id: id, source: .builtIn, version: version,
                displayName: id, summary: "Catalog transaction fixture", category: "documents",
                instructionsRef: "nola-skill://\(id)/\(version)",
                instructionsDigest: valid
                    ? "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                    : "invalid",
                instructionsText: "hello", requiredTools: [], defaultMode: .inline,
                enabled: true, createdAtMs: 1, updatedAtMs: 3
            )
        }
        do {
            let application = try MacApplicationPort(databaseURL: databaseURL)
            let installed = try XCTUnwrap(application.synchronizeBuiltInSkills([
                package("word", version: "1")
            ]).first)
            let preparedUpgrade = package("word", version: "2")
            _ = try application.setSkillEnabled(installed, enabled: false, updatedAtMs: 4)
            let upgraded = try XCTUnwrap(application.synchronizeBuiltInSkills([preparedUpgrade]).first)
            XCTAssertEqual(upgraded.record.version, "2")
            XCTAssertFalse(upgraded.record.enabled)
            XCTAssertEqual(upgraded.record.updatedAtMs, 4)

            XCTAssertThrowsError(try application.synchronizeBuiltInSkills([
                package("word", version: "3"),
                package("excel", version: "1", valid: false)
            ]))
            let afterFailure = try application.skillEditors()
            XCTAssertEqual(afterFailure.count, 1)
            XCTAssertEqual(afterFailure.first?.revision, upgraded.revision)
            XCTAssertEqual(afterFailure.first?.record.version, "2")
            XCTAssertNil(try application.skillEditor("excel"))
            let repeatedRead = try application.skillEditors()
            XCTAssertEqual(repeatedRead.map(\.revision), afterFailure.map(\.revision))
        }
        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let restored = try XCTUnwrap(reopened.skillEditors().first)
        XCTAssertEqual(restored.record.id, "word")
        XCTAssertEqual(restored.record.version, "2")
        XCTAssertFalse(restored.record.enabled)
        XCTAssertNil(try reopened.skillEditor("excel"))
    }
}
