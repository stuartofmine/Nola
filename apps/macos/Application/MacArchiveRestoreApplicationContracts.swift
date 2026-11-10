// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines bounded macOS Application clients for archive restore and content upload.

import Foundation

nonisolated struct MacContentUploadOwner: Codable, Equatable, Sendable {
    let conversationID: String
    let importID: String
    let partID: String

    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id"
        case importID = "import_id"
        case partID = "part_id"
    }
}

nonisolated struct MacContentUploadProgress: Decodable, Sendable {
    enum State: String, Decodable, Sendable {
        case receiving, sealed, cancelled, expired
    }

    let uploadID: String
    let owner: MacContentUploadOwner
    let state: State
    let committedUTF8Bytes: UInt64
    let expectedUTF8Bytes: UInt64

    enum CodingKeys: String, CodingKey {
        case owner, state
        case uploadID = "upload_id"
        case committedUTF8Bytes = "committed_utf8_bytes"
        case expectedUTF8Bytes = "expected_utf8_bytes"
    }
}

nonisolated struct MacContentUploadChunk: Decodable, Sendable {
    let uploadID: String
    let owner: MacContentUploadOwner
    let offsetBytes: UInt64
    let nextOffsetBytes: UInt64
    let text: String
    let endOfContent: Bool

    enum CodingKeys: String, CodingKey {
        case owner, text
        case uploadID = "upload_id"
        case offsetBytes = "offset_bytes"
        case nextOffsetBytes = "next_offset_bytes"
        case endOfContent = "end_of_content"
    }
}

/// Bounded upload use cases with resumable, byte-checked acknowledgements.
nonisolated protocol MacContentUploadApplicationClient: Sendable {
    func beginContentUpload(
        id: String,
        owner: MacContentUploadOwner,
        byteCount: UInt64
    ) throws -> MacContentUploadProgress
    func appendContentChunk(
        id: String,
        owner: MacContentUploadOwner,
        offset: UInt64,
        text: String
    ) throws -> MacContentUploadProgress
    func sealContentUpload(
        id: String,
        owner: MacContentUploadOwner
    ) throws -> MacContentUploadProgress
    func readContentUploadChunk(
        id: String,
        owner: MacContentUploadOwner,
        offset: UInt64
    ) throws -> MacContentUploadChunk
}

/// Atomic staged restore plus bounded uploads for independent original Part fields.
nonisolated protocol MacArchiveRestoreApplicationClient: MacContentUploadApplicationClient {
    func archiveDirectoryRevision() throws -> UInt64
    func beginArchiveRestore(_ input: MacBeginArchiveRestore) throws -> MacArchiveRestoreProgress
    func stageArchiveManifest(_ input: MacStageArchiveManifest) throws -> MacArchiveRestoreProgress
    func commitArchiveRestore(_ input: MacCommitArchiveRestore) throws -> MacArchiveRestoreResult
}

extension MacApplicationPort: MacArchiveRestoreApplicationClient {}
