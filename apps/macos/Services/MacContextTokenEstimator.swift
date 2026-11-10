// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides a deterministic, multilingual context token estimate for the macOS chat UI.

import Foundation

nonisolated enum MacContextTokenEstimator {
    static func estimate(
        messages: [MacChatMessage],
        draft: String,
        promptTemplate: String,
        userPreferences: String
    ) -> Int {
        estimateComponents(
            messageTexts: messages.map(\.text),
            draft: draft,
            promptTemplate: promptTemplate,
            userPreferences: userPreferences
        )
    }

    static func estimateComponents(
        messageTexts: [String],
        draft: String = "",
        promptTemplate: String = "",
        userPreferences: String = ""
    ) -> Int {
        var total = 3
        for text in messageTexts {
            total += 4 + estimateText(text)
        }

        if !promptTemplate.isEmpty {
            total += estimateText(promptTemplate)
        }
        if !userPreferences.isEmpty {
            total += estimateText(userPreferences)
        }

        let pendingDraft = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if !pendingDraft.isEmpty {
            total += 4 + estimateText(pendingDraft)
        }
        return total
    }

    static func estimateText(_ text: String) -> Int {
        guard !text.isEmpty else { return 0 }

        var denseScriptScalars = 0
        var otherUTF8Bytes = 0
        for scalar in text.unicodeScalars {
            if isDenseScript(scalar.value) {
                denseScriptScalars += 1
            } else {
                otherUTF8Bytes += String(scalar).utf8.count
            }
        }
        return denseScriptScalars + Int(ceil(Double(otherUTF8Bytes) / 4.0))
    }

    private static func isDenseScript(_ value: UInt32) -> Bool {
        switch value {
        case 0x3400...0x4DBF,
             0x4E00...0x9FFF,
             0xF900...0xFAFF,
             0x20000...0x2FA1F,
             0x3040...0x30FF,
             0x1100...0x11FF,
             0xAC00...0xD7AF:
            return true
        default:
            return false
        }
    }
}
