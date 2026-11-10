import XCTest
@testable import Nola

final class MacUnknownEffectRecoveryPresentationTests: XCTestCase {
    private func presentation(independent: UInt64 = 0) -> MacUnknownEffectRecoveryPresentation {
        let owner = MacLocalRunRecovery(conversationID: "conversation", expectedRunID: "run",
            canonicalAssistantMessageID: "assistant", assistantMessageID: UUID(), generationID: 3)
        let targets = ["first", "second"].map { id in
            MacUnknownEffectRecoveryTarget(effectID: id, requestID: "request-\(id)", attemptID: "attempt-\(id)",
                workerGeneration: "generation", leaseOwner: "owner", leaseToken: "private", expectedAttemptRevision: 2)
        }
        return .init(owner: owner, inspection: .init(conversationID: "conversation", runID: "run",
            assistantMessageID: "assistant", invocationConfigDigest: "digest", targets: targets,
            retryAllowed: true, independentPendingEffects: independent), pendingDecision: nil)
    }

    func testDecisionKeepsExactTargetsAndBlocksClosingIndependentWork() throws {
        let snapshot = presentation()
        let retry = try snapshot.decision(targetIndex: 1, reason: "I accept the risk")
        XCTAssertEqual(retry.targets, [snapshot.inspection.targets[1]])
        XCTAssertEqual(retry.choice.kind, .authorizeRiskRetry)
        XCTAssertEqual(try snapshot.decision(targetIndex: nil, reason: "Do not repeat").targets, snapshot.inspection.targets)
        XCTAssertThrowsError(try presentation(independent: 1).decision(targetIndex: nil, reason: "Do not repeat"))
        XCTAssertThrowsError(try snapshot.decision(targetIndex: 2, reason: "Out of range"))
        XCTAssertThrowsError(try snapshot.decision(targetIndex: 0, reason: " "))
    }

    func testOwnerAndPendingIntentCannotBeSubstituted() throws {
        let snapshot = presentation()
        XCTAssertTrue(snapshot.matches(snapshot.owner, selectedConversationID: "conversation", generationID: 3))
        XCTAssertFalse(snapshot.matches(snapshot.owner, selectedConversationID: "another", generationID: 3))
        XCTAssertFalse(snapshot.matches(snapshot.owner, selectedConversationID: "conversation", generationID: 4))
        let pending = try snapshot.decision(targetIndex: 0, reason: "Already confirmed")
        let frozen = MacUnknownEffectRecoveryPresentation(owner: snapshot.owner, inspection: snapshot.inspection,
            pendingDecision: pending)
        XCTAssertThrowsError(try frozen.decision(targetIndex: 1, reason: "Must not replace pending intent"))
    }
}
