import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacCompletedRunSnapshotRetirementTests: XCTestCase {
    private enum Failure: Error { case unavailable }

    private func terminal(run: String = "run") -> MacCanonicalRunTerminal {
        let result = try! JSONDecoder().decode(MacRunResult.self,
            from: JSONSerialization.data(withJSONObject: macRunResultFixture("answered", run: run)))
        return .init(conversationID: "conversation", runID: run, assistantMessageID: "\(run):assistant",
              status: .sealed, result: result, resultDigest: String(repeating: "0", count: 64), completedAtMs: 42)
    }

    func testTransientReadFailureRetriesWithoutDuplicatingRelease() async throws {
        var reads = 0
        var releases = 0
        var waits = 0
        let result = try await MacCompletedRunSnapshotRetirement.retire(
            conversationID: "conversation", runID: "run",
            assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("run:assistant")),
            readTerminal: {
                reads += 1
                if reads == 1 { throw Failure.unavailable }
                return self.terminal()
            }, release: { _ in releases += 1 }, wait: { waits += 1 })
        XCTAssertTrue(result)
        XCTAssertEqual(reads, 2)
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(waits, 1)
    }

    func testReleaseFailureRereadsEvidenceAndRetryIsBounded() async {
        var reads = 0
        var releases = 0
        do {
            _ = try await MacCompletedRunSnapshotRetirement.retire(
                conversationID: "conversation", runID: "run",
                assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("run:assistant")),
                readTerminal: { reads += 1; return self.terminal() },
                release: { _ in releases += 1; throw Failure.unavailable }, wait: {})
            XCTFail("Expected bounded failure")
        } catch {}
        XCTAssertEqual(reads, 3)
        XCTAssertEqual(releases, 3)
    }

    func testMissingOrDifferentCanonicalOwnerNeverRetires() async throws {
        var releases = 0
        for value in [nil, terminal(run: "other"), terminal()] as [MacCanonicalRunTerminal?] {
            let result = try await MacCompletedRunSnapshotRetirement.retire(
                conversationID: "conversation", runID: "run",
                assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("wrong:assistant")),
                readTerminal: { value }, release: { _ in releases += 1 },
                wait: { XCTFail("Missing evidence is not a transient exception") })
            XCTAssertFalse(result)
        }
        XCTAssertEqual(releases, 0)
    }

    func testCancellationDuringRetryDoesNotReadOrReleaseAgain() async {
        var reads = 0
        var releases = 0
        do {
            _ = try await MacCompletedRunSnapshotRetirement.retire(
                conversationID: "conversation", runID: "run",
                assistantMessageID: XCTUnwrap(MacCanonicalMessageIdentity.stableID("run:assistant")),
                readTerminal: { reads += 1; throw Failure.unavailable },
                release: { _ in releases += 1 }, wait: { throw CancellationError() })
            XCTFail("Expected cancellation")
        } catch { XCTAssertTrue(error is CancellationError) }
        XCTAssertEqual(reads, 1)
        XCTAssertEqual(releases, 0)
    }
}
