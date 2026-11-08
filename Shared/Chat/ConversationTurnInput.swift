// SPDX-License-Identifier: Apache-2.0
// Shared Apple-client submission contract. No platform UI or provider dependencies.

nonisolated struct ConversationTurnInput: Sendable {
    struct Message: Sendable, Equatable {
        let role: String
        let content: String
    }

    enum HistorySource: Sendable {
        /// The canonical run command loads history from its conversation store.
        case coreManaged
        /// Already committed messages, never the new submission being admitted.
        case prepared([Message])
    }

    let input: String
    let history: [Message]

    init(currentUserText: String, historySource: HistorySource) {
        // Preserve the captured submission exactly, including whitespace and
        // repeated text. Neither the last historical user nor text equality can
        // identify this turn. System/context components travel separately.
        input = currentUserText
        switch historySource {
        case .coreManaged: history = []
        case .prepared(let messages): history = messages.filter { $0.role != "system" }
        }
    }
}
