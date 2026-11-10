import Foundation

/// Owns projection subscriptions, not Run state or presentation content.
/// Replacement drains the previous observer before handing over ownership.
@MainActor
final class MacProjectionObservationCoordinator {
    struct Lease: Equatable {
        let conversationID: String
        fileprivate let id: UUID
    }

    private struct Entry {
        let lease: Lease
        let task: Task<Void, Never>
    }

    private var entries: [String: Entry] = [:]

    func isObserving(conversationID: String) -> Bool {
        guard let entry = entries[conversationID] else { return false }
        return !entry.task.isCancelled
    }

    @discardableResult
    func start(
        conversationID: String,
        operation: @escaping @MainActor (Lease) async -> Void
    ) -> Lease {
        let previous = entries[conversationID]?.task
        previous?.cancel()
        let lease = Lease(conversationID: conversationID, id: UUID())
        let task = Task { @MainActor [weak self] in
            // Cancellation is cooperative. Waiting prevents the old observer's
            // in-flight presentation work from racing its replacement.
            await previous?.value
            if !Task.isCancelled, self?.isCurrent(lease) == true {
                await operation(lease)
            }
            self?.release(lease)
        }
        entries[conversationID] = Entry(lease: lease, task: task)
        return lease
    }

    func isCurrent(_ lease: Lease) -> Bool {
        guard let entry = entries[lease.conversationID] else { return false }
        return entry.lease == lease && !entry.task.isCancelled
    }

    func cancel(conversationID: String) {
        // Keep the handle until completion so a later start can still drain it.
        entries[conversationID]?.task.cancel()
    }

    /// Detaches presentation only. This never cancels an Agent Run.
    /// Retain handles so a quick reattach drains cancelled observers first.
    func cancelAll(except conversationID: String? = nil) {
        for (id, entry) in entries where id != conversationID { entry.task.cancel() }
    }

    /// Drain only the captured leases: a reattached observer may already own
    /// the same conversation by the time one of these awaits completes.
    func cancelAllAndDrain() async {
        let detached = Array(entries.values)
        for entry in detached { entry.task.cancel() }
        for entry in detached {
            await entry.task.value
            release(entry.lease)
        }
    }

    /// Called by the owner handing off presentation, never from the observer
    /// being drained. Observers return normally instead of awaiting themselves.
    func cancelAndDrain(conversationID: String) async {
        guard let entry = entries[conversationID] else { return }
        entry.task.cancel()
        await entry.task.value
        release(entry.lease)
    }

    private func release(_ lease: Lease) {
        guard entries[lease.conversationID]?.lease == lease else { return }
        entries.removeValue(forKey: lease.conversationID)
    }
}
