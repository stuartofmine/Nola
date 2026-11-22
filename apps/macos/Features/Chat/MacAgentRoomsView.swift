// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacRoomCanonicalMessage: View {
    @Bindable var model: MacWorkspaceModel
    let message: MacChatMessage
    let contextID: String
    let isCurrentOwner: () -> Bool
    let participantName: String
    let senderIdentity: String
    let showsSender: Bool
    let width: CGFloat
    @State private var journalResidency = MacJournalPresentationResidency()
    @AppStorage(AppleExpertAppearance.storageKey) private var appearance = Data()
    @Environment(\.colorScheme) private var scheme
    @ScaledMetric(relativeTo: .caption) private var identityColumnWidth: CGFloat = 48
    var isExpert = true
    var isSearchMatch = false
    private var usesExpertIdentity: Bool { message.role != .user && isExpert }
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            if usesExpertIdentity {
                VStack(spacing: 6) {
                    AppleExpertAvatar(name: participantName, size: 32)
                    AppleExpertName(name: participantName, alignment: .center)
                }
                .frame(width: identityColumnWidth)
                .padding(.top, 3)
                .opacity(showsSender ? 1 : 0)
                .accessibilityHidden(!showsSender)
            }
            let context = contextID
            let streaming = model.isSelectedConversationVisiblyRunning && message.role == .assistant && message.id == model.selectedMessages.last?.id
            VStack(alignment: .leading, spacing: 6) {
                MacMessageSurfaceContent(scope: .init(conversationID: context,
                        messageID: message.canonicalMessageID ?? message.journalReference?.cursor.identity.assistant_message_id ?? "",
                        runID: message.journalReference?.cursor.identity.run_id),
                    context: .init(readChanges: model.readUISurfaceChanges, preview: model.previewUISurface,
                        readRevision: model.readUISurfaceRevision, pendingSubmission: model.pendingUIInteraction,
                        submit: model.submitUIInteraction, permitsFollowupActions: !isExpert,
                        readLocalSource: { try model.isLocalConversationSource(context) }),
                    isCurrent: { isCurrentOwner() && model.selectedConversationID == context }) {
                let resolvedTaskRunID = model.taskParentRunID(for: context, message: message)
                MacMessageView(conversationID: context, message: message,
                    transcriptWidth: MacChatBubbleLayout.transcriptWidth(availableWidth: width - (usesExpertIdentity ? identityColumnWidth + 12 : 0)),
                    isStreaming: streaming, liveActivities: streaming ? model.visibleActivity : [], isSearchMatch: isSearchMatch,
                    onSendTaskGuidance: { taskID, text in
                        guard let runID = resolvedTaskRunID else { return }
                        model.sendTaskGuidance(conversationID: context, parentRunID: runID, taskID: taskID, message: text)
                    },
                    onRequestTaskCancellation: { taskID in
                        guard let runID = resolvedTaskRunID else { return }
                        model.requestTaskCancellation(conversationID: context, parentRunID: runID, taskID: taskID)
                    },
                    canControlTasks: resolvedTaskRunID != nil,
                    showsRoleAvatar: !isExpert,
                    assistantBubbleFill: isExpert ? AppleExpertAppearance.selected(appearance, agentID: senderIdentity).fill(scheme) : nil,
                    journalPresentationOwner: message.journalReference.map { journalResidency.owner(for: $0) })
                }
            }
        }
        .onDisappear { journalResidency.removeAll() }
    }
}

private struct MacRoomOwnerControls: View {
    @Bindable var model: MacWorkspaceModel
    let name: String
    let open: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if model.isRunning, let context = model.selectedConversationID {
                Button(String(format: String(localized: "Stop response · %@"), name), role: .destructive) { _ = model.stopComposerRun(conversationID: context) }
            }
            if let request = model.selectedPendingRemotePermission {
                MacHostPermissionCard(request: request,
                    allowOnce: { model.resolvePermission(allow: true, scope: .once, requestID: request.id, expectedRequest: request) },
                    allowConversation: { model.resolvePermission(allow: true, scope: .conversation, requestID: request.id, expectedRequest: request) },
                    deny: { model.resolvePermission(allow: false, requestID: request.id, expectedRequest: request) },
                    isResolving: model.isResolvingPermission,
                    provideInstruction: { try await model.providePermissionInstruction($0, requestID: request.id, expectedRequest: request) })
                    .id(request.presentationIdentity)
            }
            if let request = model.selectedPendingUserInputRequest {
                MacUserInputCard(request: request, submit: { model.resolveUserInput(request: request, answers: $0) },
                    cancel: { model.resolveUserInput(request: request, answers: [], decline: true) },
                    later: { model.deferUserInputWait(request) })
                    .id(request.id).disabled(model.resolvingUserInputWaitIDs.contains(request.id))
            }
            if model.hasDeferredUserInputWaits {
                Button(String(localized: "Show pending questions")) { model.reopenUserInputWaits() }
            }
            if let presentation = model.currentUnknownEffectRecovery {
                MacUnknownEffectRecoveryView(presentation: presentation,
                    isBusy: model.isResolvingUnknownEffects,
                    submit: { decision in
                        Task { await model.resolveUnknownEffects(decision, presentation: presentation) }
                    }, recoverPending: {
                        Task { await model.recoverUnknownEffectDecision(presentation: presentation) }
                    }).id(presentation.id)
            }
            if model.localRunRecovery != nil {
                if let error = model.localRecoveryError { Text(error).font(.caption).foregroundStyle(MacDesignColor.danger) }
                Button(String(format: String(localized: "Check execution status · %@"), name)) { Task { await model.checkLocalRunRecovery() } }
                    .disabled(!model.canCheckLocalRunRecovery)
                if model.canResumeLocalRun {
                    Button(String(format: String(localized: "Resume response · %@"), name)) { Task { await model.resumeLocalRun() } }
                }
            }
            if model.remoteRunRecovery != nil {
                if let error = model.remoteRecoveryError { Text(error).font(.caption).foregroundStyle(MacDesignColor.danger) }
                Button(String(format: String(localized: "Resume response · %@"), name)) { Task { await model.resumeRemoteRun() } }
                    .disabled(!model.canRecoverRemoteRun)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(name)
        .accessibilityIdentifier("room.owner.controls." + (model.selectedConversationID ?? ""))
    }
}


