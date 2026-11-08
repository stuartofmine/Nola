// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines chat messages and attachment values shared across the macOS chat feature.

import Foundation

nonisolated struct MacJournalMessageReference: Hashable, Sendable {
    let databaseURL: URL
    let cursor: MacAssistantJournalCursor
    var ownerKey: MacJournalOwnerKey {
        .init(databaseURL: databaseURL, identity: cursor.identity)
    }
}

/// Process-local provenance/state reuse key. Advancing the same journal does
/// not change ownership; changing stores, epochs, Runs or assistants does.
nonisolated struct MacJournalOwnerKey: Hashable, Sendable {
    let databaseURL: URL
    let identity: MacAssistantJournalIdentity
}

nonisolated struct MacDeferredMessageContent: Hashable, Codable, Sendable {
    let conversationID: String
    let messageID: String
    let parts: [MacCanonicalConversationWindow.DeferredTextPart]
}

/// Ephemeral canonical reading value. Equality observes all typed Parts; hashing
/// uses the canonical revision/identity, never UI UUIDs or an encoded archive.
nonisolated struct MacCanonicalPresentationSource: Hashable, Sendable {
    let databaseURL: URL
    let sourceOwner: AppleConversationSourceOwner
    func hash(into hasher: inout Hasher) {
        hasher.combine(databaseURL)
        switch sourceOwner {
        case .local: hasher.combine("local")
        case .remote(let hostID, let storeEpoch): hasher.combine("remote"); hasher.combine(hostID); hasher.combine(storeEpoch)
        case .unavailable: hasher.combine("unavailable")
        }
    }
}

nonisolated struct MacCanonicalReadingOwner: Hashable {
    let conversationID: String
    let messageID: String
    let source: MacCanonicalPresentationSource?
}

nonisolated struct MacCanonicalMessagePresentation: Hashable, Sendable {
    let message: MacCanonicalConversationProjectionV1.Message
    var source: MacCanonicalPresentationSource? = nil
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.message == rhs.message && lhs.source == rhs.source }
    func hash(into hasher: inout Hasher) {
        hasher.combine(message.messageID); hasher.combine(message.revision); hasher.combine(source)
    }
}

nonisolated struct MacChatMessage: Identifiable, Hashable, Codable, Sendable {
    enum Role: String, Hashable, Codable, Sendable {
        case user
        case assistant
        case system
    }

    enum DeliveryState: String, Hashable, Codable, Sendable {
        case running
        case completed
        case failed
        case cancelled
    }

    let id: UUID
    let role: Role
    var text: String
    let createdAt: Date
    var attachmentNames: [String] = []
    var attachmentURLs: [URL] = []
    var attachmentMetadata: [MacCorePartArtifact]? = nil
    var attachmentPartIDs: [String]? = nil
    var presentation: MacAssistantPresentation?
    var deliveryState: DeliveryState? = nil
    var deferredContent: MacDeferredMessageContent? = nil
    var historicalDetails: [MacHistoricalPartSelection]? = nil
    var canonicalItemIDs: Set<String>? = nil
    // Exact Core message identity for presentation joins; never archive or
    // persist it through this UI model's explicit CodingKeys.
    var canonicalMessageID: String? = nil
    var canonicalPresentation: MacCanonicalMessagePresentation? = nil
    var uiSurfaceParts: [AppleUISurfacePart] = []
    var uiHistoryReferences: [AppleUISurfaceReference] = []
    var journalReference: MacJournalMessageReference? = nil

    // Store location is supplied by the live repository, never by an imported
    // message/archive. Journal display bindings are ephemeral read handles.
    private enum CodingKeys: String, CodingKey {
        case id, role, text, createdAt, attachmentNames, attachmentURLs, attachmentMetadata
        case presentation, deliveryState, deferredContent, historicalDetails, canonicalItemIDs
    }

    init(
        id: UUID = UUID(),
        role: Role,
        text: String,
        createdAt: Date,
        attachmentNames: [String] = [],
        attachmentURLs: [URL] = [],
        attachmentMetadata: [MacCorePartArtifact]? = nil,
        attachmentPartIDs: [String]? = nil,
        presentation: MacAssistantPresentation? = nil,
        deliveryState: DeliveryState? = nil,
        canonicalItemIDs: Set<String>? = nil,
        canonicalMessageID: String? = nil,
        canonicalPresentation: MacCanonicalMessagePresentation? = nil,
        uiSurfaceParts: [AppleUISurfacePart] = [],
        uiHistoryReferences: [AppleUISurfaceReference] = []
    ) {
        self.id = id
        self.role = role
        self.text = text
        self.createdAt = createdAt
        self.attachmentNames = attachmentNames
        self.attachmentURLs = attachmentURLs
        self.attachmentMetadata = attachmentMetadata
        self.attachmentPartIDs = attachmentPartIDs
        self.presentation = presentation
        self.deliveryState = deliveryState
        self.canonicalItemIDs = canonicalItemIDs
        self.canonicalMessageID = canonicalMessageID
        self.canonicalPresentation = canonicalPresentation
        self.uiSurfaceParts = uiSurfaceParts
        self.uiHistoryReferences = uiHistoryReferences
    }
}

struct MacConversationAttachmentReference: Identifiable, Hashable {
    let name: String
    let url: URL

    var id: String { url.standardizedFileURL.path }

    var fileType: String {
        let value = url.pathExtension.uppercased()
        return value.isEmpty ? String(localized: "File") : value
    }

    var locationLabel: String {
        let directory = (url.deletingLastPathComponent().path as NSString)
            .abbreviatingWithTildeInPath
        return "\(fileType) · \(directory)"
    }
}

struct MacAttachment: Identifiable, Hashable {
    enum Status: Hashable {
        case extracting
        case ready
        case failed
    }

    let id = UUID()
    let url: URL
    var status: Status = .extracting
    var extractedText = ""
    var extractionMethod = ""
    var byteCount: Int64 = 0
    var errorMessage: String?

    var name: String { url.lastPathComponent }
    var fileType: String {
        let value = url.pathExtension.uppercased()
        return value.isEmpty ? "FILE" : value
    }

    var prepared: MacPreparedAttachment {
        MacPreparedAttachment(
            name: name,
            url: url,
            extractedText: extractedText,
            extractionMethod: extractionMethod
        )
    }
}
