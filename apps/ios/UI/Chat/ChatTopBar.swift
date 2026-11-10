// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatTopBar logic in the iOS UI layer.
// - Interfaces:
//   - ChatTopBar
// - APIs:
//   - none
//
import SwiftUI

struct ChatSidebarGlyph: View {
    var body: some View {
        Image("nola-sidebar").renderingMode(.template).resizable().scaledToFit()
            .frame(width: 18, height: 18)
        .accessibilityHidden(true)
    }
}

struct ChatTopBar: View {
    enum Presentation: Equatable { case full, model, composer, actions }
    var presentation: Presentation = .full
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    let showsHistoryToggle: Bool
    var showsConversationTitle: Bool = true
    let roomTitle: String?
    let onShowRooms: () -> Void
    let conversationSubtitle: String
    let modelConnectionLabel: String
    let modelSourceLabel: String
    let modelTint: Color
    let selectedModel: ModelInfo?
    let hybridDeviceToolsEnabled: Bool
    let availableModels: [ModelInfo]
    let executionBackends: [WebAgentProvider.HostAgentSettings.ExecutionBackend]
    let selectedExecutionBackendID: String
    let executionBackendSelectionEnabled: Bool
    let onToggleHistory: () -> Void
    let onShowConversationSettings: () -> Void
    let onShowModelPicker: () -> Void
    let onSelectModel: (ModelInfo) -> Void
    let onSelectExecutionBackend: (String) -> Void
    var onShowProgress: () -> Void = {}
    var onShowResources: () -> Void = {}
    var onShowWorkTools: () -> Void = {}
    var reasoningEfforts: [String] = []
    var reasoningPreference: String = "automatic"
    var reasoningSelectionEnabled = true
    var onSelectReasoning: (String) -> Void = { _ in }

