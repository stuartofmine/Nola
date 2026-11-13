// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements the chat-first macOS workspace and its native desktop interaction model.

import AppKit
import Combine
import Observation
import SwiftUI

private func macHostText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

/// A navigation intent includes its model mutations, not just its destination.
/// In particular, a cancelled editor exit must not create/select a conversation.
@MainActor
struct MacWorkspaceNavigationActions {
    let model: MacWorkspaceModel
    let extensions: MacExtensionsNavigationState
    let currentSection: () -> MacWorkspaceSection
    let setSection: (MacWorkspaceSection) -> Void

    func navigate(to section: MacWorkspaceSection,
                  ifStillValid: @escaping () -> Bool = { true },
                  then action: @escaping () -> Void = {}) {
        let commit = {
            guard ifStillValid() else { return }
            setSection(section)
            action()
        }
        if currentSection() == .extensions && section != .extensions {
            extensions.requestLeave(commit)
        } else {
            commit()
        }
    }

    func newConversation() {
        navigate(to: .chat, then: { model.newConversation() })
    }

    func selectConversation(_ id: String) {
        navigate(to: .chat, ifStillValid: { model.conversations.contains(where: { $0.id == id }) }) {
            model.selectedConversationID = id
        }
    }
}

@MainActor
struct MacDeferredTextReader: View {
    private struct Request: Hashable {
        var partIndex = 0
        var offset: UInt64 = 0
        var retry = 0
    }

    let content: MacDeferredMessageContent
    var store: MacApplicationPort = .shared
    var onPageLoaded: ((MacCanonicalTextRange) -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var request = Request()
    @State private var previous: [Request] = []
    @State private var readModel = MacOriginalTextReadModel()
    private var page: MacCanonicalTextRange? { readModel.page }
    private var failed: Bool { readModel.failed }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("history.reader.title", tableName: "Chat")
                    .font(.headline)
                Spacer()
                Button {
                    readModel.invalidate()
                    dismiss()
                } label: {
                    Text("history.reader.close", tableName: "Chat")
                }
                .keyboardShortcut(.cancelAction)
            }
            .padding(20)
            Divider()
            Group {
                if let page {
                    // Range loading bounds storage work; native text storage
                    // also bounds layout work to the viewport. A single SwiftUI
                    // Text still measures the entire multilingual page on every
                    // size proposal and can stall navigation for minutes.
                    MacSelectableResponseText(text: page.text)
                    .id(request)
                    .accessibilityIdentifier("chat.deferredText.page")
                } else if failed {
                    VStack(spacing: 12) {
                        Text("history.reader.failure", tableName: "Chat")
                        Button {
                            readModel.invalidate()
                            request.retry += 1
                        } label: {
                            Text("history.reader.retry", tableName: "Chat")
                        }
                    }
                } else {
                    ProgressView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            Divider()
            HStack(spacing: 12) {
                Button {
                    readModel.invalidate()
                    previous.removeAll()
                    request = Request()
                } label: {
                    Text("history.reader.start", tableName: "Chat")
                }
                .disabled(request.partIndex == 0 && request.offset == 0)
                .accessibilityIdentifier("chat.deferredText.start")
                .keyboardShortcut(.upArrow, modifiers: [.command, .option])
                Button {
                    if let prior = previous.popLast() {
                        readModel.invalidate()
                        request = prior
                    }
                } label: {
                    Text("history.reader.previous", tableName: "Chat")
                }
                .disabled(previous.isEmpty)
                .accessibilityIdentifier("chat.deferredText.previous")
                .keyboardShortcut(.leftArrow, modifiers: [.command, .option])
                Spacer()
                if let page {
                    Text(verbatim: "\(page.nextOffsetBytes.formatted()) / \(page.totalUTF8Bytes.formatted()) B")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                Button { advance() } label: {
                    Text("history.reader.next", tableName: "Chat")
                }
                .disabled(page == nil || (page?.endOfContent == true && request.partIndex + 1 >= content.parts.count))
                .accessibilityIdentifier("chat.deferredText.next")
                .keyboardShortcut(.rightArrow, modifiers: [.command, .option])
            }
            .padding(20)
        }
        .frame(minWidth: 560, idealWidth: 760, minHeight: 420, idealHeight: 600)
        .background(MacDesignColor.canvas)
        .task(id: request) { await load() }
        .onDisappear { readModel.invalidate() }
    }

    private func advance() {
        guard let page else { return }
        readModel.invalidate()
        previous.append(request)
        // Navigation metadata is bounded too; older pages remain reachable from Start.
        if previous.count > 64 { previous.removeFirst() }
        if page.endOfContent {
            request = Request(partIndex: request.partIndex + 1)
        } else {
            request = Request(partIndex: request.partIndex, offset: page.nextOffsetBytes)
        }
    }

    private func load() async {
        guard content.parts.indices.contains(request.partIndex) else {
            readModel.rejectInvalidSelection()
            return
        }
        let requested = request
        let reference = content.parts[requested.partIndex]
        let conversationID = content.conversationID
        let reader = store
        let input = MacOriginalTextReadModel.Request(conversationID: conversationID,
            reference: reference, offsetBytes: requested.offset)
        let result = await readModel.load(input) { input, cancellation in
            let task = Task.detached(priority: .userInitiated) {
                try Task.checkCancellation()
                return try reader.readConversationTextRange(input.conversationID,
                    messageID: input.reference.messageID, partID: input.reference.partID,
                    offsetBytes: input.offsetBytes, cancellation: cancellation)
            }
            return try await withTaskCancellationHandler {
                try await task.value
            } onCancel: {
                cancellation.cancel()
                task.cancel()
            }
        }
        guard !Task.isCancelled, request == requested, readModel.selection == input,
              let result else { return }
        onPageLoaded?(result)
    }
}

enum MacWorkspaceLayoutMode: Equatable {
    case threeColumn
    case inspectorOverlay
    case compactOverlay

    static func resolve(width: CGFloat, sidebarWidth: CGFloat = MacDesignTokens.Layout.sidebarWidth) -> MacWorkspaceLayoutMode {
        let requiredWidth = sidebarWidth + MacWorkspaceColumnMetrics.minimumConversationWidth
            + MacWorkspaceColumnMetrics.minimumInspectorWidth + 2
        if width >= max(962, requiredWidth) {
            return .threeColumn
        }
        if width >= 701 {
            return .inspectorOverlay
        }
        return .compactOverlay
    }
}

enum MacWorkspaceColumnMetrics {
    static let minimumSidebarWidth: CGFloat = 220
    static let maximumSidebarWidth: CGFloat = 360
    static let preferredSidebarWidth: CGFloat = MacDesignTokens.Layout.sidebarWidth
    static let minimumConversationWidth: CGFloat = 420
    static let minimumInspectorWidth: CGFloat = 320
    static let maximumInspectorWidth: CGFloat = 480
    static let preferredInspectorWidth: CGFloat = 360

    static func inspectorWidth(
        availableWidth: CGFloat,
        sidebarWidth: CGFloat,
        storedWidth: CGFloat
    ) -> CGFloat {
        let spaceAfterConversation = availableWidth - sidebarWidth - minimumConversationWidth - 2
        let availableInspectorWidth = min(availableWidth <= 1_000 ? 380 : maximumInspectorWidth, spaceAfterConversation)
        let effectiveMaximum = max(minimumInspectorWidth, availableInspectorWidth)
        return min(effectiveMaximum, max(minimumInspectorWidth, storedWidth))
    }
}

private enum MacWorkspaceOverlay: Equatable {
    case sidebar
    case inspector
}

/// Owns the reference-backed workspace model through SwiftUI's lazy
/// `StateObject` initialization boundary. `State(initialValue:)` evaluates its
/// model expression every time SwiftUI reconstructs the View value, even when
/// the existing State is retained; because `MacWorkspaceModel` restores the
/// canonical transcript in its initializer, that produced repeated database
/// reads and observation publications during one window launch.
@MainActor
private final class MacWorkspaceModelOwner: ObservableObject {
    let model: MacWorkspaceModel

    init(model: MacWorkspaceModel) {
        self.model = model
    }

    deinit {
        // SwiftUI may release the window owner without another view callback.
        // The owner is not retained by the model's tasks, so this still breaks
        // a detached projection task's strong reference to its Workspace.
        let detachedModel = model
        Task { @MainActor in detachedModel.presentationDidDetach() }
    }
}

struct MacChatWorkspaceView: View {
    private let initialInspectorCaptureTab: String?
    private let forcesInspectorVisible: Bool
    private let remoteHost: any MacRemoteAgentRunning
    private let inspectorDependencies: MacInspectorDependencies
    private let previewRepository: MacCanonicalConversationRepository
    @StateObject private var modelOwner: MacWorkspaceModelOwner
    @StateObject private var roomWorkspace: MacAgentRoomWorkspace
    @State private var showsExpertManagement = false
    @State private var showsAgentRooms = false
    @State private var messageEditSession: MacMessageEditSession?
    @State private var messageEditError: String?
    @State private var expertSelection: Set<String> = []
    private var model: MacWorkspaceModel { modelOwner.model }
    @State private var selectedSection: MacWorkspaceSection
    private var navigationActions: MacWorkspaceNavigationActions {
        MacWorkspaceNavigationActions(model: model, extensions: extensionsNavigation,
            currentSection: { selectedSection }, setSection: { selectedSection = $0 })
    }
    private var sectionSelection: Binding<MacWorkspaceSection> {
        Binding(get: { selectedSection }, set: { section in
            guard section != selectedSection else { return }
            navigationActions.navigate(to: section) {
                if section == .settings { settingsDestination = nil }
            }
        })
    }
    @State private var pendingPaletteNavigation: (() -> Void)?

