// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements UnifiedStorageService logic in the iOS service layer.
// - Interfaces:
//   - UnifiedStorageService
//   - FileBasedUnifiedStorage
// - APIs:
//   - none
//
import Foundation

// Minimal unified storage service protocol and a file-backed implementation.
// This is a lightweight skeleton to be expanded with CoreData or other DB integration.

public protocol UnifiedStorageService {
    func saveProcessedEvent(_ event: ProcessedEvent) throws
    func fetchProcessedEvents(limit: Int?) throws -> [ProcessedEvent]
    func deleteProcessedEvent(withDedupeKey dedupeKey: String) throws
    func findProcessedEvent(byId id: String) throws -> ProcessedEvent?
    func saveDraftsByRequest(_ requestId: String, candidates: [SummaryItem]) throws
    func confirmDraft(requestId: String, candidateIndex: Int) throws -> ProcessedEvent
    func purgeIfOverQuota() throws
}

public final class FileBasedUnifiedStorage: UnifiedStorageService, @unchecked Sendable {
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private let fileURL: URL
    private let ioQueue = DispatchQueue(label: "com.nola.unifiedstorage.io", qos: .utility)

    // In-memory cache. Populated lazily on first read; kept in sync on every
    // write so we never pay the JSON decode cost more than once per process.
    // Accessed only on `ioQueue`, so no extra locking is needed.
    private var cache: [ProcessedEvent]?
    private var dedupeIndex: [String: Int] = [:]     // dedupeKey -> index into cache
    private var idIndex: [String: Int] = [:]          // id         -> index into cache

    public init(storageFileName: String = "processed_events.json") throws {
        let fm = FileManager.default
        let dir = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        self.fileURL = dir.appendingPathComponent(storageFileName)
        self.encoder.dateEncodingStrategy = .iso8601
        self.decoder.dateDecodingStrategy = .iso8601
    }

    // MARK: - Cache helpers (must be called on ioQueue)

    private func rebuildIndicesUnlocked(_ all: [ProcessedEvent]) {
        dedupeIndex.removeAll(keepingCapacity: true)
        idIndex.removeAll(keepingCapacity: true)
        dedupeIndex.reserveCapacity(all.count)
        idIndex.reserveCapacity(all.count)
        for (i, ev) in all.enumerated() {
            dedupeIndex[ev.dedupeKey] = i
            idIndex[ev.id] = i
        }
    }

    private func loadAllUnlocked() throws -> [ProcessedEvent] {
        if let cached = cache { return cached }
        let all: [ProcessedEvent]
        if FileManager.default.fileExists(atPath: fileURL.path) {
            let data = try Data(contentsOf: fileURL)
            all = try decoder.decode([ProcessedEvent].self, from: data)
        } else {
            all = []
        }
        cache = all
        rebuildIndicesUnlocked(all)
        return all
    }

    private func fetchProcessedEventsUnlocked(limit: Int?) throws -> [ProcessedEvent] {
        var all = try loadAllUnlocked()
        all.sort { (a, b) -> Bool in
            return a.priority > b.priority
        }
        if let l = limit, all.count > l { return Array(all.prefix(l)) }
        return all
    }

    private func findProcessedEventUnlocked(byId id: String) throws -> ProcessedEvent? {
        let all = try loadAllUnlocked()
        // O(1) lookup via id index.
        guard let i = idIndex[id], i < all.count else { return nil }
        return all[i]
    }

    private func saveAllUnlocked(_ all: [ProcessedEvent]) throws {
        let tmp = fileURL.appendingPathExtension("tmp")
        let data = try encoder.encode(all)
        try data.write(to: tmp, options: .atomic)

        // If no existing file, move the tmp file into place. If exists, attempt safe replacement.
        if FileManager.default.fileExists(atPath: fileURL.path) {
            do {
                _ = try FileManager.default.replaceItemAt(fileURL, withItemAt: tmp)
            } catch {
                // Fallback to move if replace fails for any reason
                if FileManager.default.fileExists(atPath: tmp.path) {
                    try? FileManager.default.removeItem(at: fileURL)
                    try FileManager.default.moveItem(at: tmp, to: fileURL)
                }
            }
        } else {
            try FileManager.default.moveItem(at: tmp, to: fileURL)
        }

        // Sync cache + indices with what we just persisted.
        cache = all
        rebuildIndicesUnlocked(all)
    }

