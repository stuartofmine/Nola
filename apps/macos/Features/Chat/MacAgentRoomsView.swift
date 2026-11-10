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
                MacMessageView(conversationID: context, message: message,
                    transcriptWidth: MacChatBubbleLayout.transcriptWidth(availableWidth: width - (usesExpertIdentity ? identityColumnWidth + 12 : 0)),
                    isStreaming: streaming, liveActivities: streaming ? model.visibleActivity : [], isSearchMatch: false,
                    onSendTaskGuidance: { taskID, text in
                        guard let runID = message.journalReference?.cursor.identity.run_id else { return }
                        model.sendTaskGuidance(conversationID: context, parentRunID: runID, taskID: taskID, message: text)
                    },
                    onRequestTaskCancellation: { taskID in
                        guard let runID = message.journalReference?.cursor.identity.run_id else { return }
                        model.requestTaskCancellation(conversationID: context, parentRunID: runID, taskID: taskID)
                    },
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
                Button("Stop response · " + name, role: .destructive) { _ = model.stopComposerRun(conversationID: context) }
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
                Button("Show pending questions") { model.reopenUserInputWaits() }
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
                if let error = model.localRecoveryError { Text(error).font(.caption).foregroundStyle(.red) }
                Button("Check execution status · " + name) { Task { await model.checkLocalRunRecovery() } }
                    .disabled(!model.canCheckLocalRunRecovery)
                if model.canResumeLocalRun {
                    Button("Resume response · " + name) { Task { await model.resumeLocalRun() } }
                }
            }
            if model.remoteRunRecovery != nil {
                if let error = model.remoteRecoveryError { Text(error).font(.caption).foregroundStyle(.red) }
                Button("Resume response · " + name) { Task { await model.resumeRemoteRun() } }
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
    @State private var colorAgent: MacPersistentAgent?
    var manage: (() -> Void)? = nil

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
                    Button("Manage experts", action: manage)
                        .buttonStyle(MacActionButtonStyle())
                        .accessibilityIdentifier("mac-experts-manage")
                }
                Button("Done") { dismiss() }.buttonStyle(MacActionButtonStyle())
            }
            Text("chat.experts.description")
                .font(.callout).foregroundStyle(.secondary)
            TextField("chat.experts.search", text: $query)
            List(visibleAgents, id: \.id) { agent in
                HStack {
                Button {
                    if selection.contains(agent.id) { selection.remove(agent.id) }
                    else { selection.insert(agent.id) }
                } label: {
                    HStack(spacing: 10) {
                        Text(String(agent.name.prefix(2)).uppercased())
                            .font(.caption.bold()).frame(width: 32, height: 32)
                            .background(Color.accentColor.opacity(0.12), in: Circle())
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
                Button { colorAgent = agent } label: { Image(systemName: "paintpalette").frame(width: 36, height: 36) }
                    .buttonStyle(.plain).help(String(localized: "Bubble Color"))
                    .accessibilityLabel(String(localized: "Bubble Color") + " " + agent.name)
                }
            }
            if visibleAgents.isEmpty { Text("chat.experts.empty").foregroundStyle(.secondary) }
        }.padding(16)
            .sheet(item: $colorAgent) { agent in MacExpertColorEditor(agentID: agent.id, name: agent.name) }
    }
}

struct MacUnifiedExpertTranscript: View {
    @Bindable var model: MacWorkspaceModel
    @ObservedObject var workspace: MacAgentRoomWorkspace
    let width: CGFloat
    var viewportStore: MacTranscriptViewportStore? = nil
    @State private var scrollController = MacTranscriptScrollController()

    private var rows: [MacRoomTranscriptRow] {
        MacExpertTranscriptProjector.rows(conversationID: model.selectedConversationID ?? "",
            parentMessages: model.selectedMessages, association: workspace.conversationAssociation,
            receipts: workspace.conversationAssociation.flatMap { workspace.submissions[$0.roomID] } ?? [],
            contexts: workspace.executionService?.owners.mapValues(\.selectedMessages) ?? [:])
    }

