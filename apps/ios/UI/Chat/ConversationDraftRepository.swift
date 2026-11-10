// SPDX-License-Identifier: Apache-2.0
// Device-local composer storage. Canonical messages and Run ledgers remain in Core.
import Foundation
import Darwin
import CryptoKit

nonisolated struct ConversationDraftSubmissionTicket: Codable, Equatable, Sendable {
    let sourceOwner: ConversationComposerDraftStore.Owner
    let conversationID: UUID
    let userMessageID: UUID
    let configurationRevision: Int64?
    let draftRevision: Int64
    let text: String
    let attachments: [ChatAttachmentDraft]
    let configuration: ConversationComposerDraftStore.ModelConfiguration?
}

nonisolated protocol ConversationDraftRepository: Sendable {
    var identity: UUID { get }
    func pendingOwner() throws -> UUID
    func newPendingOwner() throws -> UUID
    func load(_ owner: ConversationComposerDraftStore.Owner) throws -> ConversationComposerDraftStore.Draft?
    func save(_ draft: ConversationComposerDraftStore.Draft, owner: ConversationComposerDraftStore.Owner) throws
    func remove(_ owner: ConversationComposerDraftStore.Owner) throws
    func validateForSubmission(_ owner: ConversationComposerDraftStore.Owner) throws
    func retire(_ owner: ConversationComposerDraftStore.Owner) throws
    func stageSubmission(owner: ConversationComposerDraftStore.Owner, conversationID: UUID, userMessageID: UUID,
                         configurationRevision: Int64?, draft: ConversationComposerDraftStore.Draft) throws -> ConversationDraftSubmissionTicket
    func commitSubmission(_ ticket: ConversationDraftSubmissionTicket) throws
    func pendingSubmissions() throws -> [ConversationDraftSubmissionTicket]
}

