// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements SchedulerBackgroundTests logic in the project module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
import XCTest
@testable import Nola

// Removed substitute scheduler/background tests. Background execution must be covered
// by real app integration paths or targeted pure helpers, not local test doubles.

final class MemoryWorkerWakeSchedulingPolicyTests: XCTestCase {
    func testMemoryWorkerDeadlinePreemptsDailyAndScheduledWork() {
        let now = Date(timeIntervalSince1970: 1_000)
        let daily = now.addingTimeInterval(12 * 60 * 60)
        let scheduled = now.addingTimeInterval(4 * 60 * 60)
        let memory = now.addingTimeInterval(90)

        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextRefreshDate(
                now: now,
                daily: daily,
                scheduledAction: scheduled,
                memoryWorker: memory
            ),
            memory
        )
        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextProcessingDate(
                now: now,
                scheduledAction: scheduled,
                memoryWorker: memory
            ),
            memory
        )
    }

    func testOverdueMemoryWorkerDeadlineUsesMinimumSystemLead() {
        let now = Date(timeIntervalSince1970: 1_000)
        let expected = now.addingTimeInterval(MemoryWorkerWakeSchedulingPolicy.minimumLeadTime)

        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextRefreshDate(
                now: now,
                daily: now.addingTimeInterval(10_000),
                scheduledAction: nil,
                memoryWorker: now.addingTimeInterval(-30)
            ),
            expected
        )
    }

    func testFailedMemoryWorkerAttemptsRetryWithBoundedBudget() {
        let now = Date(timeIntervalSince1970: 1_000)
        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(retryScheduled: 1),
                jobBudget: 2,
                retryAttemptCount: 1
            ),
            now.addingTimeInterval(MemoryWorkerWakeSchedulingPolicy.retryDelay)
        )
        XCTAssertEqual(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(retryScheduled: 1),
                jobBudget: 2,
                retryAttemptCount: MemoryWorkerWakeSchedulingPolicy.maximumRetryAttempts
            ),
            now.addingTimeInterval(MemoryWorkerWakeSchedulingPolicy.retryDelay)
        )
        XCTAssertNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(retryScheduled: 1),
                jobBudget: 2,
                retryAttemptCount: MemoryWorkerWakeSchedulingPolicy.maximumRetryAttempts + 1
            )
        )
    }

    func testFullJobBudgetSchedulesAnotherPass() {
        let now = Date(timeIntervalSince1970: 1_000)
        XCTAssertNotNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(attempted: 2, completed: 2),
                jobBudget: 2,
                retryAttemptCount: 0
            )
        )
        XCTAssertNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(attempted: 1, completed: 1),
                jobBudget: 2,
                retryAttemptCount: 0
            )
        )
        XCTAssertNil(
            MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
                now: now,
                report: IOSMemoryWorkerRunReport(),
                jobBudget: 2,
                retryAttemptCount: 0
            )
        )
    }
}
