// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacRunSubmissionPreparationTests: XCTestCase {
    func testKnowledgeDebounceBlocksSendWithoutConsumingMessageDraft() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        model.draft = "Keep this message"
        model.selectedKnowledgePointsText = "New knowledge before debounce"
        XCTAssertTrue(model.selectedKnowledgeIsSaving)
        XCTAssertFalse(model.selectedComposerState.canSend)
        XCTAssertNotNil(model.selectedComposerState.submissionBlockReason)
        model.send()
        XCTAssertEqual(model.draft, "Keep this message")
        XCTAssertNil(gate.captured)
        XCTAssertFalse(model.isRunning)
        try await waitUntil { !model.selectedKnowledgeIsSaving }
        XCTAssertTrue(model.selectedComposerState.canSend)
        XCTAssertEqual(model.draft, "Keep this message")
    }

    func testArchiveAcknowledgementCannotSurviveForgetAndRestore() {
        let scheduler = MacConversationArchiveScheduler()
        let oldSave = scheduler.invalidate(conversationID: "restored")
        scheduler.forget(conversationID: "restored")
        let restoredSave = scheduler.invalidate(conversationID: "restored")
        XCTAssertNotEqual(oldSave, restoredSave)
        XCTAssertFalse(scheduler.isCurrent(conversationID: "restored", revision: oldSave))
        XCTAssertTrue(scheduler.isCurrent(conversationID: "restored", revision: restoredSave))
        let otherSave = scheduler.invalidate(conversationID: "another")
        XCTAssertTrue(scheduler.isCurrent(conversationID: "restored", revision: restoredSave))
        XCTAssertFalse(scheduler.isCurrent(conversationID: "restored", revision: otherSave))
    }

    func testNewArchiveEditInvalidatesInFlightAcknowledgementBeforeDebounce() async throws {
        let scheduler = MacConversationArchiveScheduler()
        let oldSave = scheduler.invalidate(conversationID: "editing")
        let saved = expectation(description: "Only the newest debounced edit runs")
        scheduler.schedule(conversationID: "editing", delay: .milliseconds(10)) { _ in
            XCTFail("Superseded edit must not save")
        }
        scheduler.schedule(conversationID: "editing", delay: .milliseconds(10)) { revision in
            XCTAssertTrue(scheduler.isCurrent(conversationID: "editing", revision: revision))
            XCTAssertFalse(scheduler.isCurrent(conversationID: "editing", revision: oldSave))
            saved.fulfill()
        }
        XCTAssertFalse(scheduler.isCurrent(conversationID: "editing", revision: oldSave))
        await fulfillment(of: [saved], timeout: 3)
    }

    @MainActor
    private final class PreparationGate {
        var captured: MacRunSubmissionSnapshot?
        private var continuation: CheckedContinuation<Void, Never>?
        private(set) var returned = false

        func prepare(_ snapshot: MacRunSubmissionSnapshot) async throws -> [MacCoreContextComponent] {
            captured = snapshot
            // Deliberately finish even if the caller cancels while awaiting.
            // A late service result must not resurrect an abandoned submission.
            await withCheckedContinuation { continuation = $0 }
            returned = true
            return snapshot.contextComponents
        }

        func release() {
            continuation?.resume()
            continuation = nil
        }
    }

    private func waitUntil(_ predicate: () -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while !predicate() && Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(predicate(), "Expected submission boundary", file: file, line: line)
    }

    private func configureRemote(_ model: MacWorkspaceModel, host: PermissionEnvelopeRemoteHost) async throws {
        model.executionHost = .remote
        if let id = model.selectedConversationID { try await model.waitForModelConfigurationCommit(conversationID: id) }
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        if let id = model.selectedConversationID { try await model.waitForModelConfigurationCommit(conversationID: id) }
    }

    func testComposerExecutionAndStopStayWithTheirConversation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        let first = try XCTUnwrap(model.selectedConversationID)
        model.draft = "First conversation"
        model.send()
        try await waitUntil { gate.captured != nil }
        XCTAssertTrue(model.selectedComposerState.isVisiblyRunning)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .running)
        model.newConversation()
        let second = try XCTUnwrap(model.selectedConversationID)
        XCTAssertNotEqual(first, second)
        let secondStyle = model.style
        let secondPermission = model.permissionMode
        XCTAssertFalse(model.selectComposerReasoningPreference("automatic", conversationID: first),
            "A menu opened in another conversation cannot mutate the selected conversation")
        XCTAssertFalse(model.selectComposerPermissionMode(.cautious, conversationID: first))
        XCTAssertEqual(model.style, secondStyle)
        XCTAssertEqual(model.permissionMode, secondPermission)
        XCTAssertFalse(model.selectComposerReasoningPreference("automatic", conversationID: second),
            "Connected Host reasoning is owned by its model profile")
        try await model.waitForModelConfigurationCommit(conversationID: second)
        XCTAssertTrue(model.selectComposerPermissionMode(.allowAll, conversationID: second))
        model.draft = "Second conversation draft"
        XCTAssertFalse(model.selectedComposerState.canSend, "The owner's in-flight configuration receipt fences submission")
        try await model.waitForModelConfigurationCommit(conversationID: second)
        let idleComposer = model.selectedComposerState
        XCTAssertFalse(idleComposer.isRunning)
        XCTAssertFalse(idleComposer.isVisiblyRunning)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .ready,
            "Another conversation's active execution must not become this draft's status")
        XCTAssertFalse(model.isSelectedConversationLoading, "A new draft has no canonical history to load")
        XCTAssertTrue(idleComposer.canSend)
        XCTAssertNil(idleComposer.submissionBlockReason)
        XCTAssertFalse(model.stopComposerRun(conversationID: second))
        XCTAssertFalse(model.stopComposerRun(conversationID: first), "Stale action from a deselected composer is rejected")
        model.send()
        XCTAssertEqual(model.selectedConversationDisplayStatus, .waitingResource)
        XCTAssertEqual(model.draft, "Second conversation draft")
        XCTAssertEqual(host.runRequests, 0)
        model.selectedConversationID = first
        XCTAssertTrue(model.selectedComposerState.isVisiblyRunning)
        XCTAssertTrue(model.stopComposerRun(conversationID: first))
        try await waitUntil { !model.isRunning }
        model.selectedConversationID = second
        XCTAssertEqual(model.draft, "Second conversation draft")
        XCTAssertTrue(model.selectedComposerState.canSend)
        XCTAssertNil(model.selectedComposerState.submissionBlockReason)
    }

    func testStopDuringPreparationRetainsInputAndNeverDispatchesOrCancelsHostRun() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        let input = "  Keep my unsent request intact.  "
        let attachment = MacAttachment(url: root.appendingPathComponent("notes.txt"), status: .ready,
            extractedText: "Immutable attachment body", extractionMethod: "fixture")
        model.draft = input
        model.attachments = [attachment]
        let messages = model.selectedMessages.map(\.id)
        let terminalRevision = model.selectedTerminalTranscriptRevision
        model.send()
        try await waitUntil { gate.captured != nil }
        XCTAssertTrue(model.isRunning)
        XCTAssertEqual(host.runRequests, 0)
        model.stop()
        try await waitUntil { !model.isRunning }
        XCTAssertEqual(model.draft, input)
        XCTAssertEqual(model.attachments, [attachment])
        XCTAssertEqual(model.selectedMessages.map(\.id), messages)
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertNil(try port.activeRunIdentity(conversationID: conversation))
        XCTAssertNil(model.remoteRunRecovery)
        gate.release()
        try await waitUntil { gate.returned }
        await Task.yield()
        XCTAssertEqual(host.runRequests, 0, "Late preparation must not submit an abandoned Run")
        XCTAssertTrue(host.cancellationRequests.isEmpty, "There is no Host Run to cancel before dispatch")
        XCTAssertEqual(model.draft, input)
        XCTAssertEqual(model.attachments, [attachment])
        XCTAssertFalse(model.isRunning)
    }

    func testSubmissionFreezesTemplateWorkspaceAndExternalSessionBeforePreparationYields() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let gate = PreparationGate()
        defer { gate.release() }
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host,
            contextPreparation: { try await gate.prepare($0) }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        try await configureRemote(model, host: host)
        model.selectExecutionBackend(.fallback(id: "codex"), available: true)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        let index = try XCTUnwrap(model.conversations.firstIndex(where: { $0.id == conversation }))
        let template = try XCTUnwrap(model.promptTemplateService.templates.first(where: {
            model.promptTemplateService.readableTemplate(id: $0.id) != nil
        }))
        model.selectPromptTemplate(template.id)
        let selectedWorkspace = root.appendingPathComponent("original-workspace")
        model.workspaceURL = selectedWorkspace
        model.conversations[index].externalExecutionSessionIDs = ["codex": "original-session"]
        model.draft = "Original frozen request"
        model.memoryEnabledOverride = false
        model.send()
        try await waitUntil { gate.captured != nil }
        let snapshot = try XCTUnwrap(gate.captured)
        XCTAssertEqual(snapshot.workspaceSelection.selectedURL, selectedWorkspace)
        XCTAssertEqual(snapshot.externalSessionID, "original-session")
        XCTAssertTrue(snapshot.contextComponents.contains { $0.kind == "prompt_template" && $0.id == "prompt_template:\(template.id)" })
        model.selectPromptTemplate(nil)
        model.memoryEnabledOverride = true
        model.workspaceURL = root.appendingPathComponent("changed-workspace")
        model.conversations[index].externalExecutionSessionIDs = ["codex": "changed-session"]
        model.draft = "Next request typed while preparing"
        gate.release()
        try await waitUntil { host.runContinuation != nil }
        XCTAssertEqual(host.lastRunPrompt, "Original frozen request")
        XCTAssertEqual(host.lastRunMemoryEnabled, false, "Preparation must not reread the new UI memory preference")
        XCTAssertEqual(host.lastRunExternalSessionID, "original-session")
        XCTAssertEqual(host.lastRunContextComponents, snapshot.contextComponents)
        XCTAssertEqual(snapshot.workspaceSelection.selectedURL, selectedWorkspace)
        // Once dispatch has happened, stop is a real cancellation request; it
        // cannot pretend the submission never existed and restore the old draft.
        let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        model.stop()
        try await waitUntil { host.cancellationRequests.count == 1 }
        XCTAssertEqual(model.draft, "Next request typed while preparing")
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, runID)
        host.runContinuation?.finish()
        try await waitUntil { !model.isRunning }
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
            runID: runID, assistantMessageID: "\(runID):assistant")
        _ = try source.finish("The dispatched request completed before cancellation took effect.")
        try source.commitTerminal(into: port)
        try await waitUntil { model.remoteRunRecovery == nil && model.selectedTerminalTranscriptRevision > 0 }
    }
}
