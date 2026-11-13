// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct IOSAgentRoomsView: View {
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.dismiss) private var dismiss
    @State private var agents: [IOSPersistentAgent] = []
    @State private var editing: IOSPersistentAgent?
    @State private var agentName = ""
    @State private var agentDescription = ""
    @State private var error: String?
    @State private var models: [ModelInfo] = []
    @State private var showingTemplateLibrary = false
    @State private var configuration = AppleExpertConfigurationDraft()
    private enum PendingEditorAction { case edit(IOSPersistentAgent?), archive, close }
    @State private var pendingEditorAction: PendingEditorAction?
    @State private var showDiscardConfirmation = false
    @FocusState private var editorFocused: Bool
    private var service: IOSAgentRoomService { IOSAgentRoomService(runtime: container.applicationRuntime) }
    private var editorIsDirty: Bool {
        guard let original = try? AppleExpertConfigurationDraft(defaults: editing?.defaults ?? .object([:])) else { return true }
        return agentName != (editing?.name ?? "") || agentDescription != (editing?.description ?? "") || configuration != original
    }

    var body: some View {
        NavigationStack {
            List {
                if let error { Text(error).foregroundStyle(.red) }
                Section("chat.experts.title") {
                    ForEach(agents) { agent in
                        Button { requestEditorAction(.edit(agent)) } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(agent.name).foregroundStyle(.primary)
                                    if !agent.description.isEmpty {
                                        Text(agent.description).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                                Spacer()
                                if agent.availability != "active" { Image(systemName: "archivebox").foregroundStyle(.secondary) }
                                Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
                            }.frame(minHeight: 44).contentShape(Rectangle())
                        }.accessibilityIdentifier("experts.edit." + agent.id)
                    }
                    Button("room.create_agent") { requestEditorAction(.edit(nil)) }
                        .accessibilityIdentifier("experts.create")
                }
                Section(String(localized: editing == nil ? "room.create_agent" : "experts.edit.title")) {
                    TextField("room.agent_name", text: $agentName).focused($editorFocused).accessibilityIdentifier("experts.name")
                    TextField("room.agent_description", text: $agentDescription, axis: .vertical)
                        .lineLimit(2...4).focused($editorFocused).accessibilityIdentifier("experts.description")
                    Picker("room.model", selection: $configuration.modelID) {
                        Text("experts.conversationModel").tag("")
                        ForEach(models, id: \.id) { model in
                            Text(model.name).tag(IOSConversationGenerationOverride.modelKey(for: model))
                        }
                        if !configuration.modelID.isEmpty,
                           !models.contains(where: { IOSConversationGenerationOverride.modelKey(for: $0) == configuration.modelID }) {
                            Text("experts.savedModelUnavailable").tag(configuration.modelID)
                        }
                    }.accessibilityIdentifier("experts.model")
                    if let service = container.promptTemplates {
                        IOSExpertTemplatePicker(service: service, selection: $configuration.templateID)
                    } else {
                        Text(String(localized: "prompt.templates.error.unavailable")).foregroundStyle(.secondary)
                    }
                    Button { showingTemplateLibrary = true } label: {
                        HStack {
                            Text(String(localized: "experts.manageTemplates"))
                            Spacer()
                            Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
                        }.frame(minHeight: 44).contentShape(Rectangle())
                    }.buttonStyle(.plain).accessibilityIdentifier("experts.templates.manage")
                    Button { perform { try saveAgent() } } label: {
                        if editing == nil { Text(String(localized: "room.create_agent")) }
                        else { Text(String(localized: "experts.save")) }
                    }
                        .disabled(agentName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("experts.save")
                    if let editing {
                        Button { requestEditorAction(.archive) } label: {
                            if editing.availability == "active" { Text(String(localized: "experts.archive")) }
                            else { Text(String(localized: "experts.restore")) }
                        }.accessibilityIdentifier("experts.archive")
                    }
                }
            }
            .scrollDismissesKeyboard(.immediately)
            .navigationTitle("experts.manage")
            .sheet(isPresented: $showingTemplateLibrary, onDismiss: {
                perform { try container.promptTemplates?.reloadFromPersistence() }
            }) {
                NavigationStack {
                    PromptTemplatesView()
                        .toolbar { ToolbarItem(placement: .cancellationAction) {
                            Button(L10n.done) { showingTemplateLibrary = false }
                                .accessibilityIdentifier("experts.templates.done")
                        } }
                }.environmentObject(container)
            }
            .toolbar { ToolbarItem(placement: .confirmationAction) {
                Button("room.done") { requestEditorAction(.close) }.accessibilityIdentifier("experts.management.done")
            } }
            .onAppear { perform {
                agents = try service.directory().agents
                models = try ModelSelectionManager.availableModels(using: container.applicationRuntime).filter { !$0.isWebBackend }
            } }
            .interactiveDismissDisabled(editorIsDirty)
            .alert(NSLocalizedString("common.discard_changes", value: "Discard Changes?", comment: "Discard expert edit"),
                isPresented: $showDiscardConfirmation) {
                Button(NSLocalizedString("common.discard", value: "Discard", comment: "Discard expert edit"), role: .destructive) {
                    if let action = pendingEditorAction { performEditorAction(action) }
                    pendingEditorAction = nil
                }.accessibilityIdentifier("experts.discard")
                Button(L10n.cancel, role: .cancel) { pendingEditorAction = nil }.accessibilityIdentifier("experts.keepEditing")
            }
        }
    }

    private func requestEditorAction(_ action: PendingEditorAction) {
        editorFocused = false
        if editorIsDirty { pendingEditorAction = action; showDiscardConfirmation = true }
        else { performEditorAction(action) }
    }

    private func performEditorAction(_ action: PendingEditorAction) {
        perform {
            switch action {
            case .edit(let agent): try installEditor(agent)
            case .close: dismiss()
            case .archive:
                guard let editing else { return }
                agents = try service.updateAgent(editing, name: editing.name, description: editing.description,
                    availability: editing.availability == "active" ? "archived" : "active", directory: service.directory()).agents
                try installEditor(agents.first { $0.id == editing.id })
            }
        }
    }

    private func installEditor(_ agent: IOSPersistentAgent?) throws {
        let next = try agent.map { try AppleExpertConfigurationDraft(defaults: $0.defaults) }
            ?? AppleExpertConfigurationDraft()
        editing = agent
        configuration = next
        agentName = agent?.name ?? ""
        agentDescription = agent?.description ?? ""
    }

    private func perform(_ action: () throws -> Void) {
        do { try action(); error = nil } catch { self.error = error.localizedDescription }
    }

    private func saveAgent() throws {
        editorFocused = false
        let original = editing?.defaults ?? .object([:])
        let saved = try AppleExpertConfigurationDraft(defaults: original)
        if configuration.modelID != saved.modelID, !configuration.modelID.isEmpty,
           !models.contains(where: { IOSConversationGenerationOverride.modelKey(for: $0) == configuration.modelID }) {
            throw IOSApplicationRuntimeError.rejected(String(localized: "room.select_model"))
        }
        if configuration.templateID != saved.templateID, !configuration.templateID.isEmpty {
            guard let id = UUID(uuidString: configuration.templateID), let service = container.promptTemplates else {
                throw IOSApplicationRuntimeError.rejected(String(localized: "experts.templateUnavailable"))
            }
            _ = try service.requireReadableTemplate(id: id)
        }
        let nextDefaults = try configuration.applying(to: original)
        if let editing {
            agents = try service.updateAgent(editing, name: agentName.trimmingCharacters(in: .whitespacesAndNewlines),
                description: agentDescription.trimmingCharacters(in: .whitespacesAndNewlines),
                availability: editing.availability, directory: service.directory(), defaults: nextDefaults).agents
            try installEditor(agents.first { $0.id == editing.id })
            return
        }
        if configuration.modelID.isEmpty {
            agents = try service.createAgent(name: agentName.trimmingCharacters(in: .whitespacesAndNewlines),
                description: agentDescription.trimmingCharacters(in: .whitespacesAndNewlines),
                defaults: nextDefaults, directory: service.directory()).agents
            try installEditor(nil)
            return
        }
        guard let model = models.first(where: { IOSConversationGenerationOverride.modelKey(for: $0) == configuration.modelID }) else {
            throw IOSApplicationRuntimeError.rejected(String(localized: "room.select_model"))
        }
        let key = IOSConversationGenerationOverride.modelKey(for: model)
        let parameters = ModelParametersStore.shared.loadParameters(for: model.id, isLocal: model.isLocal)
        let json = try ConversationParameterSnapshot.updating(nil, modelKey: key, parameters: parameters)
        let fields = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(json.utf8))
        guard case .object(let object) = fields else { throw IOSApplicationRuntimeError.rejected("Invalid model parameters") }
        var defaults = object.filter { $0.key == "modelOverrides" }
        defaults["modelSelectionID"] = .string(key)
        if !configuration.templateID.isEmpty { defaults["selectedPromptTemplateID"] = .string(configuration.templateID) }
        agents = try service.createAgent(name: agentName.trimmingCharacters(in: .whitespacesAndNewlines),
            description: agentDescription.trimmingCharacters(in: .whitespacesAndNewlines),
            defaults: .object(defaults), directory: service.directory()).agents
        try installEditor(nil)
    }
}

