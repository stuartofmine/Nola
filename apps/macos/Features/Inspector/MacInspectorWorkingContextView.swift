// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents attachments, retrieval, permission, and workspace controls.

import AppKit
import Observation
import SwiftUI

struct MacInspectorWorkingContextView: View {
    @Bindable var model: MacWorkspaceModel
    var resourcesOnly = false
    @Environment(\.openWindow) private var openWindow
    @State private var displayedMemoryDefault: Bool?
    @State private var displayedRetrievalDefault: Bool?

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
                Label(String(localized: "No attached files"), systemImage: "doc")
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .medium))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 6)
                    .frame(maxWidth: .infinity, minHeight: 40, alignment: .leading)
            }

            if !model.attachments.isEmpty {
                ForEach(model.attachments) { attachment in
                    Button {
                        MacQuickLookController.shared.preview([attachment.url])
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
                    Button {
                        MacQuickLookController.shared.preview([attachment.url])
                    } label: {
                        MacInspectorRow(
                            symbol: "paperclip",
                            title: attachment.name,
                            value: attachment.locationLabel
                        )
                    }
                    .buttonStyle(.plain)
                    .help(attachment.url.path)
                    .contextMenu {
                        Button(String(localized: "Open")) {
                            NSWorkspace.shared.open(attachment.url)
                        }
                        Button(String(localized: "Quick Look")) {
                            MacQuickLookController.shared.preview([attachment.url])
                        }
                        Button(String(localized: "Reveal in Finder")) {
                            NSWorkspace.shared.activateFileViewerSelecting([attachment.url])
                        }
                        Divider()
                        Button(String(localized: "Copy Path")) {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(attachment.url.path, forType: .string)
                        }
                    }
                }
            }
          } else {
            VStack(alignment: .leading, spacing: 4) {
                Picker(String(localized: "Long-term Memory"), selection: $model.memoryEnabledOverride) {
                    Text(String(localized: "Default")).tag(Bool?.none)
                    Text(String(localized: "Enabled")).tag(Optional(true))
                    Text(String(localized: "Disabled")).tag(Optional(false))
                }
                .accessibilityIdentifier("mac-inspector-memory-override")
                Text(String(localized: "Use long-term memory and preferences learned across conversations."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if model.memoryEnabledOverride == nil {
                    Text("\(String(localized: "Default")): \(displayedMemoryDefault.map { $0 ? String(localized: "Enabled") : String(localized: "Disabled") } ?? String(localized: "Loading"))")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
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
}
