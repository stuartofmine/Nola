// SPDX-License-Identifier: Apache-2.0
// A staged archive becomes visible only when Application commits the restore.
import Foundation

nonisolated struct MacBeginArchiveRestore: Encodable, Sendable {
    let restoreID: String
    let expectedDirectoryRevision: UInt64
    let createdAtMs: Int64
}

/// Archive bindings identify an original field explicitly. Transcript uploads
/// remain a separate text-only command contract.
nonisolated struct MacArchivePartUpload: Encodable, Sendable {
    let uploadID: String
    let messageID: String
    let partID: String
    let field: MacExportBodyField
    let expectedUTF8Bytes: UInt64

    init(_ upload: MacTranscriptPartUpload, field: MacExportBodyField) {
        uploadID = upload.uploadID
        messageID = upload.messageID
        partID = upload.partID
        self.field = field
        expectedUTF8Bytes = upload.expectedUTF8Bytes
    }
}

nonisolated enum MacArchiveManifestEntry: Encodable, Sendable {
    case profile(MacCanonicalCreateConversationInput)
    case partDerivation(MacPartDerivation)
    case toolSource(claim: MacToolSourceClaim, uploadID: String)
    case message(conversationID: String, message: MacCanonicalMessageInput, uploads: [MacArchivePartUpload])

    private enum CodingKeys: String, CodingKey {
        case kind, profile, conversationID, message, uploads, record, claim, uploadID
    }

    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .toolSource(let claim, let uploadID):
            try values.encode("tool_source", forKey: .kind)
            try values.encode(claim, forKey: .claim)
            try values.encode(uploadID, forKey: .uploadID)
        case .partDerivation(let record):
            try values.encode("part_derivation", forKey: .kind)
            try values.encode(record, forKey: .record)
        case .profile(let profile):
            try values.encode("profile", forKey: .kind)
            try values.encode(profile, forKey: .profile)
        case .message(let conversationID, let message, let uploads):
            try values.encode("message", forKey: .kind)
            try values.encode(conversationID, forKey: .conversationID)
            try values.encode(message, forKey: .message)
            try values.encode(uploads, forKey: .uploads)
        }
    }
}

nonisolated struct MacStageArchiveManifest: Encodable, Sendable {
    let restoreID: String
    let sequence: UInt64
    let entry: MacArchiveManifestEntry
}

nonisolated struct MacArchiveRestoreProgress: Decodable, Sendable {
    let restoreID: String
    let nextSequence: UInt64
    let manifestDigest: String
    let completed: Bool
    private enum CodingKeys: String, CodingKey {
        case restoreID = "restore_id"
        case nextSequence = "next_sequence"
        case manifestDigest = "manifest_digest"
        case completed
    }
}

nonisolated struct MacCommitArchiveRestore: Encodable, Sendable {
    let restoreID: String
    let expectedDirectoryRevision: UInt64
    let expectedEntryCount: UInt64
    let manifestDigest: String
    let committedAtMs: Int64
}

nonisolated struct MacArchiveRestoreResult: Decodable, Sendable {
    let restoreID: String
    let conversationCount: UInt64
    let messageCount: UInt64
    let directoryRevision: UInt64
    private enum CodingKeys: String, CodingKey {
        case restoreID = "restore_id"
        case conversationCount = "conversation_count"
        case messageCount = "message_count"
        case directoryRevision = "directory_revision"
    }
}