    private var visibleParentIDs: [String] {
        model.selectedMessages.filter { $0.role == .user }.compactMap(\.canonicalMessageID).sorted()
    }
    var body: some View {
        let transcriptRows = rows
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                ForEach(Array(transcriptRows.enumerated()), id: \.element.id) { index, row in
                    let message = row.message
                    let owner = row.name == nil ? model : workspace.executionService?.owners[row.context]
                    if let owner {
                        let parentID = model.selectedConversationID
                        MacRoomCanonicalMessage(model: owner, message: message, contextID: row.context,
                            isCurrentOwner: {
                                model.selectedConversationID == parentID && owner.selectedConversationID == row.context
                                    && (row.name == nil || workspace.executionService?.owners[row.context] === owner)
                            },
                            participantName: row.name ?? "Nola", senderIdentity: row.agentID ?? "nola",
                            showsSender: index == 0 || transcriptRows[index - 1].context != row.context || transcriptRows[index - 1].message.role != message.role,
                            width: min(720, width), isExpert: row.name != nil)
                            .background { MacTranscriptMessageAnchor(messageID: message.id, controller: scrollController) }
                    }
                }
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
                ForEach(workspace.resumableExpertMessages.keys.sorted(), id: \.self) { messageID in
                    if let manifest = workspace.resumableExpertMessages[messageID], manifest.input.conversation_id == model.selectedConversationID,
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
                if let error = workspace.expertResumeError { Text(error).font(.caption).foregroundStyle(.red) }
                Color.clear.frame(height: 1).background { MacTranscriptLatestAnchor(controller: scrollController) }
            }.padding(24).frame(maxWidth: 720, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
                .onGeometryChange(for: CGSize.self) { $0.size } action: { _ in scrollController.contentChanged() }
                .background { MacTranscriptScrollObserver(controller: scrollController).frame(width: 0, height: 0) }
        }.background(MacDesignColor.canvas)
            .overlay(alignment: .bottomTrailing) { MacTranscriptFollowControl(controller: scrollController, bottomClearance: 0) }
            .task(id: model.selectedConversationID) {
                scrollController.reset(conversationID: model.selectedConversationID ?? "", viewportStore: viewportStore)
            }
            .task(id: [model.selectedConversationID ?? "", workspace.conversationAssociation?.roomID ?? ""] + visibleParentIDs) {
                guard let owner = model.selectedConversationID else { return }
                await workspace.refreshResumableExpertMessages(conversationID: owner, messageIDs: visibleParentIDs)
            }
            .onDisappear { scrollController.saveViewport() }
    }
}

struct MacExpertPermissionDock: View {
    @ObservedObject var workspace: MacAgentRoomWorkspace
    let conversationID: String?
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
    @Environment(\.dismiss) private var dismiss
    @State private var selected: MacPersistentAgent?
    @State private var name = ""
    @State private var responsibilities = ""
    @State private var isEnabled = true
    @State private var isSaving = false
    @State private var configuration = AppleExpertConfigurationDraft()
    @State private var modelOptions: [MacChatModelOption] = []
    @State private var configurationError: String?
    @State private var showsTemplateLibrary = false
    @State private var showsDiscardConfirmation = false
    @State private var pendingNavigation: Navigation?
    @StateObject private var templates = MacPromptLibrary.service

    private enum Navigation { case select(MacPersistentAgent?), close }