struct MacChatExpertPicker: View {
    let agents: [MacPersistentAgent]
    @Binding var selection: Set<String>
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    var manage: (() -> Void)? = nil
    var selected: ((MacPersistentAgent) -> Void)? = nil

    private var visibleAgents: [MacPersistentAgent] {
        agents.filter { $0.availability == "active" &&
            (query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) ||
             $0.description.localizedCaseInsensitiveContains(query)) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("chat.experts.title").font(.headline)
                Spacer()
                if let manage {
                    Button(String(localized: "Expert Library", table: "ExpertLibrary"), action: manage)
                        .buttonStyle(MacActionButtonStyle())
                        .accessibilityIdentifier("mac-experts-manage")
                }
                Button("Done") { dismiss() }.buttonStyle(MacActionButtonStyle())
            }
            Text("chat.experts.description")
                .font(.callout).foregroundStyle(.secondary)
            TextField("chat.experts.search", text: $query)
            if visibleAgents.isEmpty {
                Text("chat.experts.empty")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
            } else {
                List(visibleAgents, id: \.id) { agent in
                    Button {
                        if let selected { selected(agent) }
                        else if selection.contains(agent.id) { selection.remove(agent.id) }
                        else { selection.insert(agent.id) }
                    } label: {
                        HStack(spacing: 10) {
                            AppleExpertAvatar(name: agent.name, size: 32)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(agent.name).font(.body.weight(.medium))
                                if !agent.description.isEmpty {
                                    Text(agent.description).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                            Image(systemName: selection.contains(agent.id) ? "checkmark.circle.fill" : "circle")
                        }.frame(minHeight: 44).contentShape(Rectangle())
                    }.buttonStyle(.plain)
                        .accessibilityLabel(agent.name)
                        .accessibilityValue(selection.contains(agent.id) ? String(localized: "chat.experts.selected") : String(localized: "chat.experts.not_selected"))
                }
                .scrollContentBackground(.hidden)
            }
        }
        .padding(16)
        .background(MacDesignColor.canvas)
    }
}

struct MacUnifiedExpertTranscript: View {
    @Bindable var model: MacWorkspaceModel
    @ObservedObject var workspace: MacAgentRoomWorkspace
    let width: CGFloat
    var viewportStore: MacTranscriptViewportStore? = nil
    @State private var scrollController = MacTranscriptScrollController()
    @State private var compactionSourceNotice: String?
    @State private var compactionNavigationID: UUID?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var rows: [MacRoomTranscriptRow] {
        MacExpertTranscriptProjector.rows(conversationID: model.selectedConversationID ?? "",
            parentMessages: model.selectedMessages, association: workspace.conversationAssociation,
            receipts: workspace.conversationAssociation.flatMap { workspace.submissions[$0.roomID] } ?? [],
            contexts: workspace.transcriptOwners.mapValues(\.selectedMessages),
            peerExchanges: workspace.peerExchanges)
    }

