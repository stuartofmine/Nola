// SPDX-License-Identifier: Apache-2.0
// Device-local composer and browser presentation. Never grants Run authority.
import Foundation
import CryptoKit
import Darwin

nonisolated struct MacConversationDraftRecord: Codable, Sendable {
    struct Attachment: Codable, Sendable {
        let url: URL
        let text: String
        let method: String
        let byteCount: Int64
        let ready: Bool
    }
    let conversationID: String
    let text: String
    let attachments: [Attachment]
    var browserSnapshot: MacConversationBrowserSnapshot? = nil

    func validate() throws {
        guard !conversationID.isEmpty, text.utf8.count <= 4_000_000, attachments.count <= 100,
              attachments.reduce(text.utf8.count, { $0 + $1.text.utf8.count })
                + (try JSONEncoder().encode(browserSnapshot).count) <= 8_000_000,
              attachments.allSatisfy({ $0.url.isFileURL && $0.text.utf8.count <= 4_000_000 && $0.byteCount >= 0 }) else {
            throw MacApplicationPortError.invalidResponse
        }
    }
}

/// A serial queue preserves edit/delete order without writing on the main actor.
/// Files are local-only: never write drafts through sync-enabled product settings.
nonisolated final class MacConversationDraftRepository: @unchecked Sendable {
    private let directory: URL
    private static let queue = DispatchQueue(label: "nola.conversation-drafts", qos: .utility)
    private var queue: DispatchQueue { Self.queue }
    private struct Envelope: Codable {
        let conversationID: String
        let revision: Int64
        let deleted: Bool
        let draft: MacConversationDraftRecord?
    }
    private let pendingLock = NSLock()
    private final class PendingSave: @unchecked Sendable {
        var record: MacConversationDraftRecord
        var completions: [@Sendable (Bool) -> Void]
        init(_ record: MacConversationDraftRecord, _ completion: @escaping @Sendable (Bool) -> Void) {
            self.record = record
            self.completions = [completion]
        }
    }
    // Coalesce only work that has not started. Queue ordering and deletion
    // barriers remain authoritative; no already-running write is cancelled.
    private var pendingSaves: [String: PendingSave] = [:]
    // Accessed exclusively on queue; a second window cannot silently overwrite
    // an edit based on an older snapshot.
    private var revisions: [String: Int64] = [:]
    init(store: MacApplicationPort) {
        directory = store.databaseURL.standardizedFileURL.resolvingSymlinksInPath().appendingPathExtension("drafts")
    }

    private func file(_ id: String) -> URL {
        directory.appendingPathComponent(SHA256.hash(data: Data(id.utf8)).map { String(format: "%02x", $0) }.joined() + ".json")
    }
    /// flock makes the entire read/CAS/atomic-write transaction cross-process safe.
    private func locked<T>(_ body: () throws -> T) throws -> T {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
        var localDirectory = directory
        var values = URLResourceValues(); values.isExcludedFromBackup = true
        try localDirectory.setResourceValues(values)
        let descriptor = Darwin.open(directory.appendingPathComponent(".lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw MacApplicationPortError.invalidResponse }
        defer { Darwin.close(descriptor) }
        guard flock(descriptor, LOCK_EX) == 0 else { throw MacApplicationPortError.invalidResponse }
        defer { flock(descriptor, LOCK_UN) }
        return try body()
    }
    private func read(_ id: String) throws -> Envelope? {
        let url = file(id)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let value = try JSONDecoder().decode(Envelope.self, from: Data(contentsOf: url))
        guard value.conversationID == id, value.revision > 0,
              value.deleted ? value.draft == nil : value.draft?.conversationID == id else {
            throw MacApplicationPortError.invalidResponse
        }
        try value.draft?.validate()
        return value
    }
    private func write(_ value: Envelope) throws {
        let url = file(value.conversationID)
        try JSONEncoder().encode(value).write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }
    func load(_ id: String) async throws -> MacConversationDraftRecord? {
        try await withCheckedThrowingContinuation { continuation in
            pendingLock.lock()
            pendingSaves.removeValue(forKey: id)
            queue.async { [self] in
                do {
                    let record: MacConversationDraftRecord? = try locked {
                        guard let value = try read(id) else { revisions[id] = nil; return nil }
                        guard !value.deleted else { throw MacApplicationPortError.invalidResponse }
                        revisions[id] = value.revision
                        return value.draft
                    }
                    continuation.resume(returning: record)
                } catch { continuation.resume(throwing: error) }
            }
            pendingLock.unlock()
        }
    }
    func save(_ record: MacConversationDraftRecord, completion: @escaping @Sendable (Bool) -> Void) {
        pendingLock.lock()
        if let pending = pendingSaves[record.conversationID] {
            pending.record = record
            pending.completions.append(completion)
            pendingLock.unlock()
            return
        }
        let pending = PendingSave(record, completion)
        pendingSaves[record.conversationID] = pending
        queue.async { [self, pending] in
            pendingLock.lock()
            if pendingSaves[record.conversationID] === pending { pendingSaves.removeValue(forKey: record.conversationID) }
            let snapshot = pending.record
            let completions = pending.completions
            pendingLock.unlock()
            let succeeded: Bool
            do {
                try snapshot.validate()
                try locked {
                    let old = try read(snapshot.conversationID)
                    guard old?.deleted != true, old?.revision == revisions[snapshot.conversationID],
                          (old?.revision ?? 0) < Int64.max else { throw MacApplicationPortError.invalidResponse }
                    let revision = (old?.revision ?? 0) + 1
                    try write(Envelope(conversationID: snapshot.conversationID, revision: revision, deleted: false, draft: snapshot))
                    revisions[snapshot.conversationID] = revision
                }
                succeeded = true
            } catch { succeeded = false }
            completions.forEach { $0(succeeded) }
        }
        pendingLock.unlock()
    }
    func delete(_ id: String, completion: @escaping @Sendable (Bool) -> Void) {
        pendingLock.lock()
        // Detach the previous batch without removing its queued write. A new
        // save cannot merge backwards across this deletion fence.
        pendingSaves.removeValue(forKey: id)
        queue.async { [self] in
            do {
                // Retain only a tombstone. CAS then prevents an older window
                // (including one that never had a draft) from resurrecting it.
                try locked {
                    let revision = try read(id)?.revision ?? 0
                    guard revision < Int64.max else { throw MacApplicationPortError.invalidResponse }
                    try write(Envelope(conversationID: id, revision: revision + 1, deleted: true, draft: nil))
                }
                revisions[id] = nil
                completion(true)
            }
            catch { completion(false) }
        }
        pendingLock.unlock()
    }
}
