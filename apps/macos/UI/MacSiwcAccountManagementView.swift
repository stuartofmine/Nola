// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacSiwcAccountManagementView: View {
    let application: MacApplicationPort
    let account: MacModelAccountSnapshot
    let changed: () -> Void
    var modelsDiscovered: ((String) -> Void)? = nil
    var reauthorize: (() -> Void)? = nil
    var activityChanged: (Bool) -> Void = { _ in }
    @State private var result: MacSiwcAccountResult?
    @State private var bridge: MacSiwcAccountBridge?
    @State private var task: Task<Void, Never>?
    @State private var busy = false
    @State private var error: String?
    @State private var notice: String?
    @State private var confirmingSignOut = false
    @State private var generation = UUID()

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text(account.account.display_name).font(.headline)
                if let session = result?.session {
                    MacStatusBadge(
                        title: sessionStatusTitle(session),
                        tint: sessionStatusTint(session),
                        symbol: sessionStatusSymbol(session)
                    )
                }
                Spacer(minLength: 8)
            }
            if let session = result?.session {
                if !session.signed_in || session.reauthorization_required || !session.plan_usage_authorized {
                    Text(String(localized: "Sign in again to use models from this account.", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    if let reauthorize {
                        Button(String(localized: "Sign in again", table: "ModelCenter"), action: reauthorize)
                            .buttonStyle(.nolaProminent)
                            .disabled(busy)
                    }
                }
                if session.revocation_unconfirmed {
                    Text(String(localized: "Local credentials were cleared, but remote revocation was not confirmed. Revoke Nola’s access in ChatGPT settings.", table: "ModelCenter"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
            if let notice { Text(notice).font(.callout).foregroundStyle(.secondary) }
            if let error { Text(error).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true) }
            HStack(spacing: 8) {
                Button(String(localized: "Manage Models", table: "ModelCenter")) { perform("models") }
                    .buttonStyle(.nolaProminent)
                    .disabled(busy || result?.session?.signed_in != true || result?.session?.reauthorization_required == true || result?.session?.plan_usage_authorized != true)

                Menu {
                    Button(String(localized: "Check Status", table: "ModelCenter")) { perform("account_status") }
                        .disabled(busy)
                    Button(String(localized: "Refresh Authorization", table: "ModelCenter")) { perform("refresh") }
                        .disabled(busy || result?.session?.signed_in != true || result?.session?.reauthorization_required == true)
                    Divider()
                    Button(String(localized: "Sign Out", table: "ModelCenter"), role: .destructive) { confirmingSignOut = true }
                        .disabled(busy || result?.session?.signed_in != true)
                } label: {
                    Image(systemName: "ellipsis")
                        .frame(width: 28, height: 28)
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .help(String(localized: "Account actions", table: "ModelCenter"))
                .accessibilityLabel(String(localized: "Account actions", table: "ModelCenter"))

                if busy {
                    ProgressView(String(localized: "Updating This Mac’s Account…", table: "ModelCenter"))
                        .controlSize(.small)
                }
            }
        }
        .task(id: account.revision) { if !busy { perform("account_status") } }
        .confirmationDialog(String(localized: "Sign Out of This Mac’s ChatGPT Account?", table: "ModelCenter"), isPresented: $confirmingSignOut) {
            Button(String(localized: "Sign Out", table: "ModelCenter"), role: .destructive) { perform("sign_out", confirmed: true) }
            Button(String(localized: "Cancel", table: "ModelCenter"), role: .cancel) {}
        } message: {
            Text(String(localized: "This clears only this Nola account’s private credentials and requests remote revocation. It does not sign out Codex or another Host. Remote revocation may need confirmation in ChatGPT settings.", table: "ModelCenter"))
        }
        .onDisappear {
            generation = UUID(); task?.cancel(); bridge?.close(); bridge = nil
            activityChanged(false)
        }
    }

    private func sessionStatusTitle(_ session: MacSiwcAccountResult.Session) -> String {
        if !session.signed_in { return String(localized: "Signed out", table: "ModelCenter") }
        if session.reauthorization_required || !session.plan_usage_authorized {
            return String(localized: "Needs attention", table: "ModelCenter")
        }
        return String(localized: "Signed in", table: "ModelCenter")
    }

    private func sessionStatusTint(_ session: MacSiwcAccountResult.Session) -> Color {
        if !session.signed_in { return .secondary }
        if session.reauthorization_required || !session.plan_usage_authorized { return .orange }
        return .green
    }

    private func sessionStatusSymbol(_ session: MacSiwcAccountResult.Session) -> String {
        if !session.signed_in { return "person.crop.circle.badge.xmark" }
        if session.reauthorization_required || !session.plan_usage_authorized { return "exclamationmark.triangle.fill" }
        return "checkmark.circle.fill"
    }
    private func perform(_ action: String, confirmed: Bool = false) {
        guard !busy else { return }
        busy = true; error = nil; notice = nil
        activityChanged(true)
        let ownerGeneration = generation
        let selected = result?.account ?? account
        let revision = result?.session?.session_revision
        task = Task {
            do {
                try Task.checkCancellation()
                let transport: MacSiwcAccountBridge
                if let bridge { transport = bridge }
                else { transport = try MacSiwcAccountBridge(application: application); bridge = transport }
                let value = try await transport.manage(action, account: selected, sessionRevision: revision,
                    acknowledgeSignOut: confirmed)
                try Task.checkCancellation()
                guard ownerGeneration == generation else { return }
                result = value
                if action == "models" { notice = String(localized: "Directory updated. Enable models in the connection below; none were selected automatically.", table: "ModelCenter") }
                else if action == "refresh" { notice = String(localized: "Authorization refreshed. No model request was sent.", table: "ModelCenter") }
                else if action == "sign_out" { notice = String(localized: "Signed out locally. Check the remote revocation status above.", table: "ModelCenter") }
                if action != "account_status" { changed() }
                if action == "models", let connection = value.connection { modelsDiscovered?(connection.id) }
            } catch {
                guard ownerGeneration == generation, !Task.isCancelled else { return }
                self.error = MacSiwcBridgeFailure.message(error)
                bridge?.close(); bridge = nil; result = nil
            }
            if ownerGeneration == generation {
                busy = false; task = nil
                activityChanged(false)
            }
        }
    }
}
