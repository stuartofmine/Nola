// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises the macOS chat workspace from send() through streamed runtime events and archive restore.

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacChatProductPathIntegrationTests: XCTestCase {
    func testCompletionAppendsOnlyLiveTailAndPreservesCommittedNodeIdentities() async throws {
        for answer in [
            "春江潮水连海平，海上明月共潮生。\n滟滟随波千万里，何处春江无月明！\n最后一行",
            "## Result\n\n```swift\nlet value = 1\n",
            "## Result\n\n- First item\n- Last item"
        ] {
            try await assertCompletionPreservesLiveNodes(answer: answer)
        }
    }

    private func assertCompletionPreservesLiveNodes(answer: String) async throws {
        // Parser identity is independent of transcript storage. Journal views
        // feed their bounded text ranges into this same incremental assembler.
        var assembler = MacSemanticStreamAssembler(identitySeed: "live-parser-identity")
        let before = assembler.append(String(answer.dropLast(2)))
        XCTAssertFalse(before.isEmpty)
        let tail = try XCTUnwrap(assembler.mutableTail)
        let suffix = assembler.append(String(answer.suffix(2))) + assembler.finish()
        let completed = before + suffix
        XCTAssertEqual(Array(completed.prefix(before.count)), before)
        XCTAssertEqual(completed.count, before.count + 1)
        XCTAssertEqual(completed.last?.id, tail.id)
        XCTAssertEqual(assembler.rawText, answer)
        XCTAssertNil(assembler.mutableTail)
        XCTAssertTrue(assembler.finish().isEmpty)
    }

    func testAgentHostLaunchEnvironmentLeavesPermissionProfileToTheConversationRun() {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-permission-profile-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(
            applicationSupportRoot: root.appendingPathComponent("ApplicationSupport", isDirectory: true),
            cachesRoot: root.appendingPathComponent("Caches", isDirectory: true)
        )
        .launchAgentEnvironment(projectURL: root)

        XCTAssertEqual(environment["NOLA_CLIENT_SURFACE"], "mac")
        XCTAssertEqual(environment["NOLA_DESKTOP_HOST"], "1")
        XCTAssertEqual(environment["NOLA_AGENT_HOST_PROTOCOL"], "1")
        XCTAssertEqual(environment["NOLA_NATIVE_PERMISSION_MODE"], "1")
        XCTAssertNil(environment["NOLA_PERMISSION_PROFILE"])
    }

    func testActivityHistoryUsesAnIsolatedStoreDuringTests() {
        let productionPath = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Nola/Activity/history.json")
            .standardizedFileURL.path
        let testPath = MacActivityStore.shared.persistenceURL.standardizedFileURL.path

        XCTAssertNotEqual(testPath, productionPath)
        XCTAssertTrue(
            testPath.contains("nola-tests-"),
            "XCTest must never persist activity records into the user's real Nola history: \(testPath)"
        )
    }

    func testTerminalContextSnapshotDoesNotDuplicateFactsInConversationMetadata() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-context-display-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let conversationID = try XCTUnwrap(model.selectedConversationID)
        try repository.ensureConversation(try XCTUnwrap(model.selectedConversation))
        let snapshot = MacConversationContextSnapshot(
            usedTokens: 80_759,
            contextWindowTokens: 1_000_000,
            promptTokenLimit: 991_808,
            isEstimated: false,
            cachedInputTokens: 1_404_416
        )

        model.applyContextSnapshot(snapshot, conversationID: conversationID)
        model.finalizeContextSnapshotAfterRun(conversationID: conversationID)

        let metadata = try XCTUnwrap(port.loadConversation(conversationID)?.settingsJSON)
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(metadata.utf8)) as? [String: Any])
        XCTAssertNil(fields["contextSnapshot"])
        XCTAssertEqual(model.contextUsed, 80_759)
        XCTAssertEqual(model.contextWindowTokens, 1_000_000)
        XCTAssertNil(model.promptCacheReadTokens,
            "Unattributed cached tokens must not become a request-scoped cache diagnostic")
    }

    func testNewContextEstimateReplacesPreviousProviderMeasurement() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-context-freshness-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let conversationID = try XCTUnwrap(model.selectedConversationID)
        try repository.ensureConversation(try XCTUnwrap(model.selectedConversation))
        model.applyContextSnapshot(.init(
            usedTokens: 48_000, contextWindowTokens: 65_536,
            promptTokenLimit: 61_440, isEstimated: false
        ), conversationID: conversationID)
        model.applyContextSnapshot(.init(
            usedTokens: 8_000, contextWindowTokens: 65_536,
            promptTokenLimit: 61_440, isEstimated: true
        ), conversationID: conversationID)
        XCTAssertEqual(model.contextUsed, 8_000)
        XCTAssertTrue(model.contextUsageIsEstimated)
        model.applyContextSnapshot(.init(
            usedTokens: 0, contextWindowTokens: 32_768,
            promptTokenLimit: 28_672, isEstimated: true
        ), conversationID: conversationID)
        XCTAssertEqual(model.contextUsed, 0)
        XCTAssertEqual(model.contextWindowTokens, 32_768)
        XCTAssertEqual(model.contextPromptLimit, 28_672)
        model.finalizeContextSnapshotAfterRun(conversationID: conversationID)
        XCTAssertEqual(model.contextUsed, 0, "Finalization must not restore the old measurement")
    }

    func testSendKeepsHistoryStreamsOneAssistantMessageAndRestoresAuthoritativeState() async throws {
        let configurationStore = MacAppleChatConfigurationStore.shared
        let previousConfiguration = configurationStore.endpoints()
        try configurationStore.save(
            baseURL: "http://127.0.0.1:9/v1",
            apiKey: "mac-product-path-e2e-key",
            model: "mac-product-path-e2e-model",
            executionCapabilities: .init(toolCalls: true, reasoning: true)
        )
        defer {
            try? configurationStore.replaceEndpoints(previousConfiguration)
        }

        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-chat-product-path-\(UUID().uuidString)", isDirectory: true)
        let applicationPort = try MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite"))
        let archiveStore = MacCanonicalConversationRepository(store: applicationPort)
        let conversationID = "product-path-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: baseDirectory) }

        let expectedResponse = """
        ## Result

        Earlier messages remain.

        - item one
        - item two
        """
        let events = try [
            tracedMacAgentEvent(
                conversationID: conversationID,
                sequence: 1,
                kind: "phase.changed",
                payload: ["phase": "processing"]
            ),
            tracedMacAgentEvent(
                conversationID: conversationID,
                sequence: 6,
                kind: "phase.changed",
                payload: ["phase": "processing"]
            ),
            tracedMacAgentEvent(
                conversationID: conversationID,
                sequence: 7,
                kind: "usage.snapshot",
                payload: [
                    "usage": [
                        "used_tokens": 4_325,
                        "context_window_tokens": 128_000,
                        "prompt_token_limit": 123_904,
                    ],
                ]
            ),
            tracedMacAgentEvent(
                conversationID: conversationID,
                sequence: 8,
                kind: "run.completed",
                status: "completed",
                payload: ["reason": "completed"]
            ),
        ]
        let earlierUser = MacChatMessage(role: .user, text: "Earlier question", createdAt: .now)
        let earlierReply = MacChatMessage(role: .assistant, text: "Earlier reply", createdAt: .now)
        let runtime = ScriptedMacLocalAgentRuntime(
            initialDelay: .milliseconds(250),
            events: events,
            canonicalAnswer: expectedResponse,
            priorMessages: [
                .init(id: earlierUser.id.uuidString, role: "user", text: earlierUser.text, createdAt: earlierUser.createdAt),
                .init(id: earlierReply.id.uuidString, role: "assistant", text: earlierReply.text, createdAt: earlierReply.createdAt),
            ],
            contextUsage: .init(used: 4_325, window: 128_000, promptLimit: 123_904)
        )
        try archiveStore.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Canonical product path",
                    preview: earlierReply.text,
                    updatedAt: .now,
                    isPinned: false
                ),
                messages: [earlierUser, earlierReply]
            )
        ])
        let committedTurn = try commitScriptedCanonicalTurn(
            applicationPort: applicationPort,
            conversationID: conversationID,
            title: "Canonical product path",
            userText: "Continue without replacing the previous answer",
            answer: expectedResponse,
            priorMessages: [
                .init(id: earlierUser.id.uuidString, role: "user", text: earlierUser.text, createdAt: earlierUser.createdAt),
                .init(id: earlierReply.id.uuidString, role: "assistant", text: earlierReply.text, createdAt: earlierReply.createdAt),
            ],
            contextUsage: .init(used: 4_325, window: 128_000, promptLimit: 123_904),
            tool: .init(callID: "read-1", capabilityID: "read_file", title: "read_file",
                        summary: "Read Sources/Main.cpp", input: ["path": "Sources/Main.cpp"],
                        output: "class Network {};", outputPreview: "class Network {};",
                        resourceURI: "file:///Sources/Main.cpp")
        )
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost()
        )
        await model.loadSelectedConversationIfNeeded()

        let immediateMessages = model.selectedMessages
        guard immediateMessages.count == 4 else {
            XCTFail("Expected the canonical projection to contain the two prior messages and committed turn, got \(immediateMessages.count)")
            return
        }
        XCTAssertEqual(immediateMessages.map(\.role), [.user, .assistant, .user, .assistant])
        let submittedUser = try XCTUnwrap(immediateMessages.dropFirst(2).first)
        let submittedAssistant = try XCTUnwrap(immediateMessages.last)
        XCTAssertEqual(immediateMessages[0].id, earlierUser.id)
        XCTAssertEqual(immediateMessages[1].id, earlierReply.id)
        XCTAssertEqual(immediateMessages[1].text, "Earlier reply")
        XCTAssertEqual(immediateMessages[2].text, "Continue without replacing the previous answer")
        let immediateBody = try await journalBody(immediateMessages[3])
        XCTAssertEqual(immediateBody, expectedResponse)
        XCTAssertEqual(submittedUser.id, committedTurn.userID)
        XCTAssertEqual(submittedAssistant.id, committedTurn.assistantID)

        let completedMessages = try XCTUnwrap(model.messagesByConversation[conversationID])
        guard completedMessages.count == 4 else {
            XCTFail("Expected four messages after canonical reload, got \(completedMessages.count)")
            return
        }
        XCTAssertEqual(completedMessages.map(\.id), [
            earlierUser.id,
            earlierReply.id,
            submittedUser.id,
            submittedAssistant.id,
        ])
        XCTAssertEqual(completedMessages[1].text, "Earlier reply")
        let completedAssistant = try XCTUnwrap(completedMessages.last)
        let completedBody = try await journalBody(completedAssistant)
        XCTAssertEqual(completedBody, expectedResponse)
        XCTAssertEqual(
            completedBody.components(separatedBy: "## Result").count - 1,
            1,
            "The final assistant body was duplicated"
        )
        let completedParts = try await MacJournalTextRead.parts(try XCTUnwrap(completedAssistant.journalReference))
        XCTAssertTrue(completedParts.contains {
            if case .tool(let tool) = $0.metadata {
                return tool.tool_call_id == "read-1" && tool.capability_id == "read_file"
            }
            return false
        })
        XCTAssertEqual(model.contextUsed, 4_325)
        XCTAssertEqual(model.contextWindowTokens, 128_000)
        XCTAssertFalse(model.contextUsageIsEstimated)

        try await Task.sleep(for: .milliseconds(800))
        let restored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost()
        )
        await restored.loadSelectedConversationIfNeeded()
        let restoredMessages = try XCTUnwrap(restored.messagesByConversation[conversationID])
        XCTAssertEqual(restoredMessages.map(\.id), completedMessages.map(\.id))
        let restoredBody = try await journalBody(try XCTUnwrap(restoredMessages.last))
        XCTAssertEqual(restoredBody, expectedResponse)
        XCTAssertEqual(restoredMessages.last?.journalReference, completedAssistant.journalReference)
        XCTAssertEqual(restored.contextUsed, 4_325)
        XCTAssertEqual(restored.contextWindowTokens, 128_000)
        XCTAssertFalse(restored.contextUsageIsEstimated)
    }

    func testLateEmptyHistoryLoadCannotEraseActiveTurn() async throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-late-history-\(UUID().uuidString)", isDirectory: true)
        let applicationPort = try MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite"))
        let archiveStore = MacCanonicalConversationRepository(store: applicationPort)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }

        let expectedResponse = "ACTIVE_TURN_SURVIVED_LATE_HISTORY"
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = applicationPort
        host.allowsRun = true
        let model = MacWorkspaceModel(
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: host, readinessIssue: { _ in nil }
        )
        defer { host.runContinuation?.finish(); model.presentationDidDetach() }
        model.presentationDidAttach()
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        let conversationID = try XCTUnwrap(model.selectedConversationID)
        let staleLoad = Task { await model.loadSelectedConversationIfNeeded() }
        model.draft = "Keep this active turn"
        model.send()
        let deadline = Date().addingTimeInterval(4)
        while host.runContinuation == nil && Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertNotNil(host.runContinuation)
        let run = try XCTUnwrap(try applicationPort.activeRunIdentity(conversationID: conversationID)?.runID)
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: applicationPort, conversationID: conversationID,
            runID: run, assistantMessageID: "\(run):assistant")
        let cursor = try source.append(expectedResponse)
        try source.ingest(into: applicationPort, through: cursor)
        host.runContinuation?.yield(source.event(through: cursor))
        await model.loadSelectedConversationIfNeeded()

        // A committed admission must wake the row observer, not wait for its
        // 250 ms missing-conversation retry. Preserve the original 120 ms bound.
        let admissionDeadline = Date().addingTimeInterval(0.12)
        while model.messagesByConversation[conversationID]?.map(\.role) != [.user, .assistant],
              Date() < admissionDeadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(
            model.messagesByConversation[conversationID]?.map(\.role),
            [.user, .assistant]
        )

        await staleLoad.value
        XCTAssertEqual(model.messagesByConversation[conversationID]?.map(\.role), [.user, .assistant])
        XCTAssertEqual(model.messagesByConversation[conversationID]?.first?.text, "Keep this active turn")
        let answer = try await journalBody(try XCTUnwrap(model.selectedMessages.last))
        XCTAssertEqual(answer, expectedResponse)
        XCTAssertTrue(model.isRunning)
        XCTAssertEqual(host.runRequests, 1)
        XCTAssertEqual(try applicationPort.activeRunIdentity(conversationID: conversationID)?.runID, run)
    }

    func testRuntimeHistoryCannotOverwriteStoredTranscriptAndContextSurvivesColdRestore() async throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-corrupted-archive-\(UUID().uuidString)", isDirectory: true)
        let applicationPort = try MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite"))
        let archiveStore = MacCanonicalConversationRepository(store: applicationPort)
        let conversationID = "corrupted-archive-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: baseDirectory) }

        let startedAt = Date(timeIntervalSince1970: 1_786_543_210)
        try archiveStore.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: MacConversation(
                    id: conversationID,
                    title: "Boost geometry",
                    preview: "Hello",
                    updatedAt: startedAt,
                    isPinned: false,
                    isPersisted: true
                ),
                messages: [
                    MacChatMessage(role: .user, text: "Explain Boost geometry boolean operations", createdAt: startedAt),
                    MacChatMessage(role: .assistant, text: "Hello!", createdAt: startedAt.addingTimeInterval(1)),
                    MacChatMessage(role: .user, text: "Hello", createdAt: startedAt.addingTimeInterval(2)),
                ]
            )
        ])

        let runtime = ScriptedMacLocalAgentRuntime(
            initialDelay: .zero,
            events: []
        )
        _ = try commitScriptedCanonicalTurn(
            applicationPort: applicationPort,
            conversationID: conversationID,
            title: "Boost geometry",
            userText: "Canonical context measurement",
            answer: "Context committed",
            contextUsage: .init(used: 19_437, window: 1_000_000, promptLimit: 995_904)
        )
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost()
        )
        defer { model.presentationDidDetach() }
        let storedMessages = try XCTUnwrap(archiveStore.load(conversationID: conversationID)).messages

        await model.loadSelectedConversationIfNeeded()

        XCTAssertEqual(model.messagesByConversation[conversationID], storedMessages)
        XCTAssertEqual(model.contextUsed, 19_437)
        XCTAssertEqual(model.contextWindowTokens, 1_000_000)
        XCTAssertFalse(model.contextUsageIsEstimated)

        let coldRestored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost()
        )
        defer { coldRestored.presentationDidDetach() }
        await coldRestored.loadSelectedConversationIfNeeded()
        XCTAssertEqual(coldRestored.messagesByConversation[conversationID], storedMessages)
        XCTAssertEqual(coldRestored.contextUsed, 19_437)
    }

    func testEmptyCoreTextPlaceholderKeepsTraceAnswerThroughArchiveRestore() async throws {
        let configurationStore = MacAppleChatConfigurationStore.shared
        let previousConfiguration = configurationStore.endpoints()
        try configurationStore.save(
            baseURL: "http://127.0.0.1:9/v1",
            apiKey: "mac-empty-placeholder-e2e-key",
            model: "mac-empty-placeholder-e2e-model",
            executionCapabilities: .init()
        )
        defer {
            try? configurationStore.replaceEndpoints(previousConfiguration)
        }

        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-empty-placeholder-\(UUID().uuidString)", isDirectory: true)
        let applicationPort = try MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite"))
        let archiveStore = MacCanonicalConversationRepository(store: applicationPort)
        let conversationID = "empty-placeholder-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: baseDirectory) }

        let expectedResponse = """
        ## Available AI competitions

        - Competition details remain visible after the run completes.
        - The restored conversation keeps the same Markdown answer.
        """
        let turnID = "turn-\(conversationID)"
        let events = try [
            tracedMacAgentEvent(
                conversationID: conversationID,
                sequence: 2,
                kind: "phase.changed",
                payload: ["phase": "processing"]
            ),
            coreTurnCompletedMacAgentEvent(
                conversationID: conversationID,
                turnID: turnID,
                revision: 2
            ),
            tracedMacAgentEvent(
                conversationID: conversationID,
                sequence: 4,
                kind: "run.completed",
                status: "completed",
                payload: ["reason": "completed"]
            ),
        ]
        let runtime = ScriptedMacLocalAgentRuntime(
            initialDelay: .milliseconds(1),
            events: events,
            canonicalAnswer: expectedResponse
        )
        _ = try commitScriptedCanonicalTurn(
            applicationPort: applicationPort,
            conversationID: conversationID,
            title: "AI competitions",
            userText: "Find recent AI competitions",
            answer: expectedResponse
        )
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost()
        )
        await model.loadSelectedConversationIfNeeded()

        let immediateMessages = try XCTUnwrap(model.messagesByConversation[conversationID])
        XCTAssertEqual(immediateMessages.map(\.role), [.user, .assistant])
        let immediateBody = try await journalBody(try XCTUnwrap(immediateMessages.last))
        XCTAssertEqual(immediateBody, expectedResponse)
        XCTAssertNotNil(model.messagesByConversation[conversationID]?.last?.journalReference)

        try await Task.sleep(for: .milliseconds(800))
        let restored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost()
        )
        defer { restored.presentationDidDetach() }
        await restored.loadSelectedConversationIfNeeded()
        let restoredAnswer = try await journalBody(try XCTUnwrap(restored.messagesByConversation[conversationID]?.last))
        XCTAssertEqual(restoredAnswer, expectedResponse)
        XCTAssertNotEqual(restoredAnswer, String(localized: "Nola could not produce a final answer. Please try again."))
    }

    func testDenseHostTraceKeepsOneStableAssistantDocumentAndVisibleToolProcess() async throws {
        let configurationStore = MacAppleChatConfigurationStore.shared
        let previousConfiguration = configurationStore.endpoints()
        try configurationStore.save(
            baseURL: "http://127.0.0.1:9/v1",
            apiKey: "mac-dense-trace-e2e-key",
            model: "mac-dense-trace-e2e-model",
            executionCapabilities: .init(toolCalls: true, reasoning: true)
        )
        defer {
            try? configurationStore.replaceEndpoints(previousConfiguration)
        }

        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-dense-trace-\(UUID().uuidString)", isDirectory: true)
        let applicationPort = try MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite"))
        let archiveStore = MacCanonicalConversationRepository(store: applicationPort)
        let conversationID = "dense-trace-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: baseDirectory) }

        let answer = """
        ## Review

        The implementation keeps a stable document while it streams.

        ```swift
        struct StableAnswer {
            let markdownRendersDuringStreaming = true
        }
        ```

        - Tool parameters remain inspectable.
        - Internal control tools stay out of the transcript.
        """
        var events: [MacLocalAgentEvent] = []
        var sequence: UInt64 = 1
        func append(
            _ kind: String,
            status: String = "running",
            payload: [String: Any]
        ) throws {
            events.append(
                try tracedMacAgentEvent(
                    conversationID: conversationID,
                    sequence: sequence,
                    kind: kind,
                    status: status,
                    payload: payload
                )
            )
            sequence += 1
        }

        try append("phase.changed", payload: ["phase": "processing"])
        let completeArguments = #"{"path":"Sources/StableAnswer.swift","start_line":1,"end_line":240}"#
        let argumentCheckpoints = [
            1,
            completeArguments.count / 3,
            completeArguments.count * 2 / 3,
            completeArguments.count,
        ]
        for characterCount in argumentCheckpoints {
            try append("tool.progress", payload: ["tool_name": "read_file", "call_id": "read-dense",
                "arguments": String(completeArguments.prefix(characterCount))])
        }
        try append(
            "tool.started",
            payload: [
                "tool_name": "set_thinking_mode",
                "call_id": "internal-control",
                "arguments": #"{"enabled":true}"#,
            ]
        )
        try append(
            "tool.completed",
            status: "completed",
            payload: [
                "tool_name": "set_thinking_mode",
                "call_id": "internal-control",
                "result": "thinking enabled",
            ]
        )
        // The real source fixture below publishes the answer through its
        // journal. Diagnostic control events never duplicate that body.
        try append(
            "usage.snapshot",
            payload: [
                "usage": [
                    "used_tokens": 9_600,
                    "context_window_tokens": 131_072,
                    "prompt_token_limit": 126_976,
                ],
            ]
        )
        try append("run.completed", status: "completed", payload: ["reason": "completed"])

        let runtime = ScriptedMacLocalAgentRuntime(
            initialDelay: .zero,
            eventDelay: .zero,
            events: events,
            canonicalAnswer: answer,
            contextUsage: .init(used: 9_600, window: 131_072, promptLimit: 126_976)
        )
        _ = try commitScriptedCanonicalTurn(
            applicationPort: applicationPort,
            conversationID: conversationID,
            title: "Dense trace",
            userText: "Review the implementation",
            answer: answer,
            contextUsage: .init(used: 9_600, window: 131_072, promptLimit: 126_976),
            tool: .init(
                callID: "read-dense",
                capabilityID: "read_file",
                title: "read_file",
                summary: "Read Sources/StableAnswer.swift",
                input: ["path": "Sources/StableAnswer.swift"],
                output: "Read 240 lines.",
                outputPreview: "Read 240 lines.",
                resourceURI: "file:///Sources/StableAnswer.swift"
            )
        )
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: runtime,
            remoteHost: PersistenceTestRemoteHost()
        )
        await model.loadSelectedConversationIfNeeded()

        let messages = try XCTUnwrap(model.messagesByConversation[conversationID])
        XCTAssertEqual(messages.map(\.role), [.user, .assistant])
        let assistant = try XCTUnwrap(messages.last)
        let body = try await journalBody(assistant)
        XCTAssertEqual(body, answer)
        XCTAssertEqual(
            body.components(separatedBy: "## Review").count - 1,
            1,
            "Transport fragments must never duplicate the final document."
        )
        let reference = try XCTUnwrap(assistant.journalReference)
        let parts = try await MacJournalTextRead.parts(reference)
        XCTAssertTrue(parts.contains {
            if case .text(let text) = $0.metadata { return text.is_final }
            return false
        })
        var assembler = MacSemanticStreamAssembler(identitySeed: assistant.id.uuidString)
        let nodes = assembler.append(body) + assembler.finish()
        XCTAssertTrue(nodes.contains { if case .code(language: "swift") = $0.kind { return true }; return false })
        let tools = parts.filter { if case .tool = $0.metadata { return true }; return false }
        XCTAssertEqual(tools.count, 1)
        let part = try XCTUnwrap(tools.first)
        guard case .tool(let metadata) = part.metadata else { return XCTFail("Missing tool metadata") }
        XCTAssertEqual(metadata.tool_call_id, "read-dense")
        XCTAssertEqual(metadata.capability_id, "read_file")
        let input = try XCTUnwrap(part.bodies.first { $0.field == "tool_input" })
        let toolArguments = try await MacJournalTextRead.completeSmallText(reference, partID: part.part_id, content: input)
        let inputJSON = try MacJournalBodyPresentation.toolField("tool_input")
            .display(try XCTUnwrap(toolArguments), complete: true)
        let decodedArguments = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(inputJSON.utf8)) as? [String: Any]
        )
        XCTAssertEqual(decodedArguments["path"] as? String, "Sources/StableAnswer.swift")
        let output = try XCTUnwrap(part.bodies.first { $0.field == "tool_output" })
        let outputText = try await MacJournalTextRead.completeSmallText(reference, partID: part.part_id, content: output)
        let outputJSON = try MacJournalBodyPresentation.toolField("tool_output")
            .display(try XCTUnwrap(outputText), complete: true)
        XCTAssertEqual(try JSONDecoder().decode(String.self, from: Data(outputJSON.utf8)), "Read 240 lines.")
        XCTAssertEqual(model.contextUsed, 9_600)
        XCTAssertEqual(model.contextWindowTokens, 131_072)
        XCTAssertFalse(model.contextUsageIsEstimated)
    }
}

