// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSUnknownEffectRecoveryTests: XCTestCase {
    @MainActor
    func testRetainedPermissionUnknownRecoveryIsPresentedWithoutExecutingWork() async throws {
        guard let source = ProcessInfo.processInfo.environment["NOLA_RECOVERY_SNAPSHOT"] else {
            throw XCTSkip("Requires the retained synthetic permission-recovery snapshot")
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("recovery-regression-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("fixture.sqlite")
        try FileManager.default.copyItem(atPath: source, toPath: database.path)
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let conversation = UUID(uuidString: "52237609-5B53-4FD8-A9AD-39B823E0B3BF")!
        let run = "b61f5bb9-0fa4-42d9-baff-2d1cddb55da2"
        let direct = try XCTUnwrap(runtime.inspectUnknownEffectRecovery(
            conversationID: conversation.uuidString, runID: run, assistantMessageID: run + ":assistant"))
        XCTAssertEqual(direct.targets.count, 1)
        let container = ServiceContainer(applicationRuntime: runtime)
        let manager = ChatManager(container: container, restorePersistedState: false)
        container.chatManager = manager
        manager.currentConversationId = conversation
        await manager.discoverUnknownEffectRecovery()
        let settled = Date().addingTimeInterval(2)
        while manager.unknownEffectRecoveryChecking && Date() < settled {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertNotNil(manager.unknownEffectRecovery,
            "Core has an exact unknown delivery; selection must expose its recovery decision")
        XCTAssertEqual(manager.unknownEffectRecovery?.runID, run)
        XCTAssertEqual(manager.unknownEffectRecovery?.phase, .inspected)
        XCTAssertFalse(manager.canBeginNewMessage)
    }

    @MainActor
    func testRetainedUnknownStopIsIdempotentAcrossReopenAndPreservesUnknownEvidence() throws {
        guard let source = ProcessInfo.processInfo.environment["NOLA_RECOVERY_SNAPSHOT"] else {
            throw XCTSkip("Requires the retained synthetic permission-recovery snapshot")
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("recovery-stop-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("fixture.sqlite")
        try FileManager.default.copyItem(atPath: source, toPath: database.path)
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let conversation = "52237609-5B53-4FD8-A9AD-39B823E0B3BF"
        let run = "b61f5bb9-0fa4-42d9-baff-2d1cddb55da2"
        let inspection = try XCTUnwrap(runtime.inspectUnknownEffectRecovery(
            conversationID: conversation, runID: run, assistantMessageID: run + ":assistant"))
        XCTAssertEqual(inspection.independent_pending_effects, 0)
        let decision = try inspection.confirmed(choice: .keepUnknownBlocked,
            reason: "Stop the isolated fixture while preserving its unknown outcome")
        let requestID = UUID().uuidString.lowercased()
        let reply = try runtime.resolveUnknownEffectRecovery(decision, requestID: requestID)
        XCTAssertEqual(reply.outcome, .unknownPreservedBlocked)
        XCTAssertEqual(try runtime.resolveUnknownEffectRecovery(decision, requestID: requestID).outcome,
            .unknownPreservedBlocked)
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(try reopened.resolveUnknownEffectRecovery(decision, requestID: requestID).outcome,
            .unknownPreservedBlocked)
        XCTAssertNil(try reopened.pendingUnknownEffectRecovery(conversationID: conversation))
        XCTAssertNil(try reopened.inspectUnknownEffectRecovery(conversationID: conversation,
            runID: run, assistantMessageID: run + ":assistant"))
        let identity = try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: run, assistantMessageID: run + ":assistant")
        let application = IOSCanonicalAgentApplication(store: reopened, descriptor: .init(
            conversationID: conversation, conversationTitle: "Fixture", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: run))
        let frozen = try application.loadExactRunInvocation()
        let seal = try XCTUnwrap(application.committedTerminalSeal(coreSessionID: XCTUnwrap(frozen.coreSessionID)))
        XCTAssertEqual(seal.result.value.kind, .blocked)
        guard case .object(let resultFields) = seal.result.value.payload,
              case .array(let refs)? = resultFields["blocking_fact_refs"] else {
            return XCTFail("A blocked result must retain its canonical blocking evidence")
        }
        XCTAssertFalse(refs.isEmpty)
        let conflicting = try inspection.confirmed(choice: .authorizeRiskRetry,
            reason: "Stale test decision must not change the closed owner", target: inspection.targets[0])
        let rejected = try reopened.resolveUnknownEffectRecovery(conflicting, requestID: UUID().uuidString.lowercased())
        XCTAssertTrue(rejected.outcome == .rejected || rejected.outcome == .conflict)
    }

    func testConfirmationCannotFollowSelectionOrFreshInspection() {
        let conversation = UUID()
        let inspection = UUID()
        func view(_ conversationID: UUID, _ inspectionID: UUID,
                  _ phase: IOSUnknownEffectRecoveryPresentation.Phase = .inspected) -> IOSUnknownEffectRecoveryPresentation {
            .init(conversationID: conversationID, inspectionID: inspectionID, runID: "run",
                assistantMessageID: "assistant", attemptIDs: ["attempt"], canRetry: true, canKeepBlocked: true, phase: phase)
        }
        let captured = view(conversation, inspection)
        XCTAssertTrue(captured.acceptsConfirmation(captured))
        XCTAssertFalse(view(UUID(), inspection).acceptsConfirmation(captured))
        XCTAssertFalse(view(conversation, UUID()).acceptsConfirmation(captured))
        XCTAssertFalse(view(conversation, inspection, .outcomePending).acceptsConfirmation(captured))
    }

    private let target = IOSUnknownEffectRecoveryTarget(effect_id: "effect", request_id: "request",
        attempt_id: "attempt", worker_generation: "worker", lease_owner: "owner", lease_token: "token",
        expected_attempt_revision: 3)

    private func inspection(independent: UInt64 = 0) -> IOSUnknownEffectRecoveryInspection {
        .init(conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant",
            invocation_config_digest: "digest", targets: [target], retry_allowed: true, independent_pending_effects: independent)
    }

    func testConfirmedDecisionFreezesFencesIdentityAndTime() throws {
        let decision = try inspection().confirmed(choice: .authorizeRiskRetry, reason: "I accept the risk",
            target: target, resolutionID: "resolution", nowMs: 42)
        let decoded = try JSONDecoder().decode(IOSUnknownEffectRecoveryDecision.self,
            from: JSONEncoder().encode(decision))
        XCTAssertEqual(decoded, decision)
        XCTAssertEqual(decoded.targets, [target])
        XCTAssertEqual(decoded.resolved_at_ms, 42)
        XCTAssertEqual(decoded.resolution_id, "resolution")
    }

    func testUnknownTargetAndIndependentWorkCannotBeAuthorizedAway() throws {
        XCTAssertThrowsError(try inspection().confirmed(choice: .authorizeRiskRetry, reason: "risk"))
        XCTAssertThrowsError(try inspection(independent: 1).confirmed(choice: .keepUnknownBlocked, reason: "stop"))
        XCTAssertThrowsError(try inspection().confirmed(choice: .keepUnknownBlocked, reason: " "))
        XCTAssertThrowsError(try inspection().confirmed(choice: .keepUnknownBlocked, reason: "stop", resolutionID: " bad"))
    }

    func testPendingEnvelopeRetainsOriginalInputNotFreshConfirmation() throws {
        let decision = try inspection().confirmed(choice: .authorizeRiskRetry, reason: "risk",
            target: target, resolutionID: "fixed-resolution", nowMs: 123)
        let input = try JSONSerialization.jsonObject(with: JSONEncoder().encode(decision))
        let envelope: [String: Any] = ["request_id": "fixed-request", "client_id": "ios-application",
            "protocol_version": IOSApplicationRuntime.expectedServiceProtocol, "command": [
            "type": "resolve_unknown_effect_recovery", "arguments": ["input": input]]]
        let first = try IOSApplicationRuntime.unknownEffectIntent(envelope)
        let persisted = try JSONSerialization.jsonObject(with:
            JSONSerialization.data(withJSONObject: envelope, options: [.sortedKeys])) as! [String: Any]
        XCTAssertEqual(try IOSApplicationRuntime.unknownEffectIntent(persisted), first)
        XCTAssertEqual(first?.requestID, "fixed-request")
        XCTAssertEqual(first?.decision.resolved_at_ms, 123)
        var foreign = envelope
        foreign["client_id"] = "remote-mirror"
        XCTAssertThrowsError(try IOSApplicationRuntime.unknownEffectIntent(foreign))
        XCTAssertNil(try IOSApplicationRuntime.unknownEffectIntent(["command": ["type": "other"]]))
    }

    func testReplyMustMatchExactDecisionAndChoice() throws {
        let decision = try inspection().confirmed(choice: .keepUnknownBlocked, reason: "stop",
            resolutionID: "resolution", nowMs: 1)
        try IOSUnknownEffectRecoveryReply(outcome: .unknownPreservedBlocked,
            resolution_id: "resolution", run_id: "run", code: nil).validate(for: decision)
        XCTAssertThrowsError(try IOSUnknownEffectRecoveryReply(outcome: .retryAuthorized,
            resolution_id: "resolution", run_id: "run", code: nil).validate(for: decision))
        XCTAssertThrowsError(try IOSUnknownEffectRecoveryReply(outcome: .unknownPreservedBlocked,
            resolution_id: "resolution", run_id: "other", code: nil).validate(for: decision))
    }
}
