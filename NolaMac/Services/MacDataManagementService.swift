// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exports, restores, sizes, and clears Nola for Mac application data.

import Foundation
import CryptoKit
import Observation
import UserNotifications

extension Notification.Name {
    static let nolaMacPersistentDataDidChange = Notification.Name("nola.mac.persistentDataDidChange")
}

nonisolated struct MacBackupSetting: Codable, Sendable {
    enum ValueType: String, Codable, Sendable {
        case string
        case integer
        case boolean
        case data
    }

    let key: String
    let type: ValueType
    let value: String
}

nonisolated struct MacBackupConversation: Codable, Sendable {
    let canonical: MacCanonicalConversation
    let toolSources: [MacArchivedToolSource]
    enum CodingKeys: String, CodingKey { case canonical, toolSources = "tool_sources" }

    init(_ canonical: MacCanonicalConversation, toolSources: [MacArchivedToolSource] = []) {
        self.canonical = canonical
        self.toolSources = toolSources
    }
}

nonisolated struct MacDataBackup: Codable, Sendable {
    let format: String
    let version: Int
    let exportedAt: Date
    let conversations: [MacBackupConversation]
    let activity: [MacActivityEntry]
    let automations: [MacAutomation]
    let settings: [MacBackupSetting]
    let productSettings: [MacCanonicalProductSetting]
    let webSources: [MacWebSource]
    let notifications: [MacNotificationInboxItem]

    init(
        format: String,
        version: Int,
        exportedAt: Date,
        conversations: [MacBackupConversation],
        activity: [MacActivityEntry],
        automations: [MacAutomation],
        settings: [MacBackupSetting],
        productSettings: [MacCanonicalProductSetting],
        webSources: [MacWebSource],
        notifications: [MacNotificationInboxItem]
    ) {
        self.format = format
        self.version = version
        self.exportedAt = exportedAt
        self.conversations = conversations
        self.activity = activity
        self.automations = automations
        self.settings = settings
        self.productSettings = productSettings
        self.webSources = webSources
        self.notifications = notifications
    }
}

nonisolated struct MacDataBackupPreview: Sendable {
    let exportedAt: Date
    let conversationCount: Int
    let activityCount: Int
    let automationCount: Int
    let notificationCount: Int
    let webSourceCount: Int
}

nonisolated enum MacDataBackupCodec {
    static let format = "com.gaojc.Nola.mac.backup"
    static let version = 3

    static func encode(_ backup: MacDataBackup) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        encoder.dateEncodingStrategy = .iso8601
        return try encoder.encode(backup)
    }

    static func encodeValue<Value: Encodable & Sendable>(_ value: Value) async throws -> Data {
        try await Task.detached(priority: .utility) {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
            encoder.dateEncodingStrategy = .iso8601
            return try encoder.encode(value)
        }.value
    }

    static func decode(_ data: Data) throws -> MacDataBackup {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let backup = try decoder.decode(MacDataBackup.self, from: data)
        guard backup.format == format else { throw MacDataManagementError.invalidFormat }
        guard backup.version == version else {
            throw MacDataManagementError.unsupportedVersion(backup.version)
        }
        return backup
    }
}

