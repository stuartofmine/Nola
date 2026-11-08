// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatThinkingParser logic in the iOS service layer.
// - Interfaces:
//   - ChatThinkingParseResult
//   - ChatThinkingParser
// - APIs:
//   - none
//
import Foundation

nonisolated struct ChatThinkingParseResult: Equatable {
    let response: String
    let thinking: String
    let hasOpenThinkingBlock: Bool
}

nonisolated enum ChatThinkingParser {
    static func splitThinkingBlocks(from text: String) -> ChatThinkingParseResult {
        var response = ""
        var thinking = ""
        var cursor = text.startIndex
        var hasOpenThinkingBlock = false

        while let start = text.range(of: "<think>", range: cursor..<text.endIndex) {
            response += String(text[cursor..<start.lowerBound])
            let thinkingStart = start.upperBound

            if let end = text.range(of: "</think>", range: thinkingStart..<text.endIndex) {
                thinking += String(text[thinkingStart..<end.lowerBound])
                cursor = end.upperBound
            } else {
                thinking += String(text[thinkingStart..<text.endIndex])
                cursor = text.endIndex
                hasOpenThinkingBlock = true
                break
            }
        }

        if cursor < text.endIndex {
            response += String(text[cursor..<text.endIndex])
        }

        return ChatThinkingParseResult(
            response: response,
            thinking: thinking.trimmingCharacters(in: .whitespacesAndNewlines),
            hasOpenThinkingBlock: hasOpenThinkingBlock
        )
    }
}