private struct IOSRoomGeometryPreference: PreferenceKey {
    static var defaultValue: [String: CGFloat] { [:] }
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
    }
}

struct IOSAgentRoomAvatar: View {
    let participant: IOSRoomParticipant
    @ScaledMetric(relativeTo: .caption) private var diameter: CGFloat = 32
    var body: some View {
        Text(String(participant.displayName.prefix(1)))
            .font(.caption.weight(.medium))
            .foregroundStyle(.primary)
            .frame(width: diameter, height: diameter)
            .background(Color.primary.opacity(0.06), in: Circle())
            .accessibilityHidden(true)
    }
}

struct IOSAgentRoomTimeline: View {
    @EnvironmentObject private var container: ServiceContainer
    @ObservedObject var workspace: IOSAgentRoomWorkspace
    var ordinaryMessages: [ChatMessage] = []
    var ordinaryMessageTimes: [UUID: Int64] = [:]
    var ordinaryConversationID: UUID?
    var showsOwnerControls = true
    var showsExpertDetails = true
    var participantID: String?
    var association: IOSConversationRoomAssociation?
    var submissions: [IOSRoomSubmission] = []
    var pending: [IOSRoomSubmissionWorkspace.Pending] = []
    var onRetry: (IOSRoomSubmissionWorkspace.Pending) -> Void = { _ in }
    var onCancelPending: (String) -> Void = { _ in }
    @ObservedObject private var expertColors = IOSExpertColorPreferences.shared
    @ScaledMetric(relativeTo: .caption) private var expertIdentityWidth: CGFloat = 38
    @ScaledMetric(relativeTo: .caption) private var compactExpertIdentityWidth: CGFloat = 34
    @State private var timelineWidth: CGFloat = 390
    private var compactExpertLayout: Bool { timelineWidth < 359 }
    private var identityWidth: CGFloat { compactExpertLayout ? compactExpertIdentityWidth : expertIdentityWidth }
    private var identityGap: CGFloat { compactExpertLayout ? 8 : 10 }
    private var timelineInset: CGFloat { compactExpertLayout ? 16 : 20 }
    @Environment(\.colorScheme) private var colorScheme
    @State private var followsBottom = true
    @State private var distanceFromBottom: CGFloat = 0
    @State private var error: String?