@MainActor
enum MacCanonicalBackupStream {
    /// Writes the existing backup format from original fields without encoding
    /// a complete conversation or large Part into either an IPC frame or buffer.
    static func appendPagedConversations(
        _ conversationIDs: [String],
        loadPage: (String, Int64?, Int64?) async throws -> MacConversationExportPage?,
        readRange: (String, String, String, MacExportBodyField, Int64, UInt64) async throws -> MacConversationExportRange,
        readDerivation: (MacCanonicalPartOwner, Int64) async throws -> MacStoredPartDerivation? = { _, _ in nil },
        readToolSources: (MacCanonicalPartOwner, Int64) async throws -> [MacToolSourceClaim],
        readToolSourceRange: (MacCanonicalPartOwner, String, Int64, UInt64) async throws -> MacToolSourceExportRange,
        append: (Data) async throws -> Void
    ) async throws {
        var conversationSeparator = false
        for conversationID in conversationIDs {
            try Task.checkCancellation()
            guard var page = try await loadPage(conversationID, nil, nil) else {
                throw MacDataManagementError.conversationUnavailable
            }
            let header = page.conversation
            guard header.id == conversationID, header.revision > 0 else {
                throw MacApplicationServiceError.invalidResponse
            }
            if conversationSeparator { try await append(Data(",".utf8)) }
            try await append(Data("{\"canonical\":".utf8))
            try await append(objectPrefix(header, removing: []))
            try await append(Data("\"messages\":[".utf8))
            var messageSeparator = false
            var afterOrdinal: Int64?
            while true {
                guard page.conversation == header, page.messages.count <= 1,
                      page.nextOrdinal == page.messages.last?.ordinal,
                      !page.hasMore || !page.messages.isEmpty else {
                    throw MacApplicationServiceError.invalidResponse
                }
                for message in page.messages {
                    guard message.ordinal > (afterOrdinal ?? 0), message.parts.count <= 512 else {
                        throw MacApplicationServiceError.invalidResponse
                    }
                    try Task.checkCancellation()
                    if messageSeparator { try await append(Data(",".utf8)) }
                    try await append(objectPrefix(message, removing: ["parts"]))
                    try await append(Data("\"parts\":[".utf8))
                    var partSeparator = false
                    for part in message.parts {
                        if partSeparator { try await append(Data(",".utf8)) }
                        try await append(objectPrefix(part, removing: ["content_text_utf8_bytes", "content_json_utf8_bytes", "has_derivation"]))
                        try await append(Data("\"content_text\":".utf8))
                        try await appendOriginalString(conversationID: conversationID, messageID: message.id,
                            partID: part.id, field: .contentText, revision: header.revision,
                            bytes: part.contentTextUTF8Bytes, readRange: readRange, append: append)
                        if let bytes = part.contentJSONUTF8Bytes {
                            try await append(Data(",\"content_json\":".utf8))
                            try await appendOriginalString(conversationID: conversationID, messageID: message.id,
                                partID: part.id, field: .contentJSON, revision: header.revision,
                                bytes: bytes, readRange: readRange, append: append)
                        }
                        if part.hasDerivation {
                            let owner = MacCanonicalPartOwner(conversationID: conversationID,
                                messageID: message.id, partID: part.id)
                            guard let stored = try await readDerivation(owner, header.revision),
                                  stored.derivation.target.owner == owner else {
                                throw MacApplicationServiceError.invalidResponse
                            }
                            let encoder = JSONEncoder()
                            encoder.outputFormatting = [.withoutEscapingSlashes]
                            let claim = try encoder.encode(stored.derivation)
                            guard claim.count <= 16 * 1024 else { throw MacApplicationServiceError.invalidResponse }
                            try await append(Data(",\"derivation\":".utf8))
                            try await append(claim)
                        }
                        try await append(Data("}".utf8))
                        partSeparator = true
                    }
                    try await append(Data("]}".utf8))
                    messageSeparator = true
                    afterOrdinal = message.ordinal
                }
                if !page.hasMore { break }
                guard let next = try await loadPage(conversationID, afterOrdinal, header.revision) else {
                    throw MacDataManagementError.conversationUnavailable
                }
                page = next
            }
            try await append(Data("]},\"tool_sources\":[".utf8))
            // A second bounded metadata pass keeps tool sources independent
            // of live conversation DTOs and never buffers all original bodies.
            var sourceSeparator = false
            var sourceIDs: Set<String> = []
            var sourceAfter: Int64?
            while true {
                guard let sourcePage = try await loadPage(conversationID, sourceAfter, header.revision),
                      sourcePage.conversation == header, sourcePage.messages.count <= 1,
                      sourcePage.nextOrdinal == sourcePage.messages.last?.ordinal,
                      !sourcePage.hasMore || !sourcePage.messages.isEmpty else { throw MacApplicationServiceError.invalidResponse }
                for message in sourcePage.messages {
                    guard message.ordinal > (sourceAfter ?? 0), message.parts.count <= 512 else { throw MacApplicationServiceError.invalidResponse }
                    for part in message.parts where part.kind == "tool" {
                        try Task.checkCancellation()
                        let owner = MacCanonicalPartOwner(conversationID: conversationID, messageID: message.id, partID: part.id)
                        let claims = try await readToolSources(owner, header.revision)
                        try MacToolSourceClaim.validate(claims, owner: owner)
                        for claim in claims {
                            guard sourceIDs.insert(claim.sourceItemID).inserted else { throw MacApplicationServiceError.invalidResponse }
                            if sourceSeparator { try await append(Data(",".utf8)) }
                            try await append(Data("{\"claim\":".utf8))
                            try await append(JSONEncoder().encode(claim))
                            try await append(Data(",\"body_json\":".utf8))
                            try await appendOriginalString(conversationID: conversationID, messageID: message.id,
                                partID: part.id, field: .contentJSON, revision: header.revision,
                                bytes: claim.body.totalUTF8Bytes, expectedSHA256: claim.bodySHA256,
                                readRange: { _, _, _, field, revision, offset in
                                    let range = try await readToolSourceRange(owner, claim.sourceItemID, revision, offset)
                                    guard range.owner == owner, range.sourceItemID == claim.sourceItemID,
                                          range.bodySHA256 == claim.bodySHA256 else { throw MacApplicationServiceError.invalidResponse }
                                    return MacConversationExportRange(conversationID: conversationID, messageID: message.id,
                                        partID: part.id, field: field, revision: range.revision, offsetBytes: range.offsetBytes,
                                        nextOffsetBytes: range.nextOffsetBytes, totalUTF8Bytes: range.totalUTF8Bytes,
                                        text: range.text, endOfContent: range.endOfContent)
                                }, append: append)
                            try await append(Data("}".utf8))
                            sourceSeparator = true
                        }
                    }
                    sourceAfter = message.ordinal
                }
                if !sourcePage.hasMore { break }
            }
            // Includes empty/metadata-only conversations, which made no body
            // requests. Never publish an archive whose source changed mid-read.
            guard let final = try await loadPage(conversationID, nil, header.revision),
                  final.conversation == header else { throw MacDataManagementError.conversationUnavailable }
            try await append(Data("]}".utf8))
            conversationSeparator = true
        }
    }

