// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacMessageEditSession: Identifiable {
    let draft: AppleMessageEditDraft
    var id: UUID { draft.operationID }
}

struct MacMessageEditAction {
    let enabled: Bool
    let open: (String, MacChatMessage) -> Void
}
private struct MacMessageEditActionKey: EnvironmentKey {
    static let defaultValue = MacMessageEditAction(enabled: false, open: { _, _ in })
}
extension EnvironmentValues {
    var macMessageEditAction: MacMessageEditAction {
        get { self[MacMessageEditActionKey.self] }
        set { self[MacMessageEditActionKey.self] = newValue }
    }
}

struct MacMessageEditSheet: View {
    @State var draft: AppleMessageEditDraft
    let submit: (AppleMessageEditDraft) async throws -> Void
    let cancel: () -> Void
    @State private var submitting = false
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Edit and resend").font(.headline)
            Text("The original conversation and its draft remain unchanged. The new branch starts with this edited message.")
                .font(.callout).foregroundStyle(.secondary)
            TextEditor(text: $draft.text)
                .font(.system(size: MacMarkdownTypography.bodySize))
                .lineSpacing(MacMarkdownTypography.lineSpacing)
                .scrollContentBackground(.hidden)
                .padding(10)
                .frame(minHeight: 220)
                .background(MacDesignColor.canvas, in: RoundedRectangle(cornerRadius: 10))
                .overlay { RoundedRectangle(cornerRadius: 10).strokeBorder(MacDesignColor.controlOutline, lineWidth: 1) }
                .disabled(submitting).accessibilityIdentifier("mac-message-edit-text")
            if let error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled)
                .accessibilityIdentifier("mac-message-edit-error") }
            HStack {
                Button("Cancel", action: cancel).buttonStyle(MacActionButtonStyle()).keyboardShortcut(.cancelAction).disabled(submitting)
                Spacer()
                if submitting { ProgressView().controlSize(.small) }
                Button("Save and resend") {
                    submitting = true; error = nil
                    Task { @MainActor in
                        do { try await submit(draft) }
                        catch { self.error = error.localizedDescription; submitting = false }
                    }
                }.buttonStyle(MacActionButtonStyle()).keyboardShortcut(.defaultAction)
                    .disabled(submitting || !draft.hasSourceWitness || draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("mac-message-edit-submit")
            }
        }.padding(24).frame(width: 620, height: 430)
            .background(MacDesignColor.canvas)
            .interactiveDismissDisabled(submitting)
    }
}