    private var expertGroupIsBusy: Bool {
        workspace.hasActiveRuns || submissions.contains { ["accepted", "selected", "executing"].contains($0.state) }
    }

    private struct TimelineItem: Identifiable {
        let id: String
        let atMs: Int64
        let row: IOSAgentRoomWorkspace.Row?
        let receiptID: String?
        var ordinary: ChatMessage? = nil
    }
    private var timeline: [TimelineItem] {
        let representedUsers = Set(submissions.compactMap { receipt -> String? in
            guard let context = receipt.contextConversationID, let execution = receipt.executionID else { return nil }
            return context + "/" + execution
        })
        let rows = workspace.rows.filter { row in
            if let participantID, row.participant.id != participantID { return false }
            if !showsExpertDetails && row.message.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return false }
            if let association {
                return row.message.role != .user && submissions.contains {
                    IOSAgentRoomService.sender(for: $0, association: association)?.id == row.participant.id &&
                    $0.executionID.map { $0 + ":assistant" } == row.projected.messageID
                }
            }
            return row.message.role != .user || !representedUsers.contains(row.id)
        }.map { TimelineItem(id: $0.id, atMs: $0.projected.createdAtMs, row: $0, receiptID: nil) }
        let receipts = IOSRoomSubmissionWorkspace.timeline(receipts: submissions.filter { $0.input.parentMessageID == nil },
            pending: pending.filter { $0.input.parentMessageID == nil }).map {
            TimelineItem(id: "submission/" + $0.id, atMs: $0.input.atMs, row: nil, receiptID: $0.id)
        }
        let ordinary = ordinaryMessages.map { TimelineItem(id: "ordinary/" + $0.id.uuidString,
            atMs: ordinaryMessageTimes[$0.id] ?? 0, row: nil, receiptID: nil, ordinary: $0) }
        // The ordinary stream already has canonical order. Never sort it by a
        // secondary timestamp cache (live messages may not have a timestamp yet).
        // A reply is attached to its admitted parent, not its private Context clock.
        if association != nil {
            return IOSExpertTimelineOrder.attach(ordinary: ordinary, replies: rows,
                ordinaryID: { $0.ordinary.map { $0.metadata["nola.canonicalMessageID"] ?? $0.id.uuidString } },
                parentID: { child in
                    guard let row = child.row else { return nil }
                    return submissions.first {
                        $0.contextConversationID == row.participant.contextConversationID &&
                        $0.executionID.map { $0 + ":assistant" } == row.projected.messageID
                    }?.input.parentMessageID
                }) + receipts
        }