    private func performAfterPaletteDismissal(_ action: @escaping () -> Void) {
        pendingPaletteNavigation = action
        showsCommandPalette = false
    }
    @StateObject private var inspectorNavigation: MacInspectorNavigationState
    @StateObject private var dataNavigation = MacDataCenterNavigationState()
    @StateObject private var automationNavigation = MacAutomationCenterNavigationState()
    @StateObject private var extensionsNavigation = MacExtensionsNavigationState()
    @StateObject private var modelNavigation = MacModelCenterNavigationState(selectedTab: .api)
    @StateObject private var remoteModelDraft = MacRemoteModelEditingState()
    @StateObject private var localModelEditing = MacLocalModelEditingState()
    @State private var columnVisibility = NavigationSplitViewVisibility.all
    @AppStorage("nola.mac.sidebar.width") private var storedSidebarWidth = Double(
        MacWorkspaceColumnMetrics.preferredSidebarWidth
    )
    @State private var showsInspector = false
    @AppStorage("nola.mac.inspector.width") private var storedInspectorWidth = Double(
        MacWorkspaceColumnMetrics.preferredInspectorWidth
    )
    @State private var layoutMode = MacWorkspaceLayoutMode.threeColumn
    @State private var hasResolvedLayout = false
    @State private var activeOverlay: MacWorkspaceOverlay?
    @State private var codeReference: MacCodeReference?
    @State private var localPreviewOwner: String?
    @State private var localPreviewSourceConfirmed = false
    private var canReadLocalConversationFiles: Bool {
        localPreviewSourceConfirmed && localPreviewOwner == model.selectedConversationID && model.executionHost == .local
    }
    @State private var hasActivatedContentOwner = false
    @State private var sidebarArea = MacWorkspaceSidebarArea.conversations
    @State private var resourceSearch = ""
    @State private var settingsDestination: String?
    @State private var showsActivitySummaries = false
    @State private var detailScrollPositions = MacDetailScrollPositions()
    @State private var contentWorkspaces: [String: MacConversationContentWorkspace] = [:]
    @FocusState private var focusedContentTab: String?
    private var contentOwner: String { model.selectedConversationID ?? "new-conversation" }
    private var contentWorkspace: MacConversationContentWorkspace { contentWorkspaces[contentOwner] ?? MacConversationContentWorkspace() }
    private func changeContent(_ change: (inout MacConversationContentWorkspace) -> Void) {
        var workspace = contentWorkspace
        change(&workspace)
        contentWorkspaces[contentOwner] = workspace
        if let object = workspace.selectedObject, case .section(let section) = object { selectedSection = section }
        else { selectedSection = .chat }
    }
    private func navigateContent(_ change: (inout MacConversationContentWorkspace) -> Void, afterNavigation: (() -> Void)? = nil) {
        let owner = contentOwner, previous = contentWorkspace
        var next = previous; change(&next)
        let target: MacWorkspaceSection
        if let object = next.selectedObject, case .section(let section) = object { target = section }
        else { target = .chat }
        navigationActions.navigate(to: target, ifStillValid: { contentOwner == owner && contentWorkspace == previous }) {
            contentWorkspaces[owner] = next
            activeOverlay = nil
            focusedContentTab = next.selection
            afterNavigation?()
        }
    }
    private var isManagementContent: Bool {
        guard let object = contentWorkspace.selectedObject else { return false }
        switch object {
        case .section, .activity: return true
        default: return false
        }
    }

    private func openActivitySource(_ id: String) {
        navigationActions.navigate(to: .chat,
            ifStillValid: { model.conversations.contains { $0.id == id } }) {
            model.selectedConversationID = id
            var workspace = contentWorkspaces[id] ?? MacConversationContentWorkspace()
            workspace.select(MacConversationContentWorkspace.conversationID)
            contentWorkspaces[id] = workspace
            sidebarArea = .conversations
            activeOverlay = nil
            if layoutMode != .threeColumn { showsInspector = false }
        }
    }
    private func openContent(_ object: MacWorkspaceContentObject) {
        navigateContent({ $0.open(object) }, afterNavigation: {
            if object == .section(.settings) { settingsDestination = nil }
        })
    }
    private func selectContent(_ id: String) { navigateContent { $0.select(id) } }
    private func closeContent(_ id: String) { navigateContent { $0.close(id) } }
    @State private var sidebarDragStartWidth: CGFloat?
    @State private var inspectorDragStartWidth: CGFloat?
    @State private var showsCommandPalette = false
    @State private var remoteHostConnectionState = MacRemoteHostConnectionState.initial
    @State private var localExecutionBackends: [MacRemoteExecutionBackend] = [
        MacRemoteExecutionBackend(id: "nola", displayName: "Nola", availability: "available",
            version: nil, diagnostic: nil)
    ]
    @State private var isDiscoveringLocalExecutionBackends = false
    @State private var localExecutionBackendDiscoveryError: String?
    @State private var remoteExecutionBackends: [MacRemoteExecutionBackend] = []
    @State private var remoteHostVerificationRevision = 0
    @FocusState private var composerFocused: Bool
    @State private var isEditingTitle = false
    @State private var inlineTitleDraft = ""
    @FocusState private var titleFocused: Bool
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var colorScheme

    init(
        initialConversationID: String? = nil,
        initialInspectorCaptureTab: String? = nil,
        forcesInspectorVisible: Bool = false,
        canonicalRepository: MacCanonicalConversationRepository,
        localAgentRuntime: any MacLocalAgentRuntime,
        remoteHost: any MacRemoteAgentRunning,
        inspectorDependencies: MacInspectorDependencies,
        model injectedModel: MacWorkspaceModel? = nil
    ) {
        self.initialInspectorCaptureTab = initialInspectorCaptureTab
        self.forcesInspectorVisible = forcesInspectorVisible
        _showsInspector = State(initialValue: forcesInspectorVisible)
        self.remoteHost = remoteHost
        self.inspectorDependencies = inspectorDependencies
        self.previewRepository = canonicalRepository
        let roomExecution = MacRoomExecutionService(repository: canonicalRepository,
            localRuntime: localAgentRuntime, remoteHost: remoteHost)
        let roomWorkspace = MacAgentRoomWorkspace(
            service: canonicalRepository.makeAgentRoomService(),
            executeOwner: { try await roomExecution.execute($0, intent: $1) })
        roomWorkspace.prepareOwner = { try await roomExecution.prepare($0) }
        roomWorkspace.recoverOwner = { receipt, _ in
            guard let context = receipt.contextConversationID else { throw MacEditedResendError.sourceUnavailable }
            roomExecution.observe(contextID: context)
        }
        roomWorkspace.cancelPreparedOwner = { try await roomExecution.cancelPrepared($0) }
        roomWorkspace.executionService = roomExecution
        roomWorkspace.agentModelOptions = { try canonicalRepository.agentModelOptions() }
        let roomRouting = canonicalRepository.makeRoomRoutingService()
        roomWorkspace.routeSubmission = { try await roomRouting.route($0, room: $1) }
        _roomWorkspace = StateObject(wrappedValue: roomWorkspace)
        _inspectorNavigation = StateObject(wrappedValue: MacInspectorNavigationState(initialPane: initialInspectorCaptureTab))
        _modelOwner = StateObject(
            wrappedValue: MacWorkspaceModelOwner(
                model: injectedModel
                    ?? MacWorkspaceModel(
                        initialConversationID: initialConversationID,
                        canonicalRepository: canonicalRepository,
                        localAgentRuntime: localAgentRuntime,
                        remoteHost: remoteHost
                    )
            )
        )
        _selectedSection = State(
            initialValue: forcesInspectorVisible ? .chat : MacWorkspaceSection.initialSection()
        )
    }