    private var visibleParentIDs: [String] {
        model.selectedMessages.filter { $0.role == .user }.compactMap(\.canonicalMessageID).sorted()
    }
    var body: some View {
        let transcriptRows = rows
        let conversationID = model.selectedConversationID ?? ""
        VStack(spacing: 0) {
            if let compactionSourceNotice {
                MacInlineNotice(
                    title: String(localized: "Original context", table: "Chat"),
                    message: compactionSourceNotice,
                    kind: .failure,
                    dismiss: { self.compactionSourceNotice = nil }
                )
                .padding(.horizontal, MacDesignTokens.Layout.chatTranscriptHorizontalPadding)
                .padding(.top, MacDesignTokens.Spacing.sm)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
            ScrollViewReader { proxy in
                ScrollView {
                    transcriptListContent(transcriptRows: transcriptRows)
                }.background(MacDesignColor.canvas)
                    .overlay(alignment: .bottomTrailing) { MacTranscriptFollowControl(controller: scrollController, bottomClearance: 0) }
                    .onChange(of: model.transcriptSearchNavigationToken) { _, _ in
                        handleSearchNavigation(proxy: proxy)
                    }
                    .onReceive(NotificationCenter.default.publisher(for: .macCompactionSourceRequested)) { notification in
                        handleCompactionSource(notification, conversationID: conversationID, proxy: proxy)
                    }
                    .task(id: model.selectedConversationID) {
                        compactionNavigationID = nil
                        compactionSourceNotice = nil
                        scrollController.reset(conversationID: model.selectedConversationID ?? "", viewportStore: viewportStore)
                    }
                    .task(id: [model.selectedConversationID ?? "", workspace.conversationAssociation?.roomID ?? ""] + visibleParentIDs) {
                        guard let owner = model.selectedConversationID else { return }
                        await workspace.refreshResumableExpertMessages(conversationID: owner, messageIDs: visibleParentIDs)
                    }
                    .onDisappear {
                        scrollController.saveViewport()
                        compactionNavigationID = nil
                    }
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: compactionSourceNotice)
    }

    private func handleSearchNavigation(proxy: ScrollViewProxy) {
        guard let targetID = model.transcriptSearchFocusedMessageID else { return }
        _ = scrollController.beginNavigation()
        Task { @MainActor in
            await Task.yield()
            if reduceMotion {
                proxy.scrollTo(targetID, anchor: .center)
            } else {
                withAnimation(.easeOut(duration: 0.18)) {
                    proxy.scrollTo(targetID, anchor: .center)
                }
            }
        }
    }

    private func handleCompactionSource(
        _ notification: Notification,
        conversationID: String,
        proxy: ScrollViewProxy
    ) {
        guard let request = notification.object as? MacCompactionSourceRequest,
              !request.sourceItemIDs.isEmpty,
              request.matches(conversationID: conversationID, messages: model.selectedMessages) else { return }
        guard compactionNavigationID == nil else { return }
        let requestID = UUID()
        compactionNavigationID = requestID
        let navigationGeneration = scrollController.beginNavigation()
        Task { @MainActor in
            let outcome = await model.locateCompactionSource(request: request)
            guard compactionNavigationID == requestID else { return }
            compactionNavigationID = nil
            guard scrollController.acceptsNavigation(navigationGeneration) else { return }
            switch outcome {
            case .found(let target, _):
                await Task.yield()
                guard scrollController.acceptsNavigation(navigationGeneration) else { return }
                proxy.scrollTo(target, anchor: .center)
            case .end:
                compactionSourceNotice = String(localized: "mac.chat.compaction_source.not_found")
            case .stalled:
                compactionSourceNotice = String(localized: "mac.chat.compaction_source.cursor_stalled")
            case .snapshotChanged:
                compactionSourceNotice = String(localized: "mac.chat.compaction_source.snapshot_changed")
            case .readError:
                compactionSourceNotice = String(localized: "mac.chat.compaction_source.read_failed")
            case .cancelled:
                break
            }
        }
    }

    private func transcriptListContent(transcriptRows: [MacRoomTranscriptRow]) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            historyNavigationControls
            ForEach(Array(transcriptRows.enumerated()), id: \.element.id) { index, row in
                transcriptRowView(index: index, row: row, transcriptRows: transcriptRows)
            }
            roomErrorSection
            resumableExpertSection
            if let error = workspace.expertResumeError {
                Text(error).font(.caption).foregroundStyle(.red)
            }
            Color.clear.frame(height: 1).background { MacTranscriptLatestAnchor(controller: scrollController) }
        }
        .padding(24)
        .frame(maxWidth: 720, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .center)
        .onGeometryChange(for: CGSize.self) { $0.size } action: { _ in scrollController.contentChanged() }
        .background { MacTranscriptScrollObserver(controller: scrollController).frame(width: 0, height: 0) }
    }

    @ViewBuilder
    private var historyNavigationControls: some View {
        if model.isViewingStoredHistory {
            Button {
                Task { await model.returnToLatestStoredMessages() }
            } label: {
                Label(String(localized: "Go to Latest Message"), systemImage: "arrow.down.circle")
            }
            .buttonStyle(.plain)
            .font(.system(size: 12, weight: .medium))
            .disabled(model.isLoadingHistoryPage)
            .frame(maxWidth: .infinity)
            .accessibilityIdentifier("chat.history.returnToLatest")
        }
        if model.hasEarlierStoredMessages {
            Button {
                Task { await model.loadEarlierStoredMessages() }
            } label: {
                Label(String(localized: "Load earlier messages"), systemImage: "arrow.up.circle")
            }
            .buttonStyle(.plain)
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.vertical, 6)
            .disabled(model.isLoadingHistoryPage)
            .frame(maxWidth: .infinity)
            .accessibilityIdentifier("chat.history.loadEarlier")
        }
    }

    @ViewBuilder
    private func transcriptRowView(index: Int, row: MacRoomTranscriptRow, transcriptRows: [MacRoomTranscriptRow]) -> some View {
        let message = row.message
        let owner = row.name == nil ? model : workspace.executionService?.owners[row.context]
        if let owner {
            let parentID = model.selectedConversationID
            let isMatch = !model.transcriptSearchText.isEmpty
                && message.text.localizedCaseInsensitiveContains(model.transcriptSearchText)
            MacRoomCanonicalMessage(
                model: owner,
                message: message,
                contextID: row.context,
                isCurrentOwner: {
                    model.selectedConversationID == parentID && owner.selectedConversationID == row.context
                        && (row.name == nil || workspace.executionService?.owners[row.context] === owner)
                },
                participantName: row.name ?? "Nola",
                senderIdentity: row.agentID ?? "nola",
                showsSender: index == 0 || transcriptRows[index - 1].context != row.context || transcriptRows[index - 1].message.role != message.role,
                width: min(720, width),
                isExpert: row.name != nil,
                isSearchMatch: isMatch
            )
            .id(message.id)
            .background { MacTranscriptMessageAnchor(messageID: message.id, controller: scrollController) }
        }
    }

    @ViewBuilder
    private var roomErrorSection: some View {
        if let association = workspace.conversationAssociation {
            if let error = workspace.errors[association.roomID] {
                Text(error).foregroundStyle(.red).textSelection(.enabled)
                    .accessibilityIdentifier("chat.expert-execution-error")
            }
            ForEach(workspace.pending[association.roomID] ?? []) { item in
                if let failure = item.failure {
                    Text(failure).foregroundStyle(.red)
                    Button("Retry expert reply") { workspace.retry(item) }
                }
            }
        }
    }

    @ViewBuilder
    private var resumableExpertSection: some View {
        ForEach(workspace.resumableExpertMessages.keys.sorted(), id: \.self) { messageID in
            if let manifest = workspace.resumableExpertMessages[messageID],
               manifest.input.conversation_id == model.selectedConversationID,
               !workspace.isDeliveringExpertMessage(manifest) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("room.pending_expert_message").font(.caption).foregroundStyle(.secondary)
                    Text(manifest.input.text).lineLimit(3)
                    Button("room.resume_complete_experts") { Task { await workspace.resumeCompleteExpertMessage(messageID) } }
                        .disabled(workspace.isAdmitting || workspace.resumingExpertMessage != nil)
                        .accessibilityIdentifier("mac-experts-resume-" + messageID)
                }
            }
        }
    }
}

