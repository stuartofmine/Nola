// SPDX-License-Identifier: Apache-2.0
import Foundation

/// Bounded cleanup of retained launch secrets, not execution recovery. Every
/// attempt re-reads exact canonical evidence; absence never means completed.
@MainActor
enum MacCompletedRunSnapshotRetirement {
    static func retire(
        conversationID: String, runID: String, assistantMessageID: UUID,
        readTerminal: () throws -> MacCanonicalRunTerminal?,
        release: (MacCanonicalRunTerminal) async throws -> Void,
        wait: () async throws -> Void = { try await Task.sleep(for: .milliseconds(200)) }
    ) async throws -> Bool {
        for attempt in 0..<3 {
            try Task.checkCancellation()
            do {
                guard let terminal = try readTerminal(),
                      terminal.conversationID == conversationID,
                      terminal.runID == runID,
                      MacCanonicalMessageIdentity.stableID(terminal.assistantMessageID) == assistantMessageID else {
                    return false
                }
                try await release(terminal)
                return true
            } catch {
                if error is CancellationError || attempt == 2 { throw error }
                try await wait()
            }
        }
        return false
    }
}