    @ViewBuilder var body: some View {
        switch presentation {
        case .model: modelPicker
        case .composer:
            HStack(spacing: 2) {
                modelPicker
                ReasoningEffortMenu(efforts: reasoningEfforts, preference: reasoningPreference,
                    disabled: !reasoningSelectionEnabled, select: onSelectReasoning).layoutPriority(1)
            }
        case .actions:
            HStack(spacing: 0) {
                Button(action: onShowResources) {
                    Image("nola-file").renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 20, height: 20).frame(width: 44, height: 44)
                }
                .accessibilityLabel(Text(String(localized: "inspector.resources", table: "ConversationInspector")))
                .help(String(localized: "inspector.resources", table: "ConversationInspector"))
                .accessibilityIdentifier("chat.inspector.resources")
                Button(action: onShowRooms) {
                    Image("nola-person").renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 20, height: 20).frame(width: 44, height: 44)
                }
                .accessibilityLabel(Text(String(localized: "chat.experts.title")))
                .help(String(localized: "chat.experts.title"))
                .accessibilityIdentifier("chat.experts.open")
                actionsMenu
            }.buttonStyle(.plain)
        case .full: fullBar
        }
    }

    private var fullBar: some View {
        HStack(spacing: 0) {
            if showsHistoryToggle {
                sidebarToggleButton
            }
            if let roomTitle {
                Button(action: onShowRooms) {
                    VStack(spacing: 2) {
                        Text(roomTitle).font(.headline).foregroundStyle(.primary)
                        Text("room.members").font(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, minHeight: 44)
                        .contentShape(Rectangle())
                }.buttonStyle(.plain)
                    .accessibilityIdentifier("chat.conversation.settings")
            } else {
            modelPicker
            }
            if roomTitle == nil { actionsMenu }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background {
            if reduceTransparency {
                DesignTokens.elevatedColor
            } else {
                Rectangle().fill(.ultraThinMaterial)
            }
        }
        .overlay(NolaDivider(), alignment: .bottom)
    }

    // The same model picker is used in the phone navigation bar and wider layouts.
    var modelPicker: some View {
            Menu {
                if availableModels.isEmpty {
                    Button {
                        onShowModelPicker()
                    } label: {
                        Label(NSLocalizedString("model.setup.title", value: "Configure model", comment: "Configure model"), systemImage: "plus.circle")
                    }
                } else {
                    modelMenuSection(
                        NSLocalizedString("chat.model.group.device", value: "On Device", comment: "Device model group"),
                        models: availableModels.filter(\.isLocal)
                    )
                    modelMenuSection(
                        NSLocalizedString("chat.model.group.host", value: "Host", comment: "Host model group"),
                        models: availableModels.filter(\.isWebBackend)
                    )
                    modelMenuSection(
                        NSLocalizedString("chat.model.group.cloud", value: "Cloud", comment: "Cloud model group"),
                        models: availableModels.filter { !$0.isLocal && !$0.isWebBackend }
                    )
                    Divider()
                    Button {
                        onShowModelPicker()
                    } label: {
                        Label(NSLocalizedString("chat.model.manage", value: "管理模型与参数", comment: "Open model management"), systemImage: "slider.horizontal.3")
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    VStack(spacing: 1) {
                        if showsConversationTitle {
                        Text(conversationSubtitle)
                            .font(.subheadline.weight(.semibold))
                            .foregroundColor(.primary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        }
                        HStack(spacing: 5) {
                            Text(currentModelShortLabel)
                                .font(presentation == .composer ? .caption.weight(.medium) : .footnote.weight(.semibold))
                                .foregroundStyle(presentation == .composer ? Color.secondary : modelTint)
                                .lineLimit(dynamicTypeSize.isAccessibilitySize ? 2 : 1)
                                .truncationMode(.tail)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .accessibilityElement(children: .combine)
                    }
                    Image("nola-chevron").renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 10, height: 10).rotationEffect(.degrees(90))
                        .foregroundStyle(Color(uiColor: .label))
                }
                .frame(maxWidth: presentation == .composer ? nil : .infinity, minHeight: 44, alignment: .leading)
                .padding(.horizontal, 6)
                .contentShape(RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous))
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .hoverEffect(.highlight)
            .accessibilityIdentifier("chat.topbar.modelPicker")
            .accessibilityLabel(
                String(
                    format: NSLocalizedString("chat.topbar.accessibility.summary", value: "%@，%@", comment: "Top bar summary accessibility label"),
                    conversationSubtitle,
                    modelConnectionLabel
                )
            )
            .animation(reduceMotion ? nil : .snappy(duration: 0.24), value: currentModelShortLabel)
    }

    var actionsMenu: some View {
        Menu {
            Text(selectedModel?.isWebBackend == true ? String(localized: "execution.scope.host") : String(localized: "execution.scope.local"))
            Text(String(localized: "execution.scope.child"))
            Button(String(localized: "work.tools.title"), systemImage: "square.grid.2x2", action: onShowWorkTools)
                .accessibilityIdentifier("chat.work_tools.open")
            Divider()
            Button(String(localized: "chat.experts.title"), systemImage: "person.2", action: onShowRooms)
                .accessibilityIdentifier("chat.experts.open")
            Button(String(localized: "inspector.progress", table: "ConversationInspector"), systemImage: "checklist", action: onShowProgress)
                .accessibilityIdentifier("chat.inspector.progress")
            Button(String(localized: "inspector.resources", table: "ConversationInspector"), systemImage: "folder", action: onShowResources)
                .accessibilityIdentifier("chat.inspector.resources")
            if !executionBackends.isEmpty { executionBackendMenu }
            Divider()
            Button(NSLocalizedString("chat.more_settings.title", value: "Conversation Settings", comment: "Conversation settings"),
                   systemImage: "slider.horizontal.3", action: onShowConversationSettings)
                .accessibilityIdentifier("chat.conversation.settings")
        } label: {
            Image("nola-more").renderingMode(.template).resizable().scaledToFit()
                .frame(width: 20, height: 20).frame(width: 44, height: 44).contentShape(Rectangle())
        }
        .accessibilityLabel(Text(NSLocalizedString("chat.more_settings.title", value: "Conversation Settings", comment: "Conversation actions")))
        .accessibilityIdentifier("chat.topbar.more")
        .help(String(localized: "chat.more_settings.title"))
    }

    private var executionBackendMenu: some View {
        Menu {
            if selectedExecutionBackend?.capabilities.supports_permission_bridge == false {
                Text(String(localized: "execution.scope.noApprovalBridge"))
            }
            ForEach(executionBackends) { backend in
                Button {
                    onSelectExecutionBackend(backend.id)
                } label: {
                    Label(
                        executionBackendMenuTitle(backend),
                        systemImage: backend.id == selectedExecutionBackendID ? "checkmark.circle.fill" : "terminal"
                    )
                }
                .disabled(backend.availabilityState != .available)
            }
        } label: {
            Label(selectedExecutionBackend?.display_name ?? "Nola", systemImage: "terminal")
        }
        .buttonStyle(.plain)
        .disabled(!executionBackendSelectionEnabled)
        .opacity(executionBackendSelectionEnabled ? 1 : 0.72)
        .accessibilityIdentifier("chat.executionBackendPicker")
        .accessibilityLabel(NSLocalizedString("chat.execution_unit.choose", value: "Choose execution unit", comment: "Execution backend picker")
            + " · " + (selectedExecutionBackend.map(executionBackendMenuTitle) ?? "Nola"))
        .accessibilityHint(executionBackendSelectionEnabled ? "" : modelConnectionLabel)
        .accessibilityValue(
            selectedExecutionBackend.map(executionBackendMenuTitle) ?? "Nola"
        )
    }

    private var executionBackendTint: Color {
        selectedExecutionBackend?.availabilityState == .available ? Color.teal : DesignTokens.warningColor
    }

    private var selectedExecutionBackend: WebAgentProvider.HostAgentSettings.ExecutionBackend? {
        executionBackends.first(where: { $0.id == selectedExecutionBackendID })
            ?? executionBackends.first(where: { $0.id == "nola" })
            ?? executionBackends.first
    }

    private func executionBackendMenuTitle(
        _ backend: WebAgentProvider.HostAgentSettings.ExecutionBackend
    ) -> String {
        if backend.availability == "available" {
            guard let version = backend.version?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !version.isEmpty else { return backend.display_name }
            return "\(backend.display_name) · \(version)"
        }
        let statusKey: String
        let fallback: String
        switch backend.availabilityState {
        case .notInstalled:
            statusKey = "chat.execution_unit.not_installed"
            fallback = "Not installed"
        case .signInRequired:
            statusKey = "chat.execution_unit.sign_in_required"
            fallback = "Sign in required"
        case .incompatible:
            statusKey = "chat.execution_unit.incompatible"
            fallback = "Incompatible version"
        case .unreachable, .unavailable:
            statusKey = "chat.execution_unit.unavailable"
            fallback = "Unavailable"
        case .available:
            return backend.display_name
        }
        return "\(backend.display_name) · \(NSLocalizedString(statusKey, value: fallback, comment: "Execution backend availability"))"
    }

    private var currentModelShortLabel: String {
        selectedModel == nil ? NSLocalizedString("model.not_selected", value: "No model selected", comment: "No selected model") : modelConnectionLabel
    }

    private func modelIcon(for model: ModelInfo) -> String {
        if model.isLocal { return "iphone" }
        if model.isWebBackend { return hybridDeviceToolsEnabled ? "arrow.triangle.2.circlepath" : "desktopcomputer" }
        return "cloud"
    }

    @ViewBuilder
    private func modelMenuSection(_ title: String, models: [ModelInfo]) -> some View {
        if !models.isEmpty {
            Section(title) {
                ForEach(models) { model in
                    Button {
                        onSelectModel(model)
                    } label: {
                        Label {
                            Text(model.name)
                        } icon: {
                            Image(systemName: isSelected(model) ? "checkmark.circle.fill" : modelIcon(for: model))
                        }
                    }
                    .disabled(!isConfigured(model))
                }
            }
        }
    }

    private func isConfigured(_ model: ModelInfo) -> Bool {
        guard model.isWebBackend else { return true }
        return model.baseURL?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
            && model.resolvedAPIKey?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
    }

    private func isSelected(_ model: ModelInfo) -> Bool {
        selectedModel?.id == model.id && selectedModel?.isLocal == model.isLocal
    }

    private var sidebarToggleButton: some View {
        Button(action: onToggleHistory) {
            ChatSidebarGlyph()
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .hoverEffect(.highlight)
        .help(String(localized: "chat.history.short"))
        .accessibilityLabel(
            NSLocalizedString("chat.history.short", value: "历史", comment: "Short history button")
        )
    }

    private func headerIconButton(systemImage: String, accessibilityLabel: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 17, weight: .semibold))
                .foregroundColor(.primary)
                .frame(width: 44, height: 44)
                .contentShape(RoundedRectangle(cornerRadius: 11, style: .continuous))
        }
        .buttonStyle(.plain)
        .hoverEffect(.highlight)
        .accessibilityLabel(accessibilityLabel)
        .help(accessibilityLabel)
    }
}
