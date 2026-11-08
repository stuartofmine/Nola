// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns one remote task subscription, its conversation identity, and monotonic cursor.

import Foundation

@MainActor
final class IOSRemoteTaskObservation {
    private var task: Task<Void, Never>?
    private var conversationID: String?
    private var generation: UInt64 = 0
    private(set) var cursor: UInt64 = 0

    func cancel() {
        generation &+= 1
        task?.cancel()
        task = nil
    }

    func start(
        conversationID: String,
        operation: @escaping @MainActor (IOSRemoteTaskObservation) async -> Void
    ) {
        if self.conversationID == conversationID, task != nil {
            return
        }
        task?.cancel()
        generation &+= 1
        let observationGeneration = generation
        if self.conversationID != conversationID {
            cursor = 0
        }
        self.conversationID = conversationID
        task = Task { @MainActor [weak self] in
            guard let self else { return }
            await operation(self)
            finish(
                conversationID: conversationID,
                generation: observationGeneration
            )
        }
    }

    func advance(to candidate: UInt64, conversationID: String) {
        guard self.conversationID == conversationID else { return }
        cursor = max(cursor, candidate)
    }

    private func finish(conversationID: String, generation: UInt64) {
        guard self.conversationID == conversationID,
              self.generation == generation else { return }
        task = nil
        self.conversationID = nil
    }
}
