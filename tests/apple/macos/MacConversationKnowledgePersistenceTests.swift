import Foundation
import XCTest
@testable import Nola

nonisolated private func unexpectedPersistenceHostCall(_ operation: String) -> NSError {
    XCTFail("Persistence-only test unexpectedly invoked Host operation: \(operation)")
    return NSError(domain: "Nola.PersistenceTest.Host", code: 1)
}

actor PersistenceTestLocalRuntime: MacLocalAgentRuntime {
    private var permissionRequest: MacRemotePermissionRequest?
    func setPermissionRequest(_ request: MacRemotePermissionRequest?) {
        permissionRequest = request
    }
    func activePermissionRequest(conversationID: String, requestID: String) -> MacRemotePermissionRequest? {
        guard permissionRequest?.sessionID == conversationID,
              permissionRequest?.id == requestID else { return nil }
        return permissionRequest
    }
    func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    try request.submissionTicket?.claimDispatch()
                } catch {
                    continuation.finish(throwing: error)
                    return
                }
                request.submissionTicket?.markAccepted()
                await request.submissionAccepted?()
                continuation.finish(throwing: unexpectedPersistenceHostCall("local.run"))
            }
        }
    }
    func cancel(conversationID: String) {}
    func resolvePermission(conversationID: String, request: MacRemotePermissionRequest,
                           allow: Bool, scope: MacPermissionGrantScope) throws {
        throw unexpectedPersistenceHostCall("local.resolvePermission")
    }
    func listSessions() async throws -> [MacLocalAgentSessionSummary] {
        throw unexpectedPersistenceHostCall("local.listSessions")
    }
    func deleteSession(id: String) async throws {
        throw unexpectedPersistenceHostCall("local.deleteSession")
    }
}

@MainActor
final class PersistenceTestRemoteHost: MacRemoteAgentRunning {
    func permissionConfiguration(identity: MacRemoteSubmissionIdentity, target: MacRemoteSubmissionTarget,
                                 expectedRevision: Int64?, profile: MacPermissionMode?, minimumPolicyRevision: Int64?) async throws -> MacRemotePermissionConfiguration {
        throw unexpectedPersistenceHostCall("remote.permissionConfiguration")
    }
    nonisolated func captureSubmissionTarget() throws -> MacRemoteSubmissionTarget {
        throw unexpectedPersistenceHostCall("remote.captureSubmissionTarget")
    }
    func isConfigured() async -> Bool { false }
    func testSavedConnection() async throws -> MacRemoteHostConnectionReport {
        throw unexpectedPersistenceHostCall("remote.testConnection")
    }
    func requestWebManagementURL(target: String) async throws -> URL {
        throw unexpectedPersistenceHostCall("remote.managementURL")
    }
    func fetchExtensions() async throws -> [MacExtensionItem] {
        throw unexpectedPersistenceHostCall("remote.extensions")
    }
    func fetchExecutionBackends() async throws -> [MacRemoteExecutionBackend] {
        throw unexpectedPersistenceHostCall("remote.backends")
    }
    func fetchRemoteModelCatalog() async throws -> MacRemoteModelCatalogSnapshot {
        throw unexpectedPersistenceHostCall("remote.models")
    }
    func run(conversationID: String, submissionID: String, userMessageID: String,
             assistantMessageID: String, prompt: String, contextComponents: [MacCoreContextComponent],
             modelID: String, modelProviderID: String?, modelIntent: MacRemoteModelIntent?, memoryEnabled: Bool, executionBackend: String,
             externalSessionID: String?, submission: MacRemoteSubmissionControl) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: unexpectedPersistenceHostCall("remote.run")) }
    }
    func resolvePermission(request: MacRemotePermissionRequest, allow: Bool,
                           scope: MacPermissionGrantScope) async throws {
        throw unexpectedPersistenceHostCall("remote.resolvePermission")
    }
    func saveConversationSelection(conversationID: String, title: String, profile: MacRemoteModelProfile?,
        allowCreate: Bool, target: MacRemoteSubmissionTarget) async throws {}
    func sendTaskGuidance(conversationID: String, parentRunID: String, taskID: String,
                          message: String, target: MacRemoteSubmissionTarget) async throws -> AppleTaskGuidanceReceipt {
        throw unexpectedPersistenceHostCall("remote.taskGuidance")
    }
    func requestTaskCancellation(conversationID: String, parentRunID: String,
                                 taskID: String, target: MacRemoteSubmissionTarget) async throws -> AppleTaskCancellationReceipt {
        throw unexpectedPersistenceHostCall("remote.cancelTask")
    }
    func cancelRun(identity: MacRemoteSubmissionIdentity, requestDigest: String) async throws {}
    func taskEvents(conversationID: String,
                    afterCursor: Int64) async -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: unexpectedPersistenceHostCall("remote.taskEvents")) }
    }
}

