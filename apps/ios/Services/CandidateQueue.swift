// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements CandidateQueue logic in the iOS service layer.
// - Interfaces:
//   - CandidateQueue
// - APIs:
//   - none
//
import Foundation

/// Simple file-backed persistent queue for candidate events.
public final class CandidateQueue {
    private let fileURL: URL
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private let lock = NSLock()

    public init(filename: String = "candidate_queue.json") throws {
        let fm = FileManager.default
        let dir = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        self.fileURL = dir.appendingPathComponent(filename)
    }

    private func withLock<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    /// Enqueue a batch of processed events with a generated request id.
    public func enqueue(_ events: [ProcessedEvent], requestId: String? = nil) {
        let id = requestId ?? UUID().uuidString
        withLock {
            var all = (try? self.loadAll()) ?? [:]
            all[id] = events
            try? self.saveAll(all)
        }
    }

    /// Drain all queued batches and remove them from storage.
    public func drainAll() -> [String: [ProcessedEvent]] {
        return withLock {
            let all = (try? self.loadAll()) ?? [:]
            if FileManager.default.fileExists(atPath: fileURL.path) {
                try? FileManager.default.removeItem(at: fileURL)
            }
            return all
        }
    }

    private func loadAll() throws -> [String: [ProcessedEvent]] {
        guard FileManager.default.fileExists(atPath: fileURL.path) else { return [:] }
        let data = try Data(contentsOf: fileURL)
        return try decoder.decode([String: [ProcessedEvent]].self, from: data)
    }

    private func saveAll(_ all: [String: [ProcessedEvent]]) throws {
        let data = try encoder.encode(all)
        try data.write(to: fileURL)
    }
}
