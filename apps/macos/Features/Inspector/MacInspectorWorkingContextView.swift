// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents attachments, retrieval, permission, and workspace controls.

import AppKit
import Observation
import SwiftUI

struct MacInspectorWorkingContextView: View {
    @Bindable var model: MacWorkspaceModel
    var resourcesOnly = false
    var canReadLocalFiles = true
    @Environment(\.openWindow) private var openWindow
    @Environment(\.macOpenFileReference) private var openFileReference
    @State private var displayedMemoryDefault: Bool?
    @State private var displayedRetrievalDefault: Bool?

    private var allowsLocalFileActions: Bool {
        canReadLocalFiles && model.executionHost == .local
    }

    private func pendingAttachmentStatus(_ attachment: MacAttachment) -> String {
        switch attachment.status {
        case .extracting:
            return String(localized: "Processing")
        case .ready:
            return attachment.fileType
        case .failed:
            return String(localized: "Failed")
        }
    }

    var body: some View {
        MacInspectorSection(title: resourcesOnly ? String(localized: "inspector.resources", table: "ConversationInspector") : String(localized: "Working Context", table: "Chat"), showsTitle: resourcesOnly) {
          if resourcesOnly {
            let conversationAttachments = model.selectedConversationAttachments
            if model.attachments.isEmpty && conversationAttachments.isEmpty {
                MacInspectorEmptyState(
                    symbol: "paperclip",
                    message: String(localized: "No attached files")
                )
            }

            if !model.attachments.isEmpty {
                ForEach(model.attachments) { attachment in
                    Button {
                        openFileReference(.init(url: attachment.url, line: nil, column: nil))
                    } label: {
                        MacInspectorRow(
                            symbol: attachment.status == .failed ? "exclamationmark.doc" : "doc",
                            title: attachment.name,
                            value: pendingAttachmentStatus(attachment)
                        )
                    }
                    .buttonStyle(.plain)
                    .help(attachment.errorMessage ?? attachment.name)
                }
            }

            if !conversationAttachments.isEmpty {
                ForEach(conversationAttachments) { attachment in
                    if allowsLocalFileActions {
                        Button {
                            openFileReference(.init(url: attachment.url, line: nil, column: nil))
                        } label: {
                            MacInspectorRow(
                                symbol: "paperclip",
                                title: attachment.name,
                                value: (attachment.locationLabel as NSString).abbreviatingWithTildeInPath
                            )
                        }
                        .buttonStyle(.plain)
                        .help((attachment.url.path as NSString).abbreviatingWithTildeInPath)
                        .contextMenu {
                            Button(String(localized: "Preview")) {
                                openFileReference(.init(url: attachment.url, line: nil, column: nil))
                            }
                            Button(String(localized: "Reveal in Finder")) {
                                NSWorkspace.shared.activateFileViewerSelecting([attachment.url])
                            }
                            Button(String(localized: "Open with Default App")) {
                                NSWorkspace.shared.open(attachment.url)
                            }
                            Divider()
                            Button(String(localized: "Copy Path")) {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(attachment.url.path, forType: .string)
                            }
                        }
                    } else {
                        MacInspectorRow(
                            symbol: "paperclip",
                            title: attachment.name,
                            value: String(localized: "Remote Host")
                        )
                        .opacity(0.85)
                        .help(String(localized: "This file belongs to a remote Host.", table: "Chat"))
                        .contextMenu {
                            Button(attachment.url.isFileURL ? String(localized: "Copy Path") : String(localized: "Copy URI")) {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(
                                    attachment.url.isFileURL ? attachment.url.path : attachment.url.absoluteString,
                                    forType: .string
                                )
                            }
                        }
                    }
                }
            }
          } else {
            VStack(alignment: .leading, spacing: 6) {
                MacHorizontalDropdownBar(
                    symbol: "brain.head.profile",
                    title: String(localized: "Long-term Memory"),
                    selectionTitle: memoryOverrideDisplayTitle,
                    identifier: "mac-inspector-memory-override",
                    options: [
                        MacDropdownOption(
                            id: "default",
                            title: String(localized: "Default"),
                            isSelected: model.memoryEnabledOverride == nil
                        ) {
                            model.memoryEnabledOverride = nil
                        },
                        MacDropdownOption(
                            id: "enabled",
                            title: String(localized: "Enabled"),
                            isSelected: model.memoryEnabledOverride == true
                        ) {
                            model.memoryEnabledOverride = true
                        },
                        MacDropdownOption(
                            id: "disabled",
                            title: String(localized: "Disabled"),
                            isSelected: model.memoryEnabledOverride == false
                        ) {
                            model.memoryEnabledOverride = false
                        }
                    ]
                )
                Text(String(localized: "Use long-term memory and preferences learned across conversations."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 2)
            }
            MacInspectorToggleRow(
                symbol: "text.magnifyingglass",
                title: String(localized: "Use Retrieval"),
                detail: String(localized: "Recall relevant indexed context for this conversation."),
                isOn: Binding(
                    get: { model.retrievalEnabledForDisplay(inheritedDefault: displayedRetrievalDefault ?? true) },
                    set: { model.retrievalEnabled = $0 }
                )
            )
            .disabled(displayedRetrievalDefault == nil && model.selectedConversation?.retrievalEnabledOverride == nil)
          }
        }
        .task(id: model.modelConfigurationRevision) {
            guard !resourcesOnly else { return }
            let value = await model.memoryDefaultForDisplay()
            guard !Task.isCancelled else { return }
            displayedMemoryDefault = value
            let retrieval = await model.retrievalDefaultForDisplay()
            guard !Task.isCancelled else { return }
            displayedRetrievalDefault = retrieval
        }
    }

    private var memoryOverrideDisplayTitle: String {
        switch model.memoryEnabledOverride {
        case .some(true):
            return String(localized: "Enabled")
        case .some(false):
            return String(localized: "Disabled")
        case .none:
            if let displayedMemoryDefault {
                let fallback = displayedMemoryDefault ? String(localized: "Enabled") : String(localized: "Disabled")
                return "\(String(localized: "Default")) · \(fallback)"
            }
            return String(localized: "Default")
        }
    }
}
