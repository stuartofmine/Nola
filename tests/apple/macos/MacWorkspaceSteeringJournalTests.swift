// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacWorkspaceSteeringJournalTests: XCTestCase {
    private func waitUntil(_ predicate: () -> Bool, file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(4)
        while !predicate() && Date() < deadline { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(predicate(), "Expected canonical steering publication", file: file, line: line)
    }

    func testAttachedWindowReceivesPromotedRunThroughCanonicalProjectionObservation() async throws {
        let conversation = "observed-steering-\(UUID())"
        let run = "initial-\(UUID())"
        let source = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant", userText: "Initial request before steering")
        let first = try source.append("Initial answer")
        let port = try MacApplicationPort(databaseURL: source.databaseURL)
        let host = PermissionEnvelopeRemoteHost()
        let model = MacWorkspaceModel(
            canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        model.selectedConversationID = conversation
        model.presentationDidAttach()
        defer { model.presentationDidDetach() }
        try await waitUntil { model.selectedMessages.contains { $0.journalReference?.cursor == first } }
        XCTAssertEqual(model.selectedMessages.count, 2)

        // Publish through Application's actual admission/Host-response commands.
        // No reload, synthetic UI event, remote receipt or parent Run rebinding:
        // the attached window's production projection subscription must wake.
        let next = try source.publishSteeringSuccessor(
            userText: "Queued follow-up", answer: "Live promoted answer")
        try await waitUntil {
            model.selectedMessages.count == 4 && model.selectedMessages.contains {
                $0.journalReference?.cursor.identity.run_id == next
            }
        }
        XCTAssertNotEqual(next, run)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: conversation, runID: run))
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, next)
        XCTAssertEqual(model.selectedMessages.map { $0.role == .user }, [true, false, true, false])
        XCTAssertEqual(model.selectedMessages.filter { $0.role == .user }.map(\.text),
            ["Initial request before steering", "Queued follow-up"])
        let successor = try XCTUnwrap(model.selectedMessages.last?.journalReference)
        XCTAssertEqual(successor.cursor.identity.run_id, next)
        XCTAssertNotEqual(successor.cursor.identity.assistant_message_id, source.assistantMessageID)
        let answer = try await MacJournalTextRead.markdown(successor, historicalFallback: "")
        XCTAssertEqual(answer, "Live promoted answer")
        XCTAssertEqual(Set(model.selectedMessages.map(\.id)).count, 4)
        XCTAssertEqual(host.runRequests, 0)
        XCTAssertTrue(host.cancellationRequests.isEmpty)
    }

    func testActiveWindowShowsQueuedUserOnlyAfterSealedRunPromotion() async throws {
        let conversation = "steering-\(UUID())"
        let run = "initial-\(UUID())"
        let source = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant", userText: "Initial request before steering")
        // The local projection reads the same canonical database. A locally
        // queued turn must not acquire the predecessor's remote client binding.
        let port = try MacApplicationPort(databaseURL: source.databaseURL)
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        model.presentationDidAttach()
        model.selectedConversationID = conversation
        let firstHead = try source.append("Initial answer")
        model.reloadPersistentData()
        await model.loadSelectedConversationIfNeeded()
        try await waitUntil { model.selectedMessages.contains { $0.journalReference?.cursor == firstHead } }
        XCTAssertEqual(model.selectedMessages.count, 2)

        let successorRun = try source.publishSteeringSuccessor(
            userText: "Steering request after initial answer", answer: "Successor answer in the promoted Run")
        XCTAssertNotEqual(successorRun, run)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: conversation, runID: run))
        model.reloadPersistentData()
        await model.loadSelectedConversationIfNeeded()

        try await waitUntil {
            model.selectedMessages.contains { $0.role == .user && $0.text == "Steering request after initial answer" } &&
            model.selectedMessages.contains { $0.journalReference?.cursor.identity.run_id == successorRun }
        }
        var visibleText: [String] = []
        for message in model.selectedMessages {
            if let reference = message.journalReference {
                visibleText.append(try await MacJournalTextRead.markdown(reference, historicalFallback: ""))
            } else { visibleText.append(message.text) }
        }
        XCTAssertEqual(visibleText, [
            "Initial request before steering", "Initial answer",
            "Steering request after initial answer", "Successor answer in the promoted Run",
        ])
        XCTAssertEqual(model.selectedMessages.map { $0.role == .user }, [true, false, true, false])
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, successorRun)
        XCTAssertEqual(host.runRequests, 0, "Projection must not dispatch a new remote execution")
        XCTAssertTrue(host.cancellationRequests.isEmpty)
        XCTAssertEqual(Set(model.selectedMessages.map(\.id)).count, 4)
    }
}

