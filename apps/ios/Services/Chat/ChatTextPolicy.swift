// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Pure text normalization used by the iOS chat feature.

import Foundation

enum ChatTextPolicy {
    static func trimmingLeadingNewlines(from text: String) -> String {
        var value = text
        while value.hasPrefix("\n") || value.hasPrefix("\r") {
            value.removeFirst()
        }
        return value
    }

    static func trimmingTrailingWhitespaceAndNewlines(from text: String) -> String {
        var value = text
        while let last = value.last, last.isWhitespace {
            value.removeLast()
        }
        return value
    }

    static func normalizedConversationTitle(from text: String) -> String {
        text.components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
    }
}
