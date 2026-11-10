// SPDX-License-Identifier: Apache-2.0
// Shared iOS/macOS authority rule for complete text snapshots. The renderer
// decides how to parse and reveal a delta; this rule never paces or queues it.
// A shorter or divergent snapshot is a replacement, including empty content.

nonisolated enum AuthoritativeTextReconciliation: Equatable, Sendable {
    case unchanged
    case append(String)
    case replace(String)

    static func reconcile(authoritative: String, accepted: String) -> Self {
        if authoritative == accepted {
            return .unchanged
        }
        if authoritative.hasPrefix(accepted) {
            return .append(String(authoritative.dropFirst(accepted.count)))
        }
        // Core can retract provisional model text with `text_replaced` before
        // opening the final model transaction. That is an authoritative reset,
        // not an append-only protocol violation.
        return .replace(authoritative)
    }
}