final class MacConversationKnowledgePersistenceTests: XCTestCase {
    @MainActor
    func testRecallFailureIsNotAnEmptyResultAndClearsForNextQuery() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("recall-error-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try repository.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(id: "recall-error", title: "Recall", preview: "", updatedAt: .now, isPinned: false),
                messages: [MacChatMessage(role: .user, text: "Original evidence", createdAt: .now)]
            )
        ])
        let model = MacWorkspaceModel(
            initialConversationID: "recall-error", canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = "recall-error"
        model.conversationRecallQuery = String(repeating: "x", count: 501)
        await waitForRecallSearch(model)
        XCTAssertTrue(model.conversationRecallFailed)
        XCTAssertTrue(model.conversationRecallResults.isEmpty)
        model.retryConversationRecallSearch()
        XCTAssertFalse(model.conversationRecallFailed)
        await waitForRecallSearch(model)
        XCTAssertTrue(model.conversationRecallFailed)
        model.conversationRecallQuery = "Original evidence"
        await waitForRecallSearch(model)
        XCTAssertFalse(model.conversationRecallFailed)
        XCTAssertEqual(model.conversationRecallResults.count, 1)
    }

    @MainActor
    func testConversationKnowledgeEndToEndSurvivesSwitchReloadRestartAndRepeatedCompaction() async throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-conversation-knowledge-e2e-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        let applicationPort = try MacApplicationPort(
            databaseURL: baseDirectory.appendingPathComponent("nola.sqlite")
        )
        let store = MacCanonicalConversationRepository(store: applicationPort)
        let firstConversationID = "knowledge-first-\(UUID().uuidString)"
        try store.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: firstConversationID,
                    title: "Atlas",
                    preview: "",
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [
                    MacChatMessage(
                        role: .user,
                        text: "The original Atlas decision requires blue-green deployment in Shanghai.",
                        createdAt: Date(timeIntervalSince1970: 100)
                    )
                ]
            )
        ])
        let model = MacWorkspaceModel(
            initialConversationID: firstConversationID,
            canonicalRepository: store,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = firstConversationID
        model.selectedKnowledgePointsText = "- Atlas uses blue-green deployment"
        model.selectedWikiText = "# Atlas\n\nPrimary region: Shanghai"
        XCTAssertTrue(model.selectedKnowledgeIsSaving)

        model.newConversation()
        let secondConversationID = try XCTUnwrap(model.selectedConversationID)
        model.selectedKnowledgePointsText = "- Borealis is private"
        model.selectedWikiText = "# Borealis\n\nVisibility: private"

        for index in 1...3 {
            try commitCanonicalTestContextCheckpoint(
                store: applicationPort,
                id: "archive-\(index)",
                conversationID: firstConversationID,
                summary: index == 3
                    ? "The retained rollback owner is Mina."
                    : "Earlier compacted continuation \(index).",
                estimatedTokens: 16,
                payloadDigest: String(repeating: "a", count: 64),
                createdAtMs: Int64(200 + index) * 1_000
            )
        }

        // Selection only exposes the second owner's pending state. Flush and
        // await each original owner's exact metadata/configuration receipt.
        try await model.waitForModelConfigurationCommit(conversationID: firstConversationID)
        try await model.waitForModelConfigurationCommit(conversationID: secondConversationID)
        XCTAssertFalse(model.selectedKnowledgeIsSaving)
        XCTAssertFalse(model.selectedKnowledgeSaveFailed)
        for (id, markdown) in [(firstConversationID, "- Atlas uses blue-green deployment"),
                               (secondConversationID, "- Borealis is private")] {
            let persisted = try XCTUnwrap(applicationPort.loadConversation(id))
            let settings = try XCTUnwrap(persisted.settingsJSON)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(settings.utf8)) as? [String: Any])
            let knowledge = try XCTUnwrap(object["knowledge"] as? [String: Any])
            XCTAssertEqual(Set(knowledge.keys), Set(["keyPointsMarkdown", "wikiMarkdown", "updatedAt"]),
                "Writes own only Markdown; Core derives the committed knowledge timestamp")
            XCTAssertNotNil(knowledge["updatedAt"] as? NSNumber)
            XCTAssertEqual(knowledge["keyPointsMarkdown"] as? String, markdown)
        }

        model.reloadPersistentData()
        model.selectedConversationID = firstConversationID
        XCTAssertEqual(model.selectedKnowledgePointsText, "- Atlas uses blue-green deployment")
        XCTAssertTrue(model.selectedWikiText.contains("Shanghai"))

        let restarted = MacWorkspaceModel(
            initialConversationID: secondConversationID,
            canonicalRepository: store,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        restarted.selectedConversationID = secondConversationID
        await restarted.loadSelectedConversationIfNeeded()
        XCTAssertEqual(restarted.selectedKnowledgePointsText, "- Borealis is private")
        XCTAssertTrue(restarted.selectedWikiText.contains("Visibility: private"))

        restarted.selectedConversationID = firstConversationID
        await restarted.loadSelectedConversationIfNeeded()
        XCTAssertEqual(restarted.selectedKnowledgePointsText, "- Atlas uses blue-green deployment")
        XCTAssertTrue(restarted.selectedWikiText.contains("Primary region: Shanghai"))
        await restarted.refreshContextArchiveCount()
        XCTAssertEqual(restarted.selectedContextArchiveCount, 3)
        XCTAssertEqual(
            restarted.conversationRecallResults,
            []
        )

        restarted.conversationRecallQuery = "blue-green"
        await waitForRecallSearch(restarted)
        XCTAssertEqual(restarted.conversationRecallResults.first?.source, .originalMessage)
        restarted.conversationRecallQuery = "rollback owner Mina"
        await waitForRecallSearch(restarted)
        XCTAssertTrue(restarted.conversationRecallResults.isEmpty)

        // Switching while a debounced request is pending must not publish
        // another conversation's originals or checkpoint summaries.
        restarted.conversationRecallQuery = "blue-green"
        restarted.selectedConversationID = secondConversationID
        await waitForRecallSearch(restarted)
        XCTAssertTrue(restarted.conversationRecallResults.isEmpty)
    }

    @MainActor
    private func waitForRecallSearch(
        _ model: MacWorkspaceModel,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async {
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while model.isConversationRecallSearching, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertFalse(model.isConversationRecallSearching, "Recall search timed out", file: file, line: line)
    }

    @MainActor
    func testKnowledgeWikiAndOriginalRecallSurviveRepeatedCompactionArchives() async throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-conversation-knowledge-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        let applicationPort = try MacApplicationPort(
            databaseURL: baseDirectory.appendingPathComponent("nola.sqlite")
        )
        let store = MacCanonicalConversationRepository(store: applicationPort)
        let conversationID = "conversation-\(UUID().uuidString)"
        let conversation = MacConversation(
            id: conversationID,
            title: "Architecture review",
            preview: "",
            updatedAt: .now,
            isPinned: false
        )
        let originalMessage = MacChatMessage(
            role: .user,
            text: "The deployment decision is to use blue-green releases in Shanghai.",
            createdAt: Date(timeIntervalSince1970: 100)
        )
        let knowledge = MacConversationKnowledge(
            keyPointsMarkdown: "- Blue-green deployment is required",
            wikiMarkdown: "# Deployment\n\nPrimary region: Shanghai",
            updatedAt: Date(timeIntervalSince1970: 200)
        )
        try store.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: conversation,
                messages: [originalMessage],
                knowledge: knowledge
            )
        ])
        for index in 1...3 {
            try commitCanonicalTestContextCheckpoint(
                store: applicationPort,
                id: "archive-\(index)",
                conversationID: conversationID,
                summary: index == 3
                    ? "The unresolved rollback owner must be assigned before release."
                    : "Earlier continuation summary \(index).",
                estimatedTokens: 16,
                payloadDigest: String(repeating: "a", count: 64),
                createdAtMs: Int64(200 + index) * 1_000
            )
        }

        let restored = try XCTUnwrap(store.loadAll().first)
        XCTAssertEqual(restored.knowledge, knowledge)
        XCTAssertEqual(restored.messages.count, 1)
        let restoredMessage = try XCTUnwrap(restored.messages.first)
        XCTAssertEqual(restoredMessage.id, originalMessage.id)
        XCTAssertEqual(restoredMessage.role, originalMessage.role)
        XCTAssertEqual(restoredMessage.text, originalMessage.text)
        XCTAssertEqual(restoredMessage.createdAt, originalMessage.createdAt)
        XCTAssertEqual(
            restoredMessage.canonicalItemIDs,
            Set([originalMessage.id.uuidString, "\(originalMessage.id.uuidString):text"])
        )

        let originalResults = try await store.searchConversationAsync(
            conversationID: conversationID,
            query: "blue-green"
        )
        XCTAssertEqual(originalResults.first?.source, .originalMessage)
        XCTAssertTrue(originalResults.first?.content.contains("Shanghai") == true)
        XCTAssertEqual(originalResults.first?.messageID, originalMessage.id.uuidString)
        XCTAssertEqual(originalResults.first?.partID, "\(originalMessage.id.uuidString):text")
        XCTAssertEqual(originalResults.first?.messageOrdinal, 1)
        XCTAssertEqual(originalResults.first?.startCharacter, 0)
        XCTAssertLessThanOrEqual(originalResults.first?.snippet.count ?? Int.max, 1_600)

        let summaryResults = try await store.searchConversationAsync(
            conversationID: conversationID,
            query: "rollback owner"
        )
        XCTAssertTrue(summaryResults.isEmpty)
    }

    @MainActor
    func testNativeRecallBridgeHonorsCrossConversationScopeAndExactSource() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-cross-recall-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: store)
        // Keep the current conversation lexically disjoint: token-based recall
        // may legitimately match “之前” in “我之前问了什么”. This fixture tests
        // scope isolation, not exact-phrase search semantics.
        try repository.restoreBackupSnapshot([("current", "你好"),
            ("earlier", "之前约定 search current：Tokyo launch is October 14.")].map { id, text in
            MacStoredConversation(conversation: MacConversation(id: id, title: id, preview: "",
                updatedAt: .now, isPinned: false),
                messages: [MacChatMessage(role: .user, text: text, createdAt: .now)])
        })
        func recall(_ arguments: [String: Any], owner: String = "current") throws -> [String: Any] {
            let data = MacConversationRecallBridge.responseData(requestID: UUID().uuidString,
                conversationID: owner, arguments: arguments, store: store)
            return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        }
        func hits(_ arguments: [String: Any]) throws -> [[String: Any]] {
            let envelope = try recall(arguments)
            XCTAssertEqual(envelope["is_error"] as? Bool, false)
            let output = try XCTUnwrap(envelope["output"] as? [String: Any])
            return try XCTUnwrap(output["hits"] as? [[String: Any]])
        }
        XCTAssertTrue(try hits(["query": "之前约定"]).isEmpty)
        let history = try hits(["query": "之前约定", "scope": "local_user_history"])
        XCTAssertEqual(history.count, 1)
        let source = try XCTUnwrap(history.first?["source"] as? [String: Any])
        XCTAssertEqual(source["conversation_id"] as? String, "earlier")
        XCTAssertEqual(try hits(["query": "之前约定", "scope": "conversations",
            "conversation_ids": ["earlier"]]).count, 1)
        XCTAssertTrue(try hits(["query": "之前约定", "scope": "conversations",
            "conversation_ids": ["current"]]).isEmpty)
        var read: [String: Any] = ["conversation_id": "earlier",
            "message_id": try XCTUnwrap(source["message_id"]),
            "part_id": try XCTUnwrap(source["part_id"]),
            "expected_conversation_revision": try XCTUnwrap(source["conversation_revision"])]
        let original = try recall(read)
        XCTAssertEqual(original["is_error"] as? Bool, false)
        XCTAssertEqual((original["output"] as? [String: Any])?["text"] as? String,
            "之前约定 search current：Tokyo launch is October 14.")
        read["conversation_id"] = "current"
        XCTAssertEqual(try recall(read)["is_error"] as? Bool, true,
            "Exact source identities must never resolve against another conversation.")
        XCTAssertEqual(try recall(["query": "之前约定"], owner: "unknown")["is_error"] as? Bool, true)
        let invalidArguments: [[String: Any]] = [
            ["query": "之前约定", "scope": "conversations"],
            ["query": "之前约定", "scope": "current_conversation", "conversation_ids": ["earlier"]],
            ["message_id": "incomplete"], ["query": "之前约定", "requester_conversation_id": "earlier"],
        ]
        for invalid in invalidArguments {
            XCTAssertEqual(try recall(invalid)["is_error"] as? Bool, true)
        }
    }

    @MainActor
    func testNativeRecallBridgeReturnsOnlyCanonicalOriginalEvidence() throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-recall-bridge-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        try FileManager.default.createDirectory(at: baseDirectory, withIntermediateDirectories: true)
        let store = try MacApplicationPort(
            databaseURL: baseDirectory.appendingPathComponent("nola.db")
        )
        let repository = MacCanonicalConversationRepository(store: store)
        let conversationID = "bridge-conversation"
        try repository.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Bridge",
                    preview: "",
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [
                    MacChatMessage(
                        role: .user,
                        text: "Keep the Atlas migration behind a feature flag.",
                        createdAt: .now
                    )
                ],
                knowledge: MacConversationKnowledge(
                    keyPointsMarkdown: "- Atlas requires a feature flag",
                    wikiMarkdown: "# Atlas\n\nOwner: Platform",
                    updatedAt: .now
                )
            )
        ])

        let data = MacConversationRecallBridge.responseData(
            requestID: "request-1",
            conversationID: conversationID,
            arguments: ["query": "Atlas migration"],
            store: store
        )
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(payload["type"] as? String, "device_tool_result")
        XCTAssertEqual(payload["is_error"] as? Bool, false)
        let output = try XCTUnwrap(payload["output"] as? [String: Any])
        XCTAssertEqual(output["kind"] as? String, "search")
        XCTAssertNil(output["knowledge_points"])
        XCTAssertNil(output["wiki"])
        let matches = try XCTUnwrap(output["hits"] as? [[String: Any]])
        XCTAssertEqual(matches.count, 1)
        XCTAssertEqual(matches.first?["snippet"] as? String, "Keep the Atlas migration behind a feature flag.")
        let source = try XCTUnwrap(matches.first?["source"] as? [String: Any])
        let readData = MacConversationRecallBridge.responseData(requestID: "read-1",
            conversationID: conversationID, arguments: [
                "message_id": try XCTUnwrap(source["message_id"]),
                "part_id": try XCTUnwrap(source["part_id"]),
                "expected_conversation_revision": try XCTUnwrap(source["conversation_revision"]),
                "limit_bytes": 8,
            ], store: store)
        let read = try XCTUnwrap(JSONSerialization.jsonObject(with: readData) as? [String: Any])
        XCTAssertEqual(read["is_error"] as? Bool, false)
        let original = try XCTUnwrap(read["output"] as? [String: Any])
        XCTAssertEqual(original["text"] as? String, "Keep the")
        XCTAssertEqual(original["next_offset_bytes"] as? Int, 8)
        XCTAssertEqual(original["end_of_content"] as? Bool, false)
        XCTAssertThrowsError(try MacConversationRecallBridge.canonicalQuery(
            conversationID: conversationID, arguments: ["message_id": "m", "part_id": "p", "query": "mixed"]))
        XCTAssertThrowsError(try MacConversationRecallBridge.canonicalQuery(
            conversationID: conversationID, arguments: ["query": "Atlas", "scope": "unknown"]))
        let scoped = try MacConversationRecallBridge.canonicalQuery(conversationID: conversationID,
            arguments: ["query": "Atlas", "scope": "local_user_history", "after_conversation_id": "cursor"])
        XCTAssertEqual((scoped["scope"] as? [String: String])?["kind"], "local_user_history")
        XCTAssertEqual(scoped["after_conversation_id"] as? String, "cursor")
        let historyData = MacConversationRecallBridge.responseData(requestID: "history-1",
            conversationID: conversationID,
            arguments: ["query": "Atlas migration", "scope": "local_user_history"], store: store)
        let history = try XCTUnwrap(JSONSerialization.jsonObject(with: historyData) as? [String: Any])
        XCTAssertEqual(history["is_error"] as? Bool, false)
        let historyOutput = try XCTUnwrap(history["output"] as? [String: Any])
        XCTAssertEqual((historyOutput["hits"] as? [[String: Any]])?.count, 1)
        let changedSourceData = MacConversationRecallBridge.responseData(requestID: "changed-source",
            conversationID: conversationID, arguments: [
                "message_id": try XCTUnwrap(source["message_id"]),
                "part_id": try XCTUnwrap(source["part_id"]),
                "expected_conversation_revision": Int64.max,
            ], store: store)
        let changedSource = try XCTUnwrap(JSONSerialization.jsonObject(with: changedSourceData) as? [String: Any])
        XCTAssertEqual(changedSource["is_error"] as? Bool, true)
    }
}