@MainActor
private func journalBody(_ message: MacChatMessage) async throws -> String {
    let reference = try XCTUnwrap(message.journalReference)
    return try await MacJournalTextRead.markdown(reference, historicalFallback: "")
}

private func tracedMacAgentEvent(
    conversationID: String,
    sequence: UInt64,
    kind: String,
    status: String = "running",
    payload: [String: Any]
) throws -> MacLocalAgentEvent {
    let object: [String: Any] = [
        "type": "trace",
        "trace": [
            "schema_version": 1,
            "trace_id": "trace-\(conversationID)",
            "run_id": "run-\(conversationID)",
            "conversation_id": conversationID,
            "message_id": "assistant-\(conversationID)",
            "turn_id": "turn-\(conversationID)",
            "event_id": "event-\(sequence)",
            "sequence": sequence,
            "emitted_at": "2026-07-19T08:00:00.000Z",
            "kind": kind,
            "status": status,
            "visibility": "summary",
            "payload": payload,
        ],
    ]
    let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    return try XCTUnwrap(
        MacAgentHostRuntime.decode(
            line: String(decoding: data, as: UTF8.self),
            conversationID: conversationID,
            coreSessionID: conversationID
        )
    )
}

private func coreTurnCompletedMacAgentEvent(
    conversationID: String,
    turnID: String,
    revision: Int
) throws -> MacLocalAgentEvent {
    let object: [String: Any] = [
        "protocol_version": 1,
        "session_id": conversationID,
        "cursor": ["sequence": revision],
        "revision": revision,
        "event": [
            "type": "turn_completed",
            "turn_id": turnID,
        ],
    ]
    let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    return try XCTUnwrap(
        MacAgentHostRuntime.decode(
            line: String(decoding: data, as: UTF8.self),
            conversationID: conversationID,
            coreSessionID: conversationID
        )
    )
}

