// SPDX-License-Identifier: Apache-2.0
// Inline, conversation-owned recovery. Opening the screen never authorizes retry.
import SwiftUI

struct UnknownEffectRecoverySurface: View {
    @ObservedObject var manager: ChatManager
    private struct DiscoveryIdentity: Equatable {
        let conversationID: UUID?
        let error: String?
    }
    private var discoveryIdentity: DiscoveryIdentity {
        let error: String?
        if case .error(let message) = manager.processingState { error = message } else { error = nil }
        return DiscoveryIdentity(conversationID: manager.currentConversationId, error: error)
    }
    var body: some View {
        Group {
            if manager.unknownEffectRecovery != nil || manager.unknownEffectRecoveryError != nil
                || manager.unknownEffectRecoveryChecking || manager.unknownEffectPendingConversationID != nil {
                UnknownEffectRecoveryCard(manager: manager)
                    .id(manager.currentConversationId)
            }
        }
        .task(id: discoveryIdentity) {
            let identity = discoveryIdentity
            await Task.yield()
            guard !Task.isCancelled, manager.currentConversationId == identity.conversationID else { return }
            await manager.discoverUnknownEffectRecovery()
        }
    }
}

struct UnknownEffectRecoveryCard: View {
    @ObservedObject var manager: ChatManager
    @State private var confirmation: Confirmation?
    @State private var showConfirmation = false
    @State private var working = false
    @State private var failure: String?

    private struct Confirmation {
        let owner: IOSUnknownEffectRecoveryPresentation
        let attemptID: String?
        var retry: Bool { attemptID != nil }
    }
    private func text(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(key, value: fallback, comment: "Unknown operation recovery")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let pendingConversation = manager.unknownEffectPendingConversationID {
                Text(text("chat.recovery.other_conversation", "Another conversation has an unconfirmed recovery decision."))
                    .font(.body)
                Button(text("chat.recovery.open_conversation", "Open That Conversation")) {
                    perform { try await manager.openPendingRecoveryConversation(pendingConversation) }
                }.frame(minHeight: 44)
            } else if let owner = manager.unknownEffectRecovery,
               owner.conversationID == manager.currentConversationId {
                Label(text("chat.recovery.title", "Previous Operation Needs Review"), systemImage: "exclamationmark.triangle")
                    .font(.headline)
                switch owner.phase {
                case .inspected:
                    Text(text("chat.recovery.risk", "The operation may already have taken effect. Retrying can repeat changes. You can leave it unresolved."))
                        .font(.body).fixedSize(horizontal: false, vertical: true)
                    ForEach(Array((owner.canRetry ? owner.attemptIDs : []).enumerated()), id: \.element) { index, attempt in
                        Button {
                            confirmation = .init(owner: owner, attemptID: attempt)
                            showConfirmation = true
                        } label: {
                            Text(String(format: text("chat.recovery.retry_operation", "Review Retry for Operation %ld"), index + 1))
                                .frame(minHeight: 44, alignment: .leading)
                        }
                        .buttonStyle(.bordered)
                    }
                    Button {
                        confirmation = .init(owner: owner, attemptID: nil)
                        showConfirmation = true
                    } label: {
                        Text(text("chat.recovery.keep_blocked", "End Waiting and Keep Unknown"))
                            .frame(minHeight: 44, alignment: .leading)
                    }
                    .buttonStyle(.bordered)
                    .disabled(!owner.canKeepBlocked)
                    .accessibilityIdentifier("chat.recovery.keep_unknown_blocked")
                    if !owner.canKeepBlocked {
                        Text(text("chat.recovery.independent_work", "Other work is still pending. This task cannot be closed yet."))
                            .font(.callout).foregroundStyle(.secondary)
                    }
                case .outcomePending:
                    Text(text("chat.recovery.pending", "Your decision was saved, but its result is not confirmed. Check the original receipt without making a new decision."))
                        .font(.body)
                    Button(text("chat.recovery.check_receipt", "Check Original Decision")) {
                        perform {
                            try await manager.recoverPendingUnknownEffectRecovery(conversationID: owner.conversationID,
                                runID: owner.runID, assistantMessageID: owner.assistantMessageID)
                        }
                    }.frame(minHeight: 44)
                case .retryAuthorized:
                    Text(text("chat.recovery.authorized", "Risk retry was authorized for the original task. This does not confirm that the operation succeeded."))
                        .font(.body)
                    checkButton
                case .blocked:
                    Text(text("chat.recovery.blocked", "The task is closed as blocked. The previous operation’s outcome remains unknown."))
                        .font(.body)
                case .rejected:
                    Text(text("chat.recovery.changed", "The task changed. Check its current state before choosing again."))
                        .font(.body)
                    checkButton
                }
            } else if let message = manager.unknownEffectRecoveryError {
                Text(message).font(.body)
                checkButton
            }
            if working || manager.unknownEffectRecoveryChecking {
                ProgressView(text("chat.recovery.checking", "Checking Task…"))
            }
            if let failure {
                Text(failure).font(.callout).foregroundStyle(.primary)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
        .disabled(working || manager.unknownEffectRecoveryChecking)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.recovery.unknown_effect")
        .confirmationDialog(text("chat.recovery.confirm_title", "Review Recovery Decision"),
            isPresented: $showConfirmation, titleVisibility: .visible, presenting: confirmation) { captured in
            Button(captured.retry ? text("chat.recovery.confirm_retry", "Accept Risk and Retry")
                   : text("chat.recovery.keep_blocked", "End Waiting and Keep Unknown"), role: .destructive) {
                perform {
                    try await manager.confirmUnknownEffectRecovery(captured.owner,
                        choice: captured.retry ? .authorizeRiskRetry : .keepUnknownBlocked,
                        attemptID: captured.attemptID,
                        reason: captured.retry ? "User explicitly accepted the risk of repeating the unknown operation."
                            : "User chose to preserve the unknown outcome and close the blocked task.")
                }
            }
            .accessibilityIdentifier(captured.retry ? "chat.recovery.confirm_risk_retry" : "chat.recovery.confirm_keep_unknown_blocked")
            Button(text("common.cancel", "Cancel"), role: .cancel) { confirmation = nil }
        } message: { captured in
            Text(captured.retry
                 ? text("chat.recovery.confirm_risk", "The previous operation may already have succeeded. Retrying may duplicate changes and cannot undo the original operation.")
                 : text("chat.recovery.confirm_block", "Close this task as blocked without retrying. External execution may still be running; this does not undo or confirm the operation."))
        }
        .onChange(of: manager.currentConversationId) { _, _ in
            showConfirmation = false; confirmation = nil; failure = nil
        }
        .onChange(of: manager.unknownEffectRecovery?.inspectionID) { _, _ in
            showConfirmation = false; confirmation = nil
        }
    }

    private var checkButton: some View {
        Button(text("chat.recovery.check_status", "Check Task Status")) {
            perform { await manager.discoverUnknownEffectRecovery() }
        }.frame(minHeight: 44)
    }

    private func perform(_ operation: @escaping @MainActor () async throws -> Void) {
        guard !working else { return }
        working = true; failure = nil
        let owner = manager.currentConversationId
        Task { @MainActor in
            defer { working = false }
            do { try await operation() }
            catch {
                guard owner == manager.currentConversationId else { return }
                failure = text("chat.recovery.action_failed", "The decision could not be confirmed. Check the original task before trying again.")
            }
        }
    }
}
