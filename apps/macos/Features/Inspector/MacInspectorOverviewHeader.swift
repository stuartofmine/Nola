// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI

/// An always-expanded overview with direct workspace actions.
struct MacInspectorOverviewHeader: View {
    @Bindable var model: MacWorkspaceModel
    let hostDisplayName: String

    var body: some View {
        let status = model.selectedConversationDisplayStatus
        VStack(alignment: .leading, spacing: 13) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 7) {
                    Circle().fill(statusColor(status))
                        .frame(width: 6, height: 6)
                    Text(status.title)
                        .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .medium))
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(hostDisplayName).font(.system(size: MacDesignTokens.Typography.inspectorDetail)).foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                if status == .waitingApproval || status == .waitingInput || status == .uncertain {
                    Text("Review the pending request in this conversation before continuing.")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Divider().opacity(0.65)
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label(String(localized: "Workspace"), systemImage: "folder")
                        .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .semibold))
                    Spacer()
                    Menu {
                        Button(String(localized: "Choose Folder…")) { model.chooseWorkspace() }
                        Button(String(localized: "Session Sandbox")) { model.useSessionSandbox() }
                    } label: { Text(String(localized: "Change…")).frame(minHeight: 36).contentShape(Rectangle()) }
                    .menuStyle(.borderlessButton)
                    .fixedSize()
                }
                Text(model.workspaceDisplayName).font(.system(size: 13, weight: .medium))
                if !model.usesSessionSandbox {
                  Text(model.effectiveWorkspaceURL.path)
                    .font(.system(size: MacDesignTokens.Typography.inspectorDetail, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(3).truncationMode(.middle)
                    .textSelection(.enabled)
                    .help(model.effectiveWorkspaceURL.path)
                }
                HStack(spacing: 8) {
                    Button {
                        NSWorkspace.shared.activateFileViewerSelecting([model.effectiveWorkspaceURL])
                    } label: { Image(systemName: "folder") }
                    .help(String(localized: "Reveal in Finder"))
                    .accessibilityLabel(String(localized: "Reveal in Finder"))
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(model.effectiveWorkspaceURL.path, forType: .string)
                    } label: { Image(systemName: "doc.on.doc") }
                    .help(String(localized: "Copy Path"))
                    .accessibilityLabel(String(localized: "Copy Path"))
                }
                .buttonStyle(MacPressFeedbackButtonStyle())
            }
        }
        .accessibilityIdentifier("mac-inspector-workspace")
        .padding(.vertical, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func statusColor(_ status: MacConversationDisplayStatus) -> Color {
        switch status {
        case .ready: .secondary
        case .uncertain, .waitingApproval, .waitingInput: .orange
        default: .accentColor
        }
    }
}
