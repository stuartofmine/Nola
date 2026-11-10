// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Builds deterministic, tool-bearing conversation fixtures for Mac performance tests.

import Foundation

#if DEBUG
@MainActor
struct MacLargeConversationFixture {
    static let environmentKey = "NOLA_MAC_LARGE_CONVERSATION_CHARACTERS"

    let conversation: MacConversation
    let messages: [MacChatMessage]
    let assistantCharacterCount: Int
    let toolCallCount: Int
    let skillCallCount: Int

    static func configured() -> Self? {
        guard let rawValue = ProcessInfo.processInfo.environment[environmentKey],
              let characterCount = Int(rawValue), characterCount > 0 else {
            return nil
        }
        return make(assistantCharacterCount: characterCount)
    }

    static func make(
        assistantCharacterCount: Int,
        assistantMessageCount: Int = 1_000,
        conversationID: String = "nola-large-conversation-performance",
        title: String = "10M conversation performance"
    ) -> Self {
        precondition(assistantCharacterCount >= assistantMessageCount * 128)
        let conversation = MacConversation(
            id: conversationID,
            title: title,
            preview: "Deterministic tool and Skill transcript",
            updatedAt: Date(timeIntervalSince1970: 10_000),
            isPinned: false,
            isPersisted: true
        )
        var messages: [MacChatMessage] = []
        messages.reserveCapacity(assistantMessageCount * 2)
        var toolCallCount = 0
        var skillCallCount = 0
        let baseLength = assistantCharacterCount / assistantMessageCount
        let remainder = assistantCharacterCount % assistantMessageCount

        for index in 0..<assistantMessageCount {
            let timestamp = Date(timeIntervalSince1970: 10_000 + Double(index * 2))
            messages.append(
                MacChatMessage(
                    role: .user,
                    text: "Continue deterministic transformer analysis batch \(index + 1).",
                    createdAt: timestamp
                )
            )
            let targetLength = baseLength + (index < remainder ? 1 : 0)
            let isLast = index == assistantMessageCount - 1
            let prefix = isLast
                ? "## Final transformer result\n\nLarge conversation final marker.\n\n"
                : "## Transformer batch \(index + 1)\n\n"
            let seed = "Attention, normalization, feed-forward, residual connection, and deterministic validation.\n"
            let available = targetLength - prefix.count
            let repetitions = max(0, available / seed.count)
            var text = prefix + String(repeating: seed, count: repetitions)
            if text.count < targetLength {
                text += String(repeating: "x", count: targetLength - text.count)
            }
            if text.count > targetLength {
                text = String(text.prefix(targetLength))
            }

            var presentation: MacAssistantPresentation?
            if index.isMultiple(of: 100) || isLast {
                var process = MacAssistantPresentation.processing()
                process.upsertTool(
                    externalID: "tool:file-read:\(index)",
                    title: "Read source file",
                    detail: "Read transformer reference batch \(index + 1)",
                    state: .completed,
                    arguments: #"{"path":"/tmp/transformer.cpp"}"#,
                    result: "Read completed",
                    affectedFiles: ["/tmp/transformer.cpp"]
                )
                process.upsertTool(
                    externalID: "skill:word-document:\(index)",
                    title: "Create Word document",
                    detail: "Rendered transformer implementation report",
                    state: .completed,
                    arguments: #"{"format":"docx","template":"technical-report"}"#,
                    result: "/tmp/transformer-report.docx",
                    affectedFiles: ["/tmp/transformer-report.docx"]
                )
                process.finish()
                presentation = process
                toolCallCount += 1
                skillCallCount += 1
            }
            messages.append(
                MacChatMessage(
                    role: .assistant,
                    text: text,
                    createdAt: timestamp.addingTimeInterval(1),
                    presentation: presentation
                )
            )
        }

        return Self(
            conversation: conversation,
            messages: messages,
            assistantCharacterCount: assistantCharacterCount,
            toolCallCount: toolCallCount,
            skillCallCount: skillCallCount
        )
    }
}
#endif