    public func saveProcessedEvent(_ event: ProcessedEvent) throws {
        try ioQueue.sync {
            var all = try loadAllUnlocked()
            if let idx = dedupeIndex[event.dedupeKey], idx < all.count {
                all[idx] = event
            } else {
                all.append(event)
            }
            try saveAllUnlocked(all)
        }
    }

    public func deleteProcessedEvent(withDedupeKey dedupeKey: String) throws {
        try ioQueue.sync {
            var all = try loadAllUnlocked()
            guard let idx = dedupeIndex[dedupeKey], idx < all.count else { return }
            all.remove(at: idx)
            try saveAllUnlocked(all)
        }
    }

    public func fetchProcessedEvents(limit: Int? = nil) throws -> [ProcessedEvent] {
        try ioQueue.sync {
            try fetchProcessedEventsUnlocked(limit: limit)
        }
    }

    /// Async variant to avoid blocking the caller thread.
    /// Safe because all IO + decoding happens on `ioQueue`.
    public func fetchProcessedEventsAsync(limit: Int? = nil) async throws -> [ProcessedEvent] {
        try await withCheckedThrowingContinuation { continuation in
            ioQueue.async {
                do {
                    continuation.resume(returning: try self.fetchProcessedEventsUnlocked(limit: limit))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    public func findProcessedEvent(byId id: String) throws -> ProcessedEvent? {
        try ioQueue.sync {
            try findProcessedEventUnlocked(byId: id)
        }
    }

    public func findProcessedEventAsync(byId id: String) async throws -> ProcessedEvent? {
        try await withCheckedThrowingContinuation { continuation in
            ioQueue.async {
                do {
                    continuation.resume(returning: try self.findProcessedEventUnlocked(byId: id))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    // Simple candidate storage: writes a drafts file per request id.
    public func saveDraftsByRequest(_ requestId: String, candidates: [SummaryItem]) throws {
        try ioQueue.sync {
            let dir = fileURL.deletingLastPathComponent()
            let draftsURL = dir.appendingPathComponent("draft_\(requestId).json")
            let data = try encoder.encode(candidates)
            try data.write(to: draftsURL, options: .atomic)
        }
    }

    // Confirm draft: reads candidate file and persists chosen candidate as ProcessedEvent.
    public func confirmDraft(requestId: String, candidateIndex: Int) throws -> ProcessedEvent {
        try ioQueue.sync {
            let dir = fileURL.deletingLastPathComponent()
            let draftsURL = dir.appendingPathComponent("draft_\(requestId).json")
            let data = try Data(contentsOf: draftsURL)
            let candidates = try decoder.decode([SummaryItem].self, from: data)
            guard candidateIndex >= 0 && candidateIndex < candidates.count else {
                throw NSError(domain: "UnifiedStorage", code: 1, userInfo: [NSLocalizedDescriptionKey: L10n.string("unified_storage.error.invalid_candidate_index")])
            }
            let chosen = candidates[candidateIndex]
            let event = ProcessedEvent(
                id: UUID().uuidString,
                source: EventSource.reminder,
                sourceType: .manual,
                sourceId: requestId,
                title: chosen.text,
                date: nil,
                start: nil,
                due: nil,
                priority: chosen.priority,
                tags: nil,
                dedupeKey: chosen.dedupeKey,
                provenance: nil,
                metadata: [:],
                notes: nil,
                hash: chosen.text.hashValue
            )
            var all = try loadAllUnlocked()
            if let idx = dedupeIndex[event.dedupeKey], idx < all.count {
                all[idx] = event
            } else {
                all.append(event)
            }
            try saveAllUnlocked(all)
            return event
        }
    }

    public func purgeIfOverQuota() throws {
        // Placeholder: implement quota calculation and deletion policy.
    }
}
