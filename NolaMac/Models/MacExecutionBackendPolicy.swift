// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Execution backend value and selection policy.

import Foundation

struct MacExecutionBackendOption: Identifiable, Hashable {
    let id: String
    let displayName: String

    static func fallback(id: String) -> Self {
        switch id {
        case "codex": return Self(id: id, displayName: "Codex")
        case "claude_code": return Self(id: id, displayName: "Claude Code")
        case "opencode": return Self(id: id, displayName: "OpenCode")
        case "nola": return Self(id: id, displayName: "Nola")
        default: return Self(id: id, displayName: id)
        }
    }
}

nonisolated enum MacExecutionBackendPolicy {
    static func requiresNolaModel(_ backendID: String) -> Bool {
        backendID == "nola"
    }

    static func effectiveModelID(_ selectedModelID: String?, backendID: String) -> String? {
        guard let selectedModelID else { return nil }
        let trimmed = selectedModelID.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    static let externalContextPreparationTokens = 32_768
}