/// The source retains its original strings. Uploads and manifest encoding never
/// create an encoded copy of the complete archive or a large text Part.
nonisolated final class MacArchiveRestoreWriter {
    static func restore(
        _ conversations: [MacBackupConversation],
        port: any MacArchiveRestoreApplicationClient
    ) throws {
        try Task.checkCancellation()
        let writer = try MacArchiveRestoreWriter(port: port)
        for backup in conversations {
            let conversation = backup.canonical
            try writer.appendProfile(MacCanonicalCreateConversationInput(
                id: conversation.id, title: conversation.title, settingsJSON: conversation.settingsJSON,
                createdAtMs: conversation.createdAtMs, updatedAtMs: conversation.updatedAtMs
            ))
            for message in conversation.messages {
                try writer.appendMessage(messageInput(message), conversationID: conversation.id)
                for part in message.parts {
                    if let claim = part.derivation {
                        let owner = MacCanonicalPartOwner(conversationID: conversation.id,
                            messageID: message.id, partID: part.id)
                        guard claim.target.owner == owner else { throw MacApplicationServiceError.invalidResponse }
                        try writer.appendPartDerivation(claim)
                    }
                }
            }
            var sourceIDs: Set<String> = []
            for source in backup.toolSources {
                let owner = source.claim.owner
                guard owner.conversationID == conversation.id,
                      sourceIDs.insert(source.claim.sourceItemID).inserted,
                      conversation.messages.contains(where: { message in
                          message.id == owner.messageID && message.parts.contains(where: { $0.id == owner.partID && $0.kind == "tool" })
                      }) else { throw MacApplicationServiceError.invalidResponse }
                try writer.appendToolSource(source)
            }
        }
        try writer.commit()
    }

    static func messageInput(_ message: MacCanonicalConversationMessage) -> MacCanonicalMessageInput {
        MacCanonicalMessageInput(
            id: message.id, runID: message.runID, role: message.role, status: message.status,
            ordinal: message.ordinal, createdAtMs: message.createdAtMs, completedAtMs: message.completedAtMs,
            parts: message.parts.map { part in
                MacCanonicalMessagePartInput(
                    id: part.id, kind: part.kind, ordinal: part.ordinal, state: part.state,
                    contentText: part.contentText, contentJSON: part.contentJSON, contentDigest: part.contentDigest,
                    createdAtMs: part.createdAtMs,
                    updatedAtMs: part.updatedAtMs
                )
            }
        )
    }

    private let port: any MacArchiveRestoreApplicationClient
    private let restoreID: String
    private let directoryRevision: UInt64
    private var progress: MacArchiveRestoreProgress
    private var conversationCount: UInt64 = 0
    private var messageCount: UInt64 = 0

    init(port: any MacArchiveRestoreApplicationClient) throws {
        self.port = port
        restoreID = "archive-\(UUID().uuidString)"
        directoryRevision = try port.archiveDirectoryRevision()
        progress = try port.beginArchiveRestore(MacBeginArchiveRestore(
            restoreID: restoreID, expectedDirectoryRevision: directoryRevision,
            createdAtMs: Self.nowMilliseconds()
        ))
        guard progress.restoreID == restoreID, progress.nextSequence == 0, !progress.completed else {
            throw MacApplicationPortError.rejected("Archive staging returned an inconsistent identity.")
        }
    }

    func appendProfile(_ profile: MacCanonicalCreateConversationInput) throws {
        try stage(.profile(profile))
        conversationCount += 1
    }

    func appendMessage(_ message: MacCanonicalMessageInput, conversationID: String) throws {
        var uploads: [MacArchivePartUpload] = []
        var parts: [MacCanonicalMessagePartInput] = []
        for part in message.parts {
            let owner = MacContentUploadOwner(conversationID: conversationID, importID: restoreID, partID: part.id)
            let uploader = MacTranscriptBodyUploader(port: port)
            if !part.contentText.isEmpty {
                uploads.append(MacArchivePartUpload(try uploader.upload(
                    part.contentText, owner: owner, messageID: message.id, field: .contentText
                ), field: .contentText))
            }
            if let json = part.contentJSON, !json.isEmpty {
                uploads.append(MacArchivePartUpload(try uploader.upload(
                    json, owner: owner, messageID: message.id, field: .contentJSON
                ), field: .contentJSON))
            }
            parts.append(MacCanonicalMessagePartInput(
                id: part.id, kind: part.kind, ordinal: part.ordinal, state: part.state,
                contentText: "", contentJSON: part.contentJSON == "" ? "" : nil,
                contentDigest: part.contentDigest,
                createdAtMs: part.createdAtMs, updatedAtMs: part.updatedAtMs
            ))
        }
        try stage(.message(conversationID: conversationID, message: MacCanonicalMessageInput(
            id: message.id, runID: message.runID, role: message.role, status: message.status,
            ordinal: message.ordinal, createdAtMs: message.createdAtMs, completedAtMs: message.completedAtMs,
            parts: parts
        ), uploads: uploads))
        messageCount += 1
    }

    func appendPartDerivation(_ record: MacPartDerivation) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        guard try encoder.encode(record).count <= 16 * 1024 else {
            throw MacApplicationServiceError.invalidResponse
        }
        try stage(.partDerivation(record))
    }

    func appendToolSource(_ source: MacArchivedToolSource) throws {
        try source.validate()
        let owner = source.claim.owner
        let uploadOwner = MacContentUploadOwner(conversationID: owner.conversationID, importID: restoreID, partID: owner.partID)
        let upload = try MacTranscriptBodyUploader(port: port).upload(source.bodyJSON, owner: uploadOwner,
            messageID: owner.messageID, field: .contentJSON, sourceItemID: source.claim.sourceItemID)
        try stage(.toolSource(claim: source.claim, uploadID: upload.uploadID))
    }

    func commit() throws {
        try Task.checkCancellation()
        let result = try port.commitArchiveRestore(MacCommitArchiveRestore(
            restoreID: restoreID, expectedDirectoryRevision: directoryRevision,
            expectedEntryCount: progress.nextSequence, manifestDigest: progress.manifestDigest,
            committedAtMs: Self.nowMilliseconds()
        ))
        guard result.restoreID == restoreID, result.conversationCount == conversationCount,
              result.messageCount == messageCount, result.directoryRevision >= directoryRevision else {
            throw MacApplicationPortError.rejected("Archive commit returned inconsistent counts or identity.")
        }
    }

    private func stage(_ entry: MacArchiveManifestEntry) throws {
        try Task.checkCancellation()
        let next = try port.stageArchiveManifest(MacStageArchiveManifest(
            restoreID: restoreID, sequence: progress.nextSequence, entry: entry
        ))
        guard next.restoreID == restoreID, next.nextSequence == progress.nextSequence + 1,
              !next.completed, !next.manifestDigest.isEmpty else {
            throw MacApplicationPortError.rejected("Archive staging did not acknowledge the expected entry.")
        }
        progress = next
    }

    private static func nowMilliseconds() -> Int64 {
        Int64(Date().timeIntervalSince1970 * 1_000)
    }
}