struct ScriptedCanonicalPriorMessage: Sendable {
    let id: String
    let role: String
    let text: String
    let createdAt: Date
}

struct ScriptedCanonicalContextUsage: Sendable {
    let used: Int
    let window: Int
    let promptLimit: Int
}

struct ScriptedCanonicalTool: Sendable {
    let callID: String
    let capabilityID: String
    let title: String
    let summary: String
    let input: [String: String]
    let output: String
    let outputPreview: String
    let resourceURI: String
}

@discardableResult
func commitScriptedCanonicalTurn(
    applicationPort: MacApplicationPort,
    conversationID: String,
    title: String,
    userText: String,
    answer: String,
    priorMessages: [ScriptedCanonicalPriorMessage] = [],
    contextUsage: ScriptedCanonicalContextUsage = .init(used: 0, window: 128_000, promptLimit: 123_904),
    tool: ScriptedCanonicalTool? = nil
) throws -> (userID: UUID, assistantID: UUID) {
    let submissionID = UUID().uuidString
    let userID = UUID()
    let assistantCanonicalID = "\(submissionID):assistant"
    let assistantID = try XCTUnwrap(MacCanonicalMessageIdentity.stableID(assistantCanonicalID))
    let now = Int64(Date().timeIntervalSince1970 * 1_000)
    _ = try applicationPort.prepareFixtureRemoteSubmission(.init(
        conversationID: conversationID, conversationTitle: title, submissionID: submissionID,
        sourceHostID: "scripted-mac-product-host", clientID: "fixture-client", userMessageID: userID.uuidString,
        userPartID: "\(userID.uuidString):body", userText: userText, userContentJSON: nil,
        assistantMessageID: assistantCanonicalID, assistantPartID: "\(assistantCanonicalID):text",
        contextCheckpointID: nil, createdAtMs: now
    ))
    // Historical messages already belong to the native repository. A new
    // source Run cannot replace them by returning a cumulative transcript.
    let before = try applicationPort.loadConversationWindow(conversationID)
    for prior in priorMessages {
        XCTAssertTrue(before.projection.messages.contains { $0.messageID == prior.id })
    }
    let source = try MacAssistantJournalTestFixture(
        conversationID: conversationID, runID: submissionID, assistantMessageID: assistantCanonicalID,
        sourceHostID: "scripted-mac-product-host", userText: userText,
        userMessageID: userID.uuidString, userPartID: "\(userID.uuidString):body", tool: tool, contextUsage: contextUsage
    )
    _ = try source.finish(answer)
    _ = try source.commitTerminal(into: applicationPort)
    return (userID, assistantID)
}