        // Workspace rows preserve each Context's canonical sequence.
        var result = rows
        for receipt in receipts {
            let index = result.firstIndex { $0.atMs > receipt.atMs } ?? result.endIndex
            result.insert(receipt, at: index)
        }
        return result
    }

    @MainActor private func resend(_ input: AppleMessageEditSubmission) async throws {
        let source = UUID(uuidString: input.sourceConversationID)
        let target = try await container.chatManager.sendEditedMessage(input)
        guard container.chatManager.currentConversationId == source else { return }
        NotificationCenter.default.post(name: .nolaOpenChatTab, object: target)
    }

    private func startsSenderGroup(_ item: TimelineItem) -> Bool {
        guard let index = timeline.firstIndex(where: { $0.id == item.id }), index > 0,
              let previous = timeline[index - 1].row, let row = item.row else { return true }
        return previous.participant.id != row.participant.id || previous.message.role != row.message.role
    }

    var body: some View {
        VStack(spacing: 0) {
            if showsOwnerControls { ScrollView(.horizontal) {
                HStack(spacing: 12) {
                    ForEach((workspace.room?.room.participants ?? []).filter { participantID == nil || $0.id == participantID }) { participant in
                        if let owner = UUID(uuidString: participant.contextConversationID),
                           let execution = container.chatManager.executionForConversation(owner) {
                            IOSAgentRoomMemberStatus(participant: participant, execution: execution)
                        }
                    }
                }.padding(.horizontal, 16)
            }
            .fixedSize(horizontal: false, vertical: true) }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
        ScrollViewReader { proxy in
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                if workspace.hasOlderMessages {
                    Button("room.older_messages") {
                        let anchor = workspace.rows.first?.id
                        followsBottom = false
                        do {
                            try workspace.loadOlder(container: container)
                            error = nil
                            if let anchor { proxy.scrollTo(anchor, anchor: .top) }
                        } catch { self.error = error.localizedDescription }
                    }.frame(minHeight: 44)
                }
                ForEach(timeline) { item in
                    if let ordinary = item.ordinary {
                        if let id = ordinaryConversationID, let execution = container.chatManager.executionForConversation(id) {
                            IOSOrdinaryExpertTimelineMessage(conversationID: id, message: ordinary,
                                execution: execution, isLatestAssistant: ordinaryMessages.last?.id == ordinary.id,
                                expertOwnerIsRunning: expertGroupIsBusy, onEditAndResend: resend)
                        } else {
                            ChatBubble(conversationID: ordinaryConversationID, message: ordinary,
                                isLatestAssistant: false, isStreaming: ordinary.isCanonicalLive, isRetrieving: false,
                                pendingHostPermissionRequest: nil, onHostPermissionDecision: { _, _ in },
                                ownerIsRunning: ordinary.isCanonicalLive || expertGroupIsBusy, onEditAndResend: resend)
                        }
                    } else if let row = item.row {
                    HStack(alignment: .top, spacing: identityGap) {
                        if row.message.role != .user {
                            VStack(spacing: 6) {
                                AppleExpertAvatar(name: row.participant.displayName, size: 30)
                                AppleExpertName(name: row.participant.displayName, alignment: .center)
                                    .accessibilityIdentifier("room.expert.name." + row.projected.messageID)
                            }
                            .frame(width: identityWidth)
                            .opacity(startsSenderGroup(item) ? 1 : 0)
                            .accessibilityHidden(!startsSenderGroup(item))
                        }
                        VStack(alignment: .leading, spacing: 4) {
                            if row.message.role == .user && startsSenderGroup(item) {
                                Text(String(format: String(localized: "room.sent_to"), row.participant.displayName))
                                    .font(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .trailing)
                            }
                        if !showsExpertDetails {
                            ChatSmoothStreamingMarkdownView(content: row.message.content,
                                isStreaming: row.projected.state == .running || row.projected.state == .pending)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(12)
                                .background(expertColors.color(for: row.participant.agentID).fill(colorScheme),
                                    in: RoundedRectangle(cornerRadius: 16))
                                .accessibilityIdentifier("chat.expert.answer." + row.projected.messageID)
                        } else if let owner = UUID(uuidString: row.participant.contextConversationID),
                           let execution = container.chatManager.executionForConversation(owner) {
                            IOSAgentRoomLiveMessage(row: row, execution: execution,
                                bubbleFill: expertColors.color(for: row.participant.agentID).fill(colorScheme),
                                isLatestAssistant: workspace.rows.last(where: {
                                    $0.participant.id == row.participant.id && $0.message.role == .assistant
                                })?.id == row.id)
                        } else {
                        ChatBubble(conversationID: UUID(uuidString: row.participant.contextConversationID),
                            message: row.message, isLatestAssistant: false,
                            isStreaming: row.projected.state == .running || row.projected.state == .pending,
                            isRetrieving: false, pendingHostPermissionRequest: nil,
                            onHostPermissionDecision: { _, _ in },
                            assistantBubbleFill: expertColors.color(for: row.participant.agentID).fill(colorScheme),
                            ownerIsRunning: row.projected.state == .running || row.projected.state == .pending,
                            surfaceInteractionsEnabled: false)
                        }

                        }
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier((row.message.role == .user ? "room.expert.user." : "room.expert.reply.") + row.projected.messageID)
                        .environment(\.iosExpertReplyAvailableWidth, max(24, min(720, timelineWidth) - timelineInset * 2 - identityWidth - identityGap))
                        .environment(\.iosCompactExpertReply, compactExpertLayout)

                    }

                    } else if let id = item.receiptID {
                        IOSRoomSubmissionRows(room: workspace.room,
                            submissions: submissions.filter { $0.id == id }, pending: pending.filter { $0.id == id },
                            onRetry: onRetry, onCancelPending: onCancelPending)
                    }
                }
                Color.clear.frame(height: 1).id("room-bottom")
            }.padding(.horizontal, timelineInset).padding(.vertical, 12).frame(maxWidth: 720)
                .frame(maxWidth: .infinity)
                .background(GeometryReader { geometry in
                    Color.clear.preference(key: IOSRoomGeometryPreference.self,
                        value: ["bottom": geometry.frame(in: .named("room-viewport")).maxY])
                })
        }
        .accessibilityIdentifier("chat.expert.timeline")
        .coordinateSpace(name: "room-viewport")
        .clipped()
        .scrollDismissesKeyboard(.interactively)
        .background(GeometryReader { geometry in
            Color.clear.preference(key: IOSRoomGeometryPreference.self,
                value: ["height": geometry.size.height, "width": geometry.size.width])
        })
        .onPreferenceChange(IOSRoomGeometryPreference.self) { geometry in
            if let width = geometry["width"], width > 0, abs(timelineWidth - width) >= 1 {
                DispatchQueue.main.async { timelineWidth = width }
            }
            guard let bottom = geometry["bottom"], let height = geometry["height"], height > 0 else { return }
            let distance = max(0, bottom - height)
            // Geometry preferences are delivered during layout. Publish only a
            // meaningful change on the next main-loop turn, outside that pass.
            guard abs(distanceFromBottom - distance) >= 1 else { return }
            DispatchQueue.main.async {
                guard abs(distanceFromBottom - distance) >= 1 else { return }
                distanceFromBottom = distance
                if distance <= 60 { followsBottom = true }
            }
        }
        .simultaneousGesture(DragGesture(minimumDistance: 4).onChanged { _ in followsBottom = false })
        .onAppear { proxy.scrollTo("room-bottom", anchor: .bottom) }
        .onChange(of: workspace.rows.map { "\($0.id):\($0.projected.revision):\($0.message.canonicalJournalCursor?.sequence ?? "")" }) { _, _ in
            if followsBottom { proxy.scrollTo("room-bottom", anchor: .bottom) }
        }
        .onChange(of: ordinaryMessages.map { $0.content + $0.thinking }) { _, _ in
            if followsBottom { proxy.scrollTo("room-bottom", anchor: .bottom) }
        }
        .onChange(of: submissions.map(\.id) + pending.map(\.id)) { _, _ in
            if followsBottom { proxy.scrollTo("room-bottom", anchor: .bottom) }
        }
        .overlay(alignment: .bottomTrailing) {
            if !followsBottom && distanceFromBottom > 180 {
                Button { followsBottom = true; proxy.scrollTo("room-bottom", anchor: .bottom) } label: {
                    Image(systemName: "chevron.down")
                        .font(.system(size: 14, weight: .semibold))
                        .frame(width: 44, height: 44)
                        .background(.regularMaterial, in: Circle())
                }.buttonStyle(.plain).padding(12)
                    .accessibilityLabel(Text("room.latest"))
                    .accessibilityIdentifier("chat.scroll.latest")
            }
        }
        }
        }
    }
}

