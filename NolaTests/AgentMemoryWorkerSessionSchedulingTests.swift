// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies that deferred Memory Worker scans budget jobs rather than conversation queues.

import XCTest
@testable import Nola

final class AgentMemoryWorkerSessionSchedulingTests: XCTestCase {
    func testEmptyEarlierConversationsCannotStarveLaterClaimableJob() async throws {
        var visited: [(String, Int)] = []

        let report = try await processClaimableMemoryWorkerSessions(
            conversationIDs: ["first", "second", "third"],
            maximumJobCount: 2
        ) { conversationID, remaining in
            visited.append((conversationID, remaining))
            return conversationID == "third"
                ? IOSMemoryWorkerRunReport(attempted: 1, completed: 1)
                : IOSMemoryWorkerRunReport()
        }

        XCTAssertEqual(report, IOSMemoryWorkerRunReport(attempted: 1, completed: 1))
        XCTAssertEqual(visited.map(\.0), ["first", "second", "third"])
        XCTAssertEqual(visited.map(\.1), [2, 2, 2])
    }

    func testScanStopsAfterJobBudgetIsConsumed() async throws {
        var visited: [String] = []

        let report = try await processClaimableMemoryWorkerSessions(
            conversationIDs: ["first", "second", "third"],
            maximumJobCount: 2
        ) { conversationID, remaining in
            visited.append(conversationID)
            return IOSMemoryWorkerRunReport(attempted: remaining, completed: remaining)
        }

        XCTAssertEqual(report, IOSMemoryWorkerRunReport(attempted: 2, completed: 2))
        XCTAssertEqual(visited, ["first"])
    }

    func testFailedAttemptsConsumeTheCrossConversationBudget() async throws {
        var visited: [String] = []

        let report = try await processClaimableMemoryWorkerSessions(
            conversationIDs: ["first", "second", "third"],
            maximumJobCount: 2
        ) { conversationID, _ in
            visited.append(conversationID)
            return IOSMemoryWorkerRunReport(attempted: 1, retryScheduled: 1)
        }

        XCTAssertEqual(report, IOSMemoryWorkerRunReport(attempted: 2, retryScheduled: 2))
        XCTAssertEqual(visited, ["first", "second"])
    }
}
