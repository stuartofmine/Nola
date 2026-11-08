// SPDX-License-Identifier: Apache-2.0
// Durable transport intent storage shared by Apple hosts; owners are platform validated.
import Foundation
import Darwin

nonisolated final class ApplePendingCommandJournal: @unchecked Sendable {
    // nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES, also enforced by the
    // mobile submit ABI. Check the *actual* sortedKeys encoding, not an estimate.
    static let maximumEnvelopeBytes = 16 * 1024 * 1024
    static let maximumPendingCommands = 64
    // Bound total encoded storage independently from per-command frame size.
    static let maximumJournalBytes = 2 * maximumEnvelopeBytes + 16
    private let url: URL
    private let lock = NSLock()
    private var storageUncertain = false
    private let validateOwners: ([String: [String: Any]]) throws -> Void

    enum Namespace: Equatable { case application, expertHost }
    init(databaseURL: URL, namespace: Namespace, validateOwners: @escaping ([String: [String: Any]]) throws -> Void) {
        self.validateOwners = validateOwners
        let suffix = namespace == .application ? "pending-commands.json" : "expert-host-pending-commands.json"
        url = databaseURL.appendingPathExtension(suffix)
    }

    /// Hold this scope across load → receipt lookup/send → save. Per-operation
    /// file locking cannot prevent another runtime overtaking an uncertain write.
    func beginTransaction() throws -> Transaction {
        lock.lock()
        var directoryFD: Int32 = -1
        var lockFD: Int32 = -1
        do {
            guard !storageUncertain else { throw Self.failure() }
            directoryFD = Darwin.open(url.deletingLastPathComponent().path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
            var info = stat()
            guard directoryFD >= 0, fstat(directoryFD, &info) == 0,
                  info.st_uid == geteuid(), info.st_mode & S_IFMT == S_IFDIR,
                  info.st_mode & 0o022 == 0 else { throw Self.failure() }
            lockFD = openat(directoryFD, url.lastPathComponent + ".lock", O_CREAT | O_RDWR | O_NOFOLLOW | O_CLOEXEC, 0o600)
            guard lockFD >= 0 else { throw Self.failure() }
            try Self.validatePrivateRegularFile(lockFD)
            while flock(lockFD, LOCK_EX) != 0 {
                if errno != EINTR { throw Self.failure() }
            }
            // A newly created instance must establish durability of a visible
            // predecessor's rename before interpreting it. Failure never means
            // "no pending command". No automatic deletion or format migration.
            let existing = openat(directoryFD, url.lastPathComponent, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
            if existing >= 0 {
                defer { Darwin.close(existing) }
                try Self.validatePrivateRegularFile(existing)
                guard fsync(existing) == 0 else { throw Self.failure() }
            } else if errno != ENOENT { throw Self.failure() }
            guard fsync(directoryFD) == 0 else { throw Self.failure() }
            return Transaction(owner: self, directoryFD: directoryFD, lockFD: lockFD, filename: url.lastPathComponent)
        } catch {
            if lockFD >= 0 { flock(lockFD, LOCK_UN); Darwin.close(lockFD) }
            if directoryFD >= 0 { Darwin.close(directoryFD) }
            lock.unlock()
            throw error
        }
    }

    /// Not Sendable: the scope and its runtime's NSLock stay on the same
    /// synchronous thread; no await may occur while Application is called.
    final class Transaction {
        private var owner: ApplePendingCommandJournal?
        private let directoryFD: Int32
        private let lockFD: Int32
        private let filename: String
        fileprivate init(owner: ApplePendingCommandJournal, directoryFD: Int32, lockFD: Int32, filename: String) {
            self.owner = owner
            self.directoryFD = directoryFD
            self.lockFD = lockFD
            self.filename = filename
        }
        deinit { close() }

        func close() {
            guard let journal = owner else { return }
            owner = nil
            flock(lockFD, LOCK_UN)
            Darwin.close(lockFD)
            Darwin.close(directoryFD)
            journal.lock.unlock()
        }

        func load() throws -> [String: [String: Any]] {
            guard let owner, !owner.storageUncertain else { throw ApplePendingCommandJournal.failure() }
            let fd = openat(directoryFD, filename, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
            guard fd >= 0 else {
                if errno == ENOENT { return [:] }
                throw ApplePendingCommandJournal.failure()
            }
            defer { Darwin.close(fd) }
            try ApplePendingCommandJournal.validatePrivateRegularFile(fd)
            var info = stat()
            guard fstat(fd, &info) == 0, info.st_size >= 0,
                  info.st_size <= Int64(ApplePendingCommandJournal.maximumJournalBytes) else {
                throw ApplePendingCommandJournal.failure()
            }
            let file = FileHandle(fileDescriptor: fd, closeOnDealloc: false)
            var bytes = Data()
            while bytes.count <= ApplePendingCommandJournal.maximumJournalBytes {
                let remaining = ApplePendingCommandJournal.maximumJournalBytes + 1 - bytes.count
                guard let chunk = try file.read(upToCount: min(64 * 1024, remaining)), !chunk.isEmpty else { break }
                bytes.append(chunk)
            }
            guard bytes.count <= ApplePendingCommandJournal.maximumJournalBytes,
                  let pending = try JSONSerialization.jsonObject(with: bytes) as? [String: [String: Any]] else {
                throw ApplePendingCommandJournal.failure()
            }
            try owner.validate(pending)
            return pending
        }

        func save(_ pending: [String: [String: Any]]) throws {
            guard let owner, !owner.storageUncertain else { throw ApplePendingCommandJournal.failure() }
            try owner.validate(pending)
            let bytes = try JSONSerialization.data(withJSONObject: pending, options: [.sortedKeys])
            guard bytes.count <= ApplePendingCommandJournal.maximumJournalBytes else { throw ApplePendingCommandJournal.failure() }
            do {
                let temporary = ".pending-\(UUID().uuidString).tmp"
                let fd = openat(directoryFD, temporary, O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW | O_CLOEXEC, 0o600)
                guard fd >= 0 else { throw ApplePendingCommandJournal.failure() }
                defer { Darwin.close(fd); unlinkat(directoryFD, temporary, 0) }
                guard fchmod(fd, 0o600) == 0 else { throw ApplePendingCommandJournal.failure() }
                try FileHandle(fileDescriptor: fd, closeOnDealloc: false).write(contentsOf: bytes)
                guard fsync(fd) == 0,
                      renameat(directoryFD, temporary, directoryFD, filename) == 0,
                      fsync(directoryFD) == 0 else { throw ApplePendingCommandJournal.failure() }
            } catch {
                // No send is allowed after save fails. This object cannot retry
                // with a replacement intent even if rename happened to succeed.
                owner.storageUncertain = true
                throw error
            }
        }
    }

    private func validate(_ pending: [String: [String: Any]]) throws {
        guard pending.count <= Self.maximumPendingCommands else { throw Self.failure() }
        for (id, envelope) in pending {
            guard !id.isEmpty, envelope["request_id"] as? String == id,
                  envelope["client_id"] is String, envelope["protocol_version"] is NSNumber,
                  envelope["command"] is [String: Any],
                  try JSONSerialization.data(withJSONObject: envelope, options: [.sortedKeys]).count <= Self.maximumEnvelopeBytes else {
                throw Self.failure()
            }

        }
        try validateOwners(pending)
    }

    private static func validatePrivateRegularFile(_ fd: Int32) throws {
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_uid == geteuid(), info.st_nlink == 1,
              info.st_mode & S_IFMT == S_IFREG, info.st_mode & 0o777 == 0o600 else { throw failure() }
    }

    private static func failure() -> NSError {
        NSError(domain: "NolaPendingCommandJournal", code: Int(errno), userInfo: [NSLocalizedDescriptionKey:
            "Pending command storage could not be confirmed. No replacement command was sent."])
    }
}