private struct IOSAgentRoomMemberStatus: View {
    let participant: IOSRoomParticipant
    @ObservedObject var execution: ChatManager
    var showsIdentity = true
    @State private var expandedError = false

    private var permission: HostAgentPermissionRequest? {
        execution.pendingHostPermissionRequest ?? execution.localAgentPermissionRequest.map {
            HostAgentPermissionRequest(id: $0.presentationID, allowConversation: $0.allowConversation,
                toolName: $0.toolName, argsPreview: $0.scope ?? "", match: $0.scope ?? "*",
                reason: $0.presentationRationale, message: $0.presentationRationale, permissionInstructions: $0.permissionInstructions)
        }
    }

    private var failure: String? {
        if let failure = execution.submissionFailure { return failure }
        if case .error(let message) = execution.processingState { return message }
        return execution.remoteRunObservationError
    }

    private var needsAttention: Bool {
        execution.processingState.blocksInput || failure != nil
            || execution.remoteRunNeedsResume || execution.pendingHostPermissionRequest != nil
            || execution.localAgentPermissionRequest != nil
            || execution.unknownEffectRecovery != nil || execution.unknownEffectRecoveryError != nil
            || execution.unknownEffectRecoveryChecking || execution.unknownEffectPendingConversationID != nil
    }

