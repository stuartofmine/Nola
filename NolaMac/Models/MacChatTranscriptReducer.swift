// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Pure transcript text reduction helpers shared by the macOS chat feature.

import Foundation

enum MacChatTranscriptReducer {
    private static var replaceableFailureMessages: [String] {
        [
            String(
                localized: "Nola could not produce a final answer. Please try again.",
                table: "Chat"
            ),
            String(localized: "Nola could not produce a final answer. Please try again."),
            "Nola could not produce a final answer. Please try again.",
            "Nola 未能生成最终回答，请重试。",
            "Nola 無法產生最終答案。請再試一次。",
        ]
    }

    static func isReplaceableFailurePlaceholder(_ value: String) -> Bool {
        let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines)
        let startupPrefix = String(localized: "Nola could not start the local agent: %@")
            .components(separatedBy: "%@").first ?? ""
        return replaceableFailureMessages.contains(where: {
            !$0.isEmpty && (normalized == $0 || normalized.hasPrefix($0))
        })
            || (!startupPrefix.isEmpty && normalized.hasPrefix(startupPrefix))
    }

    static func effectiveAssistantText(
        committed: String,
        buffered: String,
        projected: String = ""
    ) -> String {
        let pending = committed + buffered
        guard !projected.isEmpty else { return pending }
        guard !pending.isEmpty else { return projected }
        if projected.hasPrefix(pending) { return projected }
        if pending.hasPrefix(projected) { return pending }
        return projected
    }

    static func replacingFailurePlaceholderIfNeeded(
        current: String,
        incoming: String
    ) -> String {
        guard !incoming.isEmpty else { return current }
        if isReplaceableFailurePlaceholder(current) {
            return incoming
        }
        return current + incoming
    }

    @discardableResult
    static func upsertAssistant(
        text: String,
        messageID: UUID,
        messages: inout [MacChatMessage]
    ) -> Bool {
        guard !text.isEmpty else { return false }
        if let index = messages.firstIndex(where: { $0.id == messageID }) {
            guard messages[index].text != text else { return false }
            messages[index].text = text
            return true
        }
        messages.append(
            MacChatMessage(id: messageID, role: .assistant, text: text, createdAt: .now)
        )
        return true
    }

}
