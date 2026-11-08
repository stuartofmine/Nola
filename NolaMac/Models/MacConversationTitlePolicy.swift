// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Conversation title normalization and presentation policy.

import Foundation

enum MacConversationTitlePolicy {
    static func isDefault(_ title: String) -> Bool {
        let normalized = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.isEmpty
            || normalized == "New conversation"
            || normalized == "New Conversation"
            || normalized == "新对话"
            || normalized == String(localized: "New Conversation")
            || isLegacySessionPlaceholder(normalized)
            || UUID(uuidString: normalized) != nil
    }

    static func isRuntimePlaceholder(_ title: String, conversationID: String) -> Bool {
        let normalized = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let canonicalID = conversationID.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeFallback = String(
            format: String(localized: "Conversation %@"),
            String(canonicalID.prefix(8))
        )
        return isDefault(normalized)
            || normalized == canonicalID
            || (!canonicalID.isEmpty && normalized == runtimeFallback)
    }

    static func restoredTitle(currentTitle: String, firstUserMessage: String?) -> String {
        guard isDefault(currentTitle), let firstUserMessage else { return currentTitle }
        let normalized = firstUserMessage
            .components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        guard !normalized.isEmpty else { return currentTitle }
        return String(normalized.prefix(36))
    }

    static func displayTitle(_ title: String, conversationID: String? = nil) -> String {
        let normalized = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let isPlaceholder = conversationID.map {
            isRuntimePlaceholder(normalized, conversationID: $0)
        } ?? isDefault(normalized)
        return isPlaceholder
            ? String(localized: "New Conversation")
            : title
    }

    static func automaticTitle(
        currentTitle: String,
        conversationID: String? = nil,
        userMessages: [String],
        latestSource: String
    ) -> String? {
        let candidate = normalizedTitle(latestSource)
        guard !candidate.isEmpty else { return nil }
        let isPlaceholder = conversationID.map {
            isRuntimePlaceholder(currentTitle, conversationID: $0)
        } ?? isDefault(currentTitle)
        if isPlaceholder {
            return candidate
        }
        guard userMessages.count > 1,
              let firstMessage = userMessages.first else { return nil }
        let firstTitle = normalizedTitle(firstMessage)
        let current = currentTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        let priorTitles = userMessages.dropLast().map { normalizedTitle($0) }
        guard priorTitles.contains(current),
              isLowSignalOpening(firstTitle),
              !isLowSignalOpening(candidate),
              candidate.count > current.count,
              candidate != current else { return nil }
        return candidate
    }

    private static func normalizedTitle(_ value: String) -> String {
        let normalized = value
            .components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        return String(normalized.prefix(36))
    }

    private static func isLowSignalOpening(_ value: String) -> Bool {
        let trimmed = value
            .lowercased()
            .trimmingCharacters(
                in: CharacterSet.whitespacesAndNewlines
                    .union(.punctuationCharacters)
                    .union(.symbols)
            )
        return [
            "hi", "hello", "hey", "你好", "您好", "嗨", "哈喽", "在吗",
            "hi nola", "hello nola", "你好 nola",
        ].contains(trimmed)
    }

    private static func isLegacySessionPlaceholder(_ normalized: String) -> Bool {
        let prefix = "Session "
        guard normalized.hasPrefix(prefix) else { return false }
        let suffix = String(normalized.dropFirst(prefix.count))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if UUID(uuidString: suffix) != nil { return true }

        // Earlier Mac builds exposed the first eight hexadecimal characters
        // of a UUID as `Session XXXXXXXX`. Keep recognizing that exact legacy
        // shape without swallowing user titles such as "Session planning".
        guard suffix.count >= 8, suffix.count <= 36 else { return false }
        let hexadecimal = CharacterSet(charactersIn: "0123456789abcdefABCDEF-")
        return suffix.unicodeScalars.allSatisfy(hexadecimal.contains)
            && suffix.unicodeScalars.filter { $0 != "-" }.count >= 8
    }
}
