// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Pure client presentation overlay for a canonical conversation projection.
// - Boundary: Never persists or mutates canonical messages.

import Foundation

struct MacPendingSubmissionOverlay: Hashable, Sendable {
    let submissionID: String
    var userMessage: MacChatMessage
    var assistantMessage: MacChatMessage

    var messages: [MacChatMessage] {
        [userMessage, assistantMessage]
    }
}

struct MacConversationPresentationOverlay: Hashable, Sendable {
    enum Scope {
        case latest
        case canonicalPage
    }
    /// Metadata refreshes must not replace an already completed text layout.
    /// Canonical corrections still replace text; traces/sources always refresh.
    static func preservingCompletedText(
        in canonical: [MacChatMessage],
        previous: [MacChatMessage]
    ) -> [MacChatMessage] {
        var previousByID: [UUID: MacChatMessage] = [:]
        previousByID.reserveCapacity(previous.count)
        for message in previous where previousByID[message.id] == nil {
            previousByID[message.id] = message
        }
        return canonical.map { message in
            guard message.role == .assistant,
                  let old = previousByID[message.id],
                  old.text == message.text,
                  let completed = old.presentation,
                  completed.isComplete else { return message }
            var result = message
            if var presentation = message.presentation {
                presentation.textNodes = completed.textNodes
                presentation.mutableTextNode = nil
                presentation.lastCommittedID = completed.lastCommittedID
                let canonicalTraces = presentation.traceItems
                presentation.traceItems = completed.traceItems
                presentation.reconcileCanonicalTraceItems(canonicalTraces)
                // Completion is monotonic for an unchanged answer. A delayed
                // canonical snapshot can still carry an in-progress flag;
                // reopening the presentation would re-enable reveal behavior
                // after the user has already seen the completed Markdown.
                presentation.isComplete = true
                result.presentation = presentation
            } else {
                // A completed text-only projection deliberately has no
                // presentation tree. Keep the live tree that already rendered
                // the same canonical text so the terminal subscription refresh
                // does not replace many stable phrase views with one new
                // Markdown document.
                var textPresentation = MacAssistantPresentation.processing()
                textPresentation.textNodes = completed.textNodes
                textPresentation.lastCommittedID = completed.lastCommittedID
                textPresentation.traceItems = completed.traceItems.filter(
                    MacAssistantPresentation.preservesAcrossCanonicalProjection
                )
                textPresentation.finish()
                result.presentation = textPresentation
            }
            return result
        }
    }

    private(set) var pendingSubmission: MacPendingSubmissionOverlay?
    private var messageOverrides: [UUID: MacChatMessage] = [:]

    var isEmpty: Bool {
        pendingSubmission == nil && messageOverrides.isEmpty
    }

    var pendingAssistantMessageID: UUID? {
        pendingSubmission?.assistantMessage.id
    }

    mutating func beginSubmission(_ submission: MacPendingSubmissionOverlay) {
        pendingSubmission = submission
    }

    func renderedMessages(canonical: [MacChatMessage], scope: Scope = .latest) -> [MacChatMessage] {
        var rendered = canonical
        func upsert(_ message: MacChatMessage) {
            if let index = rendered.firstIndex(where: { $0.id == message.id }) {
                rendered[index] = message
            } else if scope == .latest {
                rendered.append(message)
            }
        }
        messageOverrides.values.forEach(upsert)
        pendingSubmission?.messages.forEach(upsert)
        return rendered
    }

    mutating func mutateMessage(
        canonical: [MacChatMessage],
        messageID: UUID,
        createAssistantIfMissing: Bool,
        mutation: (inout MacChatMessage) -> Void
    ) -> Bool {
        if var pendingSubmission {
            if pendingSubmission.userMessage.id == messageID {
                let previous = pendingSubmission.userMessage
                mutation(&pendingSubmission.userMessage)
                guard pendingSubmission.userMessage != previous else { return false }
                self.pendingSubmission = pendingSubmission
                return true
            }
            if pendingSubmission.assistantMessage.id == messageID {
                let previous = pendingSubmission.assistantMessage
                mutation(&pendingSubmission.assistantMessage)
                guard pendingSubmission.assistantMessage != previous else { return false }
                self.pendingSubmission = pendingSubmission
                return true
            }
        }

        guard var rendered = messageOverrides[messageID]
            ?? canonical.first(where: { $0.id == messageID })
            ?? (createAssistantIfMissing
                ? MacChatMessage(id: messageID, role: .assistant, text: "", createdAt: .now)
                : nil)
        else { return false }
        let previous = rendered
        mutation(&rendered)
        guard rendered != previous else { return false }
        messageOverrides[messageID] = rendered
        return true
    }

    mutating func acceptCanonical(
        _ canonical: [MacChatMessage],
        retainPendingAssistantPresentation: Bool
    ) {
        guard let pendingSubmission else { return }
        let committedIDs = Set(canonical.map(\.id))
        guard committedIDs.contains(pendingSubmission.userMessage.id),
              committedIDs.contains(pendingSubmission.assistantMessage.id)
        else { return }
        if retainPendingAssistantPresentation {
            messageOverrides[pendingSubmission.assistantMessage.id] = pendingSubmission.assistantMessage
        } else {
            messageOverrides[pendingSubmission.assistantMessage.id] = nil
        }
        self.pendingSubmission = nil
    }

    mutating func discardTransientMessage(_ messageID: UUID) {
        messageOverrides[messageID] = nil
    }
}