    private static func objectPrefix<T: Encodable>(_ value: T, removing keys: [String]) throws -> Data {
        let encoded = try JSONEncoder().encode(value)
        guard var object = try JSONSerialization.jsonObject(with: encoded) as? [String: Any] else {
            throw MacApplicationServiceError.invalidResponse
        }
        for key in keys { object.removeValue(forKey: key) }
        var prefix = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        prefix.removeLast()
        if !object.isEmpty { prefix.append(contentsOf: ",".utf8) }
        return prefix
    }

    private static func appendOriginalString(
        conversationID: String, messageID: String, partID: String, field: MacExportBodyField,
        revision: Int64, bytes: UInt64, expectedSHA256: String? = nil,
        readRange: (String, String, String, MacExportBodyField, Int64, UInt64) async throws -> MacConversationExportRange,
        append: (Data) async throws -> Void
    ) async throws {
        try await append(Data("\"".utf8))
        var offset: UInt64 = 0
        var digest = SHA256()
        while offset < bytes {
            try Task.checkCancellation()
            let range = try await readRange(conversationID, messageID, partID, field, revision, offset)
            let next = offset.addingReportingOverflow(UInt64(range.text.utf8.count))
            guard range.conversationID == conversationID, range.messageID == messageID,
                  range.partID == partID, range.field == field, range.revision == revision,
                  range.offsetBytes == offset, !next.overflow, range.nextOffsetBytes == next.partialValue,
                  range.totalUTF8Bytes == bytes, range.nextOffsetBytes > offset,
                  range.nextOffsetBytes <= bytes, range.text.utf8.count <= 32 * 1024,
                  range.endOfContent == (range.nextOffsetBytes == bytes) else {
                throw MacApplicationServiceError.invalidResponse
            }
            // Escaping each UTF-8-aligned fragment and removing only its outer
            // quotes preserves the exact original JSON string across chunks.
            let encoded = try JSONEncoder().encode(range.text)
            if expectedSHA256 != nil { digest.update(data: Data(range.text.utf8)) }
            try await append(Data(encoded.dropFirst().dropLast()))
            offset = range.nextOffsetBytes
        }
        if let expectedSHA256, digest.finalize().map({ String(format: "%02x", $0) }).joined() != expectedSHA256 {
            throw MacApplicationServiceError.invalidResponse
        }
        try await append(Data("\"".utf8))
    }

    static func appendConversations(
        _ conversationIDs: [String],
        load: (String) async throws -> MacBackupConversation?,
        append: (Data) async throws -> Void
    ) async throws {
        var writesSeparator = false
        for conversationID in conversationIDs {
            try Task.checkCancellation()
            let loaded = try await load(conversationID)
            try Task.checkCancellation()
            guard let canonical = loaded else {
                throw MacDataManagementError.conversationUnavailable
            }
            let encoded = try await MacDataBackupCodec.encodeValue(canonical)
            try Task.checkCancellation()
            if writesSeparator { try await append(Data(",".utf8)) }
            try await append(encoded)
            writesSeparator = true
        }
    }
}

