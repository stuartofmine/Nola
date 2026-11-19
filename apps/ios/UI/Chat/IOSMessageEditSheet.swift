// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct IOSMessageEditSheet: View {
    let ownerIsRunning: Bool
    let submit: ((AppleMessageEditSubmission) async throws -> Void)?
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.dismiss) private var dismiss
    @State private var draft: AppleMessageEditDraft
    @State private var pending: AppleMessageEditSubmission?
    @State private var retryAvailable = false
    @State private var submitting = false
    @State private var error: String?

    init(conversationID: UUID?, message: ChatMessage, sourceRevision: Int64? = nil, ownerIsRunning: Bool,
         submit: ((AppleMessageEditSubmission) async throws -> Void)?) {
        self.ownerIsRunning = ownerIsRunning
        self.submit = submit
        _draft = State(initialValue: AppleMessageEditDraft(sourceConversationID: conversationID?.uuidString,
            sourceMessageID: message.metadata["nola.canonicalMessageID"] ?? message.id.uuidString,
            expectedSourceRevision: sourceRevision, expectedMessageRevision: message.canonicalMessageRevision, text: message.content))
    }

    init(pending: AppleMessageEditSubmission, ownerIsRunning: Bool, submit: @escaping (AppleMessageEditSubmission) async throws -> Void) {
        self.ownerIsRunning = ownerIsRunning
        self.submit = submit
        _pending = State(initialValue: pending)
        _draft = State(initialValue: AppleMessageEditDraft(sourceConversationID: pending.sourceConversationID,
            sourceMessageID: pending.sourceMessageID, expectedSourceRevision: pending.expectedSourceRevision,
            expectedMessageRevision: pending.expectedMessageRevision, text: pending.text))
    }

    private var available: Bool { submit != nil && draft.hasSourceWitness }
    private var canCheck: Bool { pending != nil && !retryAvailable }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextEditor(text: $draft.text).frame(minHeight: 180).disabled(submitting || pending != nil)
                        .scrollContentBackground(.hidden)
                        .padding(12)
                        .background(DesignTokens.backgroundColor)
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(DesignTokens.strongBorderColor, lineWidth: 1))
                        .accessibilityLabel(String(localized: "chat.message.edit"))
                        .accessibilityIdentifier("chat.message.editor")
                } footer: { Text(pending == nil ? "chat.message.editPreservesOriginal" : "chat.message.editSavedForReview") }
                if ownerIsRunning && !canCheck { Section { Text("chat.message.editWait").foregroundStyle(.secondary) } }
                if !available { Section { Text("chat.message.resendUnavailable").foregroundStyle(.secondary) } }
                if let error { Section { Text(error).foregroundStyle(DesignTokens.dangerColor).textSelection(.enabled) } }
            }
            .scrollContentBackground(.hidden)
            .background(DesignTokens.backgroundColor)
            .navigationTitle(String(localized: "chat.message.edit"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(pending == nil ? L10n.cancel : L10n.close) { dismiss() }.frame(minHeight: 44)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(canCheck ? String(localized: "chat.message.checkOriginalRequest") : String(localized: "chat.message.saveResend")) { performAction() }
                        .frame(minHeight: 44)
                        .disabled(submitting || (!canCheck && ownerIsRunning) || !available || draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("chat.message.editor.save")
                }
            }
        }
    }

    private func performAction() {
        guard !submitting, let submit, let source = draft.sourceConversationID, let owner = UUID(uuidString: source),
              let revision = draft.expectedSourceRevision, let messageRevision = draft.expectedMessageRevision else { return }
        let store = IOSPendingMessageEdit(runtime: container.applicationRuntime, conversationID: owner)
        let input = pending ?? AppleMessageEditSubmission(operationID: draft.operationID, targetConversationID: draft.targetConversationID,
            createdAtMs: draft.createdAtMs, sourceConversationID: source, sourceMessageID: draft.sourceMessageID,
            expectedSourceRevision: revision, expectedMessageRevision: messageRevision, text: draft.text)
        if canCheck {
            do {
                if try store.runtime.editReceipt(input) == .admitted {
                    try store.clear(input)
                    dismiss()
                } else {
                    retryAvailable = true
                    error = nil
                }
            } catch { self.error = error.localizedDescription }
            return
        }
        guard !ownerIsRunning else { return }
        do { try store.retain(input) }
        catch { self.error = error.localizedDescription; return }
        pending = input
        retryAvailable = false
        submitting = true
        error = nil
        Task { @MainActor in
            defer { submitting = false }
            do {
                try await submit(input)
                try store.clear(input)
                dismiss()
            } catch {
                self.error = error.localizedDescription
                if case IOSApplicationRuntimeError.outcomePending = error { return }
                do {
                    if try store.runtime.editReceipt(input) == .absent {
                        try store.clear(input)
                        pending = nil
                    }
                } catch { self.error = error.localizedDescription }
            }
        }
    }
}

/// Rewinding can remove the originating bubble before admission. Recovery belongs to its conversation.
struct IOSPendingEditAction: View {
    @EnvironmentObject private var container: ServiceContainer
    let owner: UUID?
    let ownerIsRunning: Bool
    @State private var pending: AppleMessageEditSubmission?
    @State private var error: String?
    @State private var showing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let pending, UUID(uuidString: pending.sourceConversationID) == owner {
                Button("chat.message.pendingEdit") { showing = true }
                    .frame(minHeight: 44)
                    .accessibilityIdentifier("chat.message.pendingEdit")
                    .sheet(isPresented: $showing) {
                        IOSMessageEditSheet(pending: pending, ownerIsRunning: ownerIsRunning) { input in
                            _ = try await container.chatManager.sendEditedMessage(input)
                        }
                    }
            }
            if let error { Text(error).font(.footnote).foregroundStyle(.secondary) }
        }
        .onChange(of: owner, initial: true) { _, _ in showing = false; reload() }
        .onReceive(NotificationCenter.default.publisher(for: .nolaPendingMessageEditChanged)) { event in
            if let changed = event.object as? UUID, changed == owner { reload() }
        }
    }

    private func reload() {
        pending = nil
        error = nil
        guard let owner else { return }
        do { pending = try IOSPendingMessageEdit(runtime: container.applicationRuntime, conversationID: owner).load() }
        catch { self.error = error.localizedDescription }
    }
}