nonisolated final class FileConversationDraftRepository: ConversationDraftRepository, @unchecked Sendable {
    let identity = UUID()
    static let writeFailed = Notification.Name("ConversationDraftRepository.writeFailed")
    private static let disk = DispatchQueue(label: "nola.conversation-drafts.disk", qos: .utility)
    private struct AttachmentReference: Codable {
        let id: UUID
        let sha256: String
        var filename: String { "\(id.uuidString)-\(sha256).json" }
    }
    private struct Manifest: Codable {
        let revision: Int64
        let deleted: Bool
        let text: String
        let attachments: [AttachmentReference]
        let configuration: ConversationComposerDraftStore.ModelConfiguration?
    }
    private struct SubmissionRecord: Codable { let ticket: ConversationDraftSubmissionTicket; let committed: Bool }
    private var revisions: [ConversationComposerDraftStore.Owner: Int64] = [:]
    private enum Failure: LocalizedError {
        case conflict, deleted
        var errorDescription: String? {
            switch self {
            case .conflict: return String(localized: "This draft changed in another window. Your local edits are kept; copy them before reopening the draft.")
            case .deleted: return String(localized: "This draft was deleted or submitted in another window. Your local edits are kept but cannot overwrite it.")
            }
        }
    }
    private let root: URL
    private let beforeManifestCommit: @Sendable () throws -> Void
    private var encoder: JSONEncoder { let value = JSONEncoder(); value.outputFormatting = .sortedKeys; return value }
    private var decoder: JSONDecoder { JSONDecoder() }
    init(root: URL, beforeManifestCommit: @escaping @Sendable () throws -> Void = {}) {
        self.root = root
        self.beforeManifestCommit = beforeManifestCommit
    }
    private func digest(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
    static func application() -> FileConversationDraftRepository {
        scoped(to: IOSApplicationRuntime.shared?.databaseURL ?? IOSApplicationRuntime.defaultDatabaseURL)
    }
    static func scoped(to databaseURL: URL) -> FileConversationDraftRepository {
        .init(root: databaseURL.standardizedFileURL.appendingPathExtension("drafts"))
    }
    private func folder(_ owner: ConversationComposerDraftStore.Owner) -> URL {
        let name: String
        switch owner {
        case .conversation(let id): name = "conversation-\(id.uuidString)"
        case .unsent(let id): name = "draft-\(id.uuidString)"
        }
        return root.appendingPathComponent(name, isDirectory: true)
    }
    private func write(_ data: Data, to url: URL) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        var parent = url.deletingLastPathComponent()
        var values = URLResourceValues(); values.isExcludedFromBackup = true
        try parent.setResourceValues(values)
    }
    private func locked<T>(_ body: () throws -> T) throws -> T {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let descriptor = Darwin.open(root.appendingPathComponent(".lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw CocoaError(.fileWriteUnknown) }
        defer { Darwin.close(descriptor) }
        guard flock(descriptor, LOCK_EX) == 0 else { throw CocoaError(.fileWriteUnknown) }
        defer { flock(descriptor, LOCK_UN) }
        return try body()
    }
    private func manifest(_ owner: ConversationComposerDraftStore.Owner) throws -> Manifest? {
        let url = folder(owner).appendingPathComponent("manifest.json")
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let value = try decoder.decode(Manifest.self, from: Data(contentsOf: url))
        guard value.revision > 0 else { throw CocoaError(.fileReadCorruptFile) }
        return value
    }
    private func submissionURL(_ id: UUID) -> URL {
        root.appendingPathComponent("submissions", isDirectory: true).appendingPathComponent("\(id.uuidString).json")
    }
    func stageSubmission(owner: ConversationComposerDraftStore.Owner, conversationID: UUID, userMessageID: UUID,
                         configurationRevision: Int64?, draft: ConversationComposerDraftStore.Draft) throws -> ConversationDraftSubmissionTicket {
        try Self.disk.sync { try locked {
            try validateRevision(owner)
            let ticket = ConversationDraftSubmissionTicket(sourceOwner: owner, conversationID: conversationID,
                userMessageID: userMessageID, configurationRevision: configurationRevision,
                draftRevision: try manifest(owner)?.revision ?? 0, text: draft.text,
                attachments: draft.attachments, configuration: draft.configuration)
            let url = submissionURL(userMessageID)
            if FileManager.default.fileExists(atPath: url.path) {
                guard try decoder.decode(SubmissionRecord.self, from: Data(contentsOf: url)).ticket == ticket else { throw Failure.conflict }
            } else { try write(encoder.encode(SubmissionRecord(ticket: ticket, committed: false)), to: url) }
            return ticket
        } }
    }
    func commitSubmission(_ ticket: ConversationDraftSubmissionTicket) throws {
        try Self.disk.sync { try locked {
            let url = submissionURL(ticket.userMessageID)
            let record = try decoder.decode(SubmissionRecord.self, from: Data(contentsOf: url))
            guard record.ticket == ticket else { throw Failure.conflict }
            if record.committed { return }
            let current = try manifest(ticket.sourceOwner)
            // New edits have a new local version. Commit the intent while
            // preserving those edits; never remove content by text matching.
            if current?.deleted != true && (current?.revision ?? 0) == ticket.draftRevision {
                revisions[ticket.sourceOwner] = current?.revision ?? 0
                switch ticket.sourceOwner {
                case .unsent: try removeFromDisk(ticket.sourceOwner)
                case .conversation: try saveToDisk(.init(), owner: ticket.sourceOwner)
                }
            }
            try write(encoder.encode(SubmissionRecord(ticket: ticket, committed: true)), to: url)
        } }
    }
    func pendingSubmissions() throws -> [ConversationDraftSubmissionTicket] {
        try Self.disk.sync { try locked {
            let folder = root.appendingPathComponent("submissions", isDirectory: true)
            guard FileManager.default.fileExists(atPath: folder.path) else { return [] }
            return try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
                .filter { $0.pathExtension == "json" }.compactMap { url in
                    let record = try decoder.decode(SubmissionRecord.self, from: Data(contentsOf: url))
                    return record.committed ? nil : record.ticket
                }
        } }
    }
    func pendingOwner() throws -> UUID {
        try Self.disk.sync { try locked { try pendingOwnerFromDisk() } }
    }
    private func pendingOwnerFromDisk() throws -> UUID {
        let url = root.appendingPathComponent("pending-owner.json")
        if FileManager.default.fileExists(atPath: url.path) {
            let id = try decoder.decode(UUID.self, from: Data(contentsOf: url))
            if try manifest(.unsent(id))?.deleted != true { return id }
            // A crash may commit the tombstone but not rotate the index.
        }
        return try newPendingOwnerFromDisk()
    }
    func newPendingOwner() throws -> UUID {
        try Self.disk.sync { try locked { try newPendingOwnerFromDisk() } }
    }
    private func newPendingOwnerFromDisk() throws -> UUID {
        let url = root.appendingPathComponent("pending-owner.json")
        let id = UUID()
        try write(encoder.encode(id), to: url)
        return id
    }
    /// List previews read the text manifest, without loading attachment payloads.
    func textPreview(_ owner: ConversationComposerDraftStore.Owner) throws -> String? {
        try Self.disk.sync { try locked {
            guard let entry = try manifest(owner), !entry.deleted else { return nil }
            return entry.text
        } }
    }
    func load(_ owner: ConversationComposerDraftStore.Owner) throws -> ConversationComposerDraftStore.Draft? {
        try Self.disk.sync { try locked { try loadFromDisk(owner) } }
    }
    private func loadFromDisk(_ owner: ConversationComposerDraftStore.Owner) throws -> ConversationComposerDraftStore.Draft? {
        let directory = folder(owner)
        guard let manifest = try manifest(owner) else { revisions[owner] = 0; return nil }
        guard !manifest.deleted else { throw Failure.deleted }
        revisions[owner] = manifest.revision
        // Only app-owned UUID filenames are resolvable. Original URL/URI grants
        // and parser credentials are intentionally not part of the manifest.
        var failed = false
        let attachments = manifest.attachments.compactMap { reference -> ChatAttachmentDraft? in
            do {
            guard reference.sha256.count == 64, reference.sha256.allSatisfy({ $0.isHexDigit && !$0.isUppercase }) else {
                throw CocoaError(.fileReadCorruptFile)
            }
            let data = try Data(contentsOf: directory.appendingPathComponent(reference.filename))
            guard digest(data) == reference.sha256 else { throw CocoaError(.fileReadCorruptFile) }
            let value = try decoder.decode(ChatAttachmentDraft.self, from: data)
            guard value.id == reference.id else { throw CocoaError(.fileReadCorruptFile) }
            return value
            } catch { failed = true; return nil }
        }
        return .init(text: manifest.text, attachments: attachments,
                     notice: failed ? NSLocalizedString("Some attachments could not be restored. Please attach them again.", comment: "Draft recovery") : nil,
                     configuration: manifest.configuration, restored: true)
    }
    func save(_ draft: ConversationComposerDraftStore.Draft, owner: ConversationComposerDraftStore.Owner) throws {
        // Capture an immutable value, preserving write/delete order across all
        // repository instances. No encoding or filesystem work on keystrokes.
        Self.disk.async { [self] in
            do { try locked { try saveToDisk(draft, owner: owner) } }
            catch {
                let detail = error.localizedDescription
                DispatchQueue.main.async {
                    NotificationCenter.default.post(name: Self.writeFailed, object: nil,
                        userInfo: ["repository": self.identity, "owner": owner, "detail": detail])
                }
            }
        }
    }
    private func saveToDisk(_ draft: ConversationComposerDraftStore.Draft, owner: ConversationComposerDraftStore.Owner) throws {
        let old = try manifest(owner)
        guard old?.deleted != true else { throw Failure.deleted }
        let expected = revisions[owner] ?? 0
        guard (old?.revision ?? 0) == expected, expected < Int64.max else { throw Failure.conflict }
        let directory = folder(owner)
        var references: [AttachmentReference] = []
        for attachment in draft.attachments {
            let data = try encoder.encode(attachment)
            let reference = AttachmentReference(id: attachment.id, sha256: digest(data))
            let url = directory.appendingPathComponent(reference.filename)
            if !FileManager.default.fileExists(atPath: url.path) { try write(data, to: url) }
            else if digest(try Data(contentsOf: url)) != reference.sha256 { throw CocoaError(.fileReadCorruptFile) }
            references.append(reference)
        }
        let configuration: ConversationComposerDraftStore.ModelConfiguration?
        if case .unsent = owner { configuration = draft.configuration } else { configuration = nil }
        try beforeManifestCommit()
        try write(encoder.encode(Manifest(revision: expected + 1, deleted: false, text: draft.text,
                  attachments: references, configuration: configuration)),
                  to: directory.appendingPathComponent("manifest.json"))
        revisions[owner] = expected + 1
    }
    func remove(_ owner: ConversationComposerDraftStore.Owner) throws {
        try Self.disk.sync { try locked { try removeFromDisk(owner) } }
    }
    func validateForSubmission(_ owner: ConversationComposerDraftStore.Owner) throws {
        try Self.disk.sync { try locked { try validateRevision(owner) } }
    }
    private func validateRevision(_ owner: ConversationComposerDraftStore.Owner) throws {
        let current = try manifest(owner)
        guard current?.deleted != true else { throw Failure.deleted }
        guard (current?.revision ?? 0) == (revisions[owner] ?? 0) else { throw Failure.conflict }
    }
    func retire(_ owner: ConversationComposerDraftStore.Owner) throws {
        try Self.disk.sync { try locked {
            try validateRevision(owner)
            try removeFromDisk(owner)
        } }
    }
    private func removeFromDisk(_ owner: ConversationComposerDraftStore.Owner) throws {
        let directory = folder(owner)
        let revision = try manifest(owner)?.revision ?? 0
        guard revision < Int64.max else { throw Failure.conflict }
        try write(encoder.encode(Manifest(revision: revision + 1, deleted: true, text: "", attachments: [], configuration: nil)),
            to: directory.appendingPathComponent("manifest.json"))
        revisions[owner] = nil
        if case .unsent(let id) = owner, try pendingOwnerFromDisk() == id {
            _ = try newPendingOwnerFromDisk()
        }
    }
}
