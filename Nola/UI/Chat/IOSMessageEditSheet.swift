// SPDX-License-Identifier: Apache-2.0
// The editor owns only a draft. Shared Application must admit the actual resend.
import SwiftUI

struct IOSMessageEditSheet: View {
    let ownerIsRunning: Bool
    let submit: ((AppleMessageEditSubmission) async throws -> Void)?
    @Environment(\.dismiss) private var dismiss
    @State private var draft: AppleMessageEditDraft
    @State private var submitting = false
    @State private var outcomePending = false
    @State private var error: String?

    init(conversationID: UUID?, message: ChatMessage, sourceRevision: Int64? = nil, ownerIsRunning: Bool,
         submit: ((AppleMessageEditSubmission) async throws -> Void)?) {
        self.ownerIsRunning = ownerIsRunning
        self.submit = submit
        _draft = State(initialValue: AppleMessageEditDraft(sourceConversationID: conversationID?.uuidString,
            sourceMessageID: message.metadata["nola.canonicalMessageID"] ?? message.id.uuidString,
            expectedSourceRevision: sourceRevision, expectedMessageRevision: message.canonicalMessageRevision, text: message.content))
    }
    private var available: Bool { submit != nil && draft.hasSourceWitness }
    var body: some View {
        NavigationStack {
            Form {
                Group {
                Section {
                    TextEditor(text: $draft.text).frame(minHeight: 180).disabled(submitting || outcomePending)
                        .scrollContentBackground(.hidden)
                        .padding(12)
                        .background(DesignTokens.backgroundColor)
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(DesignTokens.strongBorderColor, lineWidth: 1))
                        .accessibilityLabel(String(localized: "chat.message.edit"))
                        .accessibilityIdentifier("chat.message.editor")
                } footer: { Text("chat.message.editPreservesOriginal") }
                if ownerIsRunning { Section { Text("chat.message.editWait").foregroundStyle(.secondary) } }
                if !available { Section { Text("chat.message.resendUnavailable").foregroundStyle(.secondary) } }
                if let error { Section { Text(error).foregroundStyle(DesignTokens.dangerColor).textSelection(.enabled) } }
                }.listRowBackground(DesignTokens.backgroundColor)
            }
            .scrollContentBackground(.hidden)
            .background(DesignTokens.backgroundColor)
            .navigationTitle(String(localized: "chat.message.edit"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(L10n.cancel) { dismiss() }.frame(minHeight: 44).disabled(submitting || outcomePending) }
                ToolbarItem(placement: .confirmationAction) {
                    Button(outcomePending ? String(localized: "chat.message.checkOriginalRequest") : String(localized: "chat.message.saveResend")) { send() }
                        .frame(minHeight: 44)
                        .disabled(submitting || ownerIsRunning || !available || draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("chat.message.editor.save")
                }
            }
            .interactiveDismissDisabled(submitting || outcomePending)
        }
    }
    private func send() {
        guard !submitting, !ownerIsRunning, let submit, let sourceConversationID = draft.sourceConversationID,
              let expectedSourceRevision = draft.expectedSourceRevision, let expectedMessageRevision = draft.expectedMessageRevision, draft.hasSourceWitness,
              !draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let input = AppleMessageEditSubmission(operationID: draft.operationID, targetConversationID: draft.targetConversationID, createdAtMs: draft.createdAtMs,
            sourceConversationID: sourceConversationID,
            sourceMessageID: draft.sourceMessageID, expectedSourceRevision: expectedSourceRevision, expectedMessageRevision: expectedMessageRevision, text: draft.text)
        submitting = true; error = nil
        Task { @MainActor in
            do { try await submit(input); dismiss() }
            catch {
                self.error = error.localizedDescription
                if case IOSApplicationRuntimeError.outcomePending = error { outcomePending = true }
                else { outcomePending = false }
            }
            submitting = false
        }
    }
}
