// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatMoreSettingsSheet logic in the iOS UI layer.
// - Interfaces:
//   - ChatMoreSettingsSheet
// - APIs:
//   - none
//
import SwiftUI

struct ChatMoreSettingsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let selectedModel: ModelInfo?
    let contextUsage: ConversationContextUsage?
    let hostUploadedSkills: [UserUploadedContextItem]
    let hostUploadedDocuments: [UserUploadedContextItem]
    let hostMCPServers: [UserMCPServer]
    let hostComputerUseStatus: WebAgentProvider.HostComputerUseStatus?
    let hostNativeBrowserStatus: WebAgentProvider.HostNativeBrowserStatus?
    let pairedHostIdentity: String?
    let conversationGenerationOverride: IOSConversationGenerationOverride
    let onSaveGenerationOverride: (IOSConversationGenerationOverride) throws -> Void
    @Binding var hybridDeviceToolsEnabled: Bool
    @Binding var hybridFilesEnabled: Bool
    @Binding var hybridClipboardEnabled: Bool
    @Binding var hybridLocationEnabled: Bool
    @Binding var hybridCalendarEnabled: Bool
    @Binding var enableRetrieval: Bool
    @Binding var permissionMode: IOSPermissionMode
    @Binding var autoCompactTriggerPercent: Int
    @Binding var responseStyle: PromptResponseStyle?
    @Binding var sessionMemory: String
    @Binding var longTermMemory: String
    @Binding var userPreferences: String
    @Binding var uploadedSkills: [UserUploadedContextItem]
    let onSaveMemoryProfile: () async throws -> Void
    let onSetSkillEnabled: (UUID, Bool) throws -> Void
    let onOpenContextSettings: () -> Void
    let onOpenSkillLibrary: () -> Void
    let onForgetHostPairing: () -> Void
    var promptTemplates: [PromptTemplate] = []
    var selectedPromptTemplateID: UUID? = nil
    var onSelectPromptTemplate: (UUID?) throws -> Void = { _ in }

    @State private var showForgetPairingConfirmation = false
    @State private var isSaving = false
    @State private var statusMessage: String?
    @State private var errorMessage: String?
    @State private var showPersonalIntelligence = false
    @State private var savedGenerationOverride: IOSConversationGenerationOverride?
    @State private var generationEditorPreventsDismiss = false
    @State private var generationEditorActive = false

    private func tr(_ key: String, _ value: String, comment: String) -> String {
        NSLocalizedString(key, value: value, comment: comment)
    }

    private func hostTransportStatus(url: URL) -> String {
        if url.scheme?.lowercased() == "https" {
            return tr("chat.host_pairing.transport.system_https", "System HTTPS", comment: "System HTTPS transport status")
        }
        return tr("chat.host_pairing.transport.encrypted_lan", "Encrypted LAN bridge", comment: "Encrypted LAN bridge transport status")
    }

    private func hostDisplayName(_ url: URL) -> String {
        guard let host = url.host, !host.isEmpty else { return url.absoluteString }
        guard let port = url.port else { return host }
        return "\(host):\(port)"
    }

    var body: some View {
        Form {
            Section {
                ChatConversationScopeSummary(
                    symbol: "slider.horizontal.3",
                    title: selectedModel?.name ?? tr("chat.more_settings.model.not_selected", "No model selected", comment: "Fallback label when no model is selected in chat more settings"),
                    detail: tr("chat.more_settings.subtitle", "Manage this conversation or adjust defaults for the selected model.", comment: "Simplified chat settings subtitle"),
                    scopeLabel: tr("chat.scope.current_conversation", "This conversation", comment: "Current conversation scope label")
                )
            }

            conversationSection
            executionSection
            intelligenceSection
            modelParametersSection
            globalSection
            Section {
                Text(tr("chat.more_settings.immediate_changes", "Changes on this page take effect immediately. Model parameter editors have their own Save action.", comment: "Immediate settings persistence explanation"))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
        .disabled(isSaving)
        .interactiveDismissDisabled(isSaving || generationEditorPreventsDismiss)
        .navigationTitle(tr("chat.more_settings.title", "Conversation Settings", comment: "Chat more settings sheet title"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if !generationEditorActive {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.close) {
                        guard !isSaving, !generationEditorPreventsDismiss else { return }
                        dismiss()
                    }
                    .disabled(isSaving || generationEditorPreventsDismiss)
                    .accessibilityIdentifier("chat-settings-close")
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            settingsStatusBanner
        }
        .task(id: statusMessage) {
            guard statusMessage != nil else { return }
            try? await Task.sleep(for: .seconds(2.6))
            guard !Task.isCancelled else { return }
            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.18)) {
                statusMessage = nil
            }
        }
        .navigationDestination(isPresented: $showPersonalIntelligence) {
            PersonalIntelligenceDashboardView(embedded: true)
        }
    }

    private var conversationSection: some View {
        Section {
            Picker(tr("chat.more_settings.response_style", "Response style", comment: "Response style picker title"), selection: $responseStyle) {
                Text(tr("chat.style.inherit_model", "Use model default", comment: "Inherit model response style"))
                    .tag(Optional<PromptResponseStyle>.none)
                ForEach(PromptResponseStyle.allCases) { style in
                    Text(style.displayName).tag(Optional(style))
                }
            }
            .pickerStyle(.menu)

            Toggle(
                tr("chat.more_settings.retrieval", "Search connected knowledge", comment: "Conversation retrieval toggle"),
                isOn: $enableRetrieval
            )

            Picker(
                tr("chat.more_settings.permission_mode", "Permission mode", comment: "Permission mode picker title"),
                selection: $permissionMode
            ) {
                ForEach(IOSPermissionMode.allCases) { mode in
                    Text(mode.title).tag(mode)
                }
            }

            Text(permissionMode.detail)
                .font(.caption)
                .foregroundStyle(.secondary)

            Button {
                openAfterDismiss(onOpenContextSettings)
            } label: {
                HStack(spacing: 12) {
                    Image(systemName: "circle.hexagongrid")
                        .foregroundStyle(DesignTokens.primaryColor)
                        .frame(width: 24)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(tr("chat.more_settings.scope.current", "Context and memory", comment: "Current conversation settings entry"))
                            .foregroundStyle(.primary)
                        if let contextUsage {
                            Text("\(formattedTokenCount(contextUsage.totalTokens)) / \(formattedTokenCount(contextUsage.estimatedTokenLimit)) · \(contextUsage.percentage)%")
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                            ProgressView(value: Double(contextUsage.percentage), total: 100)
                                .tint(contextUsage.percentage >= autoCompactTriggerPercent ? DesignTokens.warningColor : DesignTokens.primaryColor)
                        }
                    }
                    Spacer()
                }
                .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
                .contentShape(Rectangle())
            }
            .accessibilityIdentifier("chat.more_settings.context")
        } header: {
            Text(tr("chat.more_settings.conversation.header", "This conversation", comment: "Current conversation section header"))
        }
    }

    private var intelligenceSection: some View {
        Section {
            Button {
                showPersonalIntelligence = true
            } label: {
                Label(tr("chat.more_settings.scope.intelligence", "Personal intelligence", comment: "Personal intelligence dashboard entry"), systemImage: "brain.head.profile")
            }

            Button {
                openAfterDismiss(onOpenSkillLibrary)
            } label: {
                Label(tr("chat.more_settings.scope.skills", "Skills library", comment: "Global Skills library entry"), systemImage: "shippingbox")
            }
        } header: {
            Text(tr("chat.more_settings.intelligence.header", "Intelligence and knowledge", comment: "Intelligence section header"))
        } footer: {
            Text(tr("chat.more_settings.intelligence.footer", "Personal intelligence learns across conversations. Skills are reusable capabilities available throughout the app.", comment: "Intelligence section footer"))
        }
    }

    private var globalSection: some View {
        Section {
            Button {
                openAfterDismiss {
                    NotificationCenter.default.post(name: .nolaOpenSettingsTab, object: nil)
                }
            } label: {
                Label(tr("chat.more_settings.scope.global", "Global app settings", comment: "Global app settings entry"), systemImage: "gearshape")
            }
        }
    }

    @ViewBuilder
    private var settingsStatusBanner: some View {
        if let message = errorMessage ?? statusMessage, !message.isEmpty {
            HStack(spacing: 10) {
                Image(systemName: errorMessage == nil ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                    .foregroundStyle(errorMessage == nil ? DesignTokens.successColor : Color.red)
                Text(message)
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(.primary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button {
                    errorMessage = nil
                    statusMessage = nil
                } label: {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.bold))
                        .frame(width: 32, height: 32)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(L10n.close)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(.regularMaterial)
            .overlay(alignment: .top) { Divider() }
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }

    private var modelParametersSection: some View {
        Section {
            Picker(NSLocalizedString("chat.context.prompt.picker", value: "Current template", comment: "Current prompt template"), selection: Binding(
                get: { selectedPromptTemplateID },
                set: { value in
                    do { try onSelectPromptTemplate(value) }
                    catch { errorMessage = error.localizedDescription }
                }
            )) {
                Text(L10n.promptTemplatesCurrent).tag(UUID?.none)
                ForEach(promptTemplates) { template in Text(template.name).tag(Optional(template.id)) }
            }
            .accessibilityIdentifier("chat-model-prompt-template")

            if let selectedModel {
                if !selectedModel.isLocal && !selectedModel.isWebBackend {
                    NavigationLink {
                        IOSConversationGenerationSettingsView(
                            model: selectedModel,
                            initialOverride: savedGenerationOverride ?? conversationGenerationOverride,
                            preventsSheetDismiss: $generationEditorPreventsDismiss,
                            editorActive: $generationEditorActive,
                            onSave: { value in
                                try onSaveGenerationOverride(value)
                                savedGenerationOverride = value
                            }
                        )
                    } label: {
                        Label {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(tr("chat.more_settings.conversation_model_limits", "Conversation model limits", comment: "Per-conversation model parameter editor"))
                                Text(tr("chat.more_settings.conversation_model_limits_hint", "Override context and output limits, or inherit model defaults.", comment: "Per-conversation limits hint"))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        } icon: {
                            Image(systemName: "slider.horizontal.3")
                        }
                    }
                    .accessibilityIdentifier("chat-conversation-model-limits")
                }
                NavigationLink {
                    ModelParametersView(
                        modelID: selectedModel.id,
                        displayName: selectedModel.name,
                        isLocal: selectedModel.isLocal
                    )
                    .id(selectedModel.id)
                } label: {
                    Label {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(tr("chat.more_settings.model_output.advanced", "Advanced model defaults", comment: "Advanced model defaults entry"))
                            Text(selectedModel.name)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    } icon: {
                        Image(systemName: "slider.horizontal.3")
                    }
                }
                .accessibilityIdentifier("chat-model-defaults")
            } else {
                Text(tr("chat.more_settings.model.not_selected", "No model selected", comment: "Fallback label when no model is selected"))
                    .foregroundStyle(.secondary)
            }
        } header: {
            Text(tr("chat.more_settings.model_output.header", "Model", comment: "Global model output section header"))
        } footer: {
            if let selectedModel, !selectedModel.isLocal && !selectedModel.isWebBackend {
                Text(tr("chat.more_settings.model_output.scoped_footer", "Conversation limits affect only this conversation; advanced defaults apply to every conversation using the model.", comment: "Distinguishes session overrides from global model defaults"))
            } else {
                Text(tr("chat.more_settings.model_output.footer", "Advanced defaults are saved for the selected model and apply to every conversation using it.", comment: "Section footer for model and output settings"))
            }
        }
    }

    private var executionSection: some View {
        Section {
            Text(selectedModel?.isWebBackend == true ? String(localized: "execution.scope.host") : String(localized: "execution.scope.local"))
                .font(.callout).foregroundStyle(.secondary)
            Text(String(localized: "execution.scope.child")).font(.callout).foregroundStyle(.secondary)
            if selectedModel?.isWebBackend == true {
                if let model = selectedModel,
                   let rawURL = model.baseURL,
                   let url = URL(string: rawURL) {
                    let verified = pairedHostIdentity?.isEmpty == false
                    HStack(alignment: .center, spacing: 12) {
                        Image(systemName: verified ? "desktopcomputer.and.macbook" : "exclamationmark.triangle")
                            .font(.system(size: 17, weight: .semibold))
                            .foregroundStyle(verified ? DesignTokens.successColor : .orange)
                            .frame(width: 38, height: 38)
                            .background(
                                (verified ? DesignTokens.successColor : Color.orange).opacity(0.10),
                                in: RoundedRectangle(cornerRadius: 11, style: .continuous)
                            )
                        VStack(alignment: .leading, spacing: 3) {
                            Text(hostDisplayName(url))
                                .font(.subheadline.weight(.semibold))
                                .lineLimit(1)
                            Text(hostTransportStatus(url: url))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 6)
                        Label(
                            verified
                                ? tr("chat.host_pairing.verified", "Verified", comment: "Verified host identity status")
                                : tr("chat.host_pairing.unverified", "Not verified", comment: "Unverified host identity status"),
                            systemImage: verified ? "checkmark.shield.fill" : "exclamationmark.shield"
                        )
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(verified ? DesignTokens.successColor : .orange)
                    }
                    .padding(.vertical, 6)
                    .listRowSeparator(.hidden)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("chat.host.connection.summary")
                }
                Toggle(
                    tr("chat.more_settings.hybrid_device_tools", "Allow host to request device tools", comment: "Hybrid device tools toggle"),
                    isOn: $hybridDeviceToolsEnabled
                )
                Text(tr(
                    "chat.more_settings.hybrid_device_tools.hint",
                    "Each request still requires approval. Device results may be returned to this host session only.",
                    comment: "Hybrid device tools toggle hint"
                ))
                .font(.caption)
                .foregroundStyle(.secondary)
                if hybridDeviceToolsEnabled {
                    Toggle(isOn: $hybridFilesEnabled) {
                        Label(tr("chat.hybrid.files", "Selected files", comment: "Hybrid files capability"), systemImage: "doc")
                    }
                    Toggle(isOn: $hybridClipboardEnabled) {
                        Label(tr("chat.hybrid.clipboard", "Clipboard", comment: "Hybrid clipboard capability"), systemImage: "doc.on.clipboard")
                    }
                    Toggle(isOn: $hybridLocationEnabled) {
                        Label(tr("chat.hybrid.location", "Approximate location", comment: "Hybrid location capability"), systemImage: "location")
                    }
                    Toggle(isOn: $hybridCalendarEnabled) {
                        Label(tr("chat.hybrid.calendar", "Calendar", comment: "Hybrid calendar capability"), systemImage: "calendar")
                    }
                    Text(tr(
                        "chat.hybrid.capabilities.hint",
                        "Only selected capabilities are shown to the host model. Every action still requires confirmation.",
                        comment: "Hybrid capability list hint"
                    ))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                if let pairedHostIdentity, !pairedHostIdentity.isEmpty {
                    Button(role: .destructive) {
                        showForgetPairingConfirmation = true
                    } label: {
                        Label(
                            tr("chat.host_pairing.forget", "Forget host pairing", comment: "Forget paired host action"),
                            systemImage: "xmark.shield"
                        )
                    }
                    .tint(.red)
                    .confirmationDialog(
                        tr("chat.host_pairing.forget.confirm.title", "Forget this host pairing?", comment: "Forget pairing confirmation title"),
                        isPresented: $showForgetPairingConfirmation,
                        titleVisibility: .visible
                    ) {
                        Button(
                            tr("chat.host_pairing.forget.confirm.action", "Forget pairing", comment: "Confirm forget pairing action"),
                            role: .destructive,
                            action: onForgetHostPairing
                        )
                        Button(tr("common.cancel", "Cancel", comment: "Cancel action"), role: .cancel) {}
                    } message: {
                        Text(tr(
                            "chat.host_pairing.forget.confirm.message",
                            "Nola will clear the trusted host identity. Conversations remain on this device.",
                            comment: "Forget pairing confirmation message"
                        ))
                    }
                }
            }
            Stepper(
                value: $autoCompactTriggerPercent,
                in: ChatManager.autoCompactTriggerPercentRange,
                step: 5
            ) {
                settingValueRow(
                    title: tr("chat.more_settings.auto_compact_trigger", "Auto-compact trigger", comment: "Auto-compact threshold setting"),
                    value: "\(autoCompactTriggerPercent)%"
                )
            }

            if let hostComputerUseStatus {
                VStack(alignment: .leading, spacing: 8) {
                    settingValueRow(
                        title: tr("chat.more_settings.computer_use.title", "Computer-use host", comment: "Computer-use host status title"),
                        value: computerUseStatusText(hostComputerUseStatus)
                    )
                    settingValueRow(
                        title: tr("chat.more_settings.computer_use.capacity", "Desktop tools", comment: "Computer-use tool capacity title"),
                        value: String(
                            format: tr("chat.more_settings.computer_use.capacity_format", "%d hosts • %d tools", comment: "Computer-use host and tool count format"),
                            hostComputerUseStatus.hosts?.count ?? 0,
                            hostComputerUseStatus.tools?.count ?? 0
                        )
                    )
                    if let host = hostComputerUseStatus.hosts?.first {
                        Text(host.host_id)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .padding(.vertical, 4)
            }

            if let hostNativeBrowserStatus {
                VStack(alignment: .leading, spacing: 8) {
                    settingValueRow(
                        title: tr("chat.more_settings.native_browser.title", "Native Chrome", comment: "Native browser status title"),
                        value: nativeBrowserStatusText(hostNativeBrowserStatus)
                    )
                    settingValueRow(
                        title: tr("chat.more_settings.native_browser.capacity", "Chrome bridge", comment: "Native browser bridge queue title"),
                        value: String(
                            format: tr("chat.more_settings.native_browser.capacity_format", "%d hosts • %d pending • %d in flight", comment: "Native browser bridge queue format"),
                            hostNativeBrowserStatus.hosts?.count ?? 0,
                            hostNativeBrowserStatus.pending_commands ?? 0,
                            hostNativeBrowserStatus.in_flight_commands ?? 0
                        )
                    )
                    if let host = hostNativeBrowserStatus.hosts?.first {
                        Text(host.host_id)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .padding(.vertical, 4)
            }
        } header: {
            Text(tr("chat.more_settings.execution.header", "Connections and execution · Global", comment: "Global execution and host section header"))
        } footer: {
            Text(tr("chat.more_settings.execution.footer", "Execution settings apply across conversations using this model; each protected action still follows its permission policy.", comment: "Section footer for agent execution"))
        }
    }

    private func computerUseStatusText(_ status: WebAgentProvider.HostComputerUseStatus) -> String {
        if status.feature_enabled == false {
            return tr("chat.more_settings.computer_use.disabled", "Disabled on host", comment: "Computer-use disabled status")
        }
        if status.token_configured == false {
            return tr("chat.more_settings.computer_use.not_configured", "Host token not configured", comment: "Computer-use token missing status")
        }
        if status.connected == true {
            return tr("chat.more_settings.computer_use.connected", "Connected", comment: "Computer-use connected status")
        }
        return tr("chat.more_settings.computer_use.disconnected", "未连接桌面宿主机", comment: "Computer-use disconnected status")
    }

    private func nativeBrowserStatusText(_ status: WebAgentProvider.HostNativeBrowserStatus) -> String {
        if status.token_configured == false {
            return tr("chat.more_settings.native_browser.not_configured", "Native browser token not configured", comment: "Native browser token missing status")
        }
        if status.connected == true {
            return tr("chat.more_settings.native_browser.connected", "Connected", comment: "Native browser connected status")
        }
        return tr("chat.more_settings.native_browser.disconnected", "No Chrome bridge connected", comment: "Native browser disconnected status")
    }

    @ViewBuilder
    private var memorySection: some View {
        Section {
            TextEditor(text: $sessionMemory)
                .frame(minHeight: 90)
                .font(.body)
        } header: {
            Text(tr("chat.more_settings.session_memory.header", "Session memory · Current conversation", comment: "Current conversation session memory section header"))
        } footer: {
            Text(tr("chat.more_settings.session_memory.footer", "Applies only to the current conversation for tracking task state and temporary constraints.", comment: "Session memory section footer"))
        }

        Section {
            TextEditor(text: $longTermMemory)
                .frame(minHeight: 100)
                .font(.body)

            TextEditor(text: $userPreferences)
                .frame(minHeight: 90)
                .font(.body)
        } header: {
            Text(tr("chat.more_settings.long_term.header", "Long-term memory and preferences · Global", comment: "Global long-term memory and preferences section header"))
        } footer: {
            Text(tr("chat.more_settings.long_term.footer", "Long-term memory and user preferences persist across conversations and are injected into future context.", comment: "Long-term memory and preferences section footer"))
        }
    }

    private var skillsSection: some View {
        Section {
            if uploadedSkills.isEmpty && hostUploadedSkills.isEmpty && hostUploadedDocuments.isEmpty && hostMCPServers.isEmpty {
                Text(tr("chat.more_settings.skills.empty", "No skills yet.", comment: "Empty state when no skills are available"))
                    .font(.subheadline)
                    .foregroundColor(.secondary)
            }

            ForEach(uploadedSkills) { skill in
                Toggle(isOn: Binding(
                    get: { skill.enabled },
                    set: { enabled in
                        do {
                            try onSetSkillEnabled(skill.id, enabled)
                        } catch {
                            errorMessage = error.localizedDescription
                        }
                    }
                )) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(skill.name)
                        if !skill.description.isEmpty {
                            Text(skill.description)
                                .font(.caption)
                                .foregroundColor(.secondary)
                                .lineLimit(2)
                        }
                    }
                }
                .disabled(isSaving)
            }

            ForEach(hostUploadedSkills) { skill in
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: skill.enabled ? "checkmark.circle.fill" : "circle")
                        .foregroundColor(skill.enabled ? DesignTokens.successColor : .secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(skill.name)
                        Text(tr("chat.more_settings.skills.host_managed", "由宿主机提供；请在 Mac 上的技能库中管理。", comment: "Hint for host-managed skills"))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }
            }

            ForEach(hostUploadedDocuments) { document in
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: document.enabled ? "doc.text.fill" : "doc.text")
                        .foregroundColor(document.enabled ? DesignTokens.successColor : .secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(document.name)
                        Text(
                            String(
                                format: tr("chat.more_settings.documents.host_format", "Provided by the Web host • %d characters", comment: "Host document summary in more settings"),
                                document.content.count
                            )
                        )
                        .font(.caption)
                        .foregroundColor(.secondary)
                    }
                }
            }

            ForEach(hostMCPServers) { server in
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: server.enabled ? "checkmark.circle.fill" : "circle")
                        .foregroundColor(server.enabled ? DesignTokens.successColor : .secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(server.name)
                        Text(server.endpoint)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
            }

            Button {
                openAfterDismiss(onOpenSkillLibrary)
            } label: {
                Label(tr("chat.more_settings.skills.open_library", "Open full Skill Library", comment: "Button title for opening skill library"), systemImage: "square.grid.2x2")
            }

            Button {
                openAfterDismiss(onOpenContextSettings)
            } label: {
                Label(tr("chat.more_settings.skills.open_context_settings", "Open context, MCP, and data source settings", comment: "Button title for opening context settings"), systemImage: "text.badge.checkmark")
            }
        } header: {
            Text(tr("chat.more_settings.skills.header", "Skills and extended context · Global", comment: "Global skills section header"))
        } footer: {
            Text(tr("chat.more_settings.skills.footer", "Use this panel to quickly enable or disable local skills. For uploading skills and managing MCP, documents, and data sources, open the full management page.", comment: "Skills section footer"))
        }
    }

    private func settingValueRow(title: String, value: String) -> some View {
        HStack {
            Text(title)
            Spacer()
            Text(value)
                .font(.caption)
                .foregroundColor(.secondary)
        }
    }

    private func formattedTokenCount(_ value: Int) -> String {
        if value >= 1_000_000 {
            return String(format: "%.1fM", Double(value) / 1_000_000)
        }
        if value >= 1_000 {
            return String(format: "%.1fK", Double(value) / 1_000)
        }
        return String(value)
    }

    private func saveAll() {
        guard !isSaving else { return }
        errorMessage = nil
        statusMessage = nil
        isSaving = true
        Task {
            do {
                try await onSaveMemoryProfile()
                await MainActor.run {
                    statusMessage = tr("chat.more_settings.saved", "Settings saved.", comment: "Status message after saving chat more settings")
                    isSaving = false
                }
            } catch {
                await MainActor.run {
                    errorMessage = error.localizedDescription
                    isSaving = false
                }
            }
        }
    }

    private func openAfterDismiss(_ action: @escaping () -> Void) {
        guard !isSaving else { return }
        dismiss()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            action()
        }
    }
}

nonisolated struct IOSConversationGenerationDraft: Equatable {
    var contextEnabled: Bool
    var outputEnabled: Bool
    var contextText: String
    var outputText: String

    var value: IOSConversationGenerationOverride? {
        let context = contextEnabled ? ModelParameterNumberParser.integer(contextText) : nil
        let output = outputEnabled ? ModelParameterNumberParser.integer(outputText) : nil
        guard (!contextEnabled || context.map(ModelParameterLimits.remoteContextTokens.contains) == true),
              (!outputEnabled || output.map(ModelParameterLimits.outputTokens.contains) == true) else { return nil }
        return .init(maxContextLength: context, maxOutputTokens: output)
    }

    mutating func inheritModelDefaults() {
        contextEnabled = false
        outputEnabled = false
    }
}

private struct IOSConversationGenerationSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    let model: ModelInfo
    let onSave: (IOSConversationGenerationOverride) throws -> Void
    @Binding private var preventsSheetDismiss: Bool
    @Binding private var editorActive: Bool
    @State private var initialDraft: IOSConversationGenerationDraft
    @State private var editor: IOSConversationGenerationDraft
    @State private var errorMessage: String?
    @State private var showDiscardConfirmation = false
    private enum LimitField: Hashable {
        case context, output
    }
    @FocusState private var limitField: LimitField?

    init(
        model: ModelInfo,
        initialOverride: IOSConversationGenerationOverride,
        preventsSheetDismiss: Binding<Bool>,
        editorActive: Binding<Bool>,
        onSave: @escaping (IOSConversationGenerationOverride) throws -> Void
    ) {
        self.model = model
        self.onSave = onSave
        _preventsSheetDismiss = preventsSheetDismiss
        _editorActive = editorActive
        let defaults = ModelParametersStore.shared.loadParameters(for: model.id, isLocal: false)
        let initial = IOSConversationGenerationDraft(
            contextEnabled: initialOverride.maxContextLength != nil,
            outputEnabled: initialOverride.maxOutputTokens != nil,
            contextText: String(initialOverride.maxContextLength ?? defaults.maxContextLength),
            outputText: String(initialOverride.maxOutputTokens ?? defaults.maxOutputTokens)
        )
        _initialDraft = State(initialValue: initial)
        _editor = State(initialValue: initial)
    }

    private var hasChanges: Bool { editor != initialDraft }

    var body: some View {
        Form {
            Section {
                Toggle(NSLocalizedString("model.parameters.max_context_length", value: "Context window", comment: "Context window parameter"), isOn: $editor.contextEnabled)
                if editor.contextEnabled {
                    TextField("512–4,000,000", text: $editor.contextText)
                        .keyboardType(.numberPad)
                        .focused($limitField, equals: .context)
                        .accessibilityIdentifier("conversation-context-limit")
                }
                Toggle(NSLocalizedString("model.parameters.max_output_tokens", value: "Output token limit", comment: "Output token parameter"), isOn: $editor.outputEnabled)
                if editor.outputEnabled {
                    TextField("64–1,000,000", text: $editor.outputText)
                        .keyboardType(.numberPad)
                        .focused($limitField, equals: .output)
                        .accessibilityIdentifier("conversation-output-limit")
                }
            } footer: {
                Text(NSLocalizedString("chat.more_settings.conversation_model_limits_footer", value: "Unset limits inherit this model's defaults. These values affect only this conversation and model.", comment: "Per-conversation model limits scope"))
            }
            if let errorMessage {
                Section { Text(errorMessage).foregroundStyle(.red) }
            }
            Section {
                Button(NSLocalizedString("chat.style.inherit_model", value: "Use model default", comment: "Reset per-conversation model overrides")) {
                    editor.inheritModelDefaults()
                    errorMessage = nil
                }
            }
        }
        .navigationTitle(model.name)
        .navigationBarBackButtonHidden(hasChanges)
        .interactiveDismissDisabled(hasChanges)
        .onChange(of: hasChanges, initial: true) { _, changed in
            preventsSheetDismiss = changed
        }
        .onAppear { editorActive = true }
        .onDisappear {
            editorActive = false
            preventsSheetDismiss = false
        }
        .confirmationDialog(NSLocalizedString("common.discard_changes", value: "Discard Changes?", comment: "Discard changes confirmation"), isPresented: $showDiscardConfirmation, titleVisibility: .visible) {
            Button(L10n.commonSave) {
                if let value = editor.value { persist(value) }
            }
            .disabled(editor.value == nil)
            Button(NSLocalizedString("model.parameters.discard.action", value: "Discard Changes", comment: "Discard model parameter changes action"), role: .destructive) {
                preventsSheetDismiss = false
                dismiss()
            }
            Button(L10n.cancel, role: .cancel) {}
        }
        .toolbar {
            if hasChanges {
                ToolbarItem(placement: .cancellationAction) {
                    Button { showDiscardConfirmation = true } label: {
                        Label(L10n.commonBack, systemImage: "chevron.backward")
                    }
                    .accessibilityIdentifier("conversation-model-limits-back")
                }
            }
            ToolbarItemGroup(placement: .keyboard) {
                Spacer()
                Button(L10n.done) { limitField = nil }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button(L10n.commonSave) {
                    if let value = editor.value { persist(value) }
                }
                .disabled(editor.value == nil || !hasChanges)
                .accessibilityIdentifier("conversation-model-limits-save")
            }
        }
    }

    private func persist(_ value: IOSConversationGenerationOverride) {
        do {
            try onSave(value)
            preventsSheetDismiss = false
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
