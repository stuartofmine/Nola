// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacWorkspacePresentationLifecycleTests: XCTestCase {
    private func waitUntil(_ predicate: () -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while !predicate() && Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(predicate(), "Expected lifecycle transition", file: file, line: line)
    }

    private func seed(_ repository: MacCanonicalConversationRepository, count: Int = 1) throws {
        try repository.restoreBackupSnapshot([MacStoredConversation(
            conversation: MacConversation(id: "lifecycle-conversation", title: "Lifecycle", preview: "",
                updatedAt: .now, isPinned: false, isPersisted: true),
            messages: (1...count).map {
                MacChatMessage(role: .user, text: "Stored message \($0)",
                    createdAt: Date(timeIntervalSince1970: Double($0)))
            }
        )])
    }

    func testDetachedIdleWorkspaceCanBeReleasedAfterItsPersistentSubscriptionStarted() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try seed(repository)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        var model: MacWorkspaceModel? = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        weak var weakModel: MacWorkspaceModel?
        weakModel = model
        model?.selectedConversationID = "lifecycle-conversation"
        model?.presentationDidAttach()
        try await waitUntil { model?.selectedMessages.count == 1 }
        try repository.rename(conversationID: "lifecycle-conversation", title: "Subscription is live")
        try await waitUntil { model?.conversations.first?.title == "Subscription is live" }
        // Do not nil the selection: the presentation lifecycle itself must
        // release the polling task's reference to this Workspace.
        model?.presentationDidDetach()
        model = nil
        try await waitUntil { weakModel == nil }
        XCTAssertTrue(host.cancellationRequests.isEmpty)
    }

    func testDetachDoesNotCancelAnActiveRemoteRun() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let repository = MacCanonicalConversationRepository(store: port)
        weak var releasedModel: MacWorkspaceModel?
        do {
            let model = MacWorkspaceModel(canonicalRepository: repository,
                localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
            releasedModel = model
            model.presentationDidAttach()
            let conversation = try XCTUnwrap(model.selectedConversationID)
            model.executionHost = .remote
            model.updateRemoteHostConfigurationState(true)
            model.setFixtureRemoteModels([host.fixtureProfile])
            model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
            try await model.waitForModelConfigurationCommit()
            model.draft = "The presentation lifetime must not own execution cancellation"
            model.send()
            try await waitUntil { host.runContinuation != nil }
            let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
            model.presentationDidDetach()
            XCTAssertTrue(host.cancellationRequests.isEmpty)
            XCTAssertTrue(model.isRunning)
            XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, runID)
            model.presentationDidAttach()
            XCTAssertEqual(host.runRequests, 1, "Reattaching presentation cannot resubmit the Run")
            XCTAssertTrue(host.cancellationRequests.isEmpty)
            model.presentationDidDetach()
            host.runContinuation?.finish()
            try await waitUntil { !model.isRunning }
            XCTAssertNotNil(model.remoteRunRecovery)
            let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
                runID: runID, assistantMessageID: "\(runID):assistant")
            _ = try source.finish("The Run completed independently of presentation attachment.")
            try source.commitTerminal(into: port)
            try await waitUntil { model.remoteRunRecovery == nil && model.selectedTerminalTranscriptRevision > 0 }
            model.presentationDidDetach()
            XCTAssertTrue(host.cancellationRequests.isEmpty)
        }
        try await waitUntil { releasedModel == nil }
    }

    func testReattachDuringCommittedTerminalRevealRestoresWholeAnswerWithoutDuplicateNodes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        weak var releasedModel: MacWorkspaceModel?
        do {
            let model = MacWorkspaceModel(canonicalRepository: repository,
                localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
            releasedModel = model
            model.presentationDidAttach()
            model.executionHost = .remote
            model.updateRemoteHostConfigurationState(true)
            model.setFixtureRemoteModels([host.fixtureProfile])
            model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
            try await model.waitForModelConfigurationCommit()
            let conversation = try XCTUnwrap(model.selectedConversationID)
            model.draft = "Reveal this committed answer across presentation detachment"
            model.send()
            try await waitUntil { host.runContinuation != nil }
            let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
            let markers = (1...10).map { "REVEAL-SECTION-\($0)-END" }
            let answer = markers.map {
                "\($0): The canonical answer must survive presentation cancellation without losing or duplicating its text."
            }.joined(separator: "\n\n")
            // Author actual source records and receive the fixed terminal frontier
            // before sending its bounded presentation notification.
            let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
                runID: runID, assistantMessageID: "\(runID):assistant")
            let seal = try source.finish(answer)
            try source.commitTerminal(into: port)
            host.runContinuation?.yield(source.terminalEvent(seal: seal))
            host.runContinuation?.finish()
            // Detach while the committed frontier is being delivered. Journal
            // bodies are reader-owned, not incrementally copied into message.text.
            model.presentationDidDetach()
            await Task.yield()
            model.presentationDidAttach()
            try await waitUntil {
                model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference != nil && !model.isRunning
            }
            let assistant = try XCTUnwrap(model.selectedMessages.last(where: { $0.role == .assistant }))
            let reference = try XCTUnwrap(assistant.journalReference)
            let parts = try await MacJournalTextRead.parts(reference)
            XCTAssertFalse(parts.isEmpty)
            XCTAssertEqual(Set(parts.map(\.part_id)).count, parts.count)
            let rendered = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
            XCTAssertEqual(rendered, answer)
            for marker in markers {
                XCTAssertEqual(rendered.components(separatedBy: marker).count - 1, 1,
                    "Reattachment must preserve every original journal segment exactly once")
            }
            XCTAssertEqual(host.runRequests, 1)
            XCTAssertTrue(host.cancellationRequests.isEmpty)
            XCTAssertNil(model.remoteRunRecovery)
            XCTAssertNil(try port.activeRunIdentity(conversationID: conversation))
            model.presentationDidDetach()
        }
        try await waitUntil { releasedModel == nil }
    }

    func testReattachRefreshesCanonicalProjectionWithoutReplacingTheHistoryWindow() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try seed(repository, count: 130)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        model.selectedConversationID = "lifecycle-conversation"
        model.presentationDidAttach()
        try await waitUntil { model.hasEarlierStoredMessages && !model.selectedMessages.isEmpty }
        await model.loadEarlierStoredMessages()
        XCTAssertTrue(model.isViewingStoredHistory)
        let windowIdentity = model.transcriptWindowIdentity
        let messageIDs = model.selectedMessages.map(\.id)
        XCTAssertFalse(messageIDs.isEmpty)
        model.presentationDidDetach()
        try repository.rename(conversationID: "lifecycle-conversation", title: "Changed while detached")
        model.presentationDidAttach()
        try await waitUntil { model.conversations.first?.title == "Changed while detached" }
        XCTAssertTrue(model.isViewingStoredHistory)
        XCTAssertEqual(model.transcriptWindowIdentity, windowIdentity)
        XCTAssertEqual(model.selectedMessages.map(\.id), messageIDs)
        try repository.rename(conversationID: "lifecycle-conversation", title: "Continues observing")
        try await waitUntil { model.conversations.first?.title == "Continues observing" }
        XCTAssertEqual(model.transcriptWindowIdentity, windowIdentity)
        XCTAssertTrue(host.cancellationRequests.isEmpty)
        model.presentationDidDetach()
    }
}