private actor MacBackupStreamWriter {
    private var handle: FileHandle?

    init(url: URL) throws {
        try Data().write(to: url, options: .atomic)
        handle = try FileHandle(forWritingTo: url)
    }

    func append(_ data: Data) throws {
        guard let handle else { throw CocoaError(.fileWriteUnknown) }
        try handle.write(contentsOf: data)
    }

    func append(_ string: String) throws {
        try append(Data(string.utf8))
    }

    func finish() throws {
        guard let handle else { return }
        try handle.synchronize()
        try handle.close()
        self.handle = nil
    }

    func cancel() {
        try? handle?.close()
        handle = nil
    }
}

nonisolated enum MacDataManagementError: LocalizedError {
    case invalidFormat
    case unsupportedVersion(Int)
    case conversationUnavailable

    var errorDescription: String? {
        switch self {
        case .conversationUnavailable:
            return NSLocalizedString(
                "mac.data.error.conversation_unavailable",
                value: "A conversation became unavailable during export. No backup was published. Please try again.",
                comment: "Do not silently export an incomplete canonical backup"
            )
        case .invalidFormat:
            return NSLocalizedString(
                "mac.data.error.invalid_format",
                value: "This file is not a Nola for Mac backup.",
                comment: "Invalid Mac backup error"
            )
        case .unsupportedVersion(let version):
            return String(
                format: NSLocalizedString(
                    "mac.data.error.unsupported_version_format",
                    value: "Backup version %lld is not supported.",
                    comment: "Unsupported Mac backup version"
                ),
                Int64(version)
            )
        }
    }
}

@MainActor
@Observable
final class MacDataManagementService {
    var conversationCount = 0
    var activityCount = 0
    var automationCount = 0
    var notificationCount = 0
    var webSourceCount = 0
    var dataSize: Int64 = 0
    var isWorking = false
    var statusMessage: String?
    var errorMessage: String?

    private let canonicalRepository = MacCanonicalConversationRepository(store: MacApplicationPort.shared)
    private let activityStore = MacActivityStore.shared
    private let automationStore = MacAutomationStore.shared
    private let applicationPort = MacApplicationPort.shared
    private let fileManager = FileManager.default

    private let exportedSettings: [(String, MacBackupSetting.ValueType)] = [
        (MacRunNotificationPreferenceKeys.agentCompletion, .boolean),
    ]

    func refresh() async {
        do {
            conversationCount = try canonicalRepository.conversationIDs().count
        } catch {
            errorMessage = error.localizedDescription
        }
        activityCount = activityStore.entries.count
        automationCount = automationStore.automations.count
        notificationCount = await MacNotificationHistoryRepository.shared.items().count
        webSourceCount = MacWebSourceStore.shared.sources.count
        dataSize = await calculateDataSize()
    }