// These helpers use only typed Application operations and the actual delivery
// lease. No checkpoint, aggregate, assistant owner or terminal fact is fabricated.
private extension MacAssistantJournalTestFixture {
    func steeringResult(_ command: [String: Any]) throws -> Any {
        try JSONSerialization.jsonObject(with: source.execute(commandData: JSONSerialization.data(withJSONObject: command)), options: [.fragmentsAllowed])
    }
    func steeringObject(_ command: [String: Any]) throws -> [String: Any] {
        try XCTUnwrap(steeringResult(command) as? [String: Any])
    }
    func publishSteeringSuccessor(userText: String, answer: String) throws -> String {
        let invocation = try steeringObject(["type": "load_exact_run_invocation", "conversation_id": conversationID,
            "expected_run_id": runID, "assistant_message_id": assistantMessageID])
        var digest = try XCTUnwrap(invocation["invocation_config_digest"] as? String)
        var executingRun = runID
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let loaded = try steeringObject(["type": "load_committed_host_requests", "run_id": runID])
        var first = try XCTUnwrap((loaded["requests"] as? [[String: Any]])?.first {
            ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream"
        })
        let leases = try XCTUnwrap(steeringResult(["type": "load_run_delivery_ownership", "run_id": runID]) as? [[String: Any]])
        XCTAssertEqual(leases.count, 1)
        first["delivery"] = try XCTUnwrap(leases.first)
        _ = try steeringObject(["type": "reduce_and_commit_active_submission", "input": [
            "conversation_id": conversationID, "run_id": runID, "transition_id": "fixture-queue-steering",
            "invocation_config_digest": digest, "committed_at_ms": now,
            "submission": ["type": "user_input", "submission_id": "fixture-steering-submission", "delivery": "next_turn",
                "submitted_at_epoch_ms": now, "items": [["type": "message", "id": "fixture-steering-user", "role": "user", "content": userText]]],
        ]])
        func accept(_ request: [String: Any], _ sequence: Int, _ payload: [String: Any]) throws {
            let keys: Set<String> = ["request_id", "attempt_id", "delegated_invocation_id", "effect_id", "host_id",
                "lease_id", "session_id", "turn_id", "catalog_revision", "availability_epoch", "delivery"]
            var response = request.filter { keys.contains($0.key) }
            response["sequence"] = sequence; response["payload"] = payload
            _ = try steeringObject(["type": "reduce_and_accept_host_response", "input": [
                "conversation_id": conversationID, "run_id": executingRun, "invocation_config_digest": digest,
                "response": response, "received_at_ms": now,
            ]])
        }
        let firstPart = try XCTUnwrap((first["payload"] as? [String: Any])?["text_part_id"] as? String)
        let previousSequence = try XCTUnwrap(first["resume_after_sequence"] as? NSNumber).intValue
        try accept(first, previousSequence + 1, ["type": "model_completed", "items": [["type": "message",
            "id": firstPart, "role": "assistant", "content": "Initial answer"]], "tool_calls": [], "usage": [:]])
        let ended = try steeringObject(["type": "load_committed_host_requests", "run_id": runID])
        XCTAssertEqual((ended["requests"] as? [[String: Any]])?.count, 0)
        let port = try MacApplicationPort(databaseURL: databaseURL)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: conversationID, runID: runID))
        let promoted = try steeringObject(["type": "promote_queued_assistant_turn", "input": [
            "conversation_id": conversationID, "created_at_ms": now]])
        executingRun = try XCTUnwrap((promoted["begin"] as? [String: Any])?["run_id"] as? String)
        digest = try XCTUnwrap(promoted["invocation_config_digest"] as? String)
        XCTAssertNotEqual(executingRun, runID)
        let claimed = try steeringObject(["type": "claim_committed_host_requests", "input": [
            "run_id": executingRun, "worker_generation": "steering-fixture-generation", "lease_owner": "steering-fixture-worker",
            "claimed_at_ms": now, "lease_expires_at_ms": now + 240_000, "limit": 64,
        ]])
        var successor = try XCTUnwrap((claimed["requests"] as? [[String: Any]])?.first {
            ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream"
        })
        let lease = successor["delivery"]
        try accept(successor, 1, ["type": "execution_started"])
        try accept(successor, 2, ["type": "model_attempt_started", "provider": "fixture", "model": "fixture-model", "reason": "initial"])
        let prepared = try steeringObject(["type": "load_committed_host_requests", "run_id": executingRun])
        let effectID = try XCTUnwrap(successor["effect_id"] as? String)
        successor = try XCTUnwrap((prepared["requests"] as? [[String: Any]])?.first { $0["effect_id"] as? String == effectID })
        successor["delivery"] = lease
        let part = try XCTUnwrap((successor["payload"] as? [String: Any])?["text_part_id"] as? String)
        try accept(successor, 3, ["type": "model_committed_delta", "part_id": part, "delta": answer])
        return executingRun
    }
}
