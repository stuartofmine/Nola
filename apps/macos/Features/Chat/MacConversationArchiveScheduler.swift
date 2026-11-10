// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns debounced archive-save tasks and their monotonic revisions.

import Foundation

@MainActor
final class MacConversationArchiveScheduler {
    private var tasks: [String: Task<Void, Never>] = [:]
    private var revisions: [String: UInt64] = [:]
    private var lastRevision: UInt64 = 0

    func schedule(
        conversationID: String,
        delay: Duration,
        operation: @escaping @MainActor (UInt64) -> Void
    ) {
        let revision = nextRevision(conversationID: conversationID)
        cancelPending(conversationID: conversationID)
        tasks[conversationID] = Task { @MainActor [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled, let self,
                  self.revisions[conversationID] == revision else { return }
            self.tasks[conversationID] = nil
            operation(revision)
        }
    }

    func isCurrent(conversationID: String, revision: UInt64) -> Bool {
        revisions[conversationID] == revision
    }

    func cancelPending(conversationID: String) {
        tasks.removeValue(forKey: conversationID)?.cancel()
    }

    @discardableResult
    func invalidate(conversationID: String) -> UInt64 {
        let revision = nextRevision(conversationID: conversationID)
        cancelPending(conversationID: conversationID)
        return revision
    }

    func forget(conversationID: String) {
        cancelPending(conversationID: conversationID)
        revisions[conversationID] = nil
    }

    private func nextRevision(conversationID: String) -> UInt64 {
        // Forgetting a conversation releases its tracking entry, not the
        // identity of outstanding callbacks. Reusing revision 1 after restore
        // would let an old save acknowledge the new conversation's edits.
        lastRevision += 1
        revisions[conversationID] = lastRevision
        return lastRevision
    }
}
