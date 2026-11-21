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
    var focusNewConversation: (String) -> Void = { _ in }

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
        navigate(to: .chat) {
            let previousID = model.selectedConversationID
            model.newConversation()
            if let createdID = model.selectedConversationID, createdID != previousID {
                focusNewConversation(createdID)
            }
        }
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
                    let progress = page.totalUTF8Bytes > 0
                        ? min(100, max(1, Int((Double(page.nextOffsetBytes) / Double(page.totalUTF8Bytes)) * 100.0)))
                        : 100
                    Text(verbatim: "#\(previous.count + 1) · \(progress)%")
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
    private let forcesInspectorVisible: Bool
    private let remoteHost: any MacRemoteAgentRunning
    private let inspectorDependencies: MacInspectorDependencies
    private let previewRepository: MacCanonicalConversationRepository
    @Environment(\.macActivityApplicationClient) private var activityClient
    @StateObject private var modelOwner: MacWorkspaceModelOwner
    @StateObject private var roomWorkspace: MacAgentRoomWorkspace
    @State private var returningExpertChild: (parentID: String, agentID: String)?
    @State private var expertChildPresentation: MacExpertChildPresentation?
    @State private var showsExpertManagement = false
    @State private var opensLibraryAfterPicker = false
    @State private var showsAgentRooms = false
    @State private var selectingPickerExpertID: String?
    @State private var pendingPickerReintroduction: (agent: MacPersistentAgent, owner: String)?
    @State private var messageEditSession: MacMessageEditSession?
    @State private var messageEditError: String?
    private var expertSelection: Set<String> { Set((model.selectedConversationID.map(model.draftMentions) ?? []).map { $0.expert.agentID }) }
    private func appendMention(_ expert: AppleMainExpertSelectionEntry, name: String, owner: String) {
        let insertion = AppleDraftMention.insertion(text: model.draft, range: NSRange(location: model.draft.utf16.count, length: 0),
            name: name, expert: expert, mentions: model.draftMentions(owner),
            sourceID: model.mainExpertDraftSource(owner), epoch: roomWorkspace.mainSourceEpoch?(owner))
        model.setDraft(insertion.text, conversationID: owner, mentions: insertion.mentions)
    }

    private func selectExpertFromPicker(_ agent: MacPersistentAgent, owner: String, allowReintroduction: Bool) {
        guard selectingPickerExpertID == nil else { return }
        selectingPickerExpertID = agent.id
        roomWorkspace.directoryError = nil
        let persisted = model.selectedConversation?.isPersisted == true
        Task { @MainActor in
            defer { selectingPickerExpertID = nil }
            do {
                try await roomWorkspace.prepareExpertMention(
                    agentID: agent.id,
                    expectedRevision: agent.revision,
                    conversationID: owner,
                    isPersisted: persisted,
                    allowReintroduction: allowReintroduction,
                    useMemberSnapshot: false
                )
                guard model.selectedConversationID == owner else { return }
                let member = roomWorkspace.conversationAssociation.flatMap {
                    $0.conversationID == owner ? $0.room.room.participants.first { $0.active && $0.agentID == agent.id } : nil
                }
                appendMention(
                    .init(
                        agentID: agent.id,
                        agentRevision: member?.agentRevision ?? agent.revision,
                        participantID: member?.id,
                        contextConversationID: member?.contextConversationID
                    ),
                    name: member?.displayName ?? agent.name,
                    owner: owner
                )
                showsAgentRooms = false
            } catch MacExpertMentionPreparationError.reintroductionRequired {
                guard model.selectedConversationID == owner else { return }
                pendingPickerReintroduction = (agent, owner)
            } catch {
                roomWorkspace.directoryError = error.localizedDescription
                await roomWorkspace.refreshExpertDirectory()
            }
        }
    }
    private var model: MacWorkspaceModel { modelOwner.model }
    @Environment(\.desktopPluginClient) private var pluginClient
    @State private var pluginLibrary: MacDesktopPluginLibrary?
    @State private var pluginLibraryError: String?
    private var pluginPages: [MacDesktopPluginPage] { pluginLibrary?.workspacePages ?? [] }
    @State private var selectedPlugin: MacDesktopPluginPage?
    @State private var pluginRefresh = 0
    @State private var selectedSection: MacWorkspaceSection
    private var navigationActions: MacWorkspaceNavigationActions {
        MacWorkspaceNavigationActions(model: model, extensions: extensionsNavigation,
            currentSection: { selectedSection }, setSection: {
                if $0 != .pluginPage { selectedPlugin = nil }
                selectedSection = $0
            },
            focusNewConversation: { focusComposer(in: $0) })
    }

    private func focusComposer(in conversationID: String?) {
        Task { @MainActor in
            await Task.yield()
            guard model.selectedConversationID == conversationID, selectedSection == .chat,
                  !model.isTranscriptSearchVisible else { return }
            composerFocused = true
        }
    }
    private var sectionSelection: Binding<MacWorkspaceSection> {
        Binding(get: { selectedSection }, set: { section in
            guard section != selectedSection else { return }
            navigationActions.navigate(to: section)
        })
    }
    @State private var pendingPaletteNavigation: (() -> Void)?

    private func performAfterPaletteDismissal(_ action: @escaping () -> Void) {
        pendingPaletteNavigation = action
        showsCommandPalette = false
    }
    @StateObject private var inspectorNavigation: MacInspectorNavigationState
    @StateObject private var dataNavigation: MacDataCenterNavigationState = {
        let state = MacDataCenterNavigationState()
        #if DEBUG
        if ProcessInfo.processInfo.environment["NOLA_CAPTURE_UI"] != nil,
           let raw = ProcessInfo.processInfo.environment["NOLA_CAPTURE_DATA_SECTION"]
               ?? ProcessInfo.processInfo.environment["NOLA_CAPTURE_SECTION"],
           let parsed = MacDataCenterSection(rawValue: raw) {
            state.section = parsed
        }
        #endif
        return state
    }()
    @StateObject private var automationNavigation = MacAutomationCenterNavigationState()
    @StateObject private var extensionsNavigation = MacExtensionsNavigationState()
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
    @State private var detailScrollPositions = MacDetailScrollPositions()
    @State private var localPreviewOwner: String?
    @State private var localPreviewSourceConfirmed = false
    private var canReadLocalConversationFiles: Bool {
        localPreviewSourceConfirmed && localPreviewOwner == model.selectedConversationID && model.executionHost == .local
    }
    private var isManagementContent: Bool {
        selectedSection != .chat
    }
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
    @State private var editingTitleConversationID: String?
    @State private var inlineTitleDraft = ""
    @State private var isTitleHovered = false
    @State private var isSidebarDividerHovered = false
    @State private var isInspectorDividerHovered = false
    @State private var leadingTrafficLightInset: CGFloat = 76
    @FocusState private var titleFocused: Bool
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
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
        roomWorkspace.requiresMainSource = true
        roomWorkspace.prepareOwner = { try await roomExecution.prepare($0) }
        roomWorkspace.recoverOwner = { receipt, _ in
            guard let context = receipt.contextConversationID else { throw MacEditedResendError.sourceUnavailable }
            roomExecution.observe(contextID: context)
        }
        roomWorkspace.cancelPreparedOwner = { try await roomExecution.cancelPrepared($0) }
        roomWorkspace.executionService = roomExecution
        roomWorkspace.agentModelOptions = { try canonicalRepository.agentModelOptions() }
        roomWorkspace.agentReasoningEfforts = { try canonicalRepository.agentReasoningEfforts(modelSelectionID: $0) }
        let roomRouting = canonicalRepository.makeRoomRoutingService()
        roomWorkspace.routeSubmission = { try await roomRouting.route($0, room: $1) }
        _roomWorkspace = StateObject(wrappedValue: roomWorkspace)
        _inspectorNavigation = StateObject(wrappedValue: MacInspectorNavigationState(captureTab: initialInspectorCaptureTab))
        _modelOwner = StateObject(
            wrappedValue: MacWorkspaceModelOwner(
                model: injectedModel
                    ?? MacWorkspaceModel(
                        initialConversationID: initialConversationID,
                        createMissingConversation: false,
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

    private var workspaceCanvas: some View {
        GeometryReader { geometry in
            let resolvedLayoutMode = MacWorkspaceLayoutMode.resolve(
                width: geometry.size.width,
                sidebarWidth: columnVisibility == .detailOnly ? 0 : sidebarWidth(availableWidth: geometry.size.width)
            )

            ZStack {
                MacWorkspaceBackground()
                    .ignoresSafeArea()

                VStack(spacing: 0) {
                workspaceHeader(availableWidth: geometry.size.width, resolvedLayoutMode: resolvedLayoutMode)
                HStack(spacing: 0) {
                    if resolvedLayoutMode != .compactOverlay,
                       columnVisibility != .detailOnly {
                        workspaceSidebar
                            .frame(width: sidebarWidth(availableWidth: geometry.size.width))
                            .background(MacDesignColor.sidebar)
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
        .ignoresSafeArea(.container, edges: .top)
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
    }

    private var mentionProfile: AppleMentionProfileActions {
        .init(description: { expert in
            guard let association = roomWorkspace.conversationAssociation, association.conversationID == model.selectedConversationID else { return nil }
            return association.room.room.participants.first { $0.id == expert.participantID && $0.contextConversationID == expert.contextConversationID && $0.agentRevision == expert.agentRevision }?.description
        }, history: openMentionHistory)
    }

    private func childPage(parent: String, offset: Int) async throws -> AppleExpertChildConversations {
        guard model.selectedConversationID == parent else { throw CancellationError() }
        let service = roomWorkspace.service
        if try model.isLocalConversationSource(parent) {
            return try await MacConversationReadTask.run { try service.childConversations(conversationID: parent, offset: offset) }
        }
        guard model.executionHost == .remote else { throw MacRemoteHostError.invalidResponse }
        let value = try await model.expertRoomCommand(owner: parent, command: .object([
            "type": .string("child_conversations"), "conversation_id": .string(parent),
            "limit": .signedInteger(32), "offset": .signedInteger(Int64(offset))]))
        let page = try JSONDecoder().decode(AppleExpertChildConversations.self, from: JSONEncoder().encode(value))
        try page.validate(parentID: parent, offset: offset)
        return page
    }

    private func readRemoteChild(_ presentation: MacExpertChildPresentation, before: Int64?) async throws -> MacExpertChildHistoryReader.Page {
        let parent = presentation.parentID
        guard let identity = presentation.remoteIdentity, model.executionHost == .remote,
              identity == model.mainExpertSourceIdentity(parent), try !model.isLocalConversationSource(parent) else { throw MacRemoteHostError.invalidResponse }
        let value = try await model.expertRoomCommand(owner: parent, command: .object([
            "type": .string("child_history"), "conversation_id": .string(parent),
            "child_conversation_id": .string(presentation.child.id), "before_ordinal": before.map(MacCanonicalJSONValue.signedInteger) ?? .null,
            "limit": .signedInteger(16), "content_byte_limit": .signedInteger(160_000)]))
        guard identity == model.mainExpertSourceIdentity(parent) else { throw CancellationError() }
        let page = try JSONDecoder().decode(MacExpertChildHistoryReader.Page.self, from: JSONEncoder().encode(value))
        guard page.sourceOwner == .local else { throw MacRemoteHostError.invalidResponse }
        return page
    }

    private func presentChild(parent: String, child: AppleExpertChildConversation) throws {
        guard parent == model.selectedConversationID else { throw CancellationError() }
        var remoteIdentity: String?
        if try !model.isLocalConversationSource(parent) {
            guard model.executionHost == .remote, let identity = model.mainExpertSourceIdentity(parent) else { throw MacRemoteHostError.invalidResponse }
            remoteIdentity = identity
        }
        expertChildPresentation = .init(parentID: parent, child: child, sourceID: ObjectIdentifier(roomWorkspace.service), remoteIdentity: remoteIdentity)
    }

    private func openMentionHistory(_ expert: AppleMainExpertSelectionEntry) {
        guard let parent = model.selectedConversationID else { return }
        Task { @MainActor in
            do {
                let (_, child) = try await AppleExpertChildConversations.child(for: expert, parentID: parent) { offset in
                    try await childPage(parent: parent, offset: offset)
                }
                try presentChild(parent: parent, child: child)
            } catch { if parent == model.selectedConversationID { roomWorkspace.directoryError = error.localizedDescription } }
        }
    }

    private var configuredWorkspace: some View {
        workspaceCanvas
        .environment(\.expertMentionProfile, mentionProfile)
        .environment(\.macMessageEditAction, MacMessageEditAction(sourceAllowsEditing: canReadLocalConversationFiles, enabled: !model.isRunning && !roomWorkspace.hasUnsettledExpertWork(conversationID: model.selectedConversationID)) { conversation, message in
            Task { @MainActor in
                do { messageEditSession = .init(draft: try await model.editedResendDraft(conversationID: conversation, message: message)) }
                catch { messageEditError = error.localizedDescription }
            }
        })
        .sheet(item: $messageEditSession) { session in
            MacMessageEditSheet(draft: session.draft, candidates: roomWorkspace.directory.filter { $0.availability == "active" }.map { agent in
                let member = roomWorkspace.conversationAssociation.flatMap { $0.conversationID == session.draft.sourceConversationID ? $0.room.room.participants.first { $0.active && $0.agentID == agent.id } : nil }
                return .init(expert: .init(agentID: agent.id, agentRevision: member?.agentRevision ?? agent.revision,
                    participantID: member?.id, contextConversationID: member?.contextConversationID), name: member?.displayName ?? agent.name, detail: agent.description)
            }, submit: { draft in
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
        .environment(\.macDetailScrollPositions, detailScrollPositions)
        .environment(\.macFileReferenceBaseURL, model.effectiveWorkspaceURL)
        .environment(
            \.macOpenFileReference,
            MacOpenFileReferenceAction { reference in
                openCodeReference(reference)
            }
        )
        .background {
            MacChatWindowChromeConfigurator(title: model.selectedConversationDisplayTitle) { inset in
                leadingTrafficLightInset = inset
            }
            .frame(width: 0, height: 0)
        }
        // Mounting loads the initial selection. Subsequent selections are
        // loaded by MacWorkspaceModel.selectedConversationID's observer;
        // keying this task by the same ID duplicated recovery/storage work.
        .task {
            roomWorkspace.mainCommand = { [weak model] owner, command in
                guard let model else { throw CancellationError() }
                return try await model.expertRoomCommand(owner: owner, command: command)
            }
            roomWorkspace.mainIsRemote = { [weak model] in model?.executionHost == .remote }
            roomWorkspace.mainSourceEpoch = { [weak model] owner in model?.mainExpertSelectionSourceEpoch(owner) }
            roomWorkspace.mainSourceIdentity = { [weak model] owner in model?.mainExpertSourceIdentity(owner) }
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
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in pluginRefresh += 1 }
        .onReceive(NotificationCenter.default.publisher(for: .macDesktopPluginsChanged)) { _ in pluginRefresh += 1 }
        .task(id: pluginRefresh) {
            do {
                let library = try await pluginClient.library()
                guard !Task.isCancelled else { return }
                pluginLibrary = library
                pluginLibraryError = nil
                if let selected = selectedPlugin {
                    if let current = library.workspacePages.first(where: { $0.id == selected.id }) { selectedPlugin = current }
                    else { navigationActions.navigate(to: .chat) }
                }
            } catch { if !Task.isCancelled { pluginLibraryError = error.localizedDescription } }
        }
        .onReceive(NotificationCenter.default.publisher(for: .macDefaultChatModelDidChange)) { _ in
            model.refreshModelConfiguration()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacLocalModelRuntimeDidChange)) { _ in
            Task { await refreshRemoteHostConnection() }
        }
        .onChange(of: selectedSection) { _, section in
            if section != .chat { activeOverlay = nil }
            if section == .memory { dataNavigation.section = .memory }
        }
        .task(id: "\(model.selectedConversationID ?? "")/\(model.executionHost)/\(model.selectedMessages.count)") {
            localPreviewSourceConfirmed = false
            let owner = model.selectedConversationID
            localPreviewOwner = owner
            guard let owner, model.executionHost == .local else { return }
            localPreviewSourceConfirmed = (try? previewRepository.conversationSourceOwner(conversationID: owner)) == .local
        }
        .task(id: "\(model.selectedConversationID ?? "")/\(model.executionHost)/\(roomWorkspace.mainSourceReady)/\(!expertSelection.isEmpty)") {
            guard let owner = model.selectedConversationID, roomWorkspace.mainSourceReady, model.executionHost == .remote,
                  !expertSelection.isEmpty, model.mainExpertDraftSource(owner) == nil else { return }
            await roomWorkspace.refreshExpertDirectory()
        }
        .task(id: model.selectedConversationID) {
            if let id = model.selectedConversationID, model.selectedConversation?.isPersisted == true {
                await roomWorkspace.restoreConversation(id)
                if !Task.isCancelled {
                    if model.draft.isEmpty, let text = roomWorkspace.pendingAdmissionText {
                        model.setDraft(text, conversationID: id)
                    }
                }
            }
        }
        .task(id: "\(model.selectedConversationID ?? "")/\(model.isRunning)/\(model.selectedMessages.count)") {
            guard let id = model.selectedConversationID,
                  model.selectedConversation?.isPersisted == true else { return }
            await roomWorkspace.refreshPeerExchanges(for: id)
            while model.isRunning, !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
                guard !Task.isCancelled else { return }
                await roomWorkspace.refreshPeerExchanges(for: id)
            }
        }
        .onChange(of: model.selectedConversationID) { _, selectedID in
            if isEditingTitle { commitInlineTitle() }
            isTitleHovered = false
            if returningExpertChild?.parentID != selectedID { returningExpertChild = nil }
            activeOverlay = nil
            codeReference = nil
            if model.selectedConversation?.isPersisted == false { focusComposer(in: selectedID) }
        }
        .onChange(of: model.isTranscriptSearchVisible) { _, visible in
            if !visible { focusComposer(in: model.selectedConversationID) }
        }
        .onChange(of: selectedSection) { _, _ in
            if isEditingTitle { cancelInlineTitle() }
            isTitleHovered = false
        }
        .onChange(of: titleFocused) { _, focused in
            if !focused && isEditingTitle {
                commitInlineTitle()
            }
        }
    }

    var body: some View {
        configuredWorkspace
        .focusedSceneValue(
            \.nolaMacCommandActions,
            MacCommandActions(
                newConversation: navigationActions.newConversation,
                focusComposer: { navigationActions.navigate(to: .chat, then: { composerFocused = true }) },
                openWorkspace: { Task { await model.chooseWorkspace() } },
                reviewWorkspace: reviewWorkspace,
                showCommandPalette: { showsCommandPalette = true },
                toggleSidebar: toggleSidebar,
                toggleInspector: toggleInspector,
                stop: { model.stop() },
                openConversationWindow: openSelectedConversationWindow
            )
        )
        .sheet(item: $expertChildPresentation) { presentation in
            MacExpertChildHistoryView(presentation: presentation,
                execution: presentation.remoteIdentity == nil ? roomWorkspace.executionService : nil,
                remoteRead: { before in try await readRemoteChild(presentation, before: before) },
                remoteText: { cursor in try await model.expertHistoryMarkdown(parent: presentation.parentID, child: presentation.child.id,
                    identity: presentation.remoteIdentity, cursor: cursor) }) {
                guard presentation.sourceID == ObjectIdentifier(roomWorkspace.service) else {
                    throw MacApplicationPortError.serviceUnavailable
                }
                try await roomWorkspace.selectCurrentChild(presentation.child, parentID: presentation.parentID)
                guard expertChildPresentation?.id == presentation.id else { throw CancellationError() }
                returningExpertChild = (presentation.parentID, presentation.child.agentID)
                navigationActions.selectConversation(presentation.parentID)
                guard model.selectedConversationID == presentation.parentID else { throw CancellationError() }
                if let member = roomWorkspace.conversationAssociation?.room.room.participants.first(where: { $0.id == presentation.child.participantID && $0.active }) {
                    appendMention(.init(agentID: member.agentID, agentRevision: member.agentRevision, participantID: member.id,
                        contextConversationID: member.contextConversationID), name: member.displayName, owner: presentation.parentID)
                }
                composerFocused = true
            }
        }
        .sheet(isPresented: $showsExpertManagement) {
            MacExpertDirectoryManagementView(workspace: roomWorkspace)
        }
        .sheet(isPresented: $showsAgentRooms, onDismiss: {
            pendingPickerReintroduction = nil
            if opensLibraryAfterPicker { opensLibraryAfterPicker = false; showsExpertManagement = true }
        }) {
            VStack {
                MacChatExpertPicker(agents: roomWorkspace.directory, selection: .constant(expertSelection),
                    manage: model.executionHost == .remote ? nil : { opensLibraryAfterPicker = true; showsAgentRooms = false }, selected: { agent in
                        if let owner = model.selectedConversationID {
                            selectExpertFromPicker(agent, owner: owner, allowReintroduction: false)
                        }
                    })
                    .disabled(selectingPickerExpertID != nil)
                if let error = roomWorkspace.directoryError {
                    MacInlineNotice(message: error, kind: .failure, dismiss: { roomWorkspace.directoryError = nil })
                        .padding(.horizontal, 16)
                        .padding(.bottom, 12)
                }
            }.frame(minWidth: 420, minHeight: 400)
                .task(id: roomWorkspace.mainSourceReady) { await roomWorkspace.refreshExpertDirectory() }
                .confirmationDialog(
                    String(localized: "Use the expert’s latest configuration?"),
                    isPresented: Binding(
                        get: { pendingPickerReintroduction != nil },
                        set: { if !$0 { pendingPickerReintroduction = nil } }
                    )
                ) {
                    Button(String(localized: "Reintroduce expert")) {
                        guard let pending = pendingPickerReintroduction else { return }
                        pendingPickerReintroduction = nil
                        selectExpertFromPicker(pending.agent, owner: pending.owner, allowReintroduction: true)
                    }
                    Button(String(localized: "Cancel"), role: .cancel) { pendingPickerReintroduction = nil }
                } message: {
                    Text(String(localized: "This adds a new expert context with the latest model and template. Previous replies stay in this conversation."))
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
                close: { self.codeReference = nil }
            )
        } else {
            MacChatInspector(
                model: model,
                hostDisplayName: hostSelectorTitle,
                dependencies: inspectorDependencies,
                navigation: inspectorNavigation,
                close: { activeOverlay = nil; showsInspector = false; codeReference = nil },
                showsHeaderNavigation: false,
                participants: inspectorParticipants,
                expertHistory: AnyView(expertHistory),
                canReadLocalFiles: canReadLocalConversationFiles,
                inspectFile: openCodeReference,
                openReview: { openWindow(value: MacWorkspaceReviewRoute(path: $0.path)) }
            )
        }
    }

    @ViewBuilder
    private var expertHistory: some View {
        if let conversation = model.selectedConversation {
            MacExpertChildrenDisclosure(parentID: conversation.id, refreshRevision: conversation.updatedAt,
                roomRevision: roomWorkspace.conversationAssociation?.room.revision,
                load: { parent, offset in try await childPage(parent: parent, offset: offset) },
                open: { child in
                    do { try presentChild(parent: conversation.id, child: child) }
                    catch { roomWorkspace.directoryError = error.localizedDescription }
                })
                .id(conversation.id + ":" + String(describing: ObjectIdentifier(roomWorkspace.service)))
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
            .fill((isSidebarDividerHovered || sidebarDragStartWidth != nil) ? MacDesignColor.controlOutline : MacDesignColor.borderSubtle)
            .frame(width: 1)
            .overlay {
                Rectangle()
                    .fill(Color.clear)
                    .frame(width: 10)
                    .contentShape(Rectangle())
                    .background {
                        MacResizeCursorRectView()
                    }
                    .onHover { hovering in
                        isSidebarDividerHovered = hovering
                    }
                    .onDisappear {
                        isSidebarDividerHovered = false
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
            .accessibilityLabel(String(localized: "Sidebar width"))
            .accessibilityValue("\(Int(sidebarWidth(availableWidth: availableWidth)))")
            .accessibilityAdjustableAction { direction in
                let delta: CGFloat = direction == .increment ? 10 : -10
                storedSidebarWidth = Double(min(
                    MacWorkspaceColumnMetrics.maximumSidebarWidth,
                    max(MacWorkspaceColumnMetrics.minimumSidebarWidth, sidebarWidth(availableWidth: availableWidth) + delta)
                ))
            }
            .onKeyPress(.rightArrow) {
                storedSidebarWidth = Double(min(
                    MacWorkspaceColumnMetrics.maximumSidebarWidth,
                    sidebarWidth(availableWidth: availableWidth) + 10
                ))
                return .handled
            }
            .onKeyPress(.leftArrow) {
                storedSidebarWidth = Double(max(
                    MacWorkspaceColumnMetrics.minimumSidebarWidth,
                    sidebarWidth(availableWidth: availableWidth) - 10
                ))
                return .handled
            }
            .focusable()
            .help(String(localized: "Drag to resize the sidebar"))
    }

    private func inspectorResizeHandle(availableWidth: CGFloat) -> some View {
        Rectangle()
            .fill((isInspectorDividerHovered || inspectorDragStartWidth != nil) ? MacDesignColor.controlOutline : MacDesignColor.borderSubtle)
            .frame(width: 1)
            .overlay {
                Rectangle()
                    .fill(Color.clear)
                    .frame(width: 10)
                    .contentShape(Rectangle())
                    .background {
                        MacResizeCursorRectView()
                    }
                    .onHover { hovering in
                        isInspectorDividerHovered = hovering
                    }
                    .onDisappear {
                        isInspectorDividerHovered = false
                    }
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
            .onExitCommand {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.structure) {
                    self.activeOverlay = nil
                }
            }
        }
    }

    @ViewBuilder
    private var workspaceContent: some View {
        switch selectedSection {
        case .pluginPage:
            if let selectedPlugin {
                MacDesktopPluginPageView(page: selectedPlugin).id(selectedPlugin.id)
            }
        case .chat:
            conversationContent
        case .extensions:
            MacExpertsAndSkillsCenterView(
                roomWorkspace: roomWorkspace,
                remoteHost: remoteHost,
                skillsNavigation: extensionsNavigation,
                activeWorkspaceURL: model.workspaceURL,
                pluginLibrary: pluginLibrary, pluginLibraryError: pluginLibraryError,
                onOpenPluginPage: openPluginPage
            )
            .transition(.opacity)
        case .automations:
            MacAutomationCenterView(navigation: automationNavigation)
                .transition(.opacity)
        case .memory:
            MacDataCenterView(navigation: dataNavigation, automationNavigation: automationNavigation)
                .transition(.opacity)
        }
    }

    private var conversationContent: some View {
            VStack(spacing: 0) {
                if let error = roomWorkspace.admissionError ?? roomWorkspace.directoryError {
                    MacInlineNotice(
                        message: error,
                        kind: .failure,
                        dismiss: roomWorkspace.admissionError == nil ? { roomWorkspace.directoryError = nil } : nil
                    )
                    .padding(.horizontal, 24)
                    .padding(.top, 10)
                    .accessibilityIdentifier("chat.expert-admission-error")
                    if !expertSelection.isEmpty, let owner = model.selectedConversationID {
                        HStack {
                            Button(String(localized: "Keep names as plain text")) {
                                model.setDraft(model.draft, conversationID: owner)
                                roomWorkspace.directoryError = nil
                            }
                            .buttonStyle(MacActionButtonStyle())
                            Spacer()
                        }
                        .padding(.horizontal, 24)
                        .padding(.top, 6)
                    }
                }
                MacConversationCanvas(
                    model: model,
                    composerFocused: $composerFocused,
                    sendToExperts: {
                        do {
                            guard let owner = model.selectedConversationID else { return }
                            let spans = model.draftMentions(owner)
                            guard let wire = AppleDraftMention.segments(spans, in: model.draft),
                                  !model.attachments.isEmpty || wire.segments.contains(where: { $0.agentID == nil && !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else {
                                roomWorkspace.directoryError = String(localized: "Write what you want the expert to do.")
                                return
                            }
                            let epoch = roomWorkspace.mainSourceEpoch?(owner)
                            let source = model.mainExpertDraftSource(owner)
                            guard model.executionHost == .local || (source != nil && epoch != nil) else { throw MacApplicationPortError.serviceUnavailable }
                            let selected = try AppleDraftMention.selected(spans, sourceID: source, epoch: epoch)
                            model.send(selectedExperts: selected, expertSelectionSourceEpoch: epoch)
                        } catch { roomWorkspace.directoryError = String(localized: "The expert selection changed. Remove the mention and select the expert again.") }
                    },
                    expertWorkspace: roomWorkspace,
                    modelSetupRequired: modelSetupRequired,
                    conversationControls: AnyView(composerConversationControls),
                    modelControls: AnyView(composerModelControls),
                    onCommand: { command, argument in
                        let trimmedArgument = argument.trimmingCharacters(in: .whitespacesAndNewlines)
                        switch command {
                        case .experts: showsAgentRooms = true
                        case .resources:
                            inspectorNavigation.selectedTab = .artifacts
                            showsInspector = true
                            activeOverlay = layoutMode == .threeColumn ? nil : .inspector
                        case .permissions:
                            inspectorNavigation.selectedTab = .context
                            showsInspector = true
                            activeOverlay = layoutMode == .threeColumn ? nil : .inspector
                        case .plan, .goal:
                            if !trimmedArgument.isEmpty {
                                inspectorNavigation.goalDrafts.draft(for: model.selectedConversationID ?? "").objective = trimmedArgument
                            }
                            inspectorNavigation.selectedTab = .task
                            showsInspector = true
                            activeOverlay = layoutMode == .threeColumn ? nil : .inspector
                        default: break
                        }
                    }
                )
            }
    }

    private func workspaceHeader(availableWidth: CGFloat, resolvedLayoutMode: MacWorkspaceLayoutMode) -> some View {
        let sidebarOpen = resolvedLayoutMode != .compactOverlay && columnVisibility != .detailOnly
        let inspectorColumnOpen = resolvedLayoutMode == .threeColumn
            && !isManagementContent
            && (showsInspector || forcesInspectorVisible)
        let leadingPadding = max(10, leadingTrafficLightInset)

        return HStack(spacing: 0) {
            if sidebarOpen {
                HStack(spacing: 4) {
                    sidebarToggleButton
                        .accessibilityIdentifier("workspace.toggleSidebar")
                    Spacer(minLength: 4)
                    newConversationButton
                }
                .padding(.leading, leadingPadding)
                .padding(.trailing, 10)
                .frame(width: sidebarWidth(availableWidth: availableWidth) + 1, height: 48)
                .overlay(alignment: .trailing) {
                    Rectangle().fill(MacDesignColor.borderSubtle).frame(width: 1)
                }
            } else {
                HStack(spacing: 4) {
                    sidebarToggleButton
                        .accessibilityIdentifier("workspace.toggleSidebar")
                    newConversationButton
                }
                .padding(.leading, max(12, leadingTrafficLightInset))
                .padding(.trailing, 10)
                Rectangle()
                    .fill(MacDesignColor.borderSubtle)
                    .frame(width: 1, height: 48)
            }

            HStack(spacing: 8) {
                if selectedSection == .chat {
                    conversationTitleControl
                } else {
                    HStack(spacing: 7) {
                        Image(systemName: selectedSection.symbol)
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(.secondary)
                        (selectedPlugin.map { Text(verbatim: $0.title) } ?? Text(selectedSection.title))
                            .font(.system(size: 16, weight: .semibold))
                            .lineLimit(1)
                            .accessibilityIdentifier("workspace.heading." + selectedSection.rawValue)
                    }
                }
                Spacer(minLength: 8)
                if selectedSection == .chat && inspectorColumnOpen {
                    searchConversationButton
                }
            }
            .padding(.leading, 14)
            .padding(.trailing, inspectorColumnOpen ? 10 : 8)
            .frame(maxWidth: .infinity, alignment: .leading)

            if selectedSection == .chat {
                if inspectorColumnOpen {
                    HStack(spacing: 4) {
                        Spacer(minLength: 0)
                        inspectorButtons
                    }
                    .padding(.horizontal, 10)
                    .frame(width: inspectorWidth(availableWidth: availableWidth) + 1, height: 48)
                    .overlay(alignment: .leading) {
                        Rectangle().fill(MacDesignColor.borderSubtle).frame(width: 1)
                    }
                } else {
                    HStack(spacing: 4) {
                        searchConversationButton
                        Rectangle()
                            .fill(MacDesignColor.borderSubtle)
                            .frame(width: 1, height: 18)
                            .padding(.horizontal, 2)
                        inspectorButtons
                    }
                    .padding(.trailing, 10)
                }
            }
        }
        .frame(height: 48)
        .background {
            MacWindowDragRegion()
        }
        .background(MacDesignColor.chrome)
        .overlay(alignment: .bottom) { Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1) }
        .accessibilityElement(children: .contain)
    }

    private var searchConversationButton: some View {
        Button {
            if !model.isTranscriptSearchVisible { composerFocused = false }
            model.isTranscriptSearchVisible.toggle()
        } label: {
            Image("nola-search").renderingMode(.template).resizable().scaledToFit()
                .frame(width: 18, height: 18).frame(width: 36, height: 36)
        }
        .macToolbarIcon(isActive: model.isTranscriptSearchVisible)
        .help(String(localized: "Search Conversation"))
        .accessibilityLabel(String(localized: "Search Conversation"))
        .accessibilityIdentifier("workspace.searchConversation")
    }

    private var newConversationButton: some View {
        Button(action: navigationActions.newConversation) {
            Image("nola-edit").renderingMode(.template).resizable().scaledToFit()
                .frame(width: 18, height: 18).frame(width: 36, height: 36)
        }
        .macToolbarIcon()
        .help(String(localized: "New Conversation"))
        .accessibilityLabel(String(localized: "New Conversation"))
        .accessibilityIdentifier("workspace.newConversation")
    }

    private func openPluginPage(_ page: MacDesktopPluginPage) {
        navigationActions.navigate(to: .pluginPage, ifStillValid: { pluginPages.contains(where: { $0.id == page.id }) }) {
            selectedPlugin = page
            activeOverlay = nil
        }
    }

    private var workspaceSidebar: some View {
        MacConversationSidebar(model: model, selectedSection: sectionSelection,
            selectConversation: { id in
                navigationActions.selectConversation(id)
                activeOverlay = nil
            }, loadParticipantNames: { id in
                let service = roomWorkspace.service
                return try await Task.detached {
                    try service.forConversation(id)?.room.room.participants
                        .filter(\.active).map(\.displayName) ?? []
                }.value
            }, pluginPages: pluginPages, selectedPluginID: selectedPlugin?.id, openPluginPage: openPluginPage, onOpenSettings: {
                openSettings()
            })
    }

    private var inspectorButtons: some View {
        MacConversationInspectorButtons(selection: inspectorNavigation.selectedTab,
            isVisible: !isManagementContent && (layoutMode == .threeColumn ? showsInspector : activeOverlay == .inspector)) { tab in
                    if isManagementContent {
                        navigationActions.navigate(to: .chat, then: {
                            inspectorNavigation.selectedTab = tab
                            showsInspector = true
                            activeOverlay = layoutMode == .threeColumn ? nil : .inspector
                        })
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
            isActive: layoutMode == .compactOverlay && activeOverlay == .sidebar
        )
        .help(String(localized: "Toggle Sidebar"))
        .accessibilityLabel(String(localized: "Toggle Sidebar"))
    }

    @ViewBuilder
    private var conversationTitleControl: some View {
        let canRename = model.selectedConversation?.isPersisted == true
        if isEditingTitle, canRename {
            HStack(spacing: 6) {
                TextField(String(localized: "Conversation name"), text: $inlineTitleDraft)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14, weight: .semibold))
                    .multilineTextAlignment(.leading)
                    .focused($titleFocused)
                    .onSubmit { commitInlineTitle() }
                    .onExitCommand { cancelInlineTitle() }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .frame(maxWidth: 460, alignment: .leading)
            .background(
                MacDesignColor.panel,
                in: RoundedRectangle(cornerRadius: 7, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .strokeBorder(MacDesignColor.controlOutline, lineWidth: 1)
            }
        } else if canRename {
            Button(action: beginInlineTitle) {
                HStack(spacing: 6) {
                    Text(model.selectedConversationDisplayTitle)
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .minimumScaleFactor(0.85)
                    if isTitleHovered {
                        Image(systemName: "pencil")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(
                    isTitleHovered ? MacDesignColor.hover : .clear,
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                )
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { isTitleHovered = $0 }
            .help(model.selectedConversationDisplayTitle)
        } else {
            Text(model.selectedConversationDisplayTitle)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.middle)
                .padding(.horizontal, 4)
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
                        String(localized: "Temporary Workspace"),
                        systemImage: model.usesSessionSandbox
                            ? "checkmark"
                            : "folder.badge.questionmark"
                    )
                }
                .disabled(model.isSelectedConversationVisiblyRunning)
                Button {
                    Task { await model.chooseWorkspace() }
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
                .disabled(model.isSelectedConversationVisiblyRunning)
            }
        } label: {
            Label {
                Text(model.usesSessionSandbox ? String(localized: "Choose Folder") : model.workspaceDisplayName)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 168, alignment: .leading)
            } icon: {
                conversationMenuIcon("nola-folder")
            }
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(.secondary)
        }
        .menuStyle(MacPillMenuStyle(height: 28))
        .fixedSize(horizontal: true, vertical: false)
        .help(model.effectiveWorkspaceURL.path)
        .accessibilityLabel(
            "\(String(localized: "Workspace")): \(model.workspaceDisplayName)"
        )
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
                Text(verbatim: "@")
                    .font(.system(size: 12, weight: .semibold, design: .rounded))
                Text("chat.experts.title")
                    .lineLimit(1)

            }
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .frame(height: 28)
            .macComposerPill(isActive: showsAgentRooms)
        }
        .buttonStyle(.plain)
        .disabled(model.selectedConversationID == nil)
        .accessibilityIdentifier("chat.experts")
    }

    private var composerConversationControls: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                modelSelector
                workspaceSelector
                expertManagementButton
            }.fixedSize(horizontal: true, vertical: false)
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) { modelSelector; workspaceSelector }
                expertManagementButton
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
                VStack(alignment: .leading, spacing: model.selectedMessages.isEmpty ? 18 : 10) {
                    VStack(alignment: .leading, spacing: 7) {
                        Text(String(localized: "Choose a model to start chatting", table: "Chat"))
                            .font(.system(size: model.selectedMessages.isEmpty ? 28 : 15, weight: .semibold))
                            .foregroundStyle(Color("NolaTextPrimary"))
                            .accessibilityAddTraits(.isHeader)
                        Text(String(localized: "Connect an API account or use a model on this Mac. Your draft stays here.", table: "Chat"))
                            .font(.system(size: model.selectedMessages.isEmpty ? 15 : 13))
                            .foregroundStyle(Color("NolaTextSecondary"))
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    HStack(spacing: 10) {
                        Button(action: manageChatModels) {
                            Label(String(localized: "Configure Model", table: "Chat"), systemImage: "square.3.layers.3d")
                        }
                        .buttonStyle(.nolaProminent)
                        .tint(MacDesignColor.primaryAction)
                        .foregroundStyle(MacDesignColor.onPrimaryAction)
                        .controlSize(.large)
                        .accessibilityIdentifier("chat.configureModel")
                        if !model.availableModelOptions.isEmpty { modelSelector }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, model.selectedMessages.isEmpty ? 18 : 8)
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
                MacModelCenterNavigationState.shared.showSelectedModel(
                    remoteProfileID: option.isLocal ? nil : option.configurationID
                )
            } else {
                MacModelCenterNavigationState.shared.showRemoteModels()
            }
            openWindow(id: "model-center")
        }
    }

    private var usesNolaModelBackend: Bool {
        MacExecutionBackendPolicy.requiresNolaModel(model.executionBackend.id)
    }

    private var modelSelectorTitle: String {
        let baseTitle: String
        if usesNolaModelBackend {
            baseTitle = model.activeModelName
        } else {
            let selectedExternal = model.selectedExternalModel(in: activeExecutionBackendDescriptor)?.displayName
                ?? String(localized: "Default Model")
            baseTitle = "\(selectedExecutionBackendDisplayName) · \(selectedExternal)"
        }
        guard model.executionHost == .remote else { return baseTitle }
        return "\(hostSelectorTitle) · \(baseTitle)"
    }

    private var showsHostInModelMenu: Bool {
        model.canUseRemoteHost || model.executionHost == .remote || remoteHostConnectionState.isConfigured
    }

    private var modelSelector: some View {
        let externalBackends = activeExecutionBackendCatalog.filter { $0.id != "nola" }
        return Menu {
            Button {
                if !usesNolaModelBackend {
                    model.selectExecutionBackend(
                        MacExecutionBackendOption(id: "nola", displayName: "Nola"),
                        available: true
                    )
                }
                model.restoreDefaultModelInheritance()
            } label: {
                Label(String(localized: "Use default chat model", table: "ModelCenter"),
                      systemImage: usesNolaModelBackend && model.inheritsDefaultModel ? "checkmark" : "arrow.uturn.backward")
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
            if !externalBackends.isEmpty || model.executionHost == .local {
                Divider()
                Section(String(localized: "External Agents")) {
                    ForEach(externalBackends) { descriptor in
                        let option = MacExecutionBackendOption(
                            id: descriptor.id,
                            displayName: descriptor.displayName
                        )
                        let available = descriptor.isAvailable
                        Button {
                            model.selectExecutionBackend(option, available: available)
                            if let initialModel = descriptor.models.first(where: {
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
                    if model.executionHost == .local {
                        Button {
                            Task { await refreshLocalExecutionBackends() }
                        } label: {
                            Label(
                                isDiscoveringLocalExecutionBackends
                                    ? String(localized: "Discovering Local Execution Units…", table: "Chat")
                                    : String(localized: "Refresh External Agents"),
                                systemImage: "arrow.clockwise"
                            )
                        }
                        .disabled(isDiscoveringLocalExecutionBackends)
                        .accessibilityIdentifier("chat.discoverExecutionBackends")
                        if let error = localExecutionBackendDiscoveryError {
                            Text(error)
                        }
                    }
                }
                if !usesNolaModelBackend,
                   let descriptor = activeExecutionBackendDescriptor,
                   !descriptor.models.isEmpty {
                    let selected = model.selectedExternalModel(in: descriptor)
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
                }
            }
            if showsHostInModelMenu {
                Divider()
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
                    Label(hostConnectionStatusTitle, systemImage: hostConnectionStatusSymbol)
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
                    Button {
                        openSettings.openHostConnections()
                    } label: {
                        Label(
                            remoteHostConnectionState.isConfigured
                                ? macHostText("mac.host.action.edit", "Edit Host Connection…")
                                : macHostText("mac.host.action.configure", "Set Up Host…"),
                            systemImage: "gearshape"
                        )
                    }
                }
            }
            Divider()
            Button(String(localized: "Conversation model parameters", table: "ModelCenter")) {
                model.isModelSettingsPresented = true
            }
            .disabled(!usesNolaModelBackend || model.selectedModelOption == nil)
            if model.executionHost == .remote && !showsHostInModelMenu {
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
            Label {
                Text(modelSelectorTitle)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 220, alignment: .leading)
            } icon: {
                Image(systemName: model.executionHost == .remote ? "network" : "cpu")
            }
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(model.executionHost == .remote ? hostSelectorColor : .primary)
        }
        .menuStyle(MacPillMenuStyle(height: 28))
        .fixedSize(horizontal: true, vertical: false)
        .layoutPriority(1)
        .help({
            if usesNolaModelBackend,
               let status = model.selectedModelCapabilityStatus,
               let label = status.label {
                return "\(modelSelectorTitle) · \(label)"
            }
            return modelSelectorTitle
        }())
        .accessibilityIdentifier("chat.modelSelector")
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

    private func modelOptionButton(_ option: MacChatModelOption) -> some View {
        Button {
            if !usesNolaModelBackend {
                model.selectExecutionBackend(
                    MacExecutionBackendOption(id: "nola", displayName: "Nola"),
                    available: true
                )
            }
            model.selectModel(option)
        } label: {
            Label(
                option.displayName,
                systemImage: usesNolaModelBackend && model.selectedModelOption?.id == option.id ? "checkmark" : option.symbol
            )
        }
    }

    private func beginInlineTitle() {
        guard let conversation = model.selectedConversation, conversation.isPersisted else { return }
        editingTitleConversationID = conversation.id
        inlineTitleDraft = MacConversationTitlePolicy.displayTitle(
            conversation.title,
            conversationID: conversation.id
        )
        isEditingTitle = true
        Task { @MainActor in
            await Task.yield()
            titleFocused = true
        }
    }

    private func commitInlineTitle() {
        let targetID = editingTitleConversationID
        let draft = inlineTitleDraft
        isEditingTitle = false
        editingTitleConversationID = nil
        titleFocused = false
        isTitleHovered = false
        guard let targetID else { return }
        model.renameConversation(id: targetID, to: draft)
    }

    private func cancelInlineTitle() {
        isEditingTitle = false
        editingTitleConversationID = nil
        titleFocused = false
        isTitleHovered = false
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
                let opening = activeOverlay != .inspector
                activeOverlay = opening ? .inspector : nil
                showsInspector = opening
                if !opening {
                    codeReference = nil
                }
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
        Task {
            guard let workspaceURL = await model.workspaceForReview() else { return }
            openWindow(value: MacWorkspaceReviewRoute(path: workspaceURL.path))
        }
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