private actor ScriptedMacLocalAgentRuntime: MacLocalAgentRuntime {
    struct CanonicalPriorMessage: Sendable {
        let id: String
        let role: String
        let text: String
        let createdAt: Date
    }

    struct CanonicalContextUsage: Sendable {
        let used: Int
        let window: Int
        let promptLimit: Int
    }

    private let initialDelay: Duration
    private let eventDelay: Duration
    private let events: [MacLocalAgentEvent]
    private let canonicalAnswer: String?
    private let priorMessages: [CanonicalPriorMessage]
    private let contextUsage: CanonicalContextUsage?

    init(
        initialDelay: Duration,
        eventDelay: Duration = .milliseconds(15),
        events: [MacLocalAgentEvent],
        canonicalAnswer: String? = nil,
        priorMessages: [CanonicalPriorMessage] = [],
        contextUsage: CanonicalContextUsage? = nil
    ) {
        self.initialDelay = initialDelay
        self.eventDelay = eventDelay
        self.events = events
        self.canonicalAnswer = canonicalAnswer
        self.priorMessages = priorMessages
        self.contextUsage = contextUsage
    }

    func run(
        _ request: MacLocalAgentRunRequest
    ) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        let initialDelay = initialDelay
        let eventDelay = eventDelay
        let events = events
        let canonicalAnswer = canonicalAnswer
        let priorMessages = priorMessages
        let contextUsage = contextUsage
        return AsyncThrowingStream { continuation in
            Task {
                do {
                    try request.submissionTicket?.claimDispatch()
                } catch {
                    continuation.finish(throwing: error)
                    return
                }
                request.submissionTicket?.markAccepted()
                await request.submissionAccepted?()
                try? await Task.sleep(for: initialDelay)
                var committedCanonicalProjection = false
                for event in events {
                    if !committedCanonicalProjection,
                       let canonicalAnswer,
                       Self.isTerminal(event) {
                        do {
                            continuation.yield(
                                try Self.canonicalJournalEvent(
                                    request: request,
                                    answer: canonicalAnswer,
                                    priorMessages: priorMessages,
                                    contextUsage: contextUsage
                                )
                            )
                            committedCanonicalProjection = true
                        } catch {
                            continuation.finish(throwing: error)
                            return
                        }
                    }
                    continuation.yield(event.canonicalRunID == nil
                        ? event.bindingCanonicalRun(request.submissionID) : event)
                    if eventDelay > .zero {
                        try? await Task.sleep(for: eventDelay)
                    }
                }
                continuation.finish()
            }
        }
    }

    private nonisolated static func isTerminal(_ event: MacLocalAgentEvent) -> Bool {
        switch event.kind {
        case .finished, .failed, .cancelled: true
        default: false
        }
    }

    private nonisolated static func canonicalJournalEvent(
        request: MacLocalAgentRunRequest,
        answer: String,
        priorMessages: [CanonicalPriorMessage],
        contextUsage: CanonicalContextUsage?
    ) throws -> MacLocalAgentEvent {
        let application = try XCTUnwrap(request.applicationPort, "Scripted execution requires its actual Application port")
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        if try application.activeRunIdentity(conversationID: request.conversationID) == nil {
            _ = try application.prepareFixtureRemoteSubmission(.init(
                conversationID: request.conversationID, conversationTitle: "Canonical product path",
                submissionID: request.submissionID, sourceHostID: "scripted-mac-product-host", clientID: "fixture-client",
                userMessageID: request.userMessageID, userPartID: "\(request.userMessageID):text",
                userText: request.prompt, userContentJSON: nil,
                assistantMessageID: request.assistantMessageID, assistantPartID: "\(request.assistantMessageID):text",
                contextCheckpointID: nil, createdAtMs: now
            ))
        }
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: application, conversationID: request.conversationID,
            runID: request.submissionID, assistantMessageID: request.assistantMessageID,
            sourceHostID: "scripted-mac-product-host",
            contextUsage: contextUsage.map { .init(used: $0.used, window: $0.window, promptLimit: $0.promptLimit) })
        _ = try source.finish(answer)
        let seal = try source.commitTerminal(into: application)
        return source.terminalEvent(seal: seal)
    }

    func cancel(conversationID: String) {}

    func resolvePermission(
        conversationID: String,
        request: MacRemotePermissionRequest,
        allow: Bool,
        scope: MacPermissionGrantScope
    ) throws {}

    func listSessions() async throws -> [MacLocalAgentSessionSummary] {
        []
    }

    func deleteSession(id: String) async throws {}
}