    var body: some View {
        if needsAttention {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                if showsIdentity {
                    IOSAgentRoomAvatar(participant: participant)
                    Text(participant.displayName).font(.caption.weight(.semibold))
                }
                if failure == nil && showsIdentity {
                    Text(execution.getStateDescription()).font(.caption).foregroundStyle(.secondary)
                }
            }
            if let failure {
                DisclosureGroup(isExpanded: $expandedError) {
                    Text(failure).font(.caption).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: 260, alignment: .leading)
                } label: {
                    Label(L10n.chatErrorUnknown, systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.red)
                }
                .disclosureGroupStyle(NolaRowDisclosureStyle())
                .frame(minHeight: 44)
                .accessibilityIdentifier("room.submission.error.\(participant.id)")
            }
            if execution.processingState.blocksInput {
                Button("room.stop", role: .destructive) { execution.cancelCurrentGeneration() }
                    .frame(minHeight: 44)
            }
            if let permission {
                ScrollView {
                HostPermissionDecisionCard(request: permission,
                    onPermissionDecision: execution.permissionDecisionHandler(
                        localRequest: execution.localAgentPermissionRequest,
                        hostRequest: execution.pendingHostPermissionRequest))
                    .accessibilityElement(children: .contain)
                    .environment(\.submittingHostPermissionRequestID, execution.submittingHostPermissionRequestID)
                    .accessibilityIdentifier("room.permission." + participant.id)
                }.frame(width: 300).frame(maxHeight: 300)
            }
            if execution.remoteRunNeedsResume {
                Button("room.resume") { execution.resumeObservedRemoteRun() }.frame(minHeight: 44)
            } else if execution.remoteRunObservationError != nil {
                Button("room.resume") { execution.retryRemoteRunObservation() }.frame(minHeight: 44)
            }
            UnknownEffectRecoverySurface(manager: execution)
        }.padding(.vertical, 6)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("room.member.activity.\(participant.id)")
        }
    }
}

private struct IOSAgentRoomLiveMessage: View {
    let row: IOSAgentRoomWorkspace.Row
    @ObservedObject var execution: ChatManager
    let bubbleFill: Color
    let isLatestAssistant: Bool

    var body: some View {
        let local = execution.localAgentPermissionRequest
        let host = execution.pendingHostPermissionRequest
        ChatBubble(conversationID: UUID(uuidString: row.participant.contextConversationID),
            message: row.message, isLatestAssistant: isLatestAssistant,
            isStreaming: row.projected.state == .running || row.projected.state == .pending,
            isRetrieving: false, pendingHostPermissionRequest: nil,
            onHostPermissionDecision: execution.permissionDecisionHandler(localRequest: local, hostRequest: host),
            assistantBubbleFill: bubbleFill,
            ownerIsRunning: execution.processingState.blocksInput, surfaceInteractionsEnabled: false)
        if isLatestAssistant {
            IOSAgentRoomMemberStatus(participant: row.participant, execution: execution, showsIdentity: false)
        }
    }
}


/// Ordinary-chat selection only. No execution, permission or membership mutation occurs here.
struct IOSChatExpertPicker: View {
    let agents: [IOSPersistentAgent]
    var onManagementClosed: () -> Void = {}
    @Binding var selection: Set<String>
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var configuringAgent: IOSPersistentAgent?
    @State private var showingManagement = false