    var body: some View {
        @Bindable var model = modelOwner.model
        GeometryReader { geometry in
            let resolvedLayoutMode = MacWorkspaceLayoutMode.resolve(
                width: geometry.size.width,
                sidebarWidth: columnVisibility == .detailOnly ? 0 : sidebarWidth(availableWidth: geometry.size.width)
            )

            ZStack {
                MacWorkspaceBackground()
                    .ignoresSafeArea()

                VStack(spacing: 0) {
                workspaceHeader
                HStack(spacing: 0) {
                    if resolvedLayoutMode != .compactOverlay,
                       columnVisibility != .detailOnly {
                        workspaceSidebar
                            .frame(width: sidebarWidth(availableWidth: geometry.size.width))
                            .background(MacDesignColor.sidebar)
                            .overlay(alignment: .trailing) {
                                Rectangle().fill(MacDesignColor.borderSubtle.opacity(0.55)).frame(width: 0.5)
                            }
                            .transition(.move(edge: .leading).combined(with: .opacity))
                        sidebarResizeHandle(availableWidth: geometry.size.width)
                    }

                    workspaceContent
                        .environment(\.macUsesGlobalWorkspaceChrome, true)
                        // Chrome transitions must not interpolate transcript layout or text.
                        .transaction(value: showsInspector) { $0.animation = nil }
                        .transaction(value: activeOverlay) { $0.animation = nil }
                        .transaction(value: columnVisibility) { $0.animation = nil }
                        .frame(
                            minWidth: MacWorkspaceColumnMetrics.minimumConversationWidth,
                            maxWidth: .infinity,
                            maxHeight: .infinity
                        )

                    if resolvedLayoutMode == .threeColumn,
                       !isManagementContent, (showsInspector || forcesInspectorVisible) {
                        inspectorResizeHandle(availableWidth: geometry.size.width)
                        inspectorContent
                            .frame(width: inspectorWidth(availableWidth: geometry.size.width))
                            .transition(.move(edge: .trailing).combined(with: .opacity))
                    }
                }

                }
                workspaceOverlay(availableWidth: geometry.size.width)
                    .padding(.top, 48)
                    .zIndex(10)
            }
            .task(id: resolvedLayoutMode) {
                await Task.yield()
                guard !Task.isCancelled else { return }
                let previousLayoutMode = layoutMode
                layoutMode = resolvedLayoutMode
                if hasResolvedLayout, previousLayoutMode == .threeColumn, resolvedLayoutMode != .threeColumn,
                   showsInspector || forcesInspectorVisible {
                    if forcesInspectorVisible { activeOverlay = .inspector }
                    else { showsInspector = false; activeOverlay = nil }
                } else if resolvedLayoutMode == .threeColumn {
                    activeOverlay = nil
                }
                hasResolvedLayout = true
                if forcesInspectorVisible, resolvedLayoutMode != .threeColumn {
                    activeOverlay = .inspector
                }
            }
        }
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.structure, value: columnVisibility)
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.structure, value: showsInspector)
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.structure, value: activeOverlay)
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.pageChange, value: selectedSection)
        .toolbar(removing: .sidebarToggle)
        .controlSize(.regular)
        .font(.system(size: 13))
        .onAppear {
            model.remoteObservationForegroundChanged(NSApplication.shared.isActive)
            model.presentationDidAttach()
        }
        .onDisappear { model.presentationDidDetach() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            model.remoteObservationForegroundChanged(true)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.willResignActiveNotification)) { _ in
            model.remoteObservationForegroundChanged(false)
        }
        .background(MacDesignColor.canvas.ignoresSafeArea())
        .environment(\.macMessageEditAction, MacMessageEditAction(enabled: !model.isRunning && !roomWorkspace.hasUnsettledExpertWork(conversationID: model.selectedConversationID)) { conversation, message in
            Task { @MainActor in
                do { messageEditSession = .init(draft: try await model.editedResendDraft(conversationID: conversation, message: message)) }
                catch { messageEditError = error.localizedDescription }
            }
        })
        .sheet(item: $messageEditSession) { session in
            MacMessageEditSheet(draft: session.draft, submit: { draft in
                try await model.performEditedResend(draft, expertWorkspace: roomWorkspace)
                messageEditSession = nil
            }, cancel: {
                if let source = session.draft.sourceConversationID { model.selectedConversationID = source }
                messageEditSession = nil
            })
        }
        .alert(String(localized: "Edit and resend"), isPresented: Binding(get: { messageEditError != nil }, set: { if !$0 { messageEditError = nil } })) {
            Button("OK", role: .cancel) { messageEditError = nil }
        } message: { Text(messageEditError ?? "") }
        .environment(\.macFileReferenceBaseURL, model.effectiveWorkspaceURL)
        .environment(\.macDetailScrollPositions, detailScrollPositions)
        .environment(\.macDetailScrollScope, contentOwner)
        .environment(
            \.macOpenFileReference,
            MacOpenFileReferenceAction { reference in
                openCodeReference(reference)
            }
        )
        .background {
            MacChatWindowChromeConfigurator(title: model.selectedConversationDisplayTitle)
                .frame(width: 0, height: 0)
        }
        // Mounting loads the initial selection. Subsequent selections are
        // loaded by MacWorkspaceModel.selectedConversationID's observer;
        // keying this task by the same ID duplicated recovery/storage work.
        .task {
            await Task.yield()
            guard !Task.isCancelled else { return }
            await model.loadSelectedConversationIfNeeded()
        }
        .task {
            if let conversationID = MacNotificationRouter.shared.consumePendingConversationID() {
                openWindow(value: conversationID)
            }
            await refreshRemoteHostConnection()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacOpenConversation)) { notification in
            guard let conversationID = notification.userInfo?["conversation_id"] as? String else { return }
            _ = MacNotificationRouter.shared.consumePendingConversationID()
            openWindow(value: conversationID)
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacPersistentDataDidChange)) { _ in
            model.reloadPersistentData()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacConfigurationDidChange)) { _ in
            model.refreshModelConfiguration()
            Task {
                await refreshRemoteHostConnection()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { _ in
            model.refreshModelConfiguration()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacLocalModelRuntimeDidChange)) { _ in
            Task { await refreshRemoteHostConnection() }
        }
        .onChange(of: selectedSection) { _, section in
            if section != .chat, contentWorkspace.selectedObject != .section(section) {
                changeContent { $0.open(.section(section)) }
            }
            if section != .chat { activeOverlay = nil }
        }
        .task(id: "\(model.selectedConversationID ?? "")/\(model.executionHost)/\(model.selectedMessages.count)") {
            localPreviewSourceConfirmed = false
            let owner = model.selectedConversationID
            localPreviewOwner = owner
            guard let owner, model.executionHost == .local else { return }
            localPreviewSourceConfirmed = (try? previewRepository.conversationSourceOwner(conversationID: owner)) == .local
        }
        .task(id: model.selectedConversationID) {
            if !hasActivatedContentOwner, selectedSection != .chat {
                changeContent { $0.open(.section(selectedSection)) }
            } else { changeContent { $0.select(MacConversationContentWorkspace.conversationID) } }
            hasActivatedContentOwner = true
            if let id = model.selectedConversationID, model.selectedConversation?.isPersisted == true {
                await roomWorkspace.restoreConversation(id)
                if !Task.isCancelled {
                    expertSelection = Set(roomWorkspace.pendingExpertIDs)
                    if model.draft.isEmpty, let text = roomWorkspace.pendingAdmissionText {
                        model.setDraft(text, conversationID: id)
                    }
                }
            }
        }
        .onChange(of: model.selectedConversationID) { _, _ in
            expertSelection = []
            activeOverlay = nil
            codeReference = nil
            if model.selectedConversation?.isPersisted == false {
                Task { @MainActor in
                    await Task.yield()
                    composerFocused = true
                }
            }
        }
        .focusedSceneValue(
            \.nolaMacCommandActions,
            MacCommandActions(
                newConversation: navigationActions.newConversation,
                focusComposer: { selectContent(MacConversationContentWorkspace.conversationID); composerFocused = true },
                openWorkspace: model.chooseWorkspace,
                reviewWorkspace: reviewWorkspace,
                showCommandPalette: { showsCommandPalette = true },
                toggleSidebar: toggleSidebar,
                toggleInspector: toggleInspector,
                stop: { model.stop() },
                openConversationWindow: openSelectedConversationWindow
            )
        )
        .sheet(isPresented: $showsAgentRooms) {
            VStack {
                MacChatExpertPicker(agents: roomWorkspace.directory, selection: $expertSelection,
                    manage: { showsExpertManagement = true })
                if let error = roomWorkspace.directoryError { Text(error).foregroundStyle(.red).padding() }
            }.frame(minWidth: 420, minHeight: 400)
                .task { await roomWorkspace.refreshExpertDirectory() }
                .sheet(isPresented: $showsExpertManagement) {
                    MacExpertDirectoryManagementView(workspace: roomWorkspace)
                }
        }
        .sheet(isPresented: $showsCommandPalette, onDismiss: {
            let action = pendingPaletteNavigation
            pendingPaletteNavigation = nil
            action?()
        }) {
            MacCommandPaletteView(
                model: model,
                newConversation: { performAfterPaletteDismissal(navigationActions.newConversation) },
                selectConversation: { id in
                    performAfterPaletteDismissal { navigationActions.selectConversation(id) }
                },
                focusComposer: {
                    performAfterPaletteDismissal {
                        navigationActions.navigate(to: .chat, then: { composerFocused = true })
                    }
                },
                toggleSidebar: toggleSidebar,
                toggleInspector: toggleInspector,
                dismiss: { showsCommandPalette = false }
            )
        }
    }

    private var inspectorParticipants: [MacInspectorParticipant] {
        guard let association = roomWorkspace.conversationAssociation,
              association.conversationID == model.selectedConversationID else { return [] }
        return association.room.room.participants.map { MacInspectorParticipant(id: $0.id, name: $0.displayName) }
    }

    @ViewBuilder
    private var inspectorContent: some View {
        if let codeReference {
            MacCodeReferenceInspector(
                reference: codeReference,
                canReadLocally: canReadLocalConversationFiles,
                close: { self.codeReference = nil },
                openInTab: { openContent(.file(codeReference)) }
            )
        } else {
            MacChatInspector(
                model: model,
                hostDisplayName: hostSelectorTitle,
                initialKnowledgePaneRawValue: initialInspectorCaptureTab,
                dependencies: inspectorDependencies,
                navigation: inspectorNavigation,
                close: { activeOverlay = nil; showsInspector = false; codeReference = nil },
                showsHeaderNavigation: false,
                participants: inspectorParticipants,
                expertSessions: AnyView(MacExpertSessionsInspector(workspace: roomWorkspace,
                    conversationID: model.selectedConversationID).id(model.selectedConversationID)),
                canReadLocalFiles: canReadLocalConversationFiles,
                inspectFile: openCodeReference,
                openReview: { openContent(.review($0)) }
            )
        }
    }

    private func inspectorWidth(availableWidth: CGFloat) -> CGFloat {
        let sidebarWidth = columnVisibility == .detailOnly
            ? 0
            : sidebarWidth(availableWidth: availableWidth)
        return MacWorkspaceColumnMetrics.inspectorWidth(
            availableWidth: availableWidth,
            sidebarWidth: sidebarWidth,
            storedWidth: CGFloat(storedInspectorWidth)
        )
    }

    private func sidebarWidth(availableWidth: CGFloat) -> CGFloat {
        let availableMaximum = max(
            MacWorkspaceColumnMetrics.minimumSidebarWidth,
            availableWidth - MacWorkspaceColumnMetrics.minimumConversationWidth
        )
        return min(
            min(MacWorkspaceColumnMetrics.maximumSidebarWidth, availableMaximum),
            max(MacWorkspaceColumnMetrics.minimumSidebarWidth, CGFloat(storedSidebarWidth))
        )
    }

    private func sidebarResizeHandle(availableWidth: CGFloat) -> some View {
        Rectangle()
            .fill(MacDesignColor.borderSubtle)
            .frame(width: 1)
            .overlay {
                Rectangle()
                    .fill(Color.clear)
                    .frame(width: 10)
                    .contentShape(Rectangle())
                    .onHover { hovering in
                        if hovering {
                            NSCursor.resizeLeftRight.push()
                        } else {
                            NSCursor.pop()
                        }
                    }
                    .gesture(
                        DragGesture(minimumDistance: 1)
                            .onChanged { value in
                                let start = sidebarDragStartWidth
                                    ?? sidebarWidth(availableWidth: availableWidth)
                                if sidebarDragStartWidth == nil {
                                    sidebarDragStartWidth = start
                                }
                                storedSidebarWidth = Double(
                                    min(
                                        MacWorkspaceColumnMetrics.maximumSidebarWidth,
                                        max(
                                            MacWorkspaceColumnMetrics.minimumSidebarWidth,
                                            start + value.translation.width
                                        )
                                    )
                                )
                            }
                            .onEnded { _ in
                                storedSidebarWidth = Double(
                                    sidebarWidth(availableWidth: availableWidth)
                                )
                                sidebarDragStartWidth = nil
                            }
                    )
            }
    }

    private func inspectorResizeHandle(availableWidth: CGFloat) -> some View {
        Rectangle()
            .fill(MacDesignColor.borderSubtle)
            .frame(width: 1)
            .overlay {
                Rectangle()
                    .fill(Color.clear)
                    .frame(width: 10)
                    .contentShape(Rectangle())
                    .gesture(
                        DragGesture(minimumDistance: 1)
                            .onChanged { value in
                                let start = inspectorDragStartWidth
                                    ?? inspectorWidth(availableWidth: availableWidth)
                                if inspectorDragStartWidth == nil {
                                    inspectorDragStartWidth = start
                                }
                                storedInspectorWidth = Double(
                                    max(
                                        MacWorkspaceColumnMetrics.minimumInspectorWidth,
                                        start - value.translation.width
                                    )
                                )
                            }
                            .onEnded { _ in
                                storedInspectorWidth = Double(
                                    inspectorWidth(availableWidth: availableWidth)
                                )
                                inspectorDragStartWidth = nil
                            }
                    )
            }
            .accessibilityLabel(String(localized: "Inspector width"))
            .accessibilityValue("\(Int(inspectorWidth(availableWidth: availableWidth)))")
            .accessibilityAdjustableAction { direction in
                let delta: CGFloat = direction == .increment ? 10 : -10
                storedInspectorWidth = Double(min(MacWorkspaceColumnMetrics.maximumInspectorWidth,
                    max(MacWorkspaceColumnMetrics.minimumInspectorWidth, inspectorWidth(availableWidth: availableWidth) + delta)))
            }
            .onKeyPress(.leftArrow) {
                storedInspectorWidth = Double(min(MacWorkspaceColumnMetrics.maximumInspectorWidth, inspectorWidth(availableWidth: availableWidth) + 10))
                return .handled
            }
            .onKeyPress(.rightArrow) {
                storedInspectorWidth = Double(max(MacWorkspaceColumnMetrics.minimumInspectorWidth, inspectorWidth(availableWidth: availableWidth) - 10))
                return .handled
            }
            .focusable()
            .help(String(localized: "Drag to resize the inspector"))
    }

    @ViewBuilder
    private func workspaceOverlay(availableWidth: CGFloat) -> some View {
        if let activeOverlay, activeOverlay == .sidebar || !isManagementContent {
            ZStack {
                Color.black.opacity(0.16)
                    .ignoresSafeArea()
                    .contentShape(Rectangle())
                    .onTapGesture {
                        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.structure) {
                            self.activeOverlay = nil
                        }
                    }

                if activeOverlay == .sidebar {
                    HStack(spacing: 0) {
                        workspaceSidebar
                            .frame(
                                width: max(
                                    180,
                                    min(CGFloat(storedSidebarWidth), availableWidth - 48)
                                )
                            )
                            .background(MacDesignColor.sidebar)
                            .shadow(color: .black.opacity(0.20), radius: 20, x: 8)
                        Spacer(minLength: 0)
                    }
                    .transition(.move(edge: .leading).combined(with: .opacity))
                } else {
                    HStack(spacing: 0) {
                        Spacer(minLength: 0)
                        inspectorContent
                        .frame(
                            width: min(
                                min(availableWidth <= 1_000 ? 380 : 480, max(320, CGFloat(storedInspectorWidth))),
                                max(300, availableWidth - 56)
                            )
                        )
                        .shadow(color: .black.opacity(0.20), radius: 20, x: -8)
                    }
                    .transition(.move(edge: .trailing).combined(with: .opacity))
                }
            }
            .transition(.opacity)
        }
    }

    private var workspaceContent: some View {
        ZStack {
            conversationContent
                .opacity(contentWorkspace.selection == MacConversationContentWorkspace.conversationID ? 1 : 0)
                .allowsHitTesting(contentWorkspace.selection == MacConversationContentWorkspace.conversationID)
                .accessibilityHidden(contentWorkspace.selection != MacConversationContentWorkspace.conversationID)
            ForEach(contentWorkspace.objects) { object in
                contentObjectView(object)
                    .environment(\.macDetailScrollScope, contentOwner + "/" + object.id)
                    .id(contentOwner + "/" + object.id)
                    .opacity(contentWorkspace.selection == object.id ? 1 : 0)
                    .allowsHitTesting(contentWorkspace.selection == object.id)
                    .accessibilityHidden(contentWorkspace.selection != object.id)
            }
        }
    }

    private var conversationContent: some View {
            VStack(spacing: 0) {
                if let error = roomWorkspace.admissionError ?? roomWorkspace.directoryError {
                    Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled).padding(8)
                        .accessibilityIdentifier("chat.expert-admission-error")
                }
                MacConversationCanvas(
                    model: model,
                    composerFocused: $composerFocused,
                    expertSelection: $expertSelection,
                    sendToExperts: {
                        let text = model.draft
                        let experts = expertSelection
                        Task {
                            do {
                                let owner = try model.prepareExpertConversation()
                                let admittedText = await roomWorkspace.sendExperts(conversationID: owner, text: text, agentIDs: experts)
                                if let admittedText {
                                    if model.draft == admittedText { model.setDraft("", conversationID: owner) }
                                    model.reloadPersistentData()
                                }
                            } catch { roomWorkspace.directoryError = error.localizedDescription }
                        }
                    },
                    expertWorkspace: roomWorkspace,
                    modelSetupRequired: modelSetupRequired,
                    conversationControls: AnyView(composerConversationControls),
                    modelControls: AnyView(composerModelControls),
                    onCommand: { command in
                        switch command {
                        case .experts: showsAgentRooms = true
                        case .resources: inspectorNavigation.selectedTab = .artifacts; showsInspector = true
                        case .permissions: inspectorNavigation.selectedTab = .context; showsInspector = true
                        case .plan: dataNavigation.section = .plans; selectedSection = .data
                        default: break
                        }
                    }
                )
            }
    }

    @ViewBuilder
    private func contentObjectView(_ object: MacWorkspaceContentObject) -> some View {
        switch object {
        case .file(let reference):
            MacCodeReferenceInspector(reference: reference, canReadLocally: canReadLocalConversationFiles, close: { closeContent(object.id) })
        case .section(let section):
            sectionContent(section)
        case .review(let url):
            MacWorkspaceReviewView(route: .init(path: url.path), embedsInWorkspace: true)
        case .activity(let entry):
            AppleActivityDetailView(entry: entry, load: { target in
                try await Task.detached { try MacApplicationPort.shared.activityDetail(target) }.value
            }, openSource: openActivitySource, embedded: true)
        }
    }

    @ViewBuilder
    private func sectionContent(_ section: MacWorkspaceSection) -> some View {
        switch section {
        case .chat: EmptyView()
        case .data:
            MacDataCenterView(navigationAction: toggleSidebar, navigation: dataNavigation, automationNavigation: automationNavigation)
                .transition(.opacity)
        case .models:
            MacModelCenterView(
                remoteHost: remoteHost,
                navigationAction: toggleSidebar,
                navigation: modelNavigation,
                remoteDraft: remoteModelDraft,
                localEditing: localModelEditing
            )
                .transition(.opacity)
        case .extensions:
            MacExtensionsCenterView(remoteHost: remoteHost, kinds: [.skill, .mcp], navigationAction: toggleSidebar, navigation: extensionsNavigation)
                .transition(.opacity)
        case .capabilities:
            MacCapabilityCenterView(navigationAction: toggleSidebar)
                .transition(.opacity)
        case .memory:
            MacMemoryCenterPage()
        case .automations:
            MacAutomationCenterView()
        case .settings:
            MacSettingsView(embedsInWorkspace: true, initialCategory: settingsDestination) { profileID in
                if let profileID { modelNavigation.showSelectedModel(remoteProfileID: profileID) }
                else { modelNavigation.showRemoteModels() }
                openContent(.section(.models))
            }
        }
    }

    private var workspaceHeader: some View {
        HStack(spacing: 6) {
            HStack(spacing: 2) {
                ForEach(MacWorkspaceSidebarArea.allCases) { area in
                    Button {
                        let visible = layoutMode == .compactOverlay
                            ? activeOverlay == .sidebar : columnVisibility != .detailOnly
                        if visible && sidebarArea == area { toggleSidebar() }
                        else {
                            sidebarArea = area
                            withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.structure) {
                                if layoutMode == .compactOverlay { activeOverlay = .sidebar }
                                else { columnVisibility = .all }
                            }
                        }
                    } label: {
                        Image(area.iconAsset).renderingMode(.template).resizable().scaledToFit().frame(width: 18, height: 18)
                            .frame(width: 36, height: 36)
                    }
                    .macToolbarIcon(isActive: sidebarArea == area && (layoutMode == .compactOverlay
                        ? activeOverlay == .sidebar : columnVisibility != .detailOnly))
                    .help(area == .activity ? String(localized: "workspace.activity.help") : area.title).accessibilityLabel(area.title)
                    .accessibilityIdentifier("workspace.area.\(area.rawValue)")
                }
                Spacer(minLength: 0)
                Button(action: toggleSidebar) {
                    Image("nola-sidebar").renderingMode(.template).resizable().scaledToFit()
                        .frame(width: 18, height: 18).frame(width: 36, height: 36)
                }
                .macToolbarIcon()
                .help(String(localized: "Toggle Sidebar"))
                .accessibilityLabel(String(localized: "Toggle Sidebar"))
                .accessibilityIdentifier("workspace.toggleSidebar")
            }
            .padding(.leading, 12)
            .padding(.trailing, 8)
            .frame(width: columnVisibility == .detailOnly || layoutMode == .compactOverlay
                ? 176 : CGFloat(storedSidebarWidth), alignment: .leading)
            contentTabStrip
            Button(action: navigationActions.newConversation) {
                Image("nola-plus").renderingMode(.template).resizable().scaledToFit().frame(width: 16, height: 16).frame(width: 36, height: 36)
            }.macToolbarIcon().help(String(localized: "New Conversation"))
                .accessibilityLabel(String(localized: "New Conversation"))
                .accessibilityIdentifier("workspace.newConversation")
            Button { model.isTranscriptSearchVisible.toggle() } label: {
                Image("nola-search").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 18, height: 18).frame(width: 36, height: 36)
            }
            .macToolbarIcon(isActive: model.isTranscriptSearchVisible)
            .help(String(localized: "Search Conversation"))
            .accessibilityLabel(String(localized: "Search Conversation"))
            .accessibilityIdentifier("workspace.searchConversation")
            inspectorButtons
        }
        .padding(.trailing, 10)
        .frame(height: 48)
        .background(MacDesignColor.chrome)
        .overlay(alignment: .bottom) { Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 0.5) }
        .accessibilityElement(children: .contain)
    }

    private var contentTabStrip: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal) {
                HStack(spacing: 2) {
                    contentTab(id: MacConversationContentWorkspace.conversationID,
                        title: model.selectedConversationDisplayTitle, closeable: false)
                    ForEach(contentWorkspace.objects) { object in
                        contentTab(id: object.id, title: object.title, closeable: true)
                            .onAppear {
                                if contentWorkspace.selection == object.id {
                                    proxy.scrollTo(object.id, anchor: .center)
                                }
                            }
                    }
                }
                .fixedSize(horizontal: true, vertical: false)
            }
            .defaultScrollAnchor(.leading)
            .onChange(of: contentWorkspace.selection) { _, selection in
                // A newly opened tab needs its layout before it can be scrolled into view.
                DispatchQueue.main.async { proxy.scrollTo(selection, anchor: .center) }
            }
        }
        .scrollIndicators(.visible)
        .padding(.top, 5)
        .frame(maxWidth: .infinity, minHeight: 48, maxHeight: 48, alignment: .leading)
        .accessibilityIdentifier("workspace.contentTabs")
    }

    private func contentTab(id: String, title: String, closeable: Bool) -> some View {
        HStack(spacing: 0) {
            Button { selectContent(id) } label: {
                Text(title).font(.system(size: 12)).lineLimit(1)
                    .padding(.horizontal, 12).frame(minWidth: 82, maxWidth: closeable ? 202 : 230, minHeight: 43, alignment: .leading)
                    .contentShape(Rectangle())
            }.buttonStyle(.plain).focused($focusedContentTab, equals: id)
                .accessibilityAddTraits(contentWorkspace.selection == id ? .isSelected : [])
                .onMoveCommand { direction in moveContentTab(from: id, direction: direction) }
                .onDeleteCommand { if closeable { closeContent(id) } }
            if closeable {
                Button { closeContent(id) } label: { Image("nola-close").renderingMode(.template).resizable().scaledToFit().frame(width: 10, height: 10).frame(width: 28, height: 32) }
                    .buttonStyle(.plain).help(String(localized: "Close"))
                    .accessibilityLabel(String(localized: "Close") + " " + title)
            }
        }
        .frame(minWidth: 110, minHeight: 43, maxHeight: 43)
        .background(contentWorkspace.selection == id ? MacDesignColor.selected : Color.clear,
            in: UnevenRoundedRectangle(topLeadingRadius: 7, topTrailingRadius: 7))
        .id(id)
        .help(title)
    }

    private func moveContentTab(from id: String, direction: MoveCommandDirection) {
        let ids = [MacConversationContentWorkspace.conversationID] + contentWorkspace.objects.map(\.id)
        guard let index = ids.firstIndex(of: id) else { return }
        let offset = direction == .left ? -1 : direction == .right ? 1 : 0
        guard offset != 0 else { return }
        let next = ids[(index + ids.count + offset) % ids.count]
        selectContent(next); focusedContentTab = next
    }

    @ViewBuilder
    private var workspaceSidebar: some View {
        switch sidebarArea {
        case .conversations:
            MacConversationSidebar(model: model, selectedSection: sectionSelection,
                selectConversation: { id in
                    navigationActions.selectConversation(id)
                    if model.selectedConversationID == id { selectContent(MacConversationContentWorkspace.conversationID) }
                }, loadParticipantNames: { id in
                    let service = roomWorkspace.service
                    return try await Task.detached {
                        try service.forConversation(id)?.room.room.participants
                            .filter(\.active).map(\.displayName) ?? []
                    }.value
                })
        case .activity:
            VStack(spacing: 0) {
                AppleActivityView(load: { query in
                    try await Task.detached { try MacApplicationPort.shared.listActivity(query) }.value
                }, detail: { entry in
                    try await Task.detached { try MacApplicationPort.shared.activityDetail(entry) }.value
                }, openSource: openActivitySource,
                    openDetail: { openContent(.activity($0)) })
                Button("activity.localSummaries") { showsActivitySummaries = true }.padding(10)
                workspaceSettingsEntry
            }
            .sheet(isPresented: $showsActivitySummaries) { MacLocalActivitySummariesPanel() }
        case .resources:
            VStack(alignment: .leading, spacing: 0) {
                TextField(String(localized: "Search"), text: $resourceSearch).textFieldStyle(.roundedBorder)
                    .padding(.bottom, 12)
                ForEach(MacWorkspaceSection.allCases.filter { $0 != .chat && $0 != .settings }) { section in
                    if resourceSearch.isEmpty || String(localized: String.LocalizationValue(section.titleKey)).localizedCaseInsensitiveContains(resourceSearch) {
                        Button { openContent(.section(section)) } label: {
                            Text(section.title).frame(maxWidth: .infinity, alignment: .leading).frame(minHeight: 40)
                                .padding(.horizontal, 8).contentShape(Rectangle())
                        }.buttonStyle(MacSidebarNavigationButtonStyle(isSelected: selectedSection == section))
                            .help(section.title)
                        Divider()
                    }
                }
                resourceNavigationLink(String(localized: "Activity")) { sidebarArea = .activity }
                Spacer()
                workspaceSettingsEntry
            }.padding(16)
        }
    }

    private var workspaceSettingsEntry: some View {
        VStack(spacing: 0) {
            Divider()
            Button { openContent(.section(.settings)) } label: {
                Label { Text(String(localized: "Settings")) } icon: {
                    Image("nola-settings").renderingMode(.template).resizable().scaledToFit().frame(width: 18, height: 18)
                }.padding(.horizontal, 8).frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).contentShape(Rectangle())
            }.buttonStyle(MacSidebarNavigationButtonStyle(isSelected: selectedSection == .settings))
                .help(String(localized: "Settings"))
        }
    }

    @ViewBuilder
    private func resourceNavigationLink(_ title: String, action: @escaping () -> Void) -> some View {
        if resourceSearch.isEmpty || title.localizedCaseInsensitiveContains(resourceSearch) {
            Button(action: action) {
                Text(verbatim: title).frame(maxWidth: .infinity, alignment: .leading).frame(minHeight: 40)
                    .padding(.horizontal, 8).contentShape(Rectangle())
            }.buttonStyle(.plain)
                .help(title)
            Divider()
        }
    }

    private var inspectorButtons: some View {
        MacConversationInspectorButtons(selection: inspectorNavigation.selectedTab,
            isVisible: !isManagementContent && (layoutMode == .threeColumn ? showsInspector : activeOverlay == .inspector)) { tab in
                    if isManagementContent {
                        navigateContent({ $0.select(MacConversationContentWorkspace.conversationID) }) {
                            inspectorNavigation.selectedTab = tab
                            showsInspector = true
                            activeOverlay = layoutMode == .threeColumn ? nil : .inspector
                        }
                        return
                    }
                    let visible = layoutMode == .threeColumn ? showsInspector : activeOverlay == .inspector
                    let nextVisible = inspectorNavigation.select(tab, isVisible: visible)
                    if !nextVisible { toggleInspector() }
                    else {
                        codeReference = nil
                        showsInspector = true
                        if layoutMode != .threeColumn { activeOverlay = .inspector }
                    }
        }
    }

    private var sidebarToggleButton: some View {
        Button(action: toggleSidebar) {
            Image(systemName: "sidebar.left")
                .font(.system(size: 13, weight: .semibold))
                .frame(
                    width: MacDesignTokens.Control.iconButton,
                    height: MacDesignTokens.Control.iconButton
                )
        }
        .macToolbarIcon(
            isActive: layoutMode == .compactOverlay
                ? activeOverlay == .sidebar
                : columnVisibility != .detailOnly
        )
        .help(String(localized: "Toggle Sidebar"))
        .accessibilityLabel(String(localized: "Toggle Sidebar"))
    }

    @ViewBuilder
    private var conversationTitleControl: some View {
        if isEditingTitle {
            TextField(String(localized: "Conversation name"), text: $inlineTitleDraft)
                .textFieldStyle(.plain)
                .font(.system(size: 14.5, weight: .semibold))
                .multilineTextAlignment(.center)
                .focused($titleFocused)
                .onSubmit { commitInlineTitle() }
                .onExitCommand { cancelInlineTitle() }
        } else {
            Button(action: beginInlineTitle) {
                Text(model.selectedConversationDisplayTitle)
                    .font(.system(size: 14.5, weight: .semibold))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .minimumScaleFactor(0.82)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(
                model.selectedConversation == nil
                    ? String(localized: "Rename Conversation")
                    : model.selectedConversationDisplayTitle
            )
        }
    }

    // Borderless AppKit menus do not tint custom SVG labels consistently.
    // Bake only the menu image tint under the selected appearance; keep the
    // shared vector asset and native menu/keyboard behavior unchanged.
    private func conversationMenuIcon(_ asset: String, tint: NSColor = .secondaryLabelColor) -> some View {
        let appearance = NSAppearance(named: colorScheme == .dark ? .darkAqua : .aqua)
        let image = NSImage(size: NSSize(width: 14, height: 14), flipped: false) { bounds in
            guard let source = NSImage(named: asset) else { return false }
            appearance?.performAsCurrentDrawingAppearance {
                source.draw(in: bounds)
                tint.setFill()
                bounds.fill(using: .sourceIn)
            }
            return true
        }
        return Image(nsImage: image).renderingMode(.original)
            .resizable().scaledToFit().frame(width: 14, height: 14)
            .accessibilityHidden(true)
    }

    private var workspaceSelector: some View {
        Menu {
            Section(String(localized: "Working Environment")) {
                Button {
                    model.useSessionSandbox()
                } label: {
                    Label(
                        String(localized: "Session Sandbox"),
                        systemImage: model.usesSessionSandbox
                            ? "checkmark"
                            : "shippingbox"
                    )
                }
                .disabled(model.isSelectedConversationVisiblyRunning)
                Button {
                    model.chooseWorkspace()
                } label: {
                    Label(
                        String(localized: "Choose Folder…"),
                        systemImage: model.usesSessionSandbox
                            ? "folder.badge.plus"
                            : "checkmark"
                    )
                }
            }
            Divider()
            Button {
                NSWorkspace.shared.open(model.effectiveWorkspaceURL)
            } label: {
                Label(
                    String(localized: "Open Workspace in Finder"),
                    systemImage: "folder"
                )
            }
            Button {
                openWindow(
                    value: MacWorkspaceReviewRoute(
                        path: model.effectiveWorkspaceURL.path
                    )
                )
            } label: {
                Label(
                    String(localized: "Review Workspace Changes"),
                    systemImage: "doc.text.magnifyingglass"
                )
            }
            if !model.usesSessionSandbox {
                Divider()
                Button(role: .destructive) {
                    model.useSessionSandbox()
                } label: {
                    Label(
                        String(localized: "Disconnect Folder"),
                        systemImage: "xmark.circle"
                    )
                }
            }
        } label: {
            HStack(spacing: 5) {
                conversationMenuIcon("nola-folder")
                Text(model.workspaceDisplayName).lineLimit(1).truncationMode(.middle)
                Image("nola-chevron").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 8, height: 8).rotationEffect(.degrees(90))
            }
            .font(.system(size: 11.5, weight: .medium)).foregroundStyle(.secondary)
            .padding(.horizontal, 8).frame(maxWidth: 180).frame(height: 28)
            .background(MacDesignColor.hover, in: RoundedRectangle(cornerRadius: 7))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .tint(.secondary)
        .fixedSize(horizontal: true, vertical: false)
        .help(model.effectiveWorkspaceURL.path)
        .accessibilityLabel(
            "\(String(localized: "Workspace")): \(model.workspaceDisplayName)"
        )
    }

    private var executionBackendSelector: some View {
        Menu {
            Text(model.executionHost == .local ? String(localized: "execution.scope.local", table: "Chat") : String(localized: "execution.scope.host", table: "Chat"))
            Text(String(localized: "execution.scope.child", table: "Chat"))
            if activeExecutionBackendCatalog.first(where: { $0.id == model.executionBackend.id })?.capabilities?.supportsPermissionBridge == false {
                Text(String(localized: "execution.scope.noApprovalBridge", table: "Chat"))
            }
            Divider()
            if model.executionHost == .local {
                Button {
                    Task { await refreshLocalExecutionBackends() }
                } label: {
                    Label(String(localized: "Discover or Refresh Local Execution Units", table: "Chat"),
                          systemImage: "arrow.clockwise")
                }
                .disabled(isDiscoveringLocalExecutionBackends)
                .accessibilityIdentifier("chat.discoverExecutionBackends")
                if isDiscoveringLocalExecutionBackends {
                    Text(String(localized: "Discovering Local Execution Units…", table: "Chat"))
                }
                if let error = localExecutionBackendDiscoveryError {
                    Text(error)
                }
                Divider()
            }
            Section(String(localized: "Execution Unit")) {
                ForEach(activeExecutionBackendCatalog) { descriptor in
                    let option = MacExecutionBackendOption(
                        id: descriptor.id,
                        displayName: descriptor.displayName
                    )
                    let available = descriptor.isAvailable
                    Button {
                        model.selectExecutionBackend(option, available: available)
                        if option.id != "nola",
                           let initialModel = descriptor.models.first(where: {
                               $0.id == descriptor.defaultModelID
                           }) ?? descriptor.models.first {
                            model.selectExternalModel(initialModel, backendID: option.id)
                        }
                    } label: {
                        Label(
                            available
                                ? executionBackendLabel(option, descriptor: descriptor)
                                : "\(option.displayName) · \(executionBackendAvailabilityLabel(descriptor))",
                            systemImage: model.executionBackend.id == option.id
                                ? "checkmark"
                                : "terminal"
                        )
                    }
                    .disabled(!available)
                    .help(descriptor.diagnostic ?? executionBackendAvailabilityLabel(descriptor))
                }
            }
        } label: {
            HStack(spacing: 5) {
                conversationMenuIcon("nola-execution")
                Text(String(localized: "Execution Unit") + ": " + selectedExecutionBackendDisplayName)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.system(size: 7, weight: .semibold))
            }
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .frame(height: 28)
            .background(MacDesignColor.hover, in: RoundedRectangle(cornerRadius: 7))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .tint(.secondary)
        .fixedSize(horizontal: true, vertical: false)
        .help("\(String(localized: "Execution Unit")): \(selectedExecutionBackendDisplayName)")
        .accessibilityIdentifier("chat.executionBackendPicker")
    }

    private var selectedExecutionBackendDisplayName: String {
        activeExecutionBackendCatalog
            .first(where: { $0.id == model.executionBackend.id })?
            .displayName ?? model.executionBackend.displayName
    }

    private var activeExecutionBackendCatalog: [MacRemoteExecutionBackend] {
        model.executionHost == .remote
            ? remoteExecutionBackends
            : localExecutionBackends
    }

    private var activeExecutionBackendDescriptor: MacRemoteExecutionBackend? {
        activeExecutionBackendCatalog.first(where: { $0.id == model.executionBackend.id })
    }

    private var externalExecutionModelSelector: some View {
        let descriptor = activeExecutionBackendDescriptor
        let selected = model.selectedExternalModel(in: descriptor)
        return Menu {
            if let descriptor, !descriptor.models.isEmpty {
                Section(String(localized: "Execution Model")) {
                    ForEach(descriptor.models) { option in
                        Button {
                            model.selectExternalModel(option, backendID: descriptor.id)
                        } label: {
                            Label(
                                option.displayName,
                                systemImage: selected?.id == option.id ? "checkmark" : "cpu"
                            )
                        }
                    }
                }
            } else {
                Text(String(localized: "Managed by execution unit"))
            }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "cpu")
                    .font(.system(size: 10, weight: .medium))
                Text(selected?.displayName ?? String(localized: "Default Model"))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.down")
                    .font(.system(size: 8, weight: .semibold))
            }
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(.primary)
            .padding(.horizontal, 8)
            .frame(minWidth: 132, idealWidth: 156, maxWidth: 180)
            .frame(height: 28)
            .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 7))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .tint(.secondary)
        .help(String(localized: "Choose the model used by this execution unit"))
    }

    private func executionBackendDescriptor(
        _ option: MacExecutionBackendOption
    ) -> MacRemoteExecutionBackend? {
        activeExecutionBackendCatalog.first(where: { $0.id == option.id })
    }

    private func executionBackendLabel(
        _ option: MacExecutionBackendOption,
        descriptor: MacRemoteExecutionBackend?
    ) -> String {
        guard let version = descriptor?.version, !version.isEmpty else {
            return option.displayName
        }
        return "\(option.displayName) · \(version)"
    }

    private func executionBackendAvailabilityLabel(
        _ descriptor: MacRemoteExecutionBackend?
    ) -> String {
        switch descriptor?.availability {
        case "unauthenticated": return String(localized: "Sign in required")
        case "incompatible": return String(localized: "Incompatible version")
        case "unreachable": return String(localized: "Unavailable")
        default: return String(localized: "Not installed")
        }
    }

    private var expertManagementButton: some View {
        Button { showsAgentRooms = true } label: {
            HStack(spacing: 5) {
                Image("nola-person").renderingMode(.template).resizable().scaledToFit().frame(width: 16, height: 16)
                Text("chat.experts.title")
            }.padding(.horizontal, 8).frame(height: 28)
        }
        .buttonStyle(.plain)
        .background(MacDesignColor.hover, in: RoundedRectangle(cornerRadius: 7))
        .accessibilityIdentifier("chat.experts")
    }

    private var composerConversationControls: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                expertManagementButton
                executionBackendSelector
                hostSelector
                workspaceSelector
                if MacExecutionBackendPolicy.requiresNolaModel(model.executionBackend.id) {
                    if !modelSetupRequired { modelSelector }
                } else { externalExecutionModelSelector }
            }.fixedSize(horizontal: true, vertical: false)
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) { expertManagementButton; executionBackendSelector; hostSelector }
                HStack(spacing: 8) {
                    workspaceSelector
                    if MacExecutionBackendPolicy.requiresNolaModel(model.executionBackend.id) {
                        if !modelSetupRequired { modelSelector }
                    } else { externalExecutionModelSelector }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.conversationControls")
    }

    private var modelSetupRequired: Bool {
        MacExecutionBackendPolicy.requiresNolaModel(model.executionBackend.id) && model.selectedModelOption == nil
    }

    @ViewBuilder
    private var composerModelControls: some View {
        if MacExecutionBackendPolicy.requiresNolaModel(model.executionBackend.id) {
            if modelSetupRequired {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(alignment: .top, spacing: 10) {
                        Image("nola-models").renderingMode(.template).resizable().scaledToFit()
                            .frame(width: 20, height: 20).foregroundStyle(.secondary)
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(String(localized: "Choose a model to start chatting", table: "Chat"))
                                .font(.system(size: model.selectedMessages.isEmpty ? 20 : 14, weight: .semibold))
                            Text(String(localized: "Connect an API account or use a model on this Mac. Your draft stays here.", table: "Chat"))
                                .font(.system(size: 12)).foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    HStack(spacing: 10) {
                        Button(action: manageChatModels) {
                            Text(String(localized: "Configure Model", table: "Chat"))
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(MacDesignColor.primaryAction)
                        .controlSize(.large)
                        .accessibilityIdentifier("chat.configureModel")
                        if !model.availableModelOptions.isEmpty { modelSelector }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(14)
                .background(MacDesignColor.selected, in: RoundedRectangle(cornerRadius: 12))
                .accessibilityIdentifier("chat.modelSetupPrompt")
            }
        }
    }

    private func manageChatModels() {
        if model.executionHost == .remote {
            Task {
                do {
                    let url = try await remoteHost.requestWebManagementURL(target: "/settings#models")
                    NSWorkspace.shared.open(url)
                } catch {
                    remoteHostConnectionState = .failed(macRemoteHostUserFacingError(error))
                }
            }
        } else {
            if let option = model.selectedModelOption {
                modelNavigation.showSelectedModel(remoteProfileID: option.isLocal ? nil : option.configurationID)
            } else {
                modelNavigation.showRemoteModels()
            }
            openContent(.section(.models))
        }
    }

    private var modelSelector: some View {
        Menu {
            Button { model.restoreDefaultModelInheritance() } label: {
                Label(String(localized: "Use default chat model", table: "ModelCenter"),
                      systemImage: model.inheritsDefaultModel ? "checkmark" : "arrow.uturn.backward")
            }
            Divider()
            if model.configuredModelOptions.isEmpty && model.downloadedModelOptions.isEmpty {
                Text(String(localized: "Not configured"))
            }
            if !model.configuredModelOptions.isEmpty {
                Section(model.executionHost == .remote
                    ? String(localized: "Connected Host") : String(localized: "Provider Model")) {
                    ForEach(model.configuredModelOptions) { option in
                        modelOptionButton(option)
                    }
                }
            }
            if !model.downloadedModelOptions.isEmpty {
                Section(String(localized: "Local Model")) {
                    ForEach(model.downloadedModelOptions) { option in
                        modelOptionButton(option)
                    }
                }
            }
            Divider()
            Button(String(localized: "Conversation model parameters", table: "ModelCenter")) {
                model.isModelSettingsPresented = true
            }
            .disabled(model.selectedModelOption == nil)
            if model.executionHost == .remote {
                Button {
                    Task { await refreshRemoteHostConnection() }
                } label: {
                    Label(String(localized: "Refresh"), systemImage: "arrow.clockwise")
                }
            }
            Button(action: manageChatModels) {
                Label(String(localized: "Manage Models…"), systemImage: "slider.horizontal.3")
            }
        } label: {
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(model.activeModelName)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Image(systemName: "chevron.down")
                        .font(.system(size: 8, weight: .semibold))
                }
                if let status = model.selectedModelCapabilityStatus, let label = status.label {
                    Text(label)
                        .font(.system(size: 9.5, weight: .regular))
                        .foregroundStyle(status == .needsConfirmation ? Color.orange : Color.secondary)
                        .lineLimit(1)
                }
            }
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(.primary)
            .padding(.horizontal, 8)
            .frame(minWidth: 132, idealWidth: 190, maxWidth: 230, alignment: .leading)
            .frame(height: 28)
            .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 7))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .tint(.secondary)
        .layoutPriority(1)
        .help(model.activeModelName)
    }

    private var hostSelector: some View {
        Menu {
            Section(String(localized: "Execution")) {
                Button {
                    model.selectExecutionHost(.local)
                } label: {
                    Label(MacExecutionHost.local.title, systemImage: model.executionHost == .local ? "checkmark" : "desktopcomputer")
                }
                Button {
                    model.selectExecutionHost(.remote)
                    Task { await refreshRemoteHostConnection() }
                } label: {
                    Label(MacExecutionHost.remote.title, systemImage: model.executionHost == .remote ? "checkmark" : "network")
                }
                .disabled(!model.canUseRemoteHost)
            }
            Section(macHostText("mac.host.menu.status", "Host Status")) {
                Label(hostConnectionStatusTitle, systemImage: hostConnectionStatusSymbol)
                if case .failed(let message) = remoteHostConnectionState {
                    Text(message)
                }
            }
            if remoteHostConnectionState.isConfigured {
                Button {
                    Task { await refreshRemoteHostConnection() }
                } label: {
                    Label(macHostText("mac.host.action.retry", "Test Again"), systemImage: "arrow.clockwise")
                }
                .disabled({
                    if case .checking = remoteHostConnectionState { return true }
                    return false
                }())
            }
            Divider()
            Button {
                openHostSettings()
            } label: {
                Label(
                    remoteHostConnectionState.isConfigured
                        ? macHostText("mac.host.action.edit", "Edit Host Connection…")
                        : macHostText("mac.host.action.configure", "Set Up Host…"),
                    systemImage: "gearshape"
                )
            }
        } label: {
            HStack(spacing: 5) {
                conversationMenuIcon("nola-host", tint: NSColor(hostSelectorColor).withAlphaComponent(0.92))
                Text(hostSelectorTitle)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.system(size: 7, weight: .semibold))
            }
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(hostSelectorColor.opacity(0.92))
            .padding(.horizontal, 8)
            .frame(height: 28)
            .background(hostSelectorColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 7))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .tint(hostSelectorColor)
        .fixedSize(horizontal: true, vertical: false)
        .help(
            "\(String(localized: "Execution")): \(hostConnectionStatusTitle)"
        )
    }

    private var hostSelectorTitle: String {
        guard model.executionHost == .remote else { return MacExecutionHost.local.title }
        switch remoteHostConnectionState {
        case .notConfigured:
            return macHostText("mac.host.status.not_configured", "Set Up Host")
        case .unverified:
            return MacExecutionHost.remote.title
        case .checking:
            return macHostText("mac.host.status.checking", "Connecting…")
        case .connected(let report):
            return report.displayName
        case .failed:
            return macHostText("mac.host.status.offline", "Host Offline")
        }
    }

    private var hostConnectionStatusTitle: String {
        switch remoteHostConnectionState {
        case .notConfigured:
            return macHostText("mac.host.detail.not_configured", "No host configured")
        case .unverified:
            return macHostText("mac.host.detail.unverified", "Host is configured but not verified")
        case .checking:
            return macHostText("mac.host.detail.checking", "Checking host connection…")
        case .connected(let report):
            return "\(report.displayName) · \(macHostText("mac.host.status.connected", "Connected"))"
        case .failed:
            return macHostText("mac.host.detail.failed", "Host is currently unreachable")
        }
    }

    private var hostConnectionStatusSymbol: String {
        switch remoteHostConnectionState {
        case .notConfigured: return "network.slash"
        case .unverified: return "network.badge.shield.half.filled"
        case .checking: return "network"
        case .connected: return "checkmark.circle.fill"
        case .failed: return "exclamationmark.triangle.fill"
        }
    }

    private var hostSelectorColor: Color {
        guard model.executionHost == .remote else { return .secondary }
        switch remoteHostConnectionState {
        case .notConfigured: return .secondary
        case .unverified: return .orange
        case .checking: return .secondary
        case .connected: return .secondary
        case .failed: return .red
        }
    }

    private func refreshRemoteHostConnection() async {
        remoteHostVerificationRevision &+= 1
        let revision = remoteHostVerificationRevision
        let configured = await remoteHost.isConfigured()
        model.updateRemoteHostConfigurationState(configured)
        guard configured else {
            remoteHostConnectionState = .notConfigured
            remoteExecutionBackends = []
            return
        }
        remoteHostConnectionState = .checking
        model.updateRemoteModelCatalog(nil)
        do {
            let report = try await remoteHost.testSavedConnection()
            async let backends = remoteHost.fetchExecutionBackends()
            async let profiles = remoteHost.fetchRemoteModelCatalog()
            let (resolvedBackends, resolvedProfiles) = try await (backends, profiles)
            guard revision == remoteHostVerificationRevision else { return }
            remoteHostConnectionState = .connected(report)
            remoteExecutionBackends = resolvedBackends
            model.updateRemoteModelCatalog(resolvedProfiles)
        } catch {
            guard revision == remoteHostVerificationRevision else { return }
            remoteHostConnectionState = .failed(macRemoteHostUserFacingError(error))
            remoteExecutionBackends = []
            model.updateRemoteModelCatalog(nil)
        }
    }

    private func refreshLocalExecutionBackends() async {
        guard !isDiscoveringLocalExecutionBackends else { return }
        isDiscoveringLocalExecutionBackends = true
        localExecutionBackendDiscoveryError = nil
        defer { isDiscoveringLocalExecutionBackends = false }
        do {
            localExecutionBackends = try await MacLocalExecutionBackendCatalog.fetch()
        } catch {
            Logger.shared.log(
                .error,
                "MacExecutionBackends",
                "Local execution backend catalog failed",
                metadata: ["error": error.localizedDescription]
            )
            localExecutionBackendDiscoveryError = error.localizedDescription
        }
    }

    private func openHostSettings() {
        navigateContent({ $0.open(.section(.settings)) }, afterNavigation: {
            settingsDestination = "connections"
        })
    }

    private func modelOptionButton(_ option: MacChatModelOption) -> some View {
        Button {
            model.selectModel(option)
        } label: {
            Label(
                option.displayName,
                systemImage: model.selectedModelOption?.id == option.id ? "checkmark" : option.symbol
            )
        }
    }

    private func beginInlineTitle() {
        inlineTitleDraft = model.selectedConversation.map {
            MacConversationTitlePolicy.displayTitle($0.title, conversationID: $0.id)
        } ?? ""
        isEditingTitle = true
        Task { @MainActor in
            await Task.yield()
            titleFocused = true
        }
    }

    private func commitInlineTitle() {
        model.renameSelectedConversation(to: inlineTitleDraft)
        isEditingTitle = false
        titleFocused = false
    }

    private func cancelInlineTitle() {
        isEditingTitle = false
        titleFocused = false
    }

    private func toggleSidebar() {
        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.structure) {
            if layoutMode == .compactOverlay {
                activeOverlay = activeOverlay == .sidebar ? nil : .sidebar
            } else {
                activeOverlay = nil
                columnVisibility = columnVisibility == .detailOnly ? .all : .detailOnly
            }
        }
    }

    private func toggleInspector() {
        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.structure) {
            if layoutMode == .threeColumn {
                activeOverlay = nil
                if showsInspector {
                    showsInspector = false
                    codeReference = nil
                } else {
                    showsInspector = true
                }
            } else {
                showsInspector = true
                activeOverlay = activeOverlay == .inspector ? nil : .inspector
            }
        }
    }

    private func openCodeReference(_ reference: MacCodeReference) {
        // A file URI carries no Host authority. Recheck canonical ownership at
        // the action boundary; selecting a local endpoint cannot relabel history.
        guard let owner = model.selectedConversationID,
              MacFilePreviewPolicy.allowsLocalRead(
                sourceOwner: try? previewRepository.conversationSourceOwner(conversationID: owner),
                isLocalExecutionHost: model.executionHost == .local, url: reference.url) else { return }
        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.structure) {
            codeReference = reference
            showsInspector = true
            if layoutMode != .threeColumn {
                activeOverlay = .inspector
            }
        }
    }

    private func openSelectedConversationWindow() {
        guard let selectedConversationID = model.selectedConversationID else {
            openWindow(id: "chat")
            return
        }
        openWindow(value: selectedConversationID)
    }

    private func reviewWorkspace() {
        if model.workspaceURL == nil { model.chooseWorkspace() }
        guard let workspaceURL = model.workspaceURL else { return }
        openWindow(value: MacWorkspaceReviewRoute(path: workspaceURL.path))
    }
}

