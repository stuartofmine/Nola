// SPDX-License-Identifier: Apache-2.0
// Bounded, replay-checked original-text upload for transcript publication.
import CryptoKit
import Foundation

nonisolated struct MacTranscriptPartUpload: Codable, Equatable, Sendable {
    let uploadID: String
    let messageID: String
    let partID: String
    let expectedUTF8Bytes: UInt64
}

/// Iteration never builds a complete Data or encoded JSON copy of the body.
/// Chunk boundaries are UTF-8 scalar boundaries, including inside a grapheme.
nonisolated struct MacTranscriptTextChunks: Sequence {
    let text: String
    static let maximumBytes = 32 * 1_024

    struct Iterator: IteratorProtocol {
        let bytes: String.UTF8View
        var position: String.UTF8View.Index
        var offset: UInt64 = 0

        mutating func next() -> (offset: UInt64, text: String)? {
            guard position < bytes.endIndex else { return nil }
            var end = bytes.index(position, offsetBy: MacTranscriptTextChunks.maximumBytes,
                                  limitedBy: bytes.endIndex) ?? bytes.endIndex
            while end < bytes.endIndex && bytes[end] & 0xC0 == 0x80 {
                end = bytes.index(before: end)
            }
            let chunk = String(decoding: bytes[position..<end], as: UTF8.self)
            let start = offset
            offset += UInt64(chunk.utf8.count)
            position = end
            return (start, chunk)
        }
    }

    func makeIterator() -> Iterator {
        Iterator(bytes: text.utf8, position: text.utf8.startIndex)
    }
}

nonisolated struct MacTranscriptBodyUploader {
    let port: any MacContentUploadApplicationClient

    /// Stable identity is bound to the import/message/Part/field, not a random retry.
    /// Every previously committed byte is checked on resume; an offset alone
    /// must not cause a changed backup prefix to be silently accepted.
    func upload(_ text: String, owner: MacContentUploadOwner, messageID: String,
                field: MacExportBodyField = .contentText, sourceItemID: String? = nil) throws -> MacTranscriptPartUpload {
        try Task.checkCancellation()
        let total = UInt64(text.utf8.count)
        guard total > 0, total <= 256 * 1_024 * 1_024 else {
            throw failure("Transcript body exceeds the upload limit.")
        }
        var identityFields = [
            owner.conversationID, owner.importID, messageID, owner.partID, field.rawValue
        ]
        if let sourceItemID { identityFields.append(contentsOf: ["tool_source", sourceItemID]) }
        let identity = try JSONEncoder().encode(identityFields)
        let digest = SHA256.hash(data: identity).map { String(format: "%02x", $0) }.joined()
        let uploadID = "transcript-\(digest)"
        var progress = try port.beginContentUpload(id: uploadID, owner: owner, byteCount: total)
        try validate(progress, uploadID: uploadID, owner: owner, total: total)
        guard progress.state == .receiving || progress.state == .sealed else {
            throw failure("Transcript upload is no longer resumable.")
        }
        for chunk in MacTranscriptTextChunks(text: text) {
            try Task.checkCancellation()
            let end = chunk.offset + UInt64(chunk.text.utf8.count)
            if progress.state == .sealed {
                let stored = try port.readContentUploadChunk(id: uploadID, owner: owner, offset: chunk.offset)
                guard stored.uploadID == uploadID, stored.owner == owner,
                      stored.offsetBytes == chunk.offset, stored.nextOffsetBytes == end,
                      stored.text == chunk.text, stored.endOfContent == (end == total) else {
                    throw failure("Sealed transcript upload differs from the backup.")
                }
            } else {
                // Replaying an old chunk asks the Store to compare exact bytes.
                // Uncommitted chunks append at their original bounded offsets.
                progress = try port.appendContentChunk(
                    id: uploadID, owner: owner, offset: chunk.offset, text: chunk.text
                )
                try validate(progress, uploadID: uploadID, owner: owner, total: total)
                guard progress.state == .receiving, progress.committedUTF8Bytes >= end else {
                    throw failure("Transcript upload did not acknowledge the expected bytes.")
                }
            }
        }
        progress = try port.sealContentUpload(id: uploadID, owner: owner)
        try validate(progress, uploadID: uploadID, owner: owner, total: total)
        guard progress.state == .sealed, progress.committedUTF8Bytes == total else {
            throw failure("Transcript upload is incomplete.")
        }
        return MacTranscriptPartUpload(
            uploadID: uploadID, messageID: messageID, partID: owner.partID, expectedUTF8Bytes: total
        )
    }

    private func validate(_ progress: MacContentUploadProgress, uploadID: String,
                          owner: MacContentUploadOwner, total: UInt64) throws {
        guard progress.uploadID == uploadID, progress.owner == owner,
              progress.expectedUTF8Bytes == total, progress.committedUTF8Bytes <= total else {
            throw failure("Transcript upload returned an inconsistent identity or byte count.")
        }
    }

    private func failure(_ message: String) -> MacApplicationPortError {
        .rejected(message)
    }
}