    func exportBackup(to url: URL) async throws {
        try Task.checkCancellation()
        let accessed = url.startAccessingSecurityScopedResource()
        defer { if accessed { url.stopAccessingSecurityScopedResource() } }

        let temporaryURL = url.deletingLastPathComponent().appendingPathComponent(
            ".\(url.lastPathComponent).\(UUID().uuidString).tmp"
        )
        try? fileManager.removeItem(at: temporaryURL)
        let writer = try MacBackupStreamWriter(url: temporaryURL)
        do {
            try await writer.append("{\"format\":")
            try await writer.append(MacDataBackupCodec.encodeValue(MacDataBackupCodec.format))
            try await writer.append(",\"version\":")
            try await writer.append(MacDataBackupCodec.encodeValue(MacDataBackupCodec.version))
            try await writer.append(",\"exportedAt\":")
            try await writer.append(MacDataBackupCodec.encodeValue(Date.now))
            try await writer.append(",\"conversations\":[")

            try await MacCanonicalBackupStream.appendPagedConversations(
                canonicalRepository.conversationIDs(),
                loadPage: { conversationID, afterOrdinal, revision in
                    let port = self.applicationPort
                    return try await Task.detached(priority: .utility) {
                        try port.loadConversationExportPage(conversationID, afterOrdinal: afterOrdinal,
                            expectedRevision: revision)
                    }.value
                },
                readRange: { conversationID, messageID, partID, field, revision, offset in
                    let port = self.applicationPort
                    return try await Task.detached(priority: .utility) {
                        try port.readConversationExportRange(conversationID, messageID: messageID,
                            partID: partID, field: field, expectedRevision: revision, offsetBytes: offset)
                    }.value
                },
                readDerivation: { owner, revision in
                    let port = self.applicationPort
                    return try await Task.detached(priority: .utility) {
                        try port.readConversationPartDerivation(owner, expectedConversationRevision: revision)
                    }.value
                },
                readToolSources: { owner, revision in
                    let port = self.applicationPort
                    return try await Task.detached(priority: .utility) {
                        try port.readConversationToolSources(owner, expectedConversationRevision: revision)
                    }.value
                },
                readToolSourceRange: { owner, sourceItemID, revision, offset in
                    let port = self.applicationPort
                    return try await Task.detached(priority: .utility) {
                        try port.readConversationToolSourceRange(owner, sourceItemID: sourceItemID,
                            expectedConversationRevision: revision, offsetBytes: offset)
                    }.value
                },
                append: { data in try await writer.append(data) }
            )

            try await writer.append("],\"activity\":")
            try await writer.append(MacDataBackupCodec.encodeValue(activityStore.entries))
            try await writer.append(",\"automations\":")
            try await writer.append(MacDataBackupCodec.encodeValue(automationStore.automations))
            try await writer.append(",\"settings\":")
            try await writer.append(MacDataBackupCodec.encodeValue(exportSettings()))
            try await writer.append(",\"productSettings\":")
            try await writer.append(MacDataBackupCodec.encodeValue(try exportProductSettings()))
            try await writer.append(",\"webSources\":")
            try await writer.append(MacDataBackupCodec.encodeValue(MacWebSourceStore.shared.sources))
            try await writer.append(",\"notifications\":")
            let notifications = await MacNotificationHistoryRepository.shared.items()
            try await writer.append(MacDataBackupCodec.encodeValue(notifications))
            try await writer.append("}")
            try await writer.finish()
            try Task.checkCancellation()

            if fileManager.fileExists(atPath: url.path) {
                _ = try fileManager.replaceItemAt(url, withItemAt: temporaryURL)
            } else {
                try fileManager.moveItem(at: temporaryURL, to: url)
            }
        } catch {
            await writer.cancel()
            try? fileManager.removeItem(at: temporaryURL)
            throw error
        }
    }

    func previewBackup(from url: URL) async throws -> MacDataBackupPreview {
        let accessed = url.startAccessingSecurityScopedResource()
        defer { if accessed { url.stopAccessingSecurityScopedResource() } }
        let backup = try await Task.detached(priority: .userInitiated) {
            let data = try Data(contentsOf: url, options: .mappedIfSafe)
            return try MacDataBackupCodec.decode(data)
        }.value
        return MacDataBackupPreview(
            exportedAt: backup.exportedAt,
            conversationCount: backup.conversations.count,
            activityCount: backup.activity.count,
            automationCount: backup.automations.count,
            notificationCount: backup.notifications.count,
            webSourceCount: backup.webSources.count
        )
    }