struct MacExpertPermissionDock: View {
    @ObservedObject var workspace: MacAgentRoomWorkspace
    let conversationID: String?

    static func hasControls(workspace: MacAgentRoomWorkspace?, conversationID: String?) -> Bool {
        guard let workspace,
              let association = workspace.conversationAssociation,
              association.conversationID == conversationID else { return false }
        return association.room.room.participants.contains { participant in
            guard let owner = workspace.executionService?.owners[participant.contextConversationID] else { return false }
            return owner.isRunning
                || owner.selectedPendingRemotePermission != nil
                || owner.selectedPendingUserInputRequest != nil
                || owner.hasDeferredUserInputWaits
                || owner.currentUnknownEffectRecovery != nil
                || owner.localRunRecovery != nil
                || owner.remoteRunRecovery != nil
        }
    }

    var body: some View {
        if let association = workspace.conversationAssociation, association.conversationID == conversationID {
            ForEach(association.room.room.participants) { participant in
                if let owner = workspace.executionService?.owners[participant.contextConversationID] {
                    MacRoomOwnerControls(model: owner, name: participant.displayName, open: {})
                }
            }
        }
    }
}


/// Directory management does not open conversations or change selected peers.
struct MacExpertDirectoryManagementView: View {
    @ObservedObject var workspace: MacAgentRoomWorkspace
    var showsCloseButton: Bool = true
    var onUnsavedChangesChanged: ((Bool) -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var builtinPreview: AppleExpertRoleTemplate?
    @State private var hasChosenEditor = false
    @State private var selected: MacPersistentAgent?
    @State private var name = ""
    @State private var responsibilities = ""
    @State private var isEnabled = true
    @State private var isSaving = false
    @State private var configuration = AppleExpertConfigurationDraft()
    @State private var supportsExecutors = false
    @State private var executorOptions: [AppleExpertExecutorOption] = []
    @State private var modelOptions: [MacChatModelOption] = []
    @State private var configurationError: String?
    @State private var showsTemplateLibrary = false
    @State private var showsTemplateOverride = false
    @State private var roleTemplates: [AppleExpertRoleTemplate] = []
    @State private var supportsExpertReasoning = false
    @State private var previewRoleID = ""
    @State private var confirmsRoleReplacement = false
    @State private var reasoningEfforts: [String] = []
    @Environment(\.locale) private var locale
    @State private var sidebarQuery = ""
    @State private var confirmsDelete = false
    @State private var showsDiscardConfirmation = false
    @State private var allowsConfirmedClose = false
    @State private var pendingNavigation: Navigation?
    @StateObject private var templates = MacPromptLibrary.service

    private enum Navigation { case select(MacPersistentAgent?), preview(AppleExpertRoleTemplate), close }

    private var readableTemplates: [PromptTemplate] { templates.templates.filter(templates.canReadTemplate) }
    private var filteredRoleTemplates: [AppleExpertRoleTemplate] {
        let needle = sidebarQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !needle.isEmpty else { return roleTemplates }
        return roleTemplates.filter {
            $0.localizedName(locale: locale).localizedCaseInsensitiveContains(needle)
                || $0.localizedDescription(locale: locale).localizedCaseInsensitiveContains(needle)
        }
    }
    private var filteredCustomAgents: [MacPersistentAgent] {
        let needle = sidebarQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !needle.isEmpty else { return workspace.directory }
        return workspace.directory.filter {
            $0.name.localizedCaseInsensitiveContains(needle)
                || $0.description.localizedCaseInsensitiveContains(needle)
        }
    }
    private var hasUnsavedChanges: Bool {
        if builtinPreview != nil { return false }
        guard let original = try? AppleExpertConfigurationDraft(defaults: selected?.defaults ?? .object([:])) else { return true }
        return name != (selected?.name ?? "") || responsibilities != (selected?.description ?? "")
            || isEnabled != (selected?.availability == "active" || selected == nil) || configuration != original
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if showsCloseButton {
                HStack {
                    Text(String(localized: "Expert Library", table: "ExpertLibrary")).font(.headline)
                    Spacer()
                    Button("Done") { requestNavigation(.close) }.buttonStyle(MacActionButtonStyle()).keyboardShortcut(.cancelAction)
                        .disabled(isSaving).accessibilityIdentifier("mac-expert-done")
                }
            }
            HSplitView {
                VStack(alignment: .leading, spacing: 8) {
                    Button("Create expert") { requestNavigation(.select(nil)) }
                        .buttonStyle(MacActionButtonStyle())
                        .accessibilityIdentifier("mac-expert-create")
                    HStack(spacing: 6) {
                        Image(systemName: "magnifyingglass")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                        TextField(String(localized: "Search experts", table: "ExpertLibrary"), text: $sidebarQuery)
                            .textFieldStyle(.plain)
                            .font(.system(size: 12.5))
                        if !sidebarQuery.isEmpty {
                            Button { sidebarQuery = "" } label: {
                                Image(systemName: "xmark.circle.fill")
                                    .foregroundStyle(.tertiary)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal, 8)
                    .frame(height: 30)
                    .background(MacDesignColor.subtleFill, in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field, style: .continuous))
                    List {
                        Section(String(localized: "Built-in experts", table: "ExpertLibrary")) {
                            ForEach(filteredRoleTemplates) { role in
                                let roleName = role.localizedName(locale: locale)
                                Button { requestNavigation(.preview(role)) } label: {
                                    HStack(spacing: 8) {
                                        AppleExpertAvatar(name: roleName, size: 24)
                                        Text(roleName)
                                            .foregroundStyle(.primary)
                                            .lineLimit(1)
                                    }
                                    .frame(maxWidth: .infinity, minHeight: 34, alignment: .leading)
                                    .contentShape(Rectangle())
                                }.buttonStyle(.plain)
                                    .accessibilityIdentifier("mac-expert-builtin-" + role.id)
                                    .macSelectableListRow(isSelected: builtinPreview?.id == role.id)
                            }
                        }
                        Section(String(localized: "My experts", table: "ExpertLibrary")) {
                            if filteredCustomAgents.isEmpty {
                                Text(workspace.directory.isEmpty
                                     ? String(localized: "No custom experts yet", table: "ExpertLibrary")
                                     : String(localized: "No matching experts", table: "ExpertLibrary"))
                                    .font(.caption).foregroundStyle(.secondary)
                                    .listRowBackground(Color.clear)
                            }
                            ForEach(filteredCustomAgents, id: \.id) { agent in
                                Button { requestNavigation(.select(agent)) } label: {
                                    HStack(spacing: 8) {
                                        AppleExpertAvatar(name: agent.name, size: 24)
                                        Text(agent.name)
                                            .foregroundStyle(.primary)
                                            .lineLimit(1)
                                        Spacer()
                                        if agent.availability != "active" { Image(systemName: "pause.circle").foregroundStyle(.secondary) }
                                    }.frame(maxWidth: .infinity, minHeight: 34).contentShape(Rectangle())
                                }.buttonStyle(.plain)
                                    .accessibilityIdentifier("mac-expert-edit-" + agent.id)
                                    .macSelectableListRow(isSelected: builtinPreview == nil && selected?.id == agent.id)
                            }
                        }
                    }
                    .listStyle(.sidebar)
                    .scrollContentBackground(.hidden)
                }
                .frame(minWidth: 200, idealWidth: 230, maxWidth: 270, maxHeight: .infinity, alignment: .topLeading)
                .background(MacDesignColor.canvas)
                if let role = builtinPreview {
                    builtinDetail(role)
                } else {
                    VStack(spacing: 0) {
                        Form {
                            Section(String(localized: "Identity", table: "ExpertLibrary")) {
                                TextField("Expert name", text: $name)
                                    .accessibilityIdentifier("mac-expert-name")
                                TextField("Responsibilities and strengths", text: $responsibilities, axis: .vertical)
                                    .lineLimit(3...7).accessibilityIdentifier("mac-expert-description")
                                if selected != nil {
                                    Toggle("Enabled", isOn: $isEnabled)
                                        .toggleStyle(.macSwitch)
                                        .accessibilityIdentifier("mac-expert-enabled")
                                    Text("Changes apply to future selections. Existing conversations retain their expert snapshot.")
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            Section(String(localized: "Model & Reasoning", table: "ExpertLibrary")) {
                                AppleExpertExecutorControls(selected: $configuration.executor, catalog: executorOptions,
                                    executorLabel: "Execution Engine", modelLabel: "Model", defaultLabel: "Use default model")
                                if configuration.executor == nil {
                                    Picker("Model", selection: $configuration.modelID) {
                                        Text("Use application default model").tag("")
                                        ForEach(modelOptions, id: \.id) { option in
                                            Text(option.displayName).tag(option.id)
                                        }
                                        if !configuration.modelID.isEmpty && !modelOptions.contains(where: { $0.id == configuration.modelID }) {
                                            Text("Unavailable model: \(configuration.modelID)").tag(configuration.modelID)
                                        }
                                    }.accessibilityIdentifier("mac-expert-model")
                                    .onChange(of: configuration.modelID) { _, _ in
                                        refreshReasoningEfforts()
                                    }
                                    reasoningControls
                                }
                                Text("Unset selections inherit application defaults. Existing conversations keep their captured configuration.")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Section(String(localized: "Role & Prompt", table: "ExpertLibrary")) {
                                roleControls
                                templateControls
                            }
                        }
                        .formStyle(.grouped)
                        .scrollContentBackground(.hidden)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)

                        Divider()
                        VStack(alignment: .leading, spacing: 8) {
                            if let error = workspace.directoryError {
                                Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled)
                            }
                            if let configurationError {
                                Text(configurationError).font(.callout).foregroundStyle(.red).textSelection(.enabled)
                            }
                            HStack(spacing: 10) {
                                if selected != nil {
                                    Button(String(localized: "Delete", table: "ExpertLibrary"), role: .destructive) {
                                        confirmsDelete = true
                                    }
                                    .buttonStyle(.bordered)
                                    .disabled(isSaving)
                                    .accessibilityIdentifier("mac-expert-delete")
                                }
                                Spacer()
                                if isSaving { ProgressView().controlSize(.small) }
                                Button(selected == nil ? String(localized: "Create expert") : String(localized: "Save")) {
                                    saveExpert()
                                }
                                .buttonStyle(.nolaProminent)
                                .disabled(isSaving || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                                .accessibilityIdentifier("mac-expert-save")
                            }
                        }
                        .padding(.horizontal, 20)
                        .padding(.vertical, 12)
                        .background(MacDesignColor.chrome.opacity(0.65))
                    }
                    .frame(minWidth: 320, maxHeight: .infinity)
                }
            }.disabled(isSaving)
        }
        .confirmationDialog(
            String(localized: "Delete this custom expert?", table: "ExpertLibrary"),
            isPresented: $confirmsDelete,
            titleVisibility: .visible
        ) {
            Button(String(localized: "Delete", table: "ExpertLibrary"), role: .destructive) {
                guard let original = selected else { return }
                isSaving = true
                Task {
                    await workspace.updateAgent(
                        original,
                        name: original.name,
                        description: original.description,
                        isEnabled: false,
                        defaults: original.defaults
                    )
                    isSaving = false
                    if workspace.directoryError == nil {
                        selected = nil
                        builtinPreview = roleTemplates.first
                    }
                }
            }
            Button(String(localized: "Cancel"), role: .cancel) {}
        } message: {
            Text(String(localized: "This disables the expert for future conversations. Existing conversations keep their saved history.", table: "ExpertLibrary"))
        }
        .padding(16)
        .background(MacPageBackground())
        .frame(
            minWidth: showsCloseButton ? 760 : 0,
            idealWidth: showsCloseButton ? 860 : nil,
            maxWidth: showsCloseButton ? nil : .infinity,
            minHeight: showsCloseButton ? 560 : 0,
            idealHeight: showsCloseButton ? 640 : nil,
            maxHeight: showsCloseButton ? nil : .infinity
        )
            .interactiveDismissDisabled(showsCloseButton && (isSaving || hasUnsavedChanges))
            .onChange(of: hasUnsavedChanges, initial: true) { _, dirty in
                onUnsavedChangesChanged?(dirty)
            }
            .background {
                if showsCloseButton {
                    MacEditorWindowCloseGuard(
                        shouldPreventClose: { (isSaving || hasUnsavedChanges) && !allowsConfirmedClose },
                        onCloseAttempt: { requestNavigation(.close) }
                    )
                }
            }
            .confirmationDialog(String(localized: "Discard unsaved changes?"), isPresented: $showsDiscardConfirmation) {
                Button(String(localized: "Discard Changes"), role: .destructive) {
                    let action = pendingNavigation; pendingNavigation = nil
                    if let action { performNavigation(action) }
                }.accessibilityIdentifier("mac-expert-discard")
                Button(String(localized: "Keep Editing"), role: .cancel) { pendingNavigation = nil }
                    .accessibilityIdentifier("mac-expert-keep-editing")
            }
            .confirmationDialog(String(localized: "Replace prompt template?"), isPresented: $confirmsRoleReplacement) {
                Button("Apply role template") { applyPreviewRole() }
                Button("Keep current template", role: .cancel) {}
            } message: {
                Text("The expert will use this role instead. Your custom prompt template is kept in the library.")
            }
            .task {
                await workspace.refreshExpertDirectory()
                refreshModelOptions()
                let service = workspace.service
                do {
                    let catalog = try await Task.detached { try service.roleTemplates() }.value
                    roleTemplates = catalog.templates
                    if !hasChosenEditor && builtinPreview == nil { builtinPreview = catalog.templates.first }
                    if roleTemplates.contains(where: { $0.id == configuration.templateID }) { previewRoleID = configuration.templateID }
                    supportsExecutors = catalog.features.externalExpertExecutors
                    supportsExpertReasoning = catalog.features.reasoningEffort && catalog.features.explicitGenerationReasoning
                }
                catch { configurationError = error.localizedDescription }
            }
            .task(id: supportsExecutors) {
                guard supportsExecutors else { executorOptions = []; return }
                do {
                    executorOptions = try await MacLocalExecutionBackendCatalog.fetch().filter { $0.id != "nola" }.map { item in
                        .init(id: item.id, name: item.displayName, available: item.isAvailable,
                            models: item.models.map { .init(id: $0.id, display_name: $0.displayName, provider_id: $0.providerID) })
                    }
                } catch { configurationError = error.localizedDescription }
            }
            .onReceive(NotificationCenter.default.publisher(for: .nolaMacConfigurationDidChange)) { _ in refreshModelOptions() }
            .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { _ in
                if configuration.modelID.isEmpty { refreshReasoningEfforts() }
            }
            .sheet(isPresented: $showsTemplateLibrary) {
                VStack(alignment: .trailing, spacing: 0) {
                    Button("Done") { showsTemplateLibrary = false }.buttonStyle(MacActionButtonStyle())
                        .keyboardShortcut(.cancelAction).padding()
                    MacPromptLibraryView()
                }.frame(minWidth: 700, minHeight: 580)
            }
    }

    private func builtinDetail(_ role: AppleExpertRoleTemplate) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(role.localizedName(locale: locale)).font(.title2.weight(.semibold))
                Text(role.localizedDescription(locale: locale)).foregroundStyle(.secondary)
                Button(String(localized: "Customize a copy", table: "ExpertLibrary")) {
                    edit(nil)
                    previewRoleID = role.id
                    applyPreviewRole()
                }.buttonStyle(MacActionButtonStyle()).accessibilityIdentifier("mac-expert-customize-builtin")
                Text(String(localized: "Prompt", table: "ExpertLibrary")).font(.headline)
                Text(role.prompt).textSelection(.enabled)
                Text("Tools").font(.headline)
                ForEach(role.toolPack.tools, id: \.self) { tool in
                    Text(MacToolPresentationPolicy.displayTitle(for: tool)).font(.callout)
                }
                ForEach(role.skills) { skill in
                    Text(skill.localizedName(locale: locale)).font(.headline)
                    Text(skill.instructions).textSelection(.enabled)
                }
            }.frame(maxWidth: .infinity, alignment: .leading).padding(20)
        }.frame(minWidth: 320).accessibilityIdentifier("mac-expert-builtin-preview")
    }