    private var visibleAgents: [IOSPersistentAgent] {
        agents.filter { $0.availability == "active" &&
            (query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) ||
             $0.description.localizedCaseInsensitiveContains(query)) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("chat.experts.title").font(.headline)
                Spacer()
                Button("Done") { dismiss() }
            }
            Text("chat.experts.description")
                .font(.callout).foregroundStyle(.secondary)
            TextField("chat.experts.search", text: $query)
            List(visibleAgents, id: \.id) { agent in
                HStack(spacing: 8) {
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
                    .accessibilityIdentifier("experts.select." + agent.id)
                    .accessibilityLabel(agent.name)
                    .accessibilityValue(selection.contains(agent.id) ? String(localized: "chat.experts.selected") : String(localized: "chat.experts.not_selected"))
                Button { configuringAgent = agent } label: {
                    Image(systemName: "slider.horizontal.3").frame(width: 44, height: 44)
                }.buttonStyle(.plain).accessibilityLabel(String(localized: "expert.color.title") + " · " + agent.name)
                    .accessibilityIdentifier("experts.color." + agent.id)
                }
            }
            if visibleAgents.isEmpty { Text("chat.experts.empty").foregroundStyle(.secondary) }
            Button { showingManagement = true } label: {
                Label("experts.manage", systemImage: "person.crop.circle.badge.gearshape")
                    .font(.callout).frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    .contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityIdentifier("experts.manage")
        }.padding(16)
            .sheet(isPresented: $showingManagement, onDismiss: onManagementClosed) { IOSAgentRoomsView() }
            .sheet(item: $configuringAgent) { agent in IOSExpertColorEditor(agentID: agent.id, name: agent.name) }
    }
}

/// Child transcripts are navigated from the current conversation's More menu.
/// Navigating here does not switch the parent conversation or submit new work.
struct IOSExpertConversations: View {
    @ObservedObject var workspace: IOSAgentRoomWorkspace
    @ObservedObject var submission: IOSRoomSubmissionWorkspace
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List(workspace.room?.room.participants ?? []) { participant in
                NavigationLink {
                    IOSAgentRoomTimeline(workspace: workspace, participantID: participant.id,
                        submissions: (submission.submissions[workspace.room?.room.id ?? ""] ?? []).filter {
                            $0.contextConversationID == participant.contextConversationID
                        }, pending: (submission.pending[workspace.room?.room.id ?? ""] ?? []).filter {
                            $0.input.participantID == participant.id
                        }, onRetry: submission.retry, onCancelPending: submission.cancelPending)
                        .navigationTitle(participant.displayName)
                        .navigationBarTitleDisplayMode(.inline)
                } label: {
                    HStack(spacing: 12) {
                        IOSAgentRoomAvatar(participant: participant)
                        Text(participant.displayName)
                    }.frame(minHeight: 44)
                }.accessibilityIdentifier("chat.expert.conversation." + participant.id)
            }
            .navigationTitle(Text("chat.experts.conversations"))
            .toolbar { ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            } }
        }
    }
}