@discardableResult
func commitCanonicalTestContextCheckpoint(
    store: MacApplicationPort,
    id: String,
    conversationID: String,
    runID: String? = nil,
    policyVersion: String = "agent-core-v1",
    summary: String,
    estimatedTokens: Int64,
    measuredTokens: Int64? = nil,
    window: Int64 = 0,
    promptLimit: Int64 = 0,
    payloadDigest: String,
    createdAtMs: Int64
) throws -> MacCanonicalContextAggregate {
    let componentID = "\(id):summary"
    let checkpoint = MacCanonicalContextCheckpoint(
        id: id,
        conversationID: conversationID,
        runID: runID,
        modelProfileID: "test-model",
        policyVersion: policyVersion,
        coveredThroughMessageID: nil,
        coveredThroughSequence: nil,
        summaryPartID: componentID,
        retainedTailStartMessageID: nil,
        tokenUsage: MacCanonicalContextTokenUsage(
            estimated: estimatedTokens,
            measured: measuredTokens,
            window: window,
            promptLimit: promptLimit
        ),
        payloadDigest: payloadDigest,
        createdAtMs: createdAtMs
    )
    let component = MacCanonicalContextComponent(
        id: componentID,
        checkpointID: id,
        kind: "conversation_summary",
        sourceID: nil,
        sourceURI: nil,
        title: nil,
        mimeType: "text/plain",
        authorization: "not_required",
        observedAtMs: createdAtMs,
        maximumAgeMinutes: nil,
        freshness: "fresh",
        contentReference: MacCanonicalContextContentReference(
            type: "inline_preview",
            text: summary,
            truncated: false,
            artifactID: nil,
            blobID: nil
        ),
        contentDigest: payloadDigest,
        estimatedTokens: estimatedTokens,
        provenance: MacCanonicalContextProvenance(
            providerID: "test",
            collectionMethod: "compaction",
            sourceRevision: nil,
            disclosedToModel: true
        )
    )
    return try store.commitContextCheckpoint(
        MacCanonicalContextCommit(checkpoint: checkpoint, components: [component])
    )
}