    @ViewBuilder private var templateControls: some View {
        if roleTemplates.contains(where: { $0.id == configuration.templateID }) && !showsTemplateOverride {
            Button("Choose another prompt template") { showsTemplateOverride = true }
                .buttonStyle(.link).accessibilityIdentifier("mac-expert-change-template")
        } else {
            Picker("Prompt template", selection: $configuration.templateID) {
                Text("Use default template").tag("")
                if let role = roleTemplates.first(where: { $0.id == configuration.templateID }) {
                    Text(role.localizedName(locale: locale)).tag(role.id)
                }
                ForEach(readableTemplates, id: \.id) { template in
                    Text(template.name).tag(template.id.uuidString)
                }
                if !configuration.templateID.isEmpty,
                   !readableTemplates.contains(where: { $0.id.uuidString == configuration.templateID }),
                   !roleTemplates.contains(where: { $0.id == configuration.templateID }) {
                    Text("Unavailable template: \(configuration.templateID)").tag(configuration.templateID)
                }
            }.accessibilityIdentifier("mac-expert-template")
            Button("Manage prompt templates") { showsTemplateLibrary = true }
                .buttonStyle(MacActionButtonStyle()).accessibilityIdentifier("mac-expert-manage-templates")
        }
    }

    @ViewBuilder private var reasoningControls: some View {
        Picker(String(localized: "reasoning.effort.title", table: "Reasoning"), selection: $configuration.reasoningEffort) {
            Text(String(localized: "reasoning.effort.automatic", table: "Reasoning")).tag("automatic")
            ForEach(reasoningEfforts, id: \.self) { effort in
                Text(String(localized: String.LocalizationValue("reasoning.effort." + effort), table: "Reasoning")).tag(effort)
            }
            if configuration.reasoningEffort != "automatic", !reasoningEfforts.contains(configuration.reasoningEffort) {
                Text("\(String(localized: "Unavailable")) · \(configuration.reasoningEffort)")
                    .tag(configuration.reasoningEffort).disabled(true)
            }
        }.accessibilityIdentifier("mac-expert-reasoning-effort").disabled(!supportsExpertReasoning)
        if reasoningEfforts.isEmpty {
            Text("This model has no declared reasoning levels. Automatic inherits its configuration.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var roleControls: some View {
        Picker("Role template", selection: $previewRoleID) {
            Text("Choose a role").tag("")
            ForEach(roleTemplates) { role in Text(role.localizedName(locale: locale)).tag(role.id) }
        }.accessibilityIdentifier("mac-expert-role")
        if let role = roleTemplates.first(where: { $0.id == previewRoleID }) {
            Text(role.localizedDescription(locale: locale)).font(.caption).foregroundStyle(.secondary)
            DisclosureGroup("Prompt") {
                Text(role.prompt).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
            }.disclosureGroupStyle(MacRowDisclosureStyle(toggleIdentifier: "mac-expert-role-prompt", isPlain: true))
            DisclosureGroup("Tools") {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 130), alignment: .leading)], alignment: .leading, spacing: 6) {
                    ForEach(role.toolPack.tools, id: \.self) { tool in
                        Text(MacToolPresentationPolicy.displayTitle(for: tool)).font(.caption)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 8).padding(.vertical, 5)
                            .background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 6))
                            .help(tool)
                    }
                }
                Text("Role tools guide the workflow; they do not grant permissions.")
                    .font(.caption).foregroundStyle(.secondary)
            }.disclosureGroupStyle(MacRowDisclosureStyle(toggleIdentifier: "mac-expert-role-tools", isPlain: true))
            ForEach(role.skills) { skill in
                DisclosureGroup(skill.localizedName(locale: locale)) {
                    Text(skill.instructions).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                }.disclosureGroupStyle(MacRowDisclosureStyle(toggleIdentifier: "mac-expert-role-skill-" + skill.id, isPlain: true))
            }
            Button("Apply role template") {
                if !configuration.templateID.isEmpty, configuration.templateID != role.id { confirmsRoleReplacement = true }
                else { applyPreviewRole() }
            }.buttonStyle(MacActionButtonStyle()).accessibilityIdentifier("mac-expert-apply-role")
                .disabled(configuration.templateID == role.id)
        }
    }

    private func applyPreviewRole() {
        guard let role = roleTemplates.first(where: { $0.id == previewRoleID }) else { return }
        configuration.templateID = role.id
        showsTemplateOverride = false
        if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { name = role.localizedName(locale: locale) }
        if responsibilities.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            responsibilities = role.localizedDescription(locale: locale)
        }
    }

    private func refreshReasoningEfforts() {
        do { reasoningEfforts = try workspace.agentReasoningEfforts?(configuration.modelID) ?? [] }
        catch { reasoningEfforts = []; configurationError = error.localizedDescription }
    }

    private func edit(_ agent: MacPersistentAgent?) {
        hasChosenEditor = true
        builtinPreview = nil
        selected = agent; name = agent?.name ?? ""; responsibilities = agent?.description ?? ""
        showsTemplateOverride = false
        isEnabled = agent?.availability == "active" || agent == nil
        do {
            configuration = try AppleExpertConfigurationDraft(defaults: agent?.defaults ?? .object([:]))
            previewRoleID = roleTemplates.contains(where: { $0.id == configuration.templateID }) ? configuration.templateID : ""
            refreshReasoningEfforts()
            configurationError = nil
        } catch { configurationError = error.localizedDescription }
    }

    private func requestNavigation(_ action: Navigation) {
        guard !isSaving else { return }
        if hasUnsavedChanges { pendingNavigation = action; showsDiscardConfirmation = true }
        else { performNavigation(action) }
    }

    private func performNavigation(_ action: Navigation) {
        switch action {
        case .close: allowsConfirmedClose = true; dismiss()
        case .select(let agent): edit(agent)
        case .preview(let role): builtinPreview = role
        }
    }

    private func saveExpert() {
        isSaving = true
        let original = selected
        let editedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let description = responsibilities.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            let defaults: MacCanonicalJSONValue
            do {
                let initial = original?.defaults ?? .object([:])
                let previous = try AppleExpertConfigurationDraft(defaults: initial)
                let currentOptions = try workspace.agentModelOptions?() ?? []
                if configuration.modelID != previous.modelID, !configuration.modelID.isEmpty,
                   !currentOptions.contains(where: { $0.id == configuration.modelID }) {
                    throw MacApplicationPortError.rejected(String(localized: "Choose an available expert model."))
                }
                if configuration.templateID != previous.templateID, !configuration.templateID.isEmpty,
                   !readableTemplates.contains(where: { $0.id.uuidString == configuration.templateID }),
                   !roleTemplates.contains(where: { $0.id == configuration.templateID }) {
                    throw MacApplicationPortError.rejected(String(localized: "Choose a readable prompt template."))
                }
                if configuration.reasoningEffort != previous.reasoningEffort, !supportsExpertReasoning {
                    throw MacApplicationPortError.rejected(String(localized: "This Host does not support expert reasoning levels."))
                }
                if configuration.reasoningEffort != "automatic",
                   configuration.reasoningEffort != previous.reasoningEffort || configuration.modelID != previous.modelID {
                    let supported = try workspace.agentReasoningEfforts?(configuration.modelID) ?? []
                    guard supported.contains(configuration.reasoningEffort) else {
                        throw MacApplicationPortError.rejected(String(localized: "Choose an available expert reasoning level."))
                    }
                }
                defaults = try configuration.applying(to: initial)
                configurationError = nil
            } catch {
                configurationError = error.localizedDescription
                isSaving = false
                return
            }
            let savedID: String?
            if let original {
                savedID = original.id
                await workspace.updateAgent(original, name: editedName, description: description, isEnabled: isEnabled, defaults: defaults)
            } else {
                savedID = await workspace.createAgent(name: editedName, description: description, defaults: defaults)
            }
            isSaving = false
            if workspace.directoryError == nil {
                edit(workspace.directory.first { $0.id == savedID })
            }
        }
    }

    private func refreshModelOptions() {
        do { modelOptions = try workspace.agentModelOptions?() ?? []; refreshReasoningEfforts(); configurationError = nil }
        catch { configurationError = error.localizedDescription }
    }
}
