// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Composes transcript, readiness, permission, user-input, and composer surfaces.

import Observation
import SwiftUI

struct MacConversationCanvas: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Bindable var model: MacWorkspaceModel
    var composerFocused: FocusState<Bool>.Binding
    var showsComposer: Bool
    var expertWorkspace: MacAgentRoomWorkspace?
    @State private var selectionRange: NSRange?
    @State private var composing = false
    @State private var caretRequest: AppleMentionEditor.Caret?
    private var mentions: [AppleDraftMention] { model.draftMentions(model.selectedComposerState.conversationID) }
    private var currentQuery: ExpertMentionQuery? {
        guard !composing else { return nil }
        let text = model.selectedComposerState.draft
        return .atCaret(in: text, selection: selectionRange ?? NSRange(location: text.utf16.count, length: 0), mentions: mentions)
    }
    private struct ExpertMentionIntent {
        let query: ExpertMentionQuery
        let agent: MacPersistentAgent
        let owner: String
        let draft: String
        let persisted: Bool
    }
    @State private var pendingExpertReintroduction: ExpertMentionIntent?
    @State private var selectingExpertID: String?
    @State private var expertMentionFailure: (conversationID: String, message: String)?
    @State private var highlightedExpertID: String?
    @State private var dismissedExpertMention: String?
    var modelSetupRequired: Bool
    var conversationControls: AnyView
    var leadingComposerControls: AnyView
    var trailingComposerControls: AnyView
    var modelControls: AnyView
    var onCommand: (ComposerCommand, String) -> Void
    @State private var viewportStore = MacTranscriptViewportStore()
    @State private var commandError: String?
    @State private var highlightedCommand: String?
    @State private var dismissedCommand: String?
    @State private var composerActions: MacComposerActionSink

    init(
        model: MacWorkspaceModel,
        composerFocused: FocusState<Bool>.Binding,
        showsComposer: Bool = true,
        sendToExperts: (() -> Void)? = nil,
        expertWorkspace: MacAgentRoomWorkspace? = nil,
        modelSetupRequired: Bool = false,
        conversationControls: AnyView = AnyView(EmptyView()),
        leadingComposerControls: AnyView = AnyView(EmptyView()),
        trailingComposerControls: AnyView = AnyView(EmptyView()),
        modelControls: AnyView = AnyView(EmptyView()),
        onCommand: @escaping (ComposerCommand, String) -> Void = { _, _ in }
    ) {
        self.modelSetupRequired = modelSetupRequired
        self.conversationControls = conversationControls
        self.leadingComposerControls = leadingComposerControls
        self.trailingComposerControls = trailingComposerControls
        self.modelControls = modelControls
        self.onCommand = onCommand
        self.model = model
        self.composerFocused = composerFocused
        self.showsComposer = showsComposer
        self.expertWorkspace = expertWorkspace
        _composerActions = State(initialValue: MacComposerActionSink { action in
            switch action {
            case .updateDraft(let conversationID, let draft):
                model.setDraft(draft, conversationID: conversationID)
                return true
            case .submit(let conversationID):
                guard model.selectedConversationID == conversationID,
                      model.selectedComposerState.canSend else { return false }
                if !model.draftMentions(conversationID).isEmpty {
                    guard let sendToExperts else { return false }
                    sendToExperts()
                    return true
                }
                model.send()
                return true
            case .stop(let conversationID):
                return model.stopComposerRun(conversationID: conversationID)
            case .importFiles(let conversationID, let urls):
                model.importFiles(urls, conversationID: conversationID)
                return true
            case .importDroppedAttachments(let conversationID, let providers):
                return model.importDroppedAttachments(providers, conversationID: conversationID)
            case .importPasteboardAttachments(let conversationID, let pasteboard):
                return model.importPasteboardAttachments(pasteboard, conversationID: conversationID)
            case .removeAttachment(let conversationID, let attachmentID):
                return model.removeAttachment(attachmentID: attachmentID, conversationID: conversationID)
            case .selectReasoningPreference(let conversationID, let preference):
                return model.selectComposerReasoningPreference(preference, conversationID: conversationID)
            case .selectPermissionMode(let conversationID, let mode):
                return model.selectComposerPermissionMode(mode, conversationID: conversationID)
            case .reloadPermissionConfiguration(let conversationID):
                return model.reloadComposerPermissionConfiguration(conversationID: conversationID)
            }
        })
    }

    var body: some View {
        GeometryReader { viewport in
            VStack(spacing: 0) {
                VStack(spacing: 0) {
                    if model.isTranscriptSearchVisible {
                        MacConversationSearchBar(model: model)
                            .transition(reduceMotion ? .identity : .move(edge: .top).combined(with: .opacity))
                    }
                    MacExecutionReadinessBanner(location: model.executionLocation)
                    if let expertWorkspace, expertWorkspace.conversationAssociation?.conversationID == model.selectedConversationID {
                        MacUnifiedExpertTranscript(model: model, workspace: expertWorkspace, width: viewport.size.width, viewportStore: viewportStore)
                            .id(model.selectedConversationID)
                    } else if model.isSelectedConversationLoading {
                        MacConversationLoadingView()
                    } else if model.selectedMessages.isEmpty {
                        if modelSetupRequired {
                            modelControls.frame(maxWidth: 520).padding(24)
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                        } else {
                            MacEmptyConversationView { prompt in
                                if let owner = model.selectedConversationID {
                                    let previous = model.draft
                                    let next = ComposerSuggestion.inserting(prompt, into: previous)
                                    let snapshot = AppleMentionEditor.Snapshot(text: previous, mentions: model.draftMentions(owner),
                                        caret: selectionRange.map(NSMaxRange) ?? previous.utf16.count)
                                    if next != previous {
                                        model.setDraft(next, conversationID: owner, mentions: snapshot.mentions)
                                    }
                                    caretRequest = .init(location: next.utf16.count, before: next == previous ? nil : snapshot)
                                    selectionRange = NSRange(location: next.utf16.count, length: 0)
                                }
                                composerFocused.wrappedValue = true
                            }
                        }
                    } else {
                        let conversationID = model.selectedConversationID ?? ""
                        MacTranscriptView(
                            conversationID: conversationID,
                            messages: model.selectedMessages,
                            isRunning: model.isSelectedConversationVisiblyRunning && !model.isViewingStoredHistory,
                            liveActivities: model.visibleActivity,
                            searchText: model.transcriptSearchText,
                            focusedSearchMessageID: model.transcriptSearchFocusedMessageID,
                            searchNavigationToken: model.transcriptSearchNavigationToken,
                            bottomClearance: 24,
                            viewportStore: viewportStore,
                            terminalRevision: model.selectedTerminalTranscriptRevision,
                            onSendTaskGuidance: { taskID, parentRunID, message in
                                model.sendTaskGuidance(conversationID: conversationID, parentRunID: parentRunID, taskID: taskID, message: message)
                            },
                            onRequestTaskCancellation: { taskID, parentRunID in
                                model.requestTaskCancellation(conversationID: conversationID, parentRunID: parentRunID, taskID: taskID)
                            },
                            resolveTaskRunID: { message in
                                model.taskParentRunID(for: conversationID, message: message)
                            },
                            hasEarlierStoredMessages: model.hasEarlierStoredMessages,
                            isViewingHistory: model.isViewingStoredHistory,
                            isLoadingHistoryPage: model.isLoadingHistoryPage,
                            onLoadEarlierHistory: { Task { await model.loadEarlierStoredMessages() } },
                            onReturnToLatestHistory: { Task { await model.returnToLatestStoredMessages() } },
                            onLocateCompactionSource: model.locateCompactionSource,
                            previewUI: model.previewUISurface, readUIChanges: model.readUISurfaceChanges,
                            readUIRevision: model.readUISurfaceRevision,
                            pendingUI: model.pendingUIInteraction,
                            submitUI: model.submitUIInteraction,
                            readLocalSource: { try model.isLocalConversationSource(conversationID) }
                        )
                        .id(model.selectedConversationID)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .clipped()
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("chat.transcript-region")

                VStack(spacing: MacDesignTokens.Spacing.md) {
                    if hasInterruption {
                    MacConversationInterruptionSurface(maxHeight: min(320, viewport.size.height * 0.42)) {
                    if let expertWorkspace {
                        MacExpertPermissionDock(workspace: expertWorkspace, conversationID: model.selectedConversationID)
                    }
                    if let failure = model.localSubmissionFailure,
                       failure.conversationID == model.selectedConversationID {
                        VStack(alignment: .leading, spacing: 8) {
                            Label(String(localized: "Message was not submitted", table: "Chat"),
                                  systemImage: "exclamationmark.triangle")
                                .font(.callout.weight(.semibold))
                            DisclosureGroup(String(localized: "Diagnostic details", table: "Chat")) {
                                Text(failure.message)
                                    .font(.callout.monospaced())
                                    .foregroundStyle(.secondary)
                                    .textSelection(.enabled)
                            }
                            .disclosureGroupStyle(MacRowDisclosureStyle(isPlain: true))
                            Text(String(localized: "Your draft has been kept. Review it and send again when ready.", table: "Chat"))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Button(String(localized: "Dismiss", table: "Chat")) {
                                model.dismissLocalSubmissionFailure()
                            }
                            .accessibilityIdentifier("chat.local.dismissSubmissionFailure")
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(MacDesignColor.subtleFill, in: RoundedRectangle(cornerRadius: 12))
                        .accessibilityIdentifier("chat.local.submissionFailure")
                    }
                    if let recovery = model.localRunRecovery,
                       recovery.conversationID == model.selectedConversationID {
                        VStack(alignment: .leading, spacing: 6) {
                            Label(model.isAwaitingUserInput ? String(localized: "Waiting for your answer") : recovery.title,
                                  systemImage: model.isAwaitingUserInput || recovery.restoredFromStorage ? "pause.circle" : "exclamationmark.triangle")
                                .font(.caption)
                            if let error = model.localRecoveryError {
                                DisclosureGroup(String(localized: "Diagnostic details", table: "Chat")) {
                                    Text(error).font(.caption)
                                        .foregroundStyle(.secondary).textSelection(.enabled)
                                    if let inspection = model.localRecoveryInspection, !model.isAwaitingUserInput {
                                        Text(MacRunRecoveryPresentation.detail(inspection.observation))
                                            .font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                                .font(.caption)
                                .disclosureGroupStyle(MacRowDisclosureStyle(isPlain: true))
                            }
                            if model.localRecoveryError == nil,
                               let inspection = model.localRecoveryInspection, !model.isAwaitingUserInput {
                                Text(MacRunRecoveryPresentation.detail(inspection.observation))
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            if let presentation = model.currentUnknownEffectRecovery {
                                MacUnknownEffectRecoveryView(presentation: presentation,
                                    isBusy: model.isResolvingUnknownEffects,
                                    submit: { decision in
                                        Task { await model.resolveUnknownEffects(decision, presentation: presentation) }
                                    }, recoverPending: {
                                        Task { await model.recoverUnknownEffectDecision(presentation: presentation) }
                                    })
                                    .id(presentation.id)
                            }
                            Button {
                                Task { await model.checkLocalRunRecovery() }
                            } label: {
                                HStack(spacing: 6) {
                                    if model.isCheckingLocalRunRecovery {
                                        ProgressView().controlSize(.small)
                                    }
                                    Text(String(localized: "Check execution status", table: "Chat"))
                                }
                            }
                            .disabled(!model.canCheckLocalRunRecovery)
                            .accessibilityIdentifier("chat.local.checkStatus")
                            if model.canCancelLocalRecovery {
                                Button(String(localized: "Cancel unfinished task", table: "Chat")) {
                                    model.stop()
                                }
                                .accessibilityIdentifier("chat.local.cancelUnfinished")
                                Text(String(localized: "Cancellation keeps existing results and does not rerun commands.", table: "Chat"))
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            if model.canRetryLocalRunCancellation || model.isDeliveringLocalCancellation {
                                Button {
                                    Task { await model.retryLocalRunCancellation() }
                                } label: {
                                    HStack(spacing: 6) {
                                        if model.isDeliveringLocalCancellation { ProgressView().controlSize(.small) }
                                        Text(String(localized: "Retry cancellation delivery", table: "Chat"))
                                    }
                                }
                                .disabled(!model.canRetryLocalRunCancellation)
                                .accessibilityIdentifier("chat.local.retryCancellation")
                            }
                            if model.canResumeLocalRun || model.isResumingLocalRun {
                                Button {
                                    Task { await model.resumeLocalRun() }
                                } label: {
                                    HStack(spacing: 6) {
                                        if model.isResumingLocalRun { ProgressView().controlSize(.small) }
                                        Text(String(localized: "Resume existing run", table: "Chat"))
                                    }
                                }
                                .disabled(!model.canResumeLocalRun)
                                .accessibilityIdentifier("chat.local.resume")
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if model.canCancelRejectedUIQueue {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(model.remoteRecoveryError ?? String(localized: "Form action was not executed", table: "Chat"))
                                .font(.caption).foregroundStyle(.secondary)
                            Button(String(localized: "Remove pending form action", table: "Chat")) {
                                Task { await model.cancelRejectedUIQueue() }
                            }
                            .accessibilityIdentifier("chat.ui.cancelRejectedQueue")
                        }
                    }
                    if let pending = model.remotePendingSubmission, pending.identity.conversation_id == model.selectedConversationID {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(String(localized: "Waiting for the host to confirm this submission.", table: "Chat")).font(.caption)
                            Button(String(localized: "Cancel")) {
                                Task { await model.cancelPendingRemoteSubmission() }
                            }.accessibilityIdentifier("chat.remote.cancelSubmission")
                            Button(String(localized: "Retry")) { model.recheckRemoteRun() }
                        }
                    }
                    if let recovery = model.remoteRunRecovery,
                       recovery.conversationID == model.selectedConversationID {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(recovery.expectedRunID == nil
                                ? String(localized: "Remote run state needs confirmation. No canonical Run ID is available.", table: "Chat")
                                : model.remoteRecoveryIsCancellationPending
                                    ? String(localized: "Cancellation requested. Waiting for the remote run to confirm its final state.", table: "Chat")
                                : String(localized: "Connection interrupted. Resume the existing remote run without sending a new message.", table: "Chat"))
                                .font(.caption)
                            if let error = model.remoteRecoveryError { Text(error).font(.caption).foregroundStyle(.secondary) }
                            Button(model.remoteRecoveryIsCancellationPending
                                ? String(localized: "Check cancellation status", table: "Chat")
                                : String(localized: "Resume remote run", table: "Chat")) {
                                Task { await model.resumeRemoteRun() }
                            }
                            .disabled(!model.canRecoverRemoteRun)
                            .accessibilityIdentifier("chat.remote.resume")
                            Button(String(localized: "Check run status", table: "Chat")) {
                                model.recheckRemoteRun()
                            }
                            .disabled(!model.canRecheckRemoteRun)
                            .accessibilityIdentifier("chat.remote.recheck")
                        }
                    }
                    if model.remoteRunRecovery == nil, model.remoteObservationNeedsAttention {
                        VStack(alignment: .leading, spacing: 6) {
                            if let error = model.remoteRecoveryError { Text(error).font(.caption).foregroundStyle(.secondary) }
                            Button(String(localized: "Retry")) { model.recheckRemoteRun() }
                                .accessibilityIdentifier("chat.remote.recheck")
                        }
                    }
                    if let permissionError = model.selectedPermissionResolutionError {
                        MacInlineNotice(
                            message: permissionError,
                            kind: .failure,
                            dismiss: { model.dismissPermissionResolutionError() }
                        )
                    }
                    if let request = model.selectedPendingRemotePermission {
                        MacHostPermissionCard(
                            request: request,
                            allowOnce: {
                                model.resolvePermission(allow: true, scope: .once, requestID: request.id, expectedRequest: request)
                            },
                            allowConversation: {
                                model.resolvePermission(allow: true, scope: .conversation, requestID: request.id, expectedRequest: request)
                            },
                            deny: { model.resolvePermission(allow: false, requestID: request.id, expectedRequest: request) },
                            isResolving: model.isResolvingPermission,
                            provideInstruction: { text in
                                try await model.providePermissionInstruction(text, requestID: request.id, expectedRequest: request)
                            }
                        )
                        .id(request.presentationIdentity)
                        .transition(reduceMotion ? .identity : .move(edge: .bottom).combined(with: .opacity))
                    }
                    if let request = model.selectedPendingUserInputRequest {
                        MacUserInputCard(
                            request: request,
                            submit: { model.resolveUserInput(request: request, answers: $0) },
                            cancel: { model.resolveUserInput(request: request, answers: [], decline: true) },
                            later: { model.deferUserInputWait(request) }
                        )
                        .id(request.id)
                        .disabled(model.resolvingUserInputWaitIDs.contains(request.id))
                        .transition(reduceMotion ? .identity : .move(edge: .bottom).combined(with: .opacity))
                    }
                    if model.hasDeferredUserInputWaits {
                        Button(String(localized: "Show pending questions")) { model.reopenUserInputWaits() }
                            .buttonStyle(.bordered)
                    }
                    if let error = model.selectedUserInputWaitError {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(String(localized: "Unable to refresh pending questions. Please retry."))
                                .font(.callout)
                            DisclosureGroup(String(localized: "Diagnostic details", table: "Chat")) {
                                Text(error).font(.caption).textSelection(.enabled)
                            }
                            .disclosureGroupStyle(MacRowDisclosureStyle(isPlain: true))
                            Button(String(localized: "Show pending questions")) { model.reopenUserInputWaits() }
                        }
                    }
                    }
                    }
                    if model.isSelectedConversationDeleting {
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            Text(String(localized: "Deleting conversation…", table: "Chat"))
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                        .accessibilityElement(children: .combine)
                    }
                    if model.selectedDraftPersistenceFailed {
                        HStack {
                            Label(String(localized: "Local draft storage is unavailable. Keep this window open.", table: "Chat"), systemImage: "exclamationmark.triangle")
                                .font(.callout)
                            Button(String(localized: "Retry")) { model.retrySelectedDraftStorage() }
                        }
                        .accessibilityIdentifier("chat.draft-storage-error")
                    }
                    if showsComposer {
                    if let commandError { Text(commandError).foregroundStyle(MacDesignColor.danger).font(.callout) }
                    if !modelSetupRequired || !model.selectedMessages.isEmpty { modelControls }
                    conversationControls
                    MacComposerView(
                        state: model.selectedComposerState,
                        actions: composerActions,
                        composerFocused: composerFocused,
                        isModelConfigured: !modelSetupRequired,
                        leadingControls: leadingComposerControls,
                        trailingControls: trailingComposerControls,
                        mentionCommand: handleComposerKey,
                        mentionEditor: AppleMentionEditor(mentions: mentions, caretRequest: caretRequest, selectionChanged: { range, marked in
                            selectionRange = range; composing = marked
                        }, applyEdit: { old, range, replacement in
                            let owner = model.selectedComposerState.conversationID
                            guard model.draft == old else { return false }
                            model.setDraft((old as NSString).replacingCharacters(in: range, with: replacement), conversationID: owner,
                                mentions: AppleDraftMention.replacing(model.draftMentions(owner), range: range, with: replacement))
                            return true
                        }, restore: { value, spans in model.setDraft(value, conversationID: model.selectedComposerState.conversationID, mentions: spans) },
                        pasted: { dismissedExpertMention = $0 }), localCommand: executeCommand
                    )
                    .overlay(alignment: .topLeading) {
                        if commandsVisible {
                            ExpertMentionPanel(candidates: ComposerCommand.candidates(model.draft).map {
                                ExpertMentionCandidate(id: $0.rawValue, name: "/" + $0.rawValue, detail: $0.title)
                            }, highlightedID: highlightedCommand ?? ComposerCommand.candidates(model.draft).first?.rawValue,
                            error: nil, select: completeCommand, accessibilityTitle: "composer.command.help", identifierPrefix: "chat.command.", showsAvatar: false)
                            .shadow(color: Color.black.opacity(0.14), radius: 12, y: 4)
                            .alignmentGuide(.top) { $0[.bottom] + 8 }
                        } else if expertMentionsVisible, let expertWorkspace {
                            ExpertMentionPanel(candidates: expertMentionCandidates,
                                highlightedID: highlightedExpertID ?? expertMentionCandidates.first?.id,
                                error: expertMentionFailure?.conversationID == model.selectedComposerState.conversationID
                                    ? expertMentionFailure?.message : expertWorkspace.directoryError, select: selectExpertMention)
                                .disabled(selectingExpertID != nil)
                                .shadow(color: Color.black.opacity(0.14), radius: 12, y: 4)
                                .alignmentGuide(.top) { $0[.bottom] + 8 }
                                .task(id: expertWorkspace.mainSourceReady) { await expertWorkspace.refreshExpertDirectory() }
                        }
                    }
                    .onChange(of: model.selectedComposerState.draft) { _, _ in highlightedCommand = nil; dismissedCommand = nil; commandError = nil; highlightedExpertID = nil; if dismissedExpertMention != model.draft { dismissedExpertMention = nil } }
                    .sheet(isPresented: $model.isModelSettingsPresented) {
                        MacConversationModelSettingsView(model: model)
                            .id(model.conversationModelKey)
                    }
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: MacDesignTokens.Layout.chatTranscriptMaxWidth)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, MacDesignTokens.Layout.chatTranscriptHorizontalPadding)
                .padding(.top, MacDesignTokens.Spacing.md)
                .padding(.bottom, MacDesignTokens.Spacing.lg)
                .background(MacDesignColor.canvas)
                .zIndex(1)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("chat.composer-region")
            }
            .clipped()
            // Do not animate this ancestor: permission changes can coincide
            // with streamed text reflow and must not animate the transcript.
        }
        .onChange(of: model.selectedComposerState.conversationID) { _, _ in
            pendingExpertReintroduction = nil
            expertMentionFailure = nil
            commandError = nil
            selectionRange = nil
            composing = false
            caretRequest = nil
            highlightedCommand = nil
            dismissedCommand = nil
            highlightedExpertID = nil
            dismissedExpertMention = nil
        }
        .confirmationDialog(String(localized: "Use the expert’s latest configuration?"), isPresented: expertReintroductionPresented) {
            Button(String(localized: "Reintroduce expert")) {
                guard let intent = pendingExpertReintroduction else { return }
                pendingExpertReintroduction = nil
                performExpertMention(intent, allowReintroduction: true)
            }
            Button(String(localized: "Cancel"), role: .cancel) { pendingExpertReintroduction = nil }
        } message: {
            Text(String(localized: "This adds a new expert context with the latest model and template. Previous replies stay in this conversation."))
        }
    }

    private var expertReintroductionPresented: Binding<Bool> {
        Binding(get: { pendingExpertReintroduction != nil }, set: { if !$0 { pendingExpertReintroduction = nil } })
    }
    private var expertMentionsVisible: Bool {
        composerFocused.wrappedValue && dismissedExpertMention != model.selectedComposerState.draft &&
            currentQuery != nil
    }
    private var expertMentionCandidates: [ExpertMentionCandidate] {
        guard let query = currentQuery?.query else { return [] }
        let members = expertWorkspace?.conversationAssociation.flatMap { $0.conversationID == model.selectedComposerState.conversationID ? $0.room.room.participants : nil } ?? []
        let candidates = (expertWorkspace?.directory ?? []).filter { agent in
            agent.availability == "active" && (query.isEmpty || agent.name.localizedCaseInsensitiveContains(query) || agent.description.localizedCaseInsensitiveContains(query))
        }.map { ExpertMentionCandidate(id: $0.id, name: $0.name, detail: $0.description) }
        return ExpertMentionCandidate.ordered(candidates, members: Set(members.filter(\.active).map(\.agentID)))
    }
    private func selectExpertMention(_ id: String) {
        let draft = model.selectedComposerState.draft
        guard selectingExpertID == nil, let workspace = expertWorkspace,
              expertMentionCandidates.contains(where: { $0.id == id }),
              let agent = workspace.directory.first(where: { $0.id == id }),
              let query = currentQuery else { return }
        performExpertMention(.init(query: query, agent: agent, owner: model.selectedComposerState.conversationID,
            draft: draft, persisted: model.selectedConversation?.isPersisted == true), allowReintroduction: false)
    }

    private func performExpertMention(_ intent: ExpertMentionIntent, allowReintroduction: Bool) {
        guard selectingExpertID == nil, let workspace = expertWorkspace,
              model.selectedComposerState.conversationID == intent.owner,
              model.selectedComposerState.draft == intent.draft else { return }
        selectingExpertID = intent.agent.id
        expertMentionFailure = nil
        Task { @MainActor in
            defer { selectingExpertID = nil }
            do {
                try await workspace.prepareExpertMention(agentID: intent.agent.id, expectedRevision: intent.agent.revision,
                    conversationID: intent.owner, isPersisted: intent.persisted, allowReintroduction: allowReintroduction, useMemberSnapshot: false)
                guard model.selectedComposerState.conversationID == intent.owner else { return }
                guard model.draft == intent.draft, currentQuery == intent.query else { return }
                let member = workspace.conversationAssociation.flatMap { $0.conversationID == intent.owner ? $0.room.room.participants.first { $0.active && $0.agentID == intent.agent.id } : nil }
                let inserted = AppleDraftMention.insertion(text: intent.draft, range: intent.query.range, name: member?.displayName ?? intent.agent.name,
                    expert: .init(agentID: intent.agent.id, agentRevision: member?.agentRevision ?? intent.agent.revision,
                        participantID: member?.id, contextConversationID: member?.contextConversationID),
                    mentions: mentions, sourceID: model.mainExpertDraftSource(intent.owner), epoch: workspace.mainSourceEpoch?(intent.owner))
                let previous = AppleMentionEditor.Snapshot(text: intent.draft, mentions: mentions, caret: NSMaxRange(intent.query.range))
                model.setDraft(inserted.text, conversationID: intent.owner, mentions: inserted.mentions)
                caretRequest = .init(location: inserted.caret, before: previous)
                selectionRange = NSRange(location: inserted.caret, length: 0)
                highlightedExpertID = nil
                composerFocused.wrappedValue = true
            } catch MacExpertMentionPreparationError.reintroductionRequired {
                guard model.selectedComposerState.conversationID == intent.owner else { return }
                pendingExpertReintroduction = intent
            } catch {
                expertMentionFailure = (intent.owner, error.localizedDescription)
                await workspace.refreshExpertDirectory()
            }
        }
    }
    private var commandsVisible: Bool {
        composerFocused.wrappedValue && dismissedCommand != model.draft && ComposerCommand.query(model.draft) != nil
    }
    private func completeCommand(_ id: String) {
        model.setDraft("/" + id + " ", conversationID: model.selectedComposerState.conversationID)
        composerFocused.wrappedValue = true
    }
    private func handleComposerKey(_ key: String) -> Bool {
        guard commandsVisible else { return handleExpertMentionKey(key) }
        if key == "dismiss" { dismissedCommand = model.draft; return true }
        let candidates = ComposerCommand.candidates(model.draft)
        guard !candidates.isEmpty else { return true }
        let index = candidates.firstIndex { $0.rawValue == highlightedCommand } ?? 0
        if key == "accept" { completeCommand(candidates[index].rawValue) }
        else { highlightedCommand = candidates[(index + (key == "up" ? -1 : 1) + candidates.count) % candidates.count].rawValue }
        return true
    }
    private func executeCommand() -> Bool {
        guard let parsed = ComposerCommand.parse(model.draft) else { return false }
        guard let command = parsed.command else {
            commandError = ComposerCommand.invalidMessage; return true
        }
        if command == .model {
            guard MacExecutionBackendPolicy.requiresNolaModel(model.executionBackend.id),
                  model.selectedModelOption != nil else {
                commandError = String(localized: "Model settings are unavailable for the current backend.")
                return true
            }
        }
        if command == .stop {
            guard model.stopComposerRun(conversationID: model.selectedComposerState.conversationID) else {
                commandError = NSLocalizedString("composer.command.noRun", value: "No task is running.", comment: "Command feedback"); return true
            }
        }
        model.setDraft("", conversationID: model.selectedComposerState.conversationID)
        switch command {
        case .model: model.isModelSettingsPresented = true
        case .search: model.transcriptSearchText = parsed.argument; model.isTranscriptSearchVisible = true
        case .help: completeCommand(""); model.setDraft("/", conversationID: model.selectedComposerState.conversationID)
        case .stop: break
        default: onCommand(command, parsed.argument)
        }
        return true
    }

    private func handleExpertMentionKey(_ command: String) -> Bool {
        guard expertMentionsVisible else { return false }
        if command == "dismiss" { dismissedExpertMention = model.selectedComposerState.draft; return true }
        let candidates = expertMentionCandidates
        guard !candidates.isEmpty else { return command == "accept" }
        let index = candidates.firstIndex { $0.id == highlightedExpertID } ?? 0
        if command == "accept" { selectExpertMention(candidates[index].id) }
        else { highlightedExpertID = candidates[(index + (command == "up" ? -1 : 1) + candidates.count) % candidates.count].id }
        return true
    }

    private var hasInterruption: Bool {
        (model.localRunRecovery?.conversationID == model.selectedConversationID && model.localRunRecovery != nil)
            || (model.localSubmissionFailure?.conversationID == model.selectedConversationID && model.localSubmissionFailure != nil)
            || (model.remotePendingSubmission?.identity.conversation_id == model.selectedConversationID && model.remotePendingSubmission != nil)
            || (model.remoteRunRecovery?.conversationID == model.selectedConversationID && model.remoteRunRecovery != nil)
            || (model.remoteRunRecovery == nil && model.remoteObservationNeedsAttention)
            || model.selectedPendingRemotePermission != nil
            || model.selectedPermissionResolutionError != nil
            || model.selectedPendingUserInputRequest != nil
            || model.hasDeferredUserInputWaits
            || model.selectedUserInputWaitError != nil
            || model.canCancelRejectedUIQueue
            || MacExpertPermissionDock.hasControls(workspace: expertWorkspace, conversationID: model.selectedConversationID)
    }
}