struct IOSUnifiedExpertConversation: View {
    @EnvironmentObject private var container: ServiceContainer
    @ObservedObject var workspace: IOSAgentRoomWorkspace
    @ObservedObject var submission: IOSRoomSubmissionWorkspace
    let ordinaryMessages: [ChatMessage]
    let ordinaryMessageTimes: [UUID: Int64]
    let conversationID: UUID?
    var showsExpertDetails = false
    private var receiptReadIdentity: [String] {
        [conversationID?.uuidString ?? "", submission.conversationAssociation?.roomID ?? ""] +
            ordinaryMessages.filter { $0.role == .user }.map {
                ($0.metadata["nola.canonicalMessageID"] ?? "") + ":" + ($0.metadata["nola.canonicalMessageRevision"] ?? "")
            }
    }
    var body: some View {
        VStack(spacing: 0) {
            IOSAgentRoomTimeline(workspace: workspace, ordinaryMessages: ordinaryMessages,
                ordinaryMessageTimes: ordinaryMessageTimes,
                ordinaryConversationID: conversationID, showsOwnerControls: false, showsExpertDetails: showsExpertDetails, association: submission.conversationAssociation,
                submissions: workspace.room.flatMap { submission.submissions[$0.room.id] } ?? [],
                pending: workspace.room.flatMap { submission.pending[$0.room.id] } ?? [],
                onRetry: submission.retry, onCancelPending: submission.cancelPending)
            if let association = submission.conversationAssociation {
                ForEach(submission.pending[association.roomID] ?? []) { item in
                    if let failure = item.failure {
                        Text(failure).foregroundStyle(.red)
                        Button("Retry expert reply") { submission.retry(item) }
                    }
                }
            }

            ForEach(submission.recoverableExpertManifests.keys.sorted(), id: \.self) { messageID in
                if let manifest = submission.recoverableExpertManifests[messageID],
                   manifest.input.conversation_id == conversationID?.uuidString {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("room.pending_expert_message").font(.caption).foregroundStyle(.secondary)
                        Text(manifest.input.text).font(.callout).lineLimit(3)
                        if let failure = submission.errors[manifest.input.room_id] { Text(failure).font(.caption).foregroundStyle(.red) }
                        Button("room.resume_complete_experts") {
                            Task { await submission.resumeCompleteExpertMessage(messageID) }
                        }.frame(minHeight: 44)
                            .disabled(submission.isAdmitting || submission.resumingExpertMessages.contains(messageID))
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal)
                }
            }
            if let error = submission.expertResumeError { Text(error).font(.caption).foregroundStyle(.red).padding(.horizontal) }

            // The parent shows answer text only, so keep permission, stop and
            // recovery reachable there even after a child reply arrives.
            ForEach(workspace.room?.room.participants ?? []) { participant in
                if !showsExpertDetails || !workspace.rows.contains(where: { $0.participant.id == participant.id && $0.message.role == .assistant }),
                   let id = UUID(uuidString: participant.contextConversationID),
                   let execution = container.chatManager.executionForConversation(id) {
                    IOSAgentRoomMemberStatus(participant: participant, execution: execution)
                }
            }
        }
        .task(id: receiptReadIdentity) {
            guard let conversationID else { return }
            await submission.refreshVisibleExpertReceipts(conversationID: conversationID.uuidString,
                messageIDs: ordinaryMessages.filter { $0.role == .user }.compactMap { $0.metadata["nola.canonicalMessageID"] })
        }
    }
}


private struct IOSOrdinaryExpertTimelineMessage: View {
    let conversationID: UUID
    let message: ChatMessage
    @ObservedObject var execution: ChatManager
    let isLatestAssistant: Bool
    let expertOwnerIsRunning: Bool
    let onEditAndResend: (AppleMessageEditSubmission) async throws -> Void
    var body: some View {
        ChatBubble(conversationID: conversationID, message: message,
            isLatestAssistant: isLatestAssistant && message.role == .assistant,
            isStreaming: message.isCanonicalLive, isRetrieving: execution.retrievalInProgress,
            pendingHostPermissionRequest: nil,
            onHostPermissionDecision: execution.permissionDecisionHandler(
                localRequest: execution.localAgentPermissionRequest,
                hostRequest: execution.pendingHostPermissionRequest),
            ownerIsRunning: execution.processingState.blocksInput || expertOwnerIsRunning,
            onEditAndResend: onEditAndResend)
    }
}


/// Preserve canonical source order even with missing/skewed wall clocks.
enum IOSExpertTimelineOrder {
    static func attach<Item>(ordinary: [Item], replies: [Item],
                             ordinaryID: (Item) -> String?, parentID: (Item) -> String?) -> [Item] {
        var consumed = Set<Int>()
        var result: [Item] = []
        for item in ordinary {
            result.append(item)
            guard let id = ordinaryID(item) else { continue }
            for (index, reply) in replies.enumerated() where !consumed.contains(index) {
                guard let parent = parentID(reply), parent == id ||
                    (UUID(uuidString: parent) != nil && UUID(uuidString: parent) == UUID(uuidString: id)) else { continue }
                result.append(reply)
                consumed.insert(index)
            }
        }
        result.append(contentsOf: replies.enumerated().filter { !consumed.contains($0.offset) }.map(\.element))
        return result
    }
}

private struct IOSExpertTemplatePicker: View {
    @ObservedObject var service: PromptTemplateService
    @Binding var selection: String
    private var readableTemplates: [PromptTemplate] { service.templates.filter { service.canReadTemplate($0) } }
    var body: some View {
        Picker("experts.template", selection: $selection) {
            Text("experts.conversationTemplate").tag("")
            ForEach(readableTemplates) { template in
                Text(template.name).tag(template.id.uuidString)
                    .accessibilityIdentifier("experts.template.option." + template.id.uuidString)
            }
            if !selection.isEmpty, !readableTemplates.contains(where: { $0.id.uuidString == selection }) {
                Text("experts.savedTemplateUnavailable").tag(selection)
            }
        }.accessibilityIdentifier("experts.template")
    }
}
