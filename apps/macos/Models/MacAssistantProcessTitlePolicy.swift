// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Chooses a stable localized heading for assistant activity UI.

import Foundation

nonisolated enum MacAssistantProcessTitlePolicy {
    static func title(
        traces: [MacAssistantTrace],
        isStreaming: Bool
    ) -> String {
        // Tool names and status strings are not evidence of model reasoning.
        // Keep tool-only headings stable as individual operations change.
        guard traces.contains(where: { $0.kind == .thinking }) else {
            if !traces.isEmpty, traces.allSatisfy({ $0.kind == .status }) {
                return String(localized: "Status")
            }
            return String(localized: "Activity")
        }
        return isStreaming
            ? String(localized: "Thinking")
            : String(localized: "Thinking Process")
    }
}
