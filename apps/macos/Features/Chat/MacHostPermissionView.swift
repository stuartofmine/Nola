// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents and resolves one explicit macOS host-tool permission request.

import SwiftUI

struct MacHostPermissionCard: View {
    let request: MacRemotePermissionRequest
    let allowOnce: () -> Void
    let allowConversation: () -> Void
    let deny: () -> Void
    let isResolving: Bool
    let provideInstruction: @MainActor (String) async throws -> Void
    var availableHeight: CGFloat? = nil
    @State private var actionHeight: CGFloat = 72
    @State private var showsDetails = false
    @State private var showsInstruction = false
    @State private var instructionComposer = MacPermissionInstructionComposer()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var requestedFolder: URL? {
        MacWorkspacePermissionPromptPolicy.requestedFolderURL(
            toolName: request.toolName, arguments: request.arguments, isAuthorized: { _ in false }
        )
    }


    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let availableHeight {
                ScrollView {
                    requestContent
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxHeight: max(32, availableHeight - actionHeight - 44))
                .scrollBounceBehavior(.basedOnSize)
                .accessibilityIdentifier("chat.permission.details-scroll")
            } else {
                requestContent
            }
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { permissionActions }
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) { permissionActions }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
            .controlSize(.large)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { actionHeight = $0 }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("chat.permission.actions")
            .disabled(isResolving || instructionComposer.isSending)
        }
        .padding(MacDesignTokens.Spacing.lg)
        .background(MacDesignColor.panel, in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: MacDesignTokens.Radius.panel, style: .continuous)
                .stroke(MacDesignColor.controlOutline, lineWidth: 1)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(
            String(
                format: String(localized: "Permission required for %@"),
                request.toolName
            )
        )
    }

    private var requestContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .center, spacing: 12) {
                Image(systemName: "hand.raised.fill")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.orange)
                    .frame(width: 28, height: 28)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(
                        MacToolPresentationPolicy.displayTitle(
                            for: request.toolName,
                            arguments: request.arguments
                        )
                    )
                        .font(.system(size: 14, weight: .semibold))
                    Text(String(localized: "Permission required"))
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                    Text(request.hostID == "local"
                         ? String(localized: "This Mac") : String(localized: "Connected Host"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                if !request.arguments.isEmpty {
                    Button {
                        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                            showsDetails.toggle()
                        }
                    } label: {
                        Label(
                            String(localized: "Details"),
                            systemImage: showsDetails ? "chevron.up" : "chevron.down"
                        )
                        .font(.system(size: 12, weight: .medium))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(MacActionButtonStyle())
                }
            }

            if let folder = requestedFolder {
                Label(folder.path, systemImage: "folder")
                    .font(.system(size: 12, design: .monospaced))
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("chat.permission.folder.path")
                Text(String(localized: "Authorize this exact folder. This does not approve file modifications."))
                    .font(.caption).foregroundStyle(.secondary)
            }
            if !request.reason.isEmpty {
                Text(request.reason).font(.system(size: 13)).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if requestedFolder == nil && request.allowConversation {
                Text(String(localized: "Remembering this approval applies only to the requested operation and resource in this conversation, not every command or file.", table: "Chat"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("chat.permission.scope")
            }
            if !request.permissionInstructions.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text(String(localized: "Guidance recorded. Tool arguments are unchanged; explicit approval is still required.", table: "Chat"))
                        .font(.caption).foregroundStyle(.secondary)
                    ForEach(Array(request.permissionInstructions.enumerated()), id: \.offset) { _, record in
                        Text(record.instruction).font(.caption).textSelection(.enabled)
                    }
                }
            }
            if showsDetails, !request.arguments.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ScrollView {
                        Text(MacPermissionArgumentPresentation.readable(request.arguments))
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxHeight: 120)
                    .padding(8)
                    .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }

            if showsInstruction {
                VStack(alignment: .leading, spacing: 6) {
                    Text(String(localized: "Guidance does not approve this operation or change its arguments.", table: "Chat"))
                        .font(.caption).foregroundStyle(.secondary)
                    TextField(String(localized: "Add guidance", table: "Chat"), text: $instructionComposer.text, axis: .vertical)
                        .lineLimit(2...5)
                        .textFieldStyle(.roundedBorder)
                        .accessibilityIdentifier("chat.permission.guidance.input")
                        .disabled(instructionComposer.isSending || isResolving)
                    if let error = instructionComposer.error ?? instructionComposer.validationError(request: request) {
                        Text(error).font(.caption).foregroundStyle(.red)
                    }
                    Button(String(localized: "Send guidance", table: "Chat")) {
                        Task { await instructionComposer.submit(request: request, send: provideInstruction) }
                    }
                    .accessibilityIdentifier("chat.permission.guidance.send")
                    .disabled(isResolving || !instructionComposer.canSubmit(request: request))
                }
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
    }

    @ViewBuilder
    private var permissionActions: some View {
        Button(String(localized: "Add guidance", table: "Chat")) {
            withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                showsInstruction.toggle()
            }
        }
        .accessibilityIdentifier("chat.permission.guidance.open")
        Button(String(localized: "Deny for Run"), role: .cancel, action: deny)
            .accessibilityIdentifier("chat.permission.deny")
            .keyboardShortcut(.escape, modifiers: [])
        if requestedFolder == nil {
            if request.allowConversation {
                Button(String(localized: "Remember This Approval", table: "Chat"), action: allowConversation)
                    .buttonStyle(.bordered)
            }
            Button(String(localized: "Allow Once"), action: allowOnce)
                .accessibilityIdentifier("chat.permission.allow-once")
                .buttonStyle(.borderedProminent)
        } else if request.allowConversation {
            Button(String(localized: "Allow for Conversation"), action: allowConversation)
                .buttonStyle(.borderedProminent)
        } else {
            Button(String(localized: "Allow Once"), action: allowOnce)
                .accessibilityIdentifier("chat.permission.allow-once")
                .buttonStyle(.borderedProminent)
        }
    }
}

nonisolated enum MacPermissionArgumentPresentation {
    /// Display the actual invocation, never an inferred authorization target.
    /// Unknown tool schemas remain fully inspectable without keyword routing.
    static func readable(_ arguments: String) -> String {
        guard let data = arguments.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data),
              let formatted = try? JSONSerialization.data(withJSONObject: object,
                  options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]),
              let text = String(data: formatted, encoding: .utf8) else { return arguments }
        return text
    }
}
