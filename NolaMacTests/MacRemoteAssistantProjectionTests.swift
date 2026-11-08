// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacRemoteAssistantProjectionTests: XCTestCase {
    func testRejectedTraceCannotCreateMessageOrChangeCallerResult() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: directory.appendingPathComponent("store.db"))),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        let conversation = try XCTUnwrap(model.selectedConversationID)
        func event(_ sequence: Int, run: String = "run", conversationID: String? = nil,
                   kind: String = "phase.changed", traceID: String = "trace") throws -> MacLocalAgentEvent {
            let trace = try XCTUnwrap(MacAgentTraceEnvelope.decode([
                "schema_version": 1, "trace_id": traceID, "run_id": run,
                "conversation_id": conversationID ?? conversation, "turn_id": "turn",
                "event_id": "event-\(sequence)", "sequence": sequence,
                "emitted_at": "2026-10-12T00:00:00Z", "kind": kind,
                "status": "failed", "visibility": "summary", "payload": ["phase": "compacting"],
            ]))
            return MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil,
                isError: true, trace: trace)
        }
        var response = "unchanged"
        var assembler = MacSemanticStreamAssembler(identitySeed: "admission")
        var succeeded = true
        func consume(_ event: MacLocalAgentEvent, generation: UInt64 = 0) async {
            await model.consumeTracedAgentEvent(event, prompt: "", conversationID: conversation,
                modelID: "", assistantMessageID: UUID(), generationID: generation,
                permissionUsesLocalRuntime: true, permissionAttachmentURLs: [],
                response: &response, assembler: &assembler, succeeded: &succeeded)
        }
        await consume(try event(1))
        let count = model.selectedMessages.count
        let context = model.contextUsed
        for rejected in [try event(2, run: "foreign", kind: "run.failed"),
                         try event(2, conversationID: "foreign", kind: "run.failed"),
                         try event(1, kind: "run.failed")] {
            await consume(rejected)
            XCTAssertTrue(succeeded)
            XCTAssertEqual(model.selectedMessages.count, count)
            XCTAssertEqual(model.contextUsed, context)
            XCTAssertEqual(response, "unchanged")
        }
        await consume(try event(2, kind: "run.failed"), generation: 99)
        XCTAssertTrue(succeeded)
        XCTAssertEqual(model.selectedMessages.count, count)
        await consume(try event(2, kind: "run.failed"))
        XCTAssertFalse(succeeded, "Rejected input must not consume the legal successor sequence")
        succeeded = true
        await consume(try event(1, run: "next-turn", traceID: "next-trace"))
        let afterSwitch = model.selectedMessages.count
        await consume(try event(3, kind: "run.failed"))
        XCTAssertTrue(succeeded, "A retired Turn cannot fail the current consumer")
        XCTAssertEqual(model.selectedMessages.count, afterSwitch)
        await consume(try event(2, run: "next-turn", kind: "run.failed", traceID: "next-trace"))
        XCTAssertFalse(succeeded)
    }

    private func open(_ port: MacApplicationPort, conversation: String, run: String) throws {
        _ = try port.prepareFixtureRemoteSubmission(.init(conversationID: conversation, conversationTitle: "Live",
            submissionID: run, sourceHostID: "fixture-host", clientID: "fixture-client", userMessageID: "\(run):user",
            userPartID: "\(run):user:text", userText: "Original request", userContentJSON: nil,
            assistantMessageID: "\(run):assistant", assistantPartID: "\(run):assistant:text",
            contextCheckpointID: nil, createdAtMs: 1))
    }

    func testRunningJournalSurvivesColdOpenWithoutEndingRunAndRejectsWrongHost() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("store.db")
        let port = try MacApplicationPort(databaseURL: database)
        let conversation = "live-conversation", run = "live-run"
        try open(port, conversation: conversation, run: run)
        let source = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant")
        let cursor = try source.append("Committed running text")
        try source.ingest(into: port, through: cursor)
        let revision = try port.loadConversationWindow(conversation).projection.revision
        try source.ingest(into: port, through: cursor)
        XCTAssertEqual(try port.loadConversationWindow(conversation).projection.revision, revision,
                       "Replayed committed records must be idempotent")
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, run)
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: conversation, runID: run))
        let cold = try MacApplicationPort(databaseURL: database)
        let window = try cold.loadConversationWindow(conversation)
        XCTAssertTrue(window.assistantJournalHeads.contains(cursor))
        let reference = MacJournalMessageReference(databaseURL: database, cursor: cursor)
        let text = try await MacJournalTextRead.markdown(reference, historicalFallback: "wrong fallback")
        XCTAssertEqual(text, "Committed running text")

        let foreign = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant", sourceHostID: "another-host")
        let foreignCursor = try foreign.append("Must not replace")
        XCTAssertThrowsError(try foreign.ingest(into: cold, through: foreignCursor))
        let unrelated = try MacAssistantJournalTestFixture(conversationID: conversation, runID: "another-run",
            assistantMessageID: "another-assistant")
        let unrelatedCursor = try unrelated.append("Another Run")
        XCTAssertThrowsError(try unrelated.ingest(into: cold, through: unrelatedCursor))
        XCTAssertEqual(try cold.loadConversationWindow(conversation).projection.revision, revision)
    }

    func testDetachedRunningJournalResumesWithoutStartingAnotherRunAndIgnoresLateHint() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("store.db")
        let port = try MacApplicationPort(databaseURL: database)
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        model.presentationDidAttach()
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        model.draft = "Continue a running answer"
        model.send()
        try await waitUntil { host.runContinuation != nil }
        let continuation = try XCTUnwrap(host.runContinuation)
        let run = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        let revision = model.selectedTerminalTranscriptRevision
        let text = (1...16).map { "Paragraph \($0) contains enough text to pause the live renderer safely.\n\n" }.joined()
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant")
        let cursor = try source.append(text)
        try source.ingest(into: port, through: cursor)
        continuation.yield(source.event(through: cursor))
        try await waitUntil {
            model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference?.cursor == cursor
        }
        model.presentationDidDetach()
        model.presentationDidAttach()
        try await waitUntil {
            model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference?.cursor == cursor
        }
        let reference = try XCTUnwrap(model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference)
        let restored = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
        XCTAssertEqual(restored, text)
        XCTAssertTrue(model.isRunning)
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, revision)
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, run)
        XCTAssertTrue(host.cancellationRequests.isEmpty)
        XCTAssertEqual(host.runRequests, 1)
        _ = try source.finish()
        let seal = try source.commitTerminal(into: port)
        continuation.yield(source.terminalEvent(seal: seal))
        continuation.finish()
        try await waitUntil { !model.isRunning }
        let terminalRevision = model.selectedTerminalTranscriptRevision
        let canonicalRevision = try port.loadConversationWindow(conversation).projection.revision
        let late = source.event(through: cursor)
        var response = text
        var assembler = MacSemanticStreamAssembler(existingText: text, identitySeed: "late-test")
        var succeeded = true
        await model.consumeTracedAgentEvent(late, prompt: "", conversationID: conversation,
            modelID: "", assistantMessageID: try XCTUnwrap(MacCanonicalMessageIdentity.stableID("\(run):assistant")),
            generationID: 1, permissionUsesLocalRuntime: false, permissionAttachmentURLs: [],
            response: &response, assembler: &assembler, succeeded: &succeeded)
        XCTAssertFalse(model.isRunning)
        XCTAssertNil(model.remoteRunRecovery)
        XCTAssertNil(model.remoteRecoveryError)
        XCTAssertTrue(succeeded)
        XCTAssertEqual(response, text)
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertEqual(try port.loadConversationWindow(conversation).projection.revision, canonicalRevision)
        XCTAssertEqual(try repository.canonicalRunTerminal(conversationID: conversation, runID: run)?.result.value.kind, .answered)
    }

    private func waitUntil(_ predicate: () -> Bool, file: StaticString = #filePath,
                           line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while !predicate() && Date() < deadline { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(predicate(), "Expected canonical projection transition", file: file, line: line)
    }
}