private struct MacAgentTimelineRow: View {
    let item: MacAgentActivity
    let showsConnector: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            VStack(spacing: 0) {
                Image(systemName: symbol)
                    .foregroundStyle(color)
                    .frame(width: 16, height: 16)
                if showsConnector {
                    Rectangle()
                        .fill(Color.primary.opacity(0.09))
                        .frame(width: 1, height: 28)
                }
            }
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text(item.title)
                        .fontWeight(.medium)
                    Spacer(minLength: 8)
                    TimelineView(.periodic(from: item.startedAt, by: 1)) { context in
                        Text(duration(at: context.date))
                            .font(.system(size: 9.5, design: .monospaced))
                            .foregroundStyle(.tertiary)
                    }
                }
                Text(item.detail)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
        }
        .font(.system(size: 11))
    }

    private var symbol: String {
        switch item.state {
        case .pending: return "circle"
        case .running: return "circle.dotted"
        case .waiting: return "hand.raised.circle.fill"
        case .completed: return "checkmark.circle.fill"
        case .failed: return "exclamationmark.circle.fill"
        }
    }

    private var color: Color {
        switch item.state {
        case .pending: return .secondary
        case .running: return .accentColor
        case .waiting: return .orange
        case .completed: return .green
        case .failed: return .red
        }
    }

    private func duration(at now: Date) -> String {
        let end = item.finishedAt ?? now
        let seconds = max(0, Int(end.timeIntervalSince(item.startedAt)))
        if seconds < 60 { return "\(seconds)s" }
        return "\(seconds / 60)m \(seconds % 60)s"
    }
}

#if DEBUG
struct MacChatWorkspaceSnapshotContent: View {
    let inspectorTab: String?
    let canonicalRepository: MacCanonicalConversationRepository

    init(
        inspectorTab: String? = nil,
        canonicalRepository: MacCanonicalConversationRepository
    ) {
        self.inspectorTab = inspectorTab
        self.canonicalRepository = canonicalRepository
    }

    var body: some View {
        MacChatWorkspaceView(
            initialInspectorCaptureTab: inspectorTab,
            forcesInspectorVisible: inspectorTab != nil,
            canonicalRepository: canonicalRepository,
            localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: MacRemoteHostService.shared,
            inspectorDependencies: MacInspectorDependencies(
                dataRegistry: MacDataSourceRegistry.shared,
                platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared,
                chromeMessaging: MacChromeMessagingService.shared
            )
        )
    }
}
#endif
