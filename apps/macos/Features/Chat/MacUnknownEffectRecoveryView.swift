import SwiftUI

/// A frozen presentation owner, never an execution authority. Store revalidates
/// the private attempt fences when the explicitly confirmed command arrives.
struct MacUnknownEffectRecoveryPresentation: Equatable, Sendable {
    let id = UUID()
    let owner: MacLocalRunRecovery
    let inspection: MacUnknownEffectRecoveryInspection
    let pendingDecision: MacUnknownEffectRecoveryDecision?

    func matches(_ recovery: MacLocalRunRecovery?, selectedConversationID: String?, generationID: UInt64) -> Bool {
        owner == recovery && owner.conversationID == selectedConversationID
            && owner.generationID == generationID
            && inspection.conversationID == owner.conversationID
            && inspection.runID == owner.expectedRunID
            && inspection.assistantMessageID == owner.canonicalAssistantMessageID
    }

    func decision(targetIndex: Int?, reason: String) throws -> MacUnknownEffectRecoveryDecision {
        guard pendingDecision == nil else { throw MacApplicationServiceError.invalidResponse }
        let targets: [MacUnknownEffectRecoveryTarget]
        let kind: MacUnknownEffectRecoveryDecision.Choice.Kind
        if let targetIndex {
            guard inspection.retryAllowed, inspection.targets.indices.contains(targetIndex) else { throw MacApplicationServiceError.invalidResponse }
            targets = [inspection.targets[targetIndex]]
            kind = .authorizeRiskRetry
        } else {
            guard inspection.independentPendingEffects == 0, !inspection.targets.isEmpty else {
                throw MacApplicationServiceError.invalidResponse
            }
            targets = inspection.targets
            kind = .keepUnknownBlocked
        }
        let decision = MacUnknownEffectRecoveryDecision(conversationID: inspection.conversationID,
            runID: inspection.runID, invocationConfigDigest: inspection.invocationConfigDigest,
            resolutionID: UUID().uuidString.lowercased(), targets: targets,
            choice: .init(kind: kind, reason: reason),
            resolvedAtMs: Int64(Date().timeIntervalSince1970 * 1000))
        try decision.validate()
        return decision
    }
}

struct MacUnknownEffectRecoveryView: View {
    let presentation: MacUnknownEffectRecoveryPresentation
    let isBusy: Bool
    let submit: (MacUnknownEffectRecoveryDecision) -> Void
    let recoverPending: () -> Void
    @State private var selectedIndex: Int?
    @State private var reason = ""
    @State private var confirmation: MacUnknownEffectRecoveryDecision?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(String(localized: "Some operations have an unknown result", table: "Chat"), systemImage: "questionmark.circle")
                .font(.headline)
            Text(String(localized: "These operations may already have taken effect. Checking status does not run them again.", table: "Chat"))
                .foregroundStyle(.secondary)
            if presentation.pendingDecision != nil {
                Text(String(localized: "A recovery decision is awaiting its receipt. Only that original decision can be recovered.", table: "Chat"))
                Button(String(localized: "Recover decision receipt", table: "Chat"), action: recoverPending)
                    .accessibilityIdentifier("chat.unknown.recoverReceipt")
            } else {
                Picker(String(localized: "Operation to retry", table: "Chat"), selection: $selectedIndex) {
                    Text(String(localized: "Choose an operation", table: "Chat")).tag(nil as Int?)
                    ForEach(presentation.inspection.targets.indices, id: \.self) { index in
                        // Only the public effect identity is shown, never lease or worker fences.
                        Text(presentation.inspection.targets[index].effectID).tag(Optional(index))
                    }
                }
                TextField(String(localized: "Reason for this recovery decision", table: "Chat"), text: $reason, axis: .vertical)
                    .lineLimit(2...4)
                    .textFieldStyle(.roundedBorder)
                ViewThatFits(in: .horizontal) {
                    HStack { actions }
                    VStack(alignment: .leading) { actions }
                }
                if presentation.inspection.independentPendingEffects > 0 {
                    Text(String(localized: "Other work is still pending. This task cannot be closed as blocked yet.", table: "Chat"))
                        .foregroundStyle(.secondary)
                }
            }
            if isBusy { ProgressView().controlSize(.small) }
        }
        .font(.system(size: 13))
        .controlSize(.large)
        .padding(12)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 10))
        .disabled(isBusy)
        .sheet(isPresented: Binding(get: { confirmation != nil }, set: { if !$0 { confirmation = nil } })) {
            if let frozen = confirmation {
                VStack(alignment: .leading, spacing: 16) {
                    Text(String(localized: "Retry an operation with an unknown result?", table: "Chat")).font(.headline)
                    Text(String(localized: "The earlier attempt may already have changed files or external systems. Retrying can repeat those changes and cannot undo the earlier attempt.", table: "Chat"))
                    Text(frozen.targets[0].effectID).textSelection(.enabled)
                    HStack {
                        Button(String(localized: "Cancel", table: "Chat")) { confirmation = nil }
                            .keyboardShortcut(.cancelAction)
                        Spacer()
                        Button(String(localized: "Authorize risk retry", table: "Chat"), role: .destructive) {
                            confirmation = nil
                            submit(frozen)
                        }
                        // Deliberately no Return/default-action shortcut for risk authorization.
                    }
                }.font(.system(size: 13)).padding(24).frame(minWidth: 400, idealWidth: 460)
            }
        }
        .accessibilityIdentifier("chat.unknown.recovery")
    }

    @ViewBuilder private var actions: some View {
        Button(String(localized: "Review risk retry…", table: "Chat")) {
            confirmation = try? presentation.decision(targetIndex: selectedIndex, reason: reason)
        }
        .disabled(!presentation.inspection.retryAllowed || selectedIndex == nil || !validReason)
        .accessibilityIdentifier("chat.unknown.reviewRetry")
        Button(String(localized: "Keep unknown and close as blocked", table: "Chat")) {
            if let decision = try? presentation.decision(targetIndex: nil, reason: reason) { submit(decision) }
        }
        .disabled(!validReason || presentation.inspection.independentPendingEffects != 0)
        .accessibilityIdentifier("chat.unknown.keepBlocked")
    }

    private var validReason: Bool {
        !reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && reason.utf8.count <= 4096
    }
}
