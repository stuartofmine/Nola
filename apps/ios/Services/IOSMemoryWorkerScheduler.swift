// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Fairly schedules bounded iOS memory-worker jobs across conversations.

import Foundation

/// Scans persisted conversation owners until the job budget is consumed.
/// The budget limits model work, not empty queues inspected, so later ready
/// conversations cannot starve behind empty ones.
func processClaimableMemoryWorkerSessions(
    conversationIDs: [String],
    maximumJobCount: Int,
    run: (_ conversationID: String, _ remainingJobCount: Int) async throws -> IOSMemoryWorkerRunReport
) async throws -> IOSMemoryWorkerRunReport {
    guard maximumJobCount > 0 else { return IOSMemoryWorkerRunReport() }
    var seen = Set<String>()
    var report = IOSMemoryWorkerRunReport()
    for rawConversationID in conversationIDs {
        guard report.attempted < maximumJobCount else { break }
        let conversationID = rawConversationID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !conversationID.isEmpty, seen.insert(conversationID).inserted else { continue }
        let remaining = maximumJobCount - report.attempted
        let sessionReport = try await run(conversationID, remaining)
        guard sessionReport.attempted >= 0,
              sessionReport.completed >= 0,
              sessionReport.retryScheduled >= 0,
              sessionReport.attempted <= remaining,
              sessionReport.completed + sessionReport.retryScheduled == sessionReport.attempted else {
            throw AgentCoreRuntimeError.core("Memory Worker returned an invalid run report.")
        }
        report = report + sessionReport
    }
    return report
}