    private var readableTemplates: [PromptTemplate] { templates.templates.filter(templates.canReadTemplate) }
    private var hasUnsavedChanges: Bool {
        guard let original = try? AppleExpertConfigurationDraft(defaults: selected?.defaults ?? .object([:])) else { return true }
        return name != (selected?.name ?? "") || responsibilities != (selected?.description ?? "")
            || isEnabled != (selected?.availability == "active" || selected == nil) || configuration != original
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Manage experts").font(.headline)
                Spacer()
                Button("Done") { requestNavigation(.close) }.buttonStyle(MacActionButtonStyle()).keyboardShortcut(.cancelAction)
                    .disabled(isSaving).accessibilityIdentifier("mac-expert-done")
            }
            HSplitView {
                VStack(alignment: .leading, spacing: 8) {
                    Button("Create expert") { requestNavigation(.select(nil)) }
                        .buttonStyle(MacActionButtonStyle())
                        .accessibilityIdentifier("mac-expert-create")
                    List(workspace.directory, id: \.id) { agent in
                        Button { requestNavigation(.select(agent)) } label: {
                            HStack {
                                Text(agent.name)
                                Spacer()
                                if agent.availability != "active" { Image(systemName: "pause.circle") }
                            }.frame(maxWidth: .infinity, minHeight: 36).contentShape(Rectangle())
                        }.buttonStyle(.plain)
                            .accessibilityIdentifier("mac-expert-edit-" + agent.id)
                    }
                }.frame(minWidth: 170, idealWidth: 210, maxWidth: 260)
                Form {
                    TextField("Expert name", text: $name)
                        .accessibilityIdentifier("mac-expert-name")
                    TextField("Responsibilities and strengths", text: $responsibilities, axis: .vertical)
                        .lineLimit(3...7).accessibilityIdentifier("mac-expert-description")
                    Picker("Model", selection: $configuration.modelID) {
                        Text("Use default model").tag("")
                        ForEach(modelOptions, id: \.id) { option in
                            Text(option.displayName).tag(option.id)
                        }
                        if !configuration.modelID.isEmpty && !modelOptions.contains(where: { $0.id == configuration.modelID }) {
                            Text("Unavailable model: \(configuration.modelID)").tag(configuration.modelID)
                        }
                    }.accessibilityIdentifier("mac-expert-model")
                    Picker("Prompt template", selection: $configuration.templateID) {
                        Text("Use default template").tag("")
                        ForEach(readableTemplates, id: \.id) { template in
                            Text(template.name).tag(template.id.uuidString)
                        }
                        if !configuration.templateID.isEmpty,
                           !readableTemplates.contains(where: { $0.id.uuidString == configuration.templateID }) {
                            Text("Unavailable template: \(configuration.templateID)").tag(configuration.templateID)
                        }
                    }.accessibilityIdentifier("mac-expert-template")
                    Button("Manage prompt templates") { showsTemplateLibrary = true }
                        .buttonStyle(MacActionButtonStyle()).accessibilityIdentifier("mac-expert-manage-templates")
                    Text("Unset selections inherit application defaults. Existing conversations keep their captured configuration.")
                        .font(.caption).foregroundStyle(.secondary)
                    if selected != nil {
                        Toggle("Enabled", isOn: $isEnabled)
                            .accessibilityIdentifier("mac-expert-enabled")
                        Text("Changes apply to future selections. Existing conversations retain their expert snapshot.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Button(selected == nil ? String(localized: "Create expert") : String(localized: "Save")) {
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
                                   !readableTemplates.contains(where: { $0.id.uuidString == configuration.templateID }) {
                                    throw MacApplicationPortError.rejected(String(localized: "Choose a readable prompt template."))
                                }
                                defaults = try configuration.applying(to: initial)
                                configurationError = nil
                            } catch {
                                configurationError = error.localizedDescription
                                isSaving = false
                                return
                            }
                            if let original {
                                await workspace.updateAgent(original, name: editedName, description: description, isEnabled: isEnabled, defaults: defaults)
                            } else {
                                await workspace.createAgent(name: editedName, description: description, defaults: defaults)
                            }
                            isSaving = false
                            if workspace.directoryError == nil {
                                edit(workspace.directory.first { $0.id == original?.id })
                            }
                        }
                    }.buttonStyle(MacActionButtonStyle())
                        .disabled(isSaving || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("mac-expert-save")
                    if let error = workspace.directoryError {
                        Text(error).foregroundStyle(.red).textSelection(.enabled)
                    }
                    if let configurationError { Text(configurationError).foregroundStyle(.red).textSelection(.enabled) }
                }.formStyle(.grouped).frame(minWidth: 320)
            }.disabled(isSaving)
        }.padding(16).frame(minWidth: 620, minHeight: 430)
            .interactiveDismissDisabled(isSaving || hasUnsavedChanges)
            .confirmationDialog(String(localized: "Discard unsaved changes?"), isPresented: $showsDiscardConfirmation) {
                Button(String(localized: "Discard Changes"), role: .destructive) {
                    let action = pendingNavigation; pendingNavigation = nil
                    if let action { performNavigation(action) }
                }.accessibilityIdentifier("mac-expert-discard")
                Button(String(localized: "Keep Editing"), role: .cancel) { pendingNavigation = nil }
                    .accessibilityIdentifier("mac-expert-keep-editing")
            }
            .task { await workspace.refreshExpertDirectory(); refreshModelOptions() }
            .onReceive(NotificationCenter.default.publisher(for: .nolaMacConfigurationDidChange)) { _ in refreshModelOptions() }
            .sheet(isPresented: $showsTemplateLibrary) {
                VStack(alignment: .trailing, spacing: 0) {
                    Button("Done") { showsTemplateLibrary = false }.buttonStyle(MacActionButtonStyle())
                        .keyboardShortcut(.cancelAction).padding()
                    MacPromptLibraryView()
                }.frame(minWidth: 700, minHeight: 580)
            }
    }

    private func edit(_ agent: MacPersistentAgent?) {
        selected = agent; name = agent?.name ?? ""; responsibilities = agent?.description ?? ""
        isEnabled = agent?.availability == "active" || agent == nil
        do {
            configuration = try AppleExpertConfigurationDraft(defaults: agent?.defaults ?? .object([:]))
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
        case .close: dismiss()
        case .select(let agent): edit(agent)
        }
    }

    private func refreshModelOptions() {
        do { modelOptions = try workspace.agentModelOptions?() ?? []; configurationError = nil }
        catch { configurationError = error.localizedDescription }
    }
}