    func importBackup(from url: URL) async {
        guard !isWorking else { return }
        isWorking = true
        statusMessage = nil
        errorMessage = nil
        defer { isWorking = false }
        let accessed = url.startAccessingSecurityScopedResource()
        defer { if accessed { url.stopAccessingSecurityScopedResource() } }
        do {
            let backup = try await Task.detached(priority: .userInitiated) {
                let data = try Data(contentsOf: url, options: .mappedIfSafe)
                return try MacDataBackupCodec.decode(data)
            }.value

            try await canonicalRepository.restoreCanonicalBackupSnapshotAsync(
                backup.conversations
            )
            activityStore.restore(backup.activity)
            automationStore.replaceAll(backup.automations)
            restoreSettings(backup.settings)
            try restoreProductSettings(backup.productSettings)
            try MacPromptLibrary.service.reloadFromPersistence()
            MacWebSourceStore.shared.replaceAll(backup.webSources)
            await MacNotificationHistoryRepository.shared.replaceAll(backup.notifications)
            NotificationCenter.default.post(name: .nolaMacPersistentDataDidChange, object: nil)
            NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            statusMessage = NSLocalizedString("mac.data.status.restored", value: "Backup restored", comment: "Backup restored status")
            await refresh()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func clearApplicationData() async {
        guard !isWorking else { return }
        isWorking = true
        statusMessage = nil
        errorMessage = nil
        defer { isWorking = false }

        do {
            try canonicalRepository.clear()
            try MacPromptLibrary.service.replaceCustomTemplates([])
            try MacPromptLibrary.setDefaultTemplateID(nil)
        } catch {
            errorMessage = error.localizedDescription
            return
        }
        activityStore.clear()
        automationStore.replaceAll([])
        MacWebSourceStore.shared.replaceAll([])
        await MacNotificationHistoryRepository.shared.clear()
        let notificationCenter = UNUserNotificationCenter.current()
        notificationCenter.removeAllDeliveredNotifications()
        notificationCenter.removeAllPendingNotificationRequests()
        do {
            try MacAppleChatConfigurationStore.shared.save(baseURL: "", apiKey: "", model: "")
        } catch {
            errorMessage = error.localizedDescription
            return
        }
        do {
            try MacRemoteHostConfigurationStore.shared.save(url: "", token: "")
        } catch {
            errorMessage = error.localizedDescription
            return
        }
        for key in MacProductSettingKeys.backedUpKeys {
            do {
                try applicationPort.deleteProductSetting(key: key)
            } catch {
                errorMessage = error.localizedDescription
                return
            }
        }
        for (key, _) in exportedSettings {
            UserDefaults.standard.removeObject(forKey: key)
        }
        MacDebugLoggingConfiguration.reset()
        MacSecurityScopedBookmarkStore.shared.clearWorkspace()
        NotificationCenter.default.post(name: .nolaMacPersistentDataDidChange, object: nil)
        statusMessage = NSLocalizedString("mac.data.status.cleared", value: "Application data cleared", comment: "Application data cleared status")
        await LogFileStore.shared.clear()
        await refresh()
    }

    private func exportSettings() -> [MacBackupSetting] {
        let defaults = UserDefaults.standard
        return exportedSettings.compactMap { key, type in
            guard let object = defaults.object(forKey: key) else { return nil }
            let value: String
            switch type {
            case .string: value = object as? String ?? String(describing: object)
            case .integer: value = String((object as? NSNumber)?.intValue ?? 0)
            case .boolean: value = String((object as? NSNumber)?.boolValue ?? false)
            case .data:
                guard let data = object as? Data else { return nil }
                value = data.base64EncodedString()
            }
            return MacBackupSetting(key: key, type: type, value: value)
        }
    }

    private func exportProductSettings() throws -> [MacCanonicalProductSetting] {
        try MacProductSettingKeys.backedUpKeys.compactMap { key in
            try applicationPort.productSetting(key: key)
        }
    }

    private func restoreSettings(_ settings: [MacBackupSetting]) {
        let allowed = Dictionary(uniqueKeysWithValues: exportedSettings.map { ($0.0, $0.1) })
        let defaults = UserDefaults.standard
        for setting in settings {
            let expectedType = allowed[setting.key]
            guard expectedType == setting.type else { continue }
            switch setting.type {
            case .string: defaults.set(setting.value, forKey: setting.key)
            case .integer: defaults.set(Int(setting.value) ?? 0, forKey: setting.key)
            case .boolean: defaults.set(setting.value == "true", forKey: setting.key)
            case .data:
                if let data = Data(base64Encoded: setting.value) {
                    defaults.set(data, forKey: setting.key)
                }
            }
        }
    }

    private func restoreProductSettings(_ settings: [MacCanonicalProductSetting]) throws {
        let allowed = Set(MacProductSettingKeys.backedUpKeys)
        for setting in settings where allowed.contains(setting.key) {
            _ = try applicationPort.putProductSetting(
                key: setting.key,
                valueJSON: setting.valueJSON,
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
            )
        }
    }

    private func calculateDataSize() async -> Int64 {
        let roots = [
            fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first,
            fileManager.urls(for: .cachesDirectory, in: .userDomainMask).first,
        ].compactMap { $0 }
        let logURL = LogFileStore.shared.logFileURL()
        return await Task.detached {
            var total: Int64 = 0
            for root in roots {
                let candidates = [root.appendingPathComponent("Nola", isDirectory: true)]
                for candidate in candidates {
                    guard let enumerator = FileManager.default.enumerator(
                        at: candidate,
                        includingPropertiesForKeys: [.fileSizeKey, .isRegularFileKey],
                        options: [.skipsHiddenFiles]
                    ) else { continue }
                    while let url = enumerator.nextObject() as? URL {
                        let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
                        if values?.isRegularFile == true { total += Int64(values?.fileSize ?? 0) }
                    }
                }
            }
            if let values = try? logURL.resourceValues(forKeys: [.fileSizeKey]) {
                total += Int64(values.fileSize ?? 0)
            }
            return total
        }.value
    }
}
