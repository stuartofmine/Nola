// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatView logic in the iOS UI layer.
// - Interfaces:
//   - ChatView
// - APIs:
//   - none
//
import SwiftUI

import UniformTypeIdentifiers
import Combine

/// Unsaved drafts have distinct identities even when both conversation IDs are nil.
nonisolated enum IOSExpertMentionCompletionPolicy {
    static func accepts(owner: UUID?, draftOwner: ConversationComposerDraftStore.Owner,
                        currentOwner: UUID?, currentDraftOwner: ConversationComposerDraftStore.Owner) -> Bool {
        owner == currentOwner && draftOwner == currentDraftOwner
    }
}

nonisolated private struct IOSExpertMentionRejoinRequest: Sendable {
    let agent: IOSPersistentAgent
    let owner: UUID
    let draftOwner: ConversationComposerDraftStore.Owner
    let text: String
    let query: ExpertMentionQuery
}

private struct IOSExpertRejoinAlert: ViewModifier {
    @Binding var request: IOSExpertMentionRejoinRequest?
    let confirm: (IOSExpertMentionRejoinRequest) -> Void

    func body(content: Content) -> some View {
        content.alert(Text("experts.mention.rejoin_title"), isPresented: Binding(
            get: { request != nil }, set: { if !$0 { request = nil } }
        ), presenting: request) { captured in
            Button("experts.mention.rejoin_action") { confirm(captured) }
                .accessibilityIdentifier("experts.mention.rejoin.confirm")
            Button(L10n.cancel, role: .cancel) { request = nil }
        } message: { _ in Text("experts.mention.rejoin_message") }
    }
}

/// One visible error occurrence; dismissal never changes execution state.
struct ChatErrorPresentation {
    private(set) var occurrenceID: UUID?
    private(set) var conversationID: UUID?
    private(set) var message: String?

    mutating func observe(_ state: ChatProcessingState, conversationID: UUID?) {
        self.conversationID = conversationID
        if case .error(let message) = state {
            occurrenceID = UUID()
            self.message = message
        } else {
            occurrenceID = nil
            message = nil
        }
    }

    @discardableResult
    mutating func dismiss(occurrenceID: UUID, conversationID: UUID?) -> Bool {
        guard self.occurrenceID == occurrenceID, self.conversationID == conversationID else { return false }
        self.occurrenceID = nil
        message = nil
        return true
    }
}

/// Submission owns its optimistic turn until the store publishes those IDs.
/// A selection notification is not acknowledgement of persistence.
enum ChatSubmissionPresentationPolicy {
    static func merging(persisted: [ChatMessage], submitted: [ChatMessage]) -> [ChatMessage] {
        let persistedIDs = Set(persisted.map(\.id))
        return persisted + submitted.filter { !persistedIDs.contains($0.id) }
    }

    static func activeAssistant(submitted: UUID?, persisted: [ChatMessage]) -> UUID? {
        submitted ?? persisted.last(where: \.isCanonicalLive)?.id
    }

    static func pendingSubmission(
        _ submitted: [ChatMessage]?, persisted: [ChatMessage], conversationExists: Bool = true
    ) -> [ChatMessage]? {
        guard conversationExists, let submitted else { return nil }
        let persistedIDs = Set(persisted.map(\.id))
        return submitted.allSatisfy { persistedIDs.contains($0.id) } ? nil : submitted
    }
}

extension Publisher {
    /// Only presentation snapshots are batched; canonical events stay lossless.
    /// Fixed windows avoid starving the UI during a continuous stream.
    func coalescingChatFrames() -> AnyPublisher<Output, Failure> {
        collect(.byTime(DispatchQueue.main, .milliseconds(16)))
            .compactMap { $0.last }
            .eraseToAnyPublisher()
    }
}

/// A failed refresh cannot revoke an already confirmed transcript or borrow another owner's history.
struct ChatConversationLoadRecovery {
    struct Failure {
        let conversationID: UUID
        let message: String
    }
    private(set) var confirmedConversationID: UUID?
    private(set) var failure: Failure?

    mutating func didLoad(_ conversationID: UUID) {
        confirmedConversationID = conversationID
        failure = nil
    }

    mutating func retire() {
        confirmedConversationID = nil
        failure = nil
    }

    mutating func didFail(_ conversationID: UUID, message: String,
                         existing: [ChatMessage], submitted: [ChatMessage]) -> [ChatMessage] {
        failure = Failure(conversationID: conversationID, message: message)
        guard confirmedConversationID == conversationID else {
            confirmedConversationID = nil
            return submitted
        }
        return existing
    }
}

// MARK: - Chat View

nonisolated enum ChatHostConnectionPhase: Equatable {
    case notApplicable
    case unconfigured
    case checking
    case connected
    case unavailable
}

struct ChatView: View {
    @State private var composerReasoningEfforts: [String] = []
    @State private var composerReasoningPreference = "automatic"
    @State private var composerReasoningError: String?

    var readingPositions: ChatReadingPositions? = nil
    var expertHistoryContinuation: IOSExpertHistoryContinuation? = nil
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var messages: [ChatMessage] = []
    @State private var taskResultDestination: AppleTaskChildDestination?
    @State private var conversationLoadRecovery = ChatConversationLoadRecovery()
    @State private var composerDrafts = ConversationComposerDraftStore(repository: FileConversationDraftRepository.application())
    private var inputText: String {
        get { composerDrafts.current.text }
        nonmutating set { composerDrafts.current.text = newValue }
    }
    @State private var selectedModel: ModelInfo?
    @State private var highlightedCommandID: String?
    @State private var dismissedCommand: String?
    @State private var commandError: String?
    @State private var showNoModelWarning: Bool = false
    @State private var showExpertPicker = false
    @State private var expertSourceOwner: AppleConversationSourceOwner = .unavailable
    @State private var showExpertSourceNotice = false
    @State private var showLocalExpertManagement = false
    @State private var expertSelection: Set<String> = []
    @State private var expertSelectionRevisions: [String: Int64] = [:]
    @State private var expertRemoteSource: ChatManager.ExpertRoomSource?
    @State private var expertRemoteRoom: IOSConversationRoomAssociation?
    @State private var expertSelectionReferences: [String: AppleMainExpertSelectionEntry] = [:]
    @State private var expertSelectionRemoteSources: [String: ChatManager.ExpertRoomSource] = [:]
    @State private var expertJoinOperations: [String: UUID] = [:]
    private var joiningExpertIDs: Set<String> { Set(expertJoinOperations.keys) }
    @State private var pendingExpertRejoin: IOSExpertMentionRejoinRequest?
    @State private var highlightedExpertID: String?
    @State private var dismissedExpertMention: String?
    @State private var expertDirectory: [IOSPersistentAgent] = []
    @State private var expertDirectoryError: String?
    @State private var expertSubmission: IOSRoomSubmissionWorkspace?
    @State private var expertSendError: String?
    @State private var expertMessageTimes: [UUID: Int64] = [:]
    @StateObject private var agentRoomWorkspace = IOSAgentRoomWorkspace()
    @State private var showIMSearch = false
    @State private var imSearch = ""
    @State private var showContextSheet: Bool = false
    @State private var showSkillsSheet: Bool = false
    @State private var showMoreSettingsSheet: Bool = false
    @State private var conversationInspectorPane: IOSConversationInspectorPane?
    private struct WorkToolsPresentation: Identifiable {
        let id = UUID()
        let owner: UUID?
    }
    @State private var workToolsPresentation: WorkToolsPresentation?
    @State private var queuedWorkDestination: (owner: UUID, destination: IOSConversationWorkDestination)?
    private struct ParameterEditorOwner {
        let conversationID: UUID
        let model: ModelInfo
        var baseline: IOSConversationGenerationOverride
    }
    @State private var parameterEditorOwner: ParameterEditorOwner?
    private var pendingAttachments: [ChatAttachmentDraft] {
        get { composerDrafts.current.attachments }
        nonmutating set { composerDrafts.current.attachments = newValue }
    }
    @State private var modelConfigurationMessage: String?

    private var attachmentImportMessage: String? {
        get { composerDrafts.current.notice }
        nonmutating set { composerDrafts.current.notice = newValue }
    }
    private var isImportingAttachments: Bool { composerDrafts.current.importID != nil }
    @State private var attachmentImportTasks: [UUID: Task<Void, Never>] = [:]
    @State private var hybridDeviceToolsEnabled: Bool = false
    @State private var hybridFilesEnabled: Bool = false
    @State private var hybridClipboardEnabled: Bool = false
    @State private var hybridLocationEnabled: Bool = false
    @State private var hybridCalendarEnabled: Bool = false
    @State private var hybridCapabilitySettingsLoaded: Bool = false
    @State private var autoCompactTriggerPercent: Int = ChatManager.autoCompactTriggerPercentDefault
    @State private var conversationResponseStyle: PromptResponseStyle?
    private var selectedResponseStyle: PromptResponseStyle {
        if let conversationResponseStyle { return conversationResponseStyle }
        if container.chatManager.currentConversationId == nil,
           let parameters = composerDrafts.current.configuration?.parameters { return parameters.responseStyle }
        guard let model = selectedModel else { return .casual }
        return (try? container.chatManager.generationParameters(for: model,
            conversationID: container.chatManager.currentConversationId))?.responseStyle ?? .casual
    }

    @State private var processingState: ChatProcessingState = .idle
    @State private var errorPresentation = ChatErrorPresentation()
    @State private var stateObservers = Set<AnyCancellable>()
    @State private var convObserver: AnyCancellable?
    @State private var responseAssistantIDs: [UUID: UUID] = [:]
    @State private var submittedTurns: [UUID: [ChatMessage]] = [:]
    @State private var activeResponseConversationID: UUID? = nil
    @State private var activeAssistantMessageID: UUID? = nil
    @State private var hostPermissionSubmissionError: String? = nil
    @State private var submittingHostPermissionRequestID: String? = nil
    @State private var remoteRunNeedsResume = false
    @State private var remoteSubmissionPending = false
    @State private var remoteRunObservationError: String?
    @State private var pendingHostPermissionRequest: HostAgentPermissionRequest? = nil
    @State private var pendingHostPermissionConversationID: UUID? = nil
    @State private var pendingLocalAgentPermissionRequest: IOSAgentPermissionRequest? = nil
    @State private var pendingLocalScopedResourceRequest: IOSAgentPermissionRequest? = nil
    @State private var presentedLocalScopedResourceRequest: IOSAgentPermissionRequest? = nil
    @State private var presentedLocalScopedResourceOwner: UUID?
    @State private var showLocalAgentFileImporter: Bool = false
    @State private var modelConfigurationNotice: String? = nil
    @State private var hostConnectionPhase: ChatHostConnectionPhase = .notApplicable
    @State private var historyBeforeOrdinal: Int64?
    @State private var hasMoreHistory = false

    @State private var enableRetrieval: Bool = false
    @State private var permissionMode: IOSPermissionMode = .cautious
    @State private var permissionApplicationStatus = "saved"
    @State private var permissionChangeID: UUID?
    @State private var expandedErrorIdentity: String?
    @State private var copiedErrorIdentity: String?
    @State private var currentConversationTitle: String = L10n.chatConversationDefaultTitle
    @State private var currentSessionMemory: String = ""
    @State private var currentKnowledgePoints: String = ""
    @State private var currentConversationWiki: String = ""
    @State private var currentLongTermMemory: String = ""
    @State private var memoryEditorBaseline: ChatMemoryProfile?
    @State private var currentUserPreferences: String = ""
    @State private var currentMCPServers: [UserMCPServer] = []
    @State private var currentUploadedSkills: [UserUploadedContextItem] = []
    @State private var currentHostUploadedSkills: [UserUploadedContextItem] = []
    @State private var currentHostUploadedDocuments: [UserUploadedContextItem] = []
    @State private var currentHostMCPServers: [UserMCPServer] = []
    @State private var currentHostComputerUseStatus: WebAgentProvider.HostComputerUseStatus?
    @State private var currentHostNativeBrowserStatus: WebAgentProvider.HostNativeBrowserStatus?
    @State private var currentHostExecutionBackends: [WebAgentProvider.HostAgentSettings.ExecutionBackend] = []
    @State private var currentExecutionBackend: String = "nola"
    @State private var hostSkillIDsByName: [String: UUID] = [:]
    @State private var hostDocumentIDsByName: [String: UUID] = [:]
    @State private var hostMCPServerIDsByIdentity: [String: UUID] = [:]
    @State private var currentUploadedDocuments: [UserUploadedContextItem] = []
    @State private var currentPromptTemplateID: UUID? = nil
    @State private var currentDataSourceIDs: Set<UUID> = []
    @State private var currentSystemDataSourceTypes: Set<DataSourceType> = []
    @State private var currentContextUsage: ConversationContextUsage?
    @State private var hostSkillsFetchTask: Task<Void, Never>?
    @State private var hostSkillsInFlightKey: String?
    @State private var hostSkillsRequestID: UUID?
    @State private var hostSkillsLastSuccessKey: String?
    @State private var hostSkillsLastSuccessAt: Date?
    @State private var hostSkillsLastFailureKey: String?
    @State private var hostSkillsLastFailureAt: Date?
    @FocusState private var composerFocused: Bool
    private let hostSkillsSuccessCooldownSeconds: TimeInterval = 60
    private let hostSkillsFailureBackoffSeconds: TimeInterval = 30

    private func tr(_ key: String, _ value: String, comment: String) -> String {
        NSLocalizedString(key, value: value, comment: comment)
    }

    @State private var availableModels: [ModelInfo] = []

    private var isEditable: Bool {
        !processingState.blocksInput
    }

    private func loadHybridCapabilitySettings() {
        hybridCapabilitySettingsLoaded = false
        do {
            let settings = try IOSHybridCapabilitySettings.load()
            applyHybridCapabilitySettings(settings)
            DispatchQueue.main.async {
                hybridCapabilitySettingsLoaded = true
            }
        } catch {
            attachmentImportMessage = error.localizedDescription
        }
    }

    private func applyHybridCapabilitySettings(_ settings: IOSHybridCapabilitySettings) {
        hybridDeviceToolsEnabled = settings.deviceToolsEnabled
        hybridFilesEnabled = settings.filesEnabled
        hybridClipboardEnabled = settings.clipboardEnabled
        hybridLocationEnabled = settings.locationEnabled
        hybridCalendarEnabled = settings.calendarEnabled
    }

    private func saveHybridCapabilitySettings() {
        guard hybridCapabilitySettingsLoaded else { return }
        let boundHostURL = selectedModel?.baseURL
            .map(ModelSelectionManager.normalizedWebHostURL) ?? ""
        do {
            try IOSHybridCapabilitySettings(
                boundHostURL: boundHostURL,
                deviceToolsEnabled: hybridDeviceToolsEnabled,
                filesEnabled: hybridFilesEnabled,
                clipboardEnabled: hybridClipboardEnabled,
                locationEnabled: hybridLocationEnabled,
                calendarEnabled: hybridCalendarEnabled
            ).save()
        } catch {
            hybridCapabilitySettingsLoaded = false
            attachmentImportMessage = error.localizedDescription
            if let persisted = try? IOSHybridCapabilitySettings.load() {
                applyHybridCapabilitySettings(persisted)
                DispatchQueue.main.async {
                    hybridCapabilitySettingsLoaded = true
                }
            }
        }
    }

    private var hybridSurfaceActive: Bool {
        hybridDeviceToolsEnabled && (
            hybridFilesEnabled || hybridClipboardEnabled || hybridLocationEnabled || hybridCalendarEnabled
        )
    }

    private var currentModelTint: Color {
        if selectedModel?.isLocal == true {
            return DesignTokens.successColor
        }
        if selectedModel?.isWebBackend == true {
            switch hostConnectionPhase {
            case .connected: return DesignTokens.primaryColor
            case .checking, .unconfigured: return DesignTokens.warningColor
            case .unavailable: return Color(uiColor: .label)
            case .notApplicable: return .secondary
            }
        }
        return DesignTokens.primaryColor
    }

    private var effectiveModelRecoveryNotice: String? {
        if let modelConfigurationNotice,
           !modelConfigurationNotice.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return modelConfigurationNotice
        }
        guard selectedModel?.isWebBackend == true, hostConnectionPhase == .unavailable else { return nil }
        return tr(
            "chat.host.connection.unavailable",
            "无法连接宿主机。请确认 Mac 版 Nola 正在运行且两台设备位于同一网络。",
            comment: "Host unavailable recovery notice"
        )
    }

    private var displayedPermissionRequest: HostAgentPermissionRequest? {
        if let pendingHostPermissionRequest,
           HostPermissionPresentationPolicy.shouldPresent(
               pendingConversationID: pendingHostPermissionConversationID,
               currentConversationID: container.chatManager.currentConversationId
           ) {
            return pendingHostPermissionRequest
        }
        guard let request = pendingLocalAgentPermissionRequest else { return nil }
        return HostAgentPermissionRequest(
            id: request.presentationID,
            allowConversation: request.allowConversation,
            toolName: request.toolName,
            argsPreview: request.scope ?? "",
            match: request.scope ?? "*",
            reason: request.presentationRationale,
            message: request.presentationRationale,
            originKind: request.originKind,
            permissionInstructions: request.permissionInstructions
        )
    }

    private var availablePromptTemplates: [PromptTemplate] {
        guard let service = container.promptTemplates else { return [] }
        let currentLang = Locale.current.language.languageCode?.identifier ?? "en"
        return service.templates.filter { tmpl in
            let lang = tmpl.language ?? ""
            return lang.isEmpty || lang.hasPrefix(currentLang)
        }
    }

    private var availableContextDataSources: [URLDataSource] {
        (container.urlFetcher?.sources ?? [])
            .sorted { lhs, rhs in
                if lhs.enabled != rhs.enabled {
                    return lhs.enabled && !rhs.enabled
                }
                return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
            }
    }

    private var currentDataSourceOverrides: [String: Bool] {
        var overrides = Dictionary(uniqueKeysWithValues: availableContextDataSources.map {
            ($0.id.uuidString, currentDataSourceIDs.contains($0.id))
        })
        let policyStore = SystemDataSourceAccessPolicyStore.shared
        for type in policyStore.configurableTypes {
            overrides[policyStore.conversationOverrideKey(for: type)] = currentSystemDataSourceTypes.contains(type)
        }
        return overrides
    }

    private var currentPromptTemplateName: String? {
        guard let templateID = currentPromptTemplateID else { return nil }
        return container.promptTemplates?.templates.first(where: { $0.id == templateID })?.name
    }

    private var currentModelSourceLabel: String {
        if selectedModel?.isLocal == true {
            return NSLocalizedString("chat.model.local", value: "本地模型", comment: "Chat header local model badge")
        }
        if selectedModel?.isWebBackend == true {
            return NSLocalizedString("chat.model.host", value: "宿主机", comment: "Chat header host model badge")
        }
        return NSLocalizedString("chat.model.cloud", value: "云端模型", comment: "Chat header cloud model badge")
    }

    private var currentConversationSubtitle: String {
        IOSConversationTitlePresentationPolicy.displayTitle(
            currentConversationTitle,
            conversationID: container.chatManager.currentConversationId?.uuidString,
            fallback: L10n.chatConversationDefaultTitle
        )
    }

    private var currentModelConnectionLabel: String {
        guard let model = selectedModel else {
            return NSLocalizedString("chat.model.unconfigured", value: "未配置模型", comment: "Chat model unconfigured badge")
        }
        if model.isLocal {
            return model.name
        }
        if model.isWebBackend {
            switch hostConnectionPhase {
            case .checking:
                return NSLocalizedString("chat.model.host_checking", value: "正在连接宿主机", comment: "Checking host badge")
            case .unavailable:
                return NSLocalizedString("chat.model.host_unavailable", value: "宿主机不可用", comment: "Unavailable host badge")
            case .unconfigured:
                return NSLocalizedString("chat.model.host_unconfigured", value: "宿主机未连接", comment: "Unconfigured host badge")
            case .connected, .notApplicable:
                break
            }
            return model.name
        }
        return model.name
    }


    private var skillsChipDetailLabel: String {
        let allSkills = mergedSkillDisplayList(local: currentUploadedSkills, host: currentHostUploadedSkills)
        let enabledCount = allSkills.filter { $0.enabled }.count
        let totalCount = allSkills.count
        return totalCount > 0 ? "\(enabledCount)/\(totalCount)" : "0"
    }

    private var skillsChipTint: Color {
        let allSkills = mergedSkillDisplayList(local: currentUploadedSkills, host: currentHostUploadedSkills)
        guard !allSkills.isEmpty else { return .secondary }
        return allSkills.contains(where: { $0.enabled }) ? DesignTokens.primaryColor : .secondary
    }

    private var shouldShowStatusBar: Bool {
        activeAssistantMessageID == nil
            && (processingState.showsInlineStatus || container.chatManager.retrievalInProgress)
    }

    private var conversationParticipantSummary: String {
        let names = expertDirectory.filter { expertSelection.contains($0.id) }.map(\.name)
        return names.isEmpty ? "Nola" : names.joined(separator: " · ")
    }

    private var conversationSettingsSheet: some View {
        NavigationStack {
            ChatMoreSettingsSheet(
                selectedModel: parameterEditorOwner?.model ?? selectedModel ?? container.chatManager.getCurrentModel(),
                contextUsage: currentContextUsage,
                hostUploadedSkills: mergedSkillDisplayList(local: [], host: currentHostUploadedSkills),
                hostUploadedDocuments: currentHostUploadedDocuments,
                hostMCPServers: currentHostMCPServers,
                hostComputerUseStatus: currentHostComputerUseStatus,
                hostNativeBrowserStatus: currentHostNativeBrowserStatus,
                pairedHostIdentity: selectedModel
                    .flatMap { $0.baseURL }
                    .flatMap { URL(string: $0) }
                    .flatMap { ModelSelectionManager.pairedWebHostIdentity(for: $0) },
                conversationGenerationOverride: parameterEditorOwner?.baseline ?? .init(),
                onSaveGenerationOverride: { value in
                    guard let owner = parameterEditorOwner,
                          let store = container.conversationStore as? any ConversationGenerationOverrideStore else {
                        throw IOSApplicationRuntimeError.rejected("Select a conversation before editing parameters")
                    }
                    try store.setGenerationOverride(conversationId: owner.conversationID,
                        modelKey: IOSConversationGenerationOverride.modelKey(for: owner.model),
                        override: value, expectedBaseline: owner.baseline)
                    parameterEditorOwner?.baseline = try store.generationOverride(conversationId: owner.conversationID,
                        modelKey: IOSConversationGenerationOverride.modelKey(for: owner.model))
                },
                hybridDeviceToolsEnabled: $hybridDeviceToolsEnabled,
                hybridFilesEnabled: $hybridFilesEnabled,
                hybridClipboardEnabled: $hybridClipboardEnabled,
                hybridLocationEnabled: $hybridLocationEnabled,
                hybridCalendarEnabled: $hybridCalendarEnabled,
                enableRetrieval: $enableRetrieval,
                permissionMode: Binding(
                    get: { permissionMode },
                    set: { mode in
                        guard permissionChangeID == nil else { return }
                        let owner = container.chatManager.currentConversationId
                        let changeID = UUID()
                        permissionChangeID = changeID
                        permissionMode = mode
                        permissionApplicationStatus = "saving"
                        Task { @MainActor in
                            defer { if permissionChangeID == changeID { permissionChangeID = nil } }
                            do {
                                guard IOSPermissionChangeFence.matches(owner: owner, changeID: changeID,
                                    selectedOwner: container.chatManager.currentConversationId, currentChangeID: permissionChangeID) else { return }
                                try await container.chatManager.setPermissionModeForCurrentConversation(mode)
                                guard IOSPermissionChangeFence.matches(owner: owner, changeID: changeID,
                                    selectedOwner: container.chatManager.currentConversationId, currentChangeID: permissionChangeID) else { return }
                                permissionApplicationStatus = container.chatManager.permissionConfigurationStatus(conversationID: owner)
                            } catch {
                                guard IOSPermissionChangeFence.matches(owner: owner, changeID: changeID,
                                    selectedOwner: container.chatManager.currentConversationId, currentChangeID: permissionChangeID) else { return }
                                permissionApplicationStatus = "failed"
                                attachmentImportMessage = error.localizedDescription
                            }
                        }
                    }
                ),
                autoCompactTriggerPercent: Binding(
                    get: { autoCompactTriggerPercent },
                    set: { value in
                        var preferences = IOSChatPreferences()
                        preferences.autoCompactTriggerPercent = value
                        do {
                            try preferences.save()
                            autoCompactTriggerPercent = value
                        } catch {
                            attachmentImportMessage = error.localizedDescription
                        }
                    }
                ),
                responseStyle: Binding(
                    get: { conversationResponseStyle },
                    set: { setConversationResponseStyle($0) }
                ),
                sessionMemory: $currentSessionMemory,
                longTermMemory: $currentLongTermMemory,
                userPreferences: $currentUserPreferences,
                uploadedSkills: $currentUploadedSkills,
                onSaveMemoryProfile: {
                    guard let expected = memoryEditorBaseline else {
                        throw IOSApplicationRuntimeError.rejected("Reload the memory profile before saving")
                    }
                    let saved = try await container.chatManager.updateContextMemoryText(
                        sessionMemory: currentSessionMemory,
                        longTermMemory: currentLongTermMemory,
                        userPreferences: currentUserPreferences,
                        expected: expected
                    )
                    memoryEditorBaseline = saved
                    currentLongTermMemory = saved.longTermMemory
                    currentUserPreferences = saved.userPreferences
                },
                onSetSkillEnabled: { id, enabled in
                    guard let store = container.chatMemoryProfileStore as? any ChatSkillEditingStore else {
                        throw IOSApplicationRuntimeError.rejected("Skill editor unavailable")
                    }
                    let expected = try store.loadSkillEditor(id: id)
                    let saved = try store.setSkillEnabled(enabled, expected: expected)
                    if let index = currentUploadedSkills.firstIndex(where: { $0.id == id }),
                       let item = saved.record.uploadedContextItem {
                        currentUploadedSkills[index] = item
                    }
                },
                onOpenContextSettings: {
                    showContextSheet = true
                },
                onOpenSkillLibrary: {
                    showSkillsSheet = true
                },
                onForgetHostPairing: {
                    guard let model = selectedModel,
                          let rawURL = model.baseURL,
                          let url = URL(string: rawURL) else { return }
                    ModelSelectionManager.forgetWebHostIdentity(for: url)
                    hybridDeviceToolsEnabled = false
                    hybridFilesEnabled = false
                    hybridClipboardEnabled = false
                    hybridLocationEnabled = false
                    hybridCalendarEnabled = false
                    hostSkillsLastSuccessKey = nil
                    hostSkillsLastSuccessAt = nil
                    hostSkillsLastFailureKey = nil
                    hostSkillsLastFailureAt = nil
                    selectChatModel(model)
                    refreshHostSkillsIfNeeded(for: model)
                },
                promptTemplates: availablePromptTemplates,
                selectedPromptTemplateID: currentPromptTemplateID,
                onSelectPromptTemplate: { id in
                    _ = try container.chatManager.updateConversationContext(
                        promptTemplateID: id, dataSourceOverrides: currentDataSourceOverrides,
                        for: container.chatManager.currentConversationId)
                    currentPromptTemplateID = id
                }
            )
            .environmentObject(container)
        }
    }

    private func prepareChatPresentation() {
        do {
            availableModels = try container.applicationRuntime.listModelProfiles()
                .compactMap(ModelSelectionManager.modelInfo(from:))
        } catch {
            availableModels = []
            container.logger.log(.error, "ChatView", "Failed to refresh configured model catalog")
        }
        refreshExpertSourceOwner()
        composerDrafts.select(container.chatManager.currentConversationId)
        loadHybridCapabilitySettings()
        do {
            autoCompactTriggerPercent = try IOSChatPreferences.load().autoCompactTriggerPercent
        } catch {
            attachmentImportMessage = error.localizedDescription
        }
        if container.chatManager.currentConversationId == nil { _ = restoreUnsentConfiguration() }
        else { selectedModel = container.chatManager.getCurrentModel() }

        if let model = selectedModel {
            container.chatManager.selectModel(model, persistAsDefault: false)
        }

        container.chatManager.$processingState
            .map { state in (conversationID: container.chatManager.currentConversationId, state: state) }
            .receive(on: DispatchQueue.main)
            .sink { update in
                guard update.conversationID == container.chatManager.currentConversationId,
                      update.state == container.chatManager.processingState else { return }
                processingState = update.state
                errorPresentation.observe(update.state, conversationID: update.conversationID)
            }
            .store(in: &stateObservers)

        container.chatManager.$agentCoreSnapshot
            .receive(on: DispatchQueue.main)
            .sink { _ in
                guard permissionChangeID == nil else { return }
                permissionApplicationStatus = container.chatManager.permissionConfigurationStatus(
                    conversationID: container.chatManager.currentConversationId)
            }
            .store(in: &stateObservers)
        container.chatManager.$remotePermissionConfigurations
            .receive(on: DispatchQueue.main)
            .sink { _ in
                guard permissionChangeID == nil else { return }
                permissionApplicationStatus = container.chatManager.permissionConfigurationStatus(
                    conversationID: container.chatManager.currentConversationId)
            }
            .store(in: &stateObservers)

        container.chatManager.$permissionConfigurationOwner
            .receive(on: DispatchQueue.main)
            .sink { _ in
                guard permissionChangeID == nil else { return }
                permissionApplicationStatus = container.chatManager.permissionConfigurationStatus(
                    conversationID: container.chatManager.currentConversationId)
            }
            .store(in: &stateObservers)

        container.chatManager.$committedAssistantProjection
            .receive(on: DispatchQueue.main)
            .sink { snapshot in
                applyCommittedAssistantProjection(snapshot)
            }
            .store(in: &stateObservers)

        container.chatManager.$committedJournalPresentation
            .receive(on: DispatchQueue.main)
            .sink { update in
                guard let update, container.chatManager.ownsCommittedJournalPresentation(update),
                      let conversationID = container.chatManager.currentConversationId else { return }
                ChatMessageDraftReducer.applyJournalPresentation(update, in: &messages,
                    conversationID: conversationID.uuidString)
            }
            .store(in: &stateObservers)

        container.chatManager.$agentTaskProjection
            .receive(on: DispatchQueue.main)
            .sink { projection in
                guard let conversationID = container.chatManager.currentConversationId,
                      projection == container.chatManager.agentTaskProjection else { return }
                ChatMessageDraftReducer.applyTaskProjection(projection, in: &messages,
                    conversationID: conversationID.uuidString)
            }
            .store(in: &stateObservers)

        container.chatManager.$committedAssistantJournalContent
            .receive(on: DispatchQueue.main)
            .sink { content in
                guard let content, let conversationID = container.chatManager.currentConversationId else { return }
                ChatMessageDraftReducer.applyJournalContent(content, in: &messages,
                    conversationID: conversationID.uuidString)
                ChatMessageDraftReducer.applyTaskProjection(container.chatManager.agentTaskProjection,
                    in: &messages, conversationID: conversationID.uuidString)
            }
            .store(in: &stateObservers)

        container.chatManager.$selectedModelInfo
            .receive(on: DispatchQueue.main)
            .sink { modelInfo in
                if container.chatManager.currentConversationId != nil, let m = modelInfo {
                    selectedModel = m
                }
                refreshHostSkillsIfNeeded(for: modelInfo)
            }
            .store(in: &stateObservers)

        container.chatManager.$modelFallbackNotice
            .receive(on: DispatchQueue.main)
            .removeDuplicates()
            .sink { notice in
                withAnimation(reduceMotion ? nil : .easeOut(duration: 0.20)) {
                    modelConfigurationNotice = notice
                }
            }
            .store(in: &stateObservers)

        container.chatManager.$pendingHostPermissionRequest
            .receive(on: DispatchQueue.main)
            .sink { request in
                pendingHostPermissionRequest = request
            }
            .store(in: &stateObservers)

        container.chatManager.$submittingHostPermissionRequestID
            .receive(on: DispatchQueue.main)
            .sink { submittingHostPermissionRequestID = $0 }
            .store(in: &stateObservers)

        container.chatManager.$hostPermissionSubmissionError
            .receive(on: DispatchQueue.main)
            .sink { hostPermissionSubmissionError = $0 }
            .store(in: &stateObservers)

        container.chatManager.$remoteRunNeedsResume
            .receive(on: DispatchQueue.main)
            .sink { remoteRunNeedsResume = $0 }
            .store(in: &stateObservers)
        container.chatManager.$remoteSubmissionPending
            .receive(on: RunLoop.main)
            .sink { remoteSubmissionPending = $0 }
            .store(in: &stateObservers)
        container.chatManager.$remoteRunObservationError
            .receive(on: DispatchQueue.main)
            .sink { remoteRunObservationError = $0 }
            .store(in: &stateObservers)

        container.chatManager.$pendingHostPermissionConversationId
            .receive(on: DispatchQueue.main)
            .sink { conversationID in
                pendingHostPermissionConversationID = conversationID
            }
            .store(in: &stateObservers)

        container.chatManager.$localAgentPermissionRequest
            .receive(on: DispatchQueue.main)
            .sink { request in
                guard container.chatManager.localAgentPermissionRequest == request else { return }
                pendingLocalAgentPermissionRequest = request
                // A question can arrive for a newly opened canonical
                // assistant Message before its first journal delta.
                // Read Core's current window so the inline card has
                // the actual live owner, including queued UI actions.
                if request != nil {
                    loadMessages(for: container.chatManager.currentConversationId)
                }
            }
            .store(in: &stateObservers)

        container.chatManager.$localAgentScopedResourceRequest
            .receive(on: DispatchQueue.main)
            .sink { request in
                guard container.chatManager.localAgentScopedResourceRequest == request else { return }
                pendingLocalScopedResourceRequest = request
                if let request, presentedLocalScopedResourceRequest == nil {
                    presentedLocalScopedResourceRequest = request
                    presentedLocalScopedResourceOwner = container.chatManager.currentConversationId
                    showLocalAgentFileImporter = true
                }
            }
            .store(in: &stateObservers)

        enableRetrieval = false
        container.chatManager.enableRetrieval = false

        convObserver = container.chatManager.$currentConversationId
            .removeDuplicates()
            .receive(on: DispatchQueue.main)
            .sink { convId in
                guard convId == container.chatManager.currentConversationId else { return }
                composerDrafts.select(convId)
                activeResponseConversationID = convId.flatMap { responseAssistantIDs[$0] == nil ? nil : $0 }
                activeAssistantMessageID = convId.flatMap { responseAssistantIDs[$0] }
                loadMessages(for: convId)
                loadConversationState(for: convId)
            }

    }

    private func resolveScopedResourceImport(_ result: Result<URL, Error>) {
        let url = try? result.get()
        if let request = presentedLocalScopedResourceRequest {
            presentedLocalScopedResourceRequest = nil
            if let owner = presentedLocalScopedResourceOwner {
                container.chatManager.executionForConversation(owner)?
                    .resolveLocalAgentScopedResource(url: url, request: request)
            }
            presentedLocalScopedResourceOwner = nil
            if let pending = pendingLocalScopedResourceRequest, pending != request {
                presentedLocalScopedResourceRequest = pending
                presentedLocalScopedResourceOwner = container.chatManager.currentConversationId
                showLocalAgentFileImporter = true
            }
        }
    }

    private func receiveTaskResultDestination(_ notification: Notification) {
        guard let destination = notification.object as? AppleTaskChildDestination,
              UUID(uuidString: destination.parentConversationID) == container.chatManager.currentConversationId else { return }
        taskResultDestination = destination
    }

    private func recordDraftPersistenceFailure(_ notification: Notification) {
                if let repository = notification.userInfo?["repository"] as? UUID,
                   repository == composerDrafts.repositoryIdentity,
                   let owner = notification.userInfo?["owner"] as? ConversationComposerDraftStore.Owner,
                   let detail = notification.userInfo?["detail"] as? String {
                    composerDrafts.recordPersistenceFailure(owner: owner, detail: detail)
                }
    }

    private var compactConversationHeader: some View {
        HStack(spacing: 0) {
            Button { dismiss() } label: {
                Image("nola-back").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 23, height: 23)
                    .frame(width: 44, height: 44)
            }
            .accessibilityLabel(Text(L10n.chatTitle))
            .accessibilityIdentifier("chat.navigation.back")
            Button(action: openWorkTools) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(verbatim: currentConversationSubtitle).font(.body.weight(.semibold)).lineLimit(1)
                    Text(verbatim: conversationParticipantSummary).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
            }.accessibilityIdentifier("chat.conversation.details")
            Button { showIMSearch = true } label: {
                Image("nola-search").renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 23, height: 23).frame(width: 44, height: 44)
            }.accessibilityLabel(Text("chat.search.loaded.title"))

        }
        .buttonStyle(.plain).padding(.horizontal, 4).padding(.vertical, 8)
        .background(DesignTokens.backgroundColor)
        .overlay(alignment: .bottom) { NolaDivider() }
    }

    var body: some View {
        Group {
            chatMainContent(showsHistoryToggle: false)
            .onReceive(NotificationCenter.default.publisher(for: .nolaModelConfigured)) { notification in
                guard let model = notification.object as? ModelInfo,
                      notification.userInfo?["owner"] as? String == (container.chatManager.currentConversationId?.uuidString ?? "unsent") else { return }
                do { availableModels = try ModelSelectionManager.availableModels(using: container.applicationRuntime) }
                catch { attachmentImportMessage = error.localizedDescription; return }
                selectChatModel(model)
            }
            .background { if horizontalSizeClass == .compact { ChatNavigationGestureBridge().frame(width: 0, height: 0) } }
            .navigationTitle(horizontalSizeClass == .compact ? "" : currentConversationSubtitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(horizontalSizeClass == .compact ? .hidden : .visible, for: .navigationBar)
            .safeAreaInset(edge: .top, spacing: 0) {
                if horizontalSizeClass == .compact { compactConversationHeader }
            }
            .task(id: container.chatManager.currentConversationId) {
                guard let id = container.chatManager.currentConversationId else { return }
                prepareExpertWorkspace()
                await expertSubmission?.restoreConversation(id.uuidString)
                guard !Task.isCancelled else { return }
                expertSelection = Set(expertSubmission?.pendingExpertIDs ?? [])
                applyExpertHistoryContinuation()
                if inputText.isEmpty, let text = expertSubmission?.pendingAdmissionText { inputText = text }
                if let association = expertSubmission?.conversationAssociation {
                    do {
                        try agentRoomWorkspace.open(association.room, container: container)
                        let stored = try container.conversationStore?.getMessages(conversationId: id, limit: 1000) ?? []
                        expertMessageTimes = Dictionary(uniqueKeysWithValues: stored.map {
                            ($0.id, Int64($0.createdAt.timeIntervalSince1970 * 1000))
                        })
                    } catch { expertSendError = error.localizedDescription }
                }
            }
            .onChange(of: messages.map(\.id)) { _, _ in
                guard let id = container.chatManager.currentConversationId,
                      let stored = try? container.conversationStore?.getMessages(conversationId: id, limit: 1000) else { return }
                expertMessageTimes = Dictionary(uniqueKeysWithValues: stored.map {
                    ($0.id, Int64($0.createdAt.timeIntervalSince1970 * 1000))
                })
            }
            .onChange(of: container.chatManager.currentConversationId) { oldOwner, newOwner in
                let transfersSubmittedDraft = oldOwner == nil && newOwner == activeResponseConversationID
                    && newOwner.flatMap { submittedTurns[$0] } != nil
                if transfersSubmittedDraft, let owner = newOwner {
                    if let source = expertRemoteSource { expertRemoteSource = .init(owner: owner, provider: source.provider, epoch: source.epoch) }
                    expertSelectionRemoteSources = expertSelectionRemoteSources.mapValues { .init(owner: owner, provider: $0.provider, epoch: $0.epoch) }
                } else {
                    expertSelection = []; expertSelectionRevisions = [:]
                    expertRemoteSource = nil; expertRemoteRoom = nil; expertSelectionReferences = [:]; expertSelectionRemoteSources = [:]
                }
                expertJoinOperations = [:]
                pendingExpertRejoin = nil
                queuedWorkDestination = nil; workToolsPresentation = nil
                showExpertPicker = false
                refreshExpertSourceOwner()
            }
            .onChange(of: processingState.blocksInput) { _, _ in refreshExpertSourceOwner() }
            .alert(Text("chat.experts.sourceUnavailable.title"), isPresented: $showExpertSourceNotice) {
                Button("experts.manage") { showLocalExpertManagement = true }
                Button("Cancel", role: .cancel) {}
            } message: { Text("chat.experts.sourceUnavailable.detail") }
            .modifier(IOSExpertRejoinAlert(request: $pendingExpertRejoin, confirm: confirmExpertRejoin))
            .sheet(isPresented: $showLocalExpertManagement) { IOSAgentRoomsView() }
            .sheet(isPresented: $showExpertPicker) { expertPickerSheet }
            .onAppear(perform: prepareChatPresentation)
            .navigationDestination(item: $taskResultDestination) { destination in
                IOSTaskChildResultView(destination: destination)
            }
            .onReceive(NotificationCenter.default.publisher(for: .chatTaskResultRequested), perform: receiveTaskResultDestination)
            .onReceive(NotificationCenter.default.publisher(for: FileConversationDraftRepository.writeFailed),
                       perform: recordDraftPersistenceFailure)
            .onDisappear {
                composerFocused = false
                composerDrafts.cancelImports()
                for task in attachmentImportTasks.values { task.cancel() }
                attachmentImportTasks.removeAll()
                stateObservers.removeAll()
                convObserver?.cancel()
                hostSkillsFetchTask?.cancel()
                hostSkillsFetchTask = nil
                hostSkillsInFlightKey = nil
                hostSkillsRequestID = nil
            }
            .onChange(of: hybridDeviceToolsEnabled) { _, _ in saveHybridCapabilitySettings() }
            .onChange(of: hybridFilesEnabled) { _, _ in saveHybridCapabilitySettings() }
            .onChange(of: hybridClipboardEnabled) { _, _ in saveHybridCapabilitySettings() }
            .onChange(of: hybridLocationEnabled) { _, _ in saveHybridCapabilitySettings() }
            .onChange(of: hybridCalendarEnabled) { _, _ in saveHybridCapabilitySettings() }
            .sheet(item: $conversationInspectorPane) { pane in
                IOSConversationInspector(pane: pane,
                    conversationID: container.chatManager.currentConversationId?.uuidString,
                    loadProgress: { try await container.chatManager.loadConversationProgress(conversationID: $0) },
                    submitGoalControl: { try await container.chatManager.controlConversationGoal(conversationID: $0, control: $1) },
                    documents: currentUploadedDocuments + currentHostUploadedDocuments,
                    processingState: container.chatManager.processingState)
            }
            .sheet(item: $workToolsPresentation, onDismiss: openQueuedWorkDestination) { presentation in
                IOSConversationWorkTools(conversationTitle: currentConversationTitle,
                    modelLabel: currentModelConnectionLabel, hasOwner: presentation.owner != nil) { destination in
                    if let owner = presentation.owner { queuedWorkDestination = (owner, destination) }
                    workToolsPresentation = nil
                }
            }
            .sheet(isPresented: $showIMSearch) {
                NavigationStack {
                    List {
                        Text("chat.search.loaded.scope").font(.caption).foregroundStyle(.secondary)
                        ForEach((messages + agentRoomWorkspace.rows.map(\.message)).filter { !imSearch.isEmpty && $0.content.localizedStandardContains(imSearch) }) { message in
                            Text(verbatim: message.content).font(.body).textSelection(.enabled)
                        }
                    }
                    .searchable(text: $imSearch, prompt: Text("chat.search.loaded.title"))
                    .navigationTitle(Text("chat.search.loaded.title"))
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .cancellationAction) { Button(L10n.close) { showIMSearch = false } } }
                }
            }
            .sheet(isPresented: $showContextSheet) {
                NavigationStack {
                    ChatContextSheet(
                        displayMode: .context,
                        conversationID: container.chatManager.currentConversationId,
                        conversationTitle: currentConversationTitle,
                        contextUsage: currentContextUsage,
                        sessionMemory: $currentSessionMemory,
                        knowledgePoints: $currentKnowledgePoints,
                        conversationWiki: $currentConversationWiki,
                        longTermMemory: $currentLongTermMemory,
                        userPreferences: $currentUserPreferences,
                        mcpServers: $currentMCPServers,
                        uploadedSkills: $currentUploadedSkills,
                        hostUploadedSkills: mergedSkillDisplayList(local: [], host: currentHostUploadedSkills),
                        hostUploadedDocuments: currentHostUploadedDocuments,
                        hostMCPServers: currentHostMCPServers,
                        uploadedDocuments: $currentUploadedDocuments,
                        dataSources: availableContextDataSources,
                        selectedDataSourceIDs: $currentDataSourceIDs,
                        selectedSystemDataSourceTypes: $currentSystemDataSourceTypes
                    ) {
                        _ = try container.chatManager.updateConversationContext(
                            promptTemplateID: currentPromptTemplateID,
                            dataSourceOverrides: currentDataSourceOverrides,
                            for: container.chatManager.currentConversationId
                        )
                    } onSaveMemoryProfile: {
                        guard let expected = memoryEditorBaseline else {
                            throw IOSApplicationRuntimeError.rejected("Reload the memory profile before saving")
                        }
                        let saved = try await container.chatManager.updateContextMemoryText(
                            sessionMemory: currentSessionMemory,
                            longTermMemory: currentLongTermMemory,
                            userPreferences: currentUserPreferences,
                            expected: expected
                        )
                        memoryEditorBaseline = saved
                        currentLongTermMemory = saved.longTermMemory
                        currentUserPreferences = saved.userPreferences
                    } onSaveConversationKnowledge: { snapshot in
                        let conversationId = try container.chatManager.updateConversationKnowledge(
                            knowledgePoints: snapshot.knowledgePoints,
                            wiki: snapshot.wiki,
                            for: snapshot.conversationID
                        )
                        if conversationId == container.chatManager.currentConversationId {
                            loadConversationState(for: conversationId)
                        }
                    } onCompact: {
                        let result = try await container.chatManager.compactConversationContext()
                        guard container.chatManager.currentConversationId == result.conversationID else { return result.notice }
                        loadMessages(for: result.conversationID)
                        loadConversationState(for: result.conversationID)
                        return result.notice
                    } onReload: {
                        guard let conversationID = container.chatManager.currentConversationId else {
                            loadConversationState(for: nil)
                            return nil
                        }
                        guard let service = container.llmClient as? ModelService else {
                            loadConversationState(for: conversationID)
                            return tr("chat.context.auxiliary.refresh_unavailable", "已加载本地状态；当前模型适配器不能刷新原辅助来源。", comment: "Local state loaded; the current model adapter cannot refresh original auxiliary sources.")
                        }
                        do {
                            let report = try await service.refreshConversationAuxiliarySources(conversationID: conversationID)
                            guard container.chatManager.currentConversationId == conversationID else { return nil }
                            loadConversationState(for: conversationID)
                            let format = tr("chat.context.auxiliary.refresh_report", "辅助来源：已刷新 %d，仍未确认 %d，其他 Host 跳过 %d，刷新失败 %d。", comment: "Auxiliary sources: refreshed %d, still unconfirmed %d, other hosts skipped %d, failed %d.")
                            let result = String(format: format, report.refreshed, report.pending, report.skipped, report.failed)
                            return report.hasMore
                                ? result + " " + tr("chat.context.auxiliary.refresh_more", "还有其他来源；再次重载继续下一批。", comment: "More sources remain; reload again for the next batch.")
                                : result
                        } catch {
                            guard container.chatManager.currentConversationId == conversationID else { return nil }
                            loadConversationState(for: conversationID)
                            throw error
                        }
                    } onDiscard: {
                        loadConversationState(for: container.chatManager.currentConversationId)
                    } onUseSkill: { invocation in
                        inputText = inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? invocation
                            : "\(inputText.trimmingCharacters(in: .whitespacesAndNewlines))\n\(invocation)"
                        showContextSheet = false
                        composerFocused = true
                    } onSearchMemory: { query in
                        try await container.chatManager.searchConversationRawMemory(query, limit: 12)
                    }
                    .environmentObject(container)
                }
            }
            .sheet(isPresented: $showSkillsSheet, onDismiss: {
                loadConversationState(for: container.chatManager.currentConversationId)
            }) {
                NavigationStack {
                    PersonalCapabilitiesLibraryView(hostSkills: currentHostUploadedSkills) { invocation in
                        inputText = inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? invocation
                            : "\(inputText.trimmingCharacters(in: .whitespacesAndNewlines))\n\(invocation)"
                        showSkillsSheet = false
                        composerFocused = true
                    }
                    .environmentObject(container)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button(L10n.close) { showSkillsSheet = false }
                        }
                    }
                }
            }
            .sheet(isPresented: $showMoreSettingsSheet) { conversationSettingsSheet }
            .alert(L10n.modelNoAvailableTitle, isPresented: $showNoModelWarning) {
                Button(NSLocalizedString("model.setup.title", value: "Configure model", comment: "Configure model")) {
                    openModelTab()
                }
                Button(L10n.cancel, role: .cancel) { }
            } message: {
                Text(NSLocalizedString("model.setup.missing", value: "Configure a service, a local model, or a host to start chatting.", comment: "No configured model"))
            }
            .fileImporter(
                isPresented: $showLocalAgentFileImporter,
                allowedContentTypes: [.item],
                onCompletion: resolveScopedResourceImport
            )
        }
    }

    private func chatMainContent(showsHistoryToggle: Bool) -> some View {
        let execution = container.chatManager.currentConversationId.flatMap {
            container.chatManager.executionForConversation($0)
        }
        let localPermission = execution?.localAgentPermissionRequest
        let hostPermission = execution?.pendingHostPermissionRequest
        let permissionDecision = execution?.permissionDecisionHandler(
            localRequest: localPermission, hostRequest: hostPermission)
        return VStack(spacing: 0) {
            if horizontalSizeClass != .compact {
                chatTopBar(showsHistoryToggle: showsHistoryToggle)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let expertSendError { Text(expertSendError).font(.callout).foregroundStyle(.red).padding(8) }
            if let submission = expertSubmission,
               submission.conversationAssociation?.conversationID == container.chatManager.currentConversationId?.uuidString {
                IOSUnifiedExpertConversation(workspace: agentRoomWorkspace, submission: submission,
                    ordinaryMessages: messages, ordinaryMessageTimes: expertMessageTimes, conversationID: container.chatManager.currentConversationId)
                    .environmentObject(container)
            } else if let failure = conversationLoadRecovery.failure,
                      failure.conversationID == container.chatManager.currentConversationId,
                      messages.isEmpty {
                Spacer(minLength: 0)
            } else {
            ChatMessageTimeline(
                conversationID: container.chatManager.currentConversationId,
                messages: messages,
                activeAssistantMessageID: activeAssistantMessageID,
                retrievalInProgress: container.chatManager.retrievalInProgress,
                hasModels: !availableModels.isEmpty,
                selectedModel: selectedModel,
                hybridDeviceToolsEnabled: hybridDeviceToolsEnabled,
                hostConnectionPhase: hostConnectionPhase,
                pendingHostPermissionRequest: displayedPermissionRequest,
                onLoadCompactionSource: { request in
                    await loadCompactionSource(request)
                },
                onSuggestion: { suggestion in
                    inputText = suggestion
                    composerFocused = true
                },
                onPrimaryAction: {
                    if selectedModel?.isWebBackend == true, hostConnectionPhase == .unavailable {
                        refreshHostSkillsIfNeeded(for: selectedModel, force: true)
                    } else {
                        openModelTab()
                    }
                },
                onHostPermissionDecision: { requestID, decision in
                    permissionDecision?(requestID, decision)
                },
                readingPositions: readingPositions,
                ownerIsRunning: processingState.blocksInput,
                onEditAndResend: { input in
                    let sourceOwner = UUID(uuidString: input.sourceConversationID)
                    let target = try await container.chatManager.sendEditedMessage(input)
                    guard container.chatManager.currentConversationId == sourceOwner else { return }
                    if target != sourceOwner { NotificationCenter.default.post(name: .nolaOpenChatTab, object: target) }
                }
            )
            .environment(\.submittingHostPermissionRequestID, submittingHostPermissionRequestID)
            .environmentObject(container)
            }

            Group {
            if let failure = conversationLoadRecovery.failure,
               failure.conversationID == container.chatManager.currentConversationId {
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(NSLocalizedString("chat.history.load_failed", comment: "Conversation refresh failed"))
                            .font(.callout)
                        Spacer()
                        Button(NSLocalizedString("chat.history.retry", comment: "Retry reading conversation")) {
                            loadMessages(for: failure.conversationID)
                        }.frame(minHeight: 44)
                    }
                    DisclosureGroup(NSLocalizedString("chat.history.error_details", comment: "Conversation read error details")) {
                        Text(failure.message).font(.caption).textSelection(.enabled)
                    }
                }
                .padding(.horizontal, 16)
                .accessibilityIdentifier("chat.history.load_error")
            }
            if let message = hostPermissionSubmissionError, pendingHostPermissionRequest != nil {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("chat.permission.submission_error")
            }

            if remoteSubmissionPending {
                Button(NSLocalizedString("chat.host.cancel_submission", value: "Cancel pending submission", comment: "Cancel without sending a new request")) {
                    container.chatManager.cancelPendingRemoteSubmission()
                }
            }
            if remoteRunNeedsResume {
                Button(NSLocalizedString("chat.host.resume_existing_run", value: "Resume existing task", comment: "Resume the exact interrupted Host Run")) {
                    container.chatManager.resumeObservedRemoteRun()
                }
                .accessibilityIdentifier("chat.host.resume_existing_run")
            } else if let message = remoteRunObservationError {
                VStack(alignment: .leading, spacing: 4) {
                    Text(message).font(.caption).foregroundStyle(.secondary)
                    if container.chatManager.canCancelRejectedUIQueue {
                        Button("移除未执行的表单操作") { container.chatManager.cancelRejectedUIQueue() }
                            .accessibilityIdentifier("chat.ui.cancelRejectedQueue")
                    }
                    Button(NSLocalizedString("chat.host.retry_observation", value: "Reconnect to task", comment: "Read the existing Host Run without new input")) {
                        container.chatManager.retryRemoteRunObservation()
                    }
                }
                .accessibilityIdentifier("chat.host.retry_observation")
            }

            if let message = errorPresentation.message,
               let occurrenceID = errorPresentation.occurrenceID,
               errorPresentation.conversationID == container.chatManager.currentConversationId {
                errorRecoveryBanner(message, occurrenceID: occurrenceID)
            }

            UnknownEffectRecoverySurface(manager: container.chatManager)

            if !messages.isEmpty,
               let notice = effectiveModelRecoveryNotice,
               !notice.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                modelConfigurationBanner(notice)
                    .transition(.opacity.combined(with: .move(edge: .bottom)))
            }

            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .safeAreaInset(edge: .bottom, spacing: 0) { chatInputDock }
        .background(chatBackground)
    }

    private var chatBackground: some View {
        DesignTokens.chatBackgroundColor
    }

    private func chatTopBar(showsHistoryToggle: Bool, presentation: ChatTopBar.Presentation = .full) -> ChatTopBar {
        ChatTopBar(
            presentation: presentation,
            showsHistoryToggle: showsHistoryToggle,
            showsConversationTitle: false,
            roomTitle: nil,
            onShowRooms: openExpertPicker,
            conversationSubtitle: currentConversationSubtitle,
            modelConnectionLabel: currentModelConnectionLabel,
            modelSourceLabel: currentModelSourceLabel,
            modelTint: currentModelTint,
            selectedModel: selectedModel,
            hybridDeviceToolsEnabled: hybridSurfaceActive,
            availableModels: availableModels,
            executionBackends: selectedModel?.isWebBackend == true ? currentHostExecutionBackends : [],
            selectedExecutionBackendID: currentExecutionBackend,
            executionBackendSelectionEnabled: hostConnectionPhase == .connected,
            onToggleHistory: {},
            onShowConversationSettings: {
                openConversationSettings()
            },
            onShowModelPicker: {
                openModelTab()
            },
            onSelectModel: { model in
                selectChatModel(model)
            },
            onSelectExecutionBackend: { backendID in
                do {
                    try container.chatManager.selectExecutionBackendForCurrentConversation(
                        backendID,
                        availableBackendIDs: Set(
                            currentHostExecutionBackends
                                .filter { $0.availabilityState == .available }
                                .map(\.id)
                        )
                    )
                    currentExecutionBackend = backendID
                } catch {
                    container.logger.log(.error, "ChatView", "Failed to save execution unit", metadata: ["error": error.localizedDescription])
                }
            },
            onShowProgress: { conversationInspectorPane = .progress },
            onShowResources: { conversationInspectorPane = .resources },
            onShowWorkTools: openWorkTools,
            reasoningEfforts: composerReasoningEfforts,
            reasoningPreference: composerReasoningPreference,
            reasoningSelectionEnabled: !processingState.blocksInput,
            onSelectReasoning: selectComposerReasoning
        )
    }

    private func openWorkTools() {
        composerFocused = false
        queuedWorkDestination = nil
        workToolsPresentation = .init(owner: container.chatManager.currentConversationId)
    }

    private func openQueuedWorkDestination() {
        let request = queuedWorkDestination
        queuedWorkDestination = nil
        guard let request, request.owner == container.chatManager.currentConversationId else { return }
        switch request.destination {
        case .progress: conversationInspectorPane = .progress
        case .goals: conversationInspectorPane = .goals
        case .resources: conversationInspectorPane = .resources
        case .context: showContextSheet = true
        case .experts: openExpertPicker()
        case .execution: openConversationSettings()
        }
    }

    private func modelConfigurationBanner(_ message: String) -> some View {
        HStack(alignment: .center, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.caption.weight(.semibold))
                .foregroundStyle(DesignTokens.warningColor)
            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 6)
            Button {
                openModelTab()
            } label: {
                Text(tr("chat.model.configuration.action", "配置", comment: "Open model configuration action"))
                    .font(.caption.weight(.semibold))
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
            .accessibilityIdentifier("chat.model.configuration.action")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(DesignTokens.warningColor.opacity(0.07), in: RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .stroke(DesignTokens.warningColor.opacity(0.20), lineWidth: 1)
        )
        .padding(.horizontal, DesignTokens.spacing_lg)
        .padding(.top, DesignTokens.spacing_xs)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.model.configuration.notice")
    }

    private func openConversationSettings() {
        if let id = container.chatManager.currentConversationId, let model = container.chatManager.getCurrentModel() {
            parameterEditorOwner = .init(conversationID: id, model: model,
                baseline: container.chatManager.generationOverride(for: model, conversationID: id))
        } else { parameterEditorOwner = nil }
        showMoreSettingsSheet = true
    }

    private var chatInputControlBar: some View {
        HStack {
            Button {
                openConversationSettings()
            } label: {
                Label(permissionMode.title, systemImage: "checkmark.shield")
                    .font(.footnote)
                    .multilineTextAlignment(.leading)
                    .frame(minHeight: 44)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(tr("chat.more_settings.permission_mode", "Permission mode", comment: "Permission mode picker title"))
            .accessibilityValue(permissionMode.title)
            .accessibilityIdentifier("chat.controls.permission")
            Text(permissionApplicationStatusText)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("chat.controls.permission.status")
            Spacer(minLength: 0)
        }
        .padding(.horizontal, DesignTokens.spacing_lg)
    }

    private var chatInputDock: some View {
        VStack(spacing: 0) {
            Group {
            chatComposer
                .frame(maxWidth: 780)
            if ["saving", "pending", "failed"].contains(permissionApplicationStatus) {
                chatInputControlBar.frame(maxWidth: 760)
            }
            }
        }
        .frame(maxWidth: .infinity)
        .fixedSize(horizontal: false, vertical: true)
        .background(DesignTokens.backgroundColor)
        .overlay(NolaDivider(), alignment: .top)
    }

    private var permissionApplicationStatusText: String {
        switch permissionApplicationStatus {
        case "saving": tr("chat.permission_state.saving", "Switching permissions…", comment: "Permission configuration save in progress")
        case "checking": tr("chat.permission_state.checking", "Checking task permissions…", comment: "Run owner or policy has not been confirmed")
        case "active": tr("chat.permission_state.active", "Active for the running task", comment: "Confirmed by the exact Run policy receipt")
        case "pending": processingState.blocksInput
            ? tr("chat.permission_state.pending", "Saved; waiting for the running task", comment: "Saved configuration is not yet acknowledged by the Run")
            : tr("chat.permission_state.next_message", "Saved; applies to the next message", comment: "Saved permissions when no task is running")
        case "failed": tr("chat.permission_state.failed", "Save not confirmed. Your selection is retained for retry.", comment: "Permission configuration failed")
        default: tr("chat.permission_state.saved", "Permissions saved", comment: "Committed conversation permissions")
        }
    }

    private func errorRecoveryBanner(_ message: String, occurrenceID: UUID) -> some View {
        let identity = occurrenceID.uuidString
        let expanded = expandedErrorIdentity == identity
        return VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "exclamationmark.circle.fill")
                    .foregroundStyle(DesignTokens.dangerColor)
                Text(message)
                    .font(.footnote)
                    .foregroundStyle(.primary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .lineLimit(3)
                    .textSelection(.enabled)
                Button {
                    dismissChatError(occurrenceID)
                } label: {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 44, height: 44)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(tr("chat.error.dismiss", "Dismiss", comment: "Dismiss chat error"))
            }
            if expanded {
                ScrollView {
                    Text(message)
                        .font(.footnote)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                }
                .frame(maxHeight: 200)
                .accessibilityIdentifier("chat.error.details")
            }
            VStack(alignment: .leading, spacing: 8) {
                Button {
                    expandedErrorIdentity = expanded ? nil : identity
                } label: {
                    Label(tr("history.detail.diagnostics", "Technical details", comment: "Technical details"), systemImage: expanded ? "chevron.up" : "chevron.down")
                }
                Button {
                    UIPasteboard.general.string = message
                    copiedErrorIdentity = identity
                } label: {
                    Label(copiedErrorIdentity == identity
                        ? tr("chat.message.copied", "Copied", comment: "Copied")
                        : tr("common.copy", "Copy", comment: "Copy"),
                        systemImage: copiedErrorIdentity == identity ? "checkmark" : "doc.on.doc")
                }
            }
            .font(.footnote)
            .buttonStyle(.bordered)
            VStack(alignment: .leading, spacing: 8) {
                Button {
                    restoreLastPromptForEditing(occurrenceID)
                } label: {
                    Label(tr("chat.error.edit_retry", "Edit and try again", comment: "Edit failed prompt action"), systemImage: "pencil")
                }
                .buttonStyle(.borderedProminent)
                Button {
                    guard dismissChatError(occurrenceID) else { return }
                    openModelTab()
                } label: {
                    Label(tr("chat.error.switch_model", "Switch model", comment: "Switch model after error action"), systemImage: "arrow.triangle.2.circlepath")
                }
                .buttonStyle(.bordered)
            }
            .font(.caption.weight(.semibold))
        }
        .padding(12)
        .background(DesignTokens.dangerColor.opacity(0.07), in: RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.dangerColor.opacity(0.22), lineWidth: 1)
        )
        .padding(.horizontal, DesignTokens.spacing_lg)
        .padding(.top, DesignTokens.spacing_xs)
        .accessibilityElement(children: .contain)
    }

    private func restoreLastPromptForEditing(_ occurrenceID: UUID) {
        guard dismissChatError(occurrenceID) else { return }
        if let lastPrompt = messages.last(where: { $0.role == .user })?.content,
           !lastPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            inputText = lastPrompt
            composerFocused = true
        }
    }

    @discardableResult
    private func dismissChatError(_ occurrenceID: UUID) -> Bool {
        errorPresentation.dismiss(occurrenceID: occurrenceID, conversationID: container.chatManager.currentConversationId)
    }


    private var chatComposer: some View {
        VStack(alignment: .leading, spacing: 0) {
            if container.chatManager.currentConversationId == nil, let modelConfigurationMessage {
                Button(action: openModelTab) {
                    Text(modelConfigurationMessage)
                        .font(.callout)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundStyle(DesignTokens.primaryColor)
                .padding(.horizontal, DesignTokens.spacing_lg)
                .accessibilityIdentifier("chat.configureModel")
            }
            if let composerReasoningError { Text(composerReasoningError).font(.footnote).foregroundStyle(.red).padding(.horizontal) }
            if let commandError { Text(commandError).font(.callout).foregroundStyle(.red).padding(.horizontal) }
            if commandsVisible {
                ExpertMentionPanel(candidates: ComposerCommand.candidates(inputText).map {
                    ExpertMentionCandidate(id: $0.rawValue, name: "/" + $0.rawValue, detail: $0.title)
                }, highlightedID: highlightedCommandID ?? ComposerCommand.candidates(inputText).first?.rawValue,
                error: nil, select: completeCommand, accessibilityTitle: "composer.command.help", identifierPrefix: "chat.command.", showsAvatar: false)
            }
            if expertMentionsVisible {
                ExpertMentionPanel(candidates: expertMentionCandidates, highlightedID: highlightedExpertID ?? expertMentionCandidates.first?.id,
                                   error: canBrowseExpertDirectory ? expertDirectoryError : String(localized: "chat.experts.sourceUnavailable.detail"), select: selectExpertMention)
                    .task {
                        guard requireLocalExpertOwner() else { return }
                        await reloadExpertDirectory()
                    }
                if !joiningExpertIDs.isEmpty {
                    ProgressView("experts.mention.joining").font(.footnote).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            if !expertSelection.isEmpty {
                ScrollView(.horizontal) {
                    HStack {
                        ForEach(expertSelection.sorted(), id: \.self) { agentID in
                            Button {
                                expertSelection.remove(agentID); expertSelectionRevisions.removeValue(forKey: agentID)
                                expertSelectionReferences.removeValue(forKey: agentID); expertSelectionRemoteSources.removeValue(forKey: agentID)
                            } label: {
                                Label(selectedExpertName(agentID), systemImage: "xmark.circle")
                                    .padding(.horizontal, 8).frame(minHeight: 44)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityHint(Text("room.remove_mention"))
                            .accessibilityIdentifier("chat.expert.selected." + agentID)
                        }
                    }
                }
            }
            ChatComposer(
                inputText: Binding(get: { inputText }, set: { inputText = $0 }),
                attachments: pendingAttachments,
                attachmentMessage: composerDrafts.persistenceError ?? attachmentImportMessage,
                isImportingAttachments: isImportingAttachments,
                isEditable: isEditable,
                isGenerating: processingState.blocksInput && !inputText.hasPrefix("/"),
                focus: $composerFocused,
                onBeginAttachFiles: beginAttachmentImport,
                onRemoveAttachment: { id in
                    pendingAttachments.removeAll { $0.id == id }
                    if pendingAttachments.isEmpty {
                        attachmentImportMessage = nil
                    }
                },
                onSend: sendMessage,
                onStop: stopGeneration,
                mentionsVisible: commandsVisible || expertMentionsVisible,
                mentionCommand: handleComposerKey,
                showsModelControl: horizontalSizeClass == .compact,
                modelControl: composerModelControl
            )
            .onChange(of: inputText) { _, _ in highlightedCommandID = nil; dismissedCommand = nil; commandError = nil; highlightedExpertID = nil; dismissedExpertMention = nil }
        }
    }

    @ViewBuilder private var composerModelControl: some View {
        if horizontalSizeClass == .compact {
            chatTopBar(showsHistoryToggle: false, presentation: .composer)
                .task(id: "\(selectedModel?.id ?? ""):\(container.chatManager.currentConversationId?.uuidString ?? String(describing: composerDrafts.selected))") { refreshComposerReasoning() }
                .onReceive(ModelParametersStore.shared.$lastUpdated) { _ in refreshComposerReasoning() }
        }
    }

    private func refreshComposerReasoning() {
        composerReasoningEfforts = []
        composerReasoningError = nil
        composerReasoningPreference = "automatic"
        guard let model = selectedModel, !model.isLocal, !model.isWebBackend else { return }
        do {
            let snapshot = try ModelParametersStore.shared.parameterSnapshot(for: model.id)
            composerReasoningEfforts = try ModelParametersStore.shared.reasoningEfforts(for: snapshot)
            let parameters = container.chatManager.currentConversationId == nil
                ? composerDrafts.current.configuration?.parameters ?? snapshot.parameters
                : try container.chatManager.generationParameters(for: model, conversationID: container.chatManager.currentConversationId)
            composerReasoningPreference = parameters.reasoningPreference ?? "automatic"
        } catch { composerReasoningError = error.localizedDescription }
    }

    private func selectComposerReasoning(_ preference: String) {
        guard let model = selectedModel, !processingState.blocksInput else { return }
        do {
            let snapshot = try ModelParametersStore.shared.parameterSnapshot(for: model.id)
            let efforts = try ModelParametersStore.shared.reasoningEfforts(for: snapshot)
            guard preference == "automatic" || efforts.contains(preference) else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            var parameters = container.chatManager.currentConversationId == nil
                ? composerDrafts.current.configuration?.parameters ?? snapshot.parameters
                : try container.chatManager.generationParameters(for: model, conversationID: container.chatManager.currentConversationId)
            parameters.reasoningPreference = preference
            parameters.reasoningTier = nil
            if let id = container.chatManager.currentConversationId {
                guard let stored = try container.applicationRuntime.loadConversation(id: id.uuidString),
                      let revision = stored.configurationRevision else { throw IOSApplicationRuntimeError.invalidResponse }
                let patch: [String: Any] = ["modelOverrides": [IOSConversationGenerationOverride.modelKey(for: model):
                    ["reasoningPreference": preference, "reasoningTier": NSNull()] as [String: Any]]]
                try container.applicationRuntime.patchConversationConfiguration(id: id.uuidString, expectedRevision: revision,
                    patchJSON: String(decoding: JSONSerialization.data(withJSONObject: patch), as: UTF8.self),
                    updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)).requireCommitted()
            } else {
                composerDrafts.current.configuration = .init(model: model, parameters: parameters)
            }
            composerReasoningPreference = preference
            composerReasoningError = nil
        } catch { composerReasoningError = error.localizedDescription }
    }

    private var commandsVisible: Bool {
        composerFocused && dismissedCommand != inputText && ComposerCommand.query(inputText) != nil
    }
    private func completeCommand(_ id: String) {
        inputText = "/" + id + " "
        composerFocused = true
    }
    private func handleComposerKey(_ key: String) -> Bool {
        guard commandsVisible else { return handleExpertMentionKey(key) }
        if key == "dismiss" { dismissedCommand = inputText; return true }
        let candidates = ComposerCommand.candidates(inputText)
        guard !candidates.isEmpty else { return true }
        let index = candidates.firstIndex { $0.rawValue == highlightedCommandID } ?? 0
        if key == "accept" { completeCommand(candidates[index].rawValue) }
        else { highlightedCommandID = candidates[(index + (key == "up" ? -1 : 1) + candidates.count) % candidates.count].rawValue }
        return true
    }
    private func executeComposerCommand() -> Bool {
        guard let parsed = ComposerCommand.parse(inputText) else { return false }
        guard let command = parsed.command, command != .plan || parsed.argument.isEmpty else {
            commandError = ComposerCommand.invalidMessage; return true
        }
        if command == .stop && !processingState.blocksInput {
            commandError = NSLocalizedString("composer.command.noRun", value: "No task is running.", comment: "Command feedback")
            return true
        }
        inputText = ""
        switch command {
        case .model: openModelTab()
        case .experts: openExpertPicker()
        case .resources: conversationInspectorPane = .resources
        case .permissions: openConversationSettings()
        case .plan: NotificationCenter.default.post(name: .nolaOpenWorkspace, object: 2)
        case .search: NotificationCenter.default.post(name: .nolaSearchConversations, object: parsed.argument)
        case .stop: stopGeneration()
        case .help: inputText = "/"; composerFocused = true
        }
        return true
    }

    private func selectedExpertName(_ agentID: String) -> String {
        let room = expertRemoteRoom ?? expertSubmission?.conversationAssociation
        let reference = expertSelectionReferences[agentID]
        if let member = room?.room.room.participants.first(where: { member in
            member.active && member.agentID == agentID &&
                member.agentRevision == expertSelectionRevisions[agentID] &&
                (reference?.participantID == nil || reference?.participantID == member.id) &&
                (reference?.contextConversationID == nil || reference?.contextConversationID == member.contextConversationID)
        }) { return member.displayName }
        return expertDirectory.first(where: { $0.id == agentID })?.name ?? String(localized: "expert.history.peerName")
    }

    private func applyExpertHistoryContinuation() {
        guard let request = expertHistoryContinuation,
              container.chatManager.currentConversationId == request.parentID else { return }
        do {
            guard try container.applicationRuntime.conversationSourceOwner(conversationID: request.parentID.uuidString) == .local,
                  let association = expertSubmission?.conversationAssociation else {
                throw IOSApplicationRuntimeError.rejected(String(localized: "expert.history.unavailable"))
            }
            let member = try request.participant(in: association)
            expertSelection.insert(member.agentID)
            expertSelectionRevisions[member.agentID] = member.agentRevision
            expertSelectionReferences[member.agentID] = .init(agentID: member.agentID, agentRevision: member.agentRevision,
                participantID: member.id, contextConversationID: member.contextConversationID)
        } catch { expertSendError = error.localizedDescription }
    }

    private var expertPickerSheet: some View {
        VStack {
            IOSChatExpertPicker(agents: expertDirectory, allowsManagement: container.chatManager.getCurrentModel()?.isWebBackend != true, onManagementClosed: {
                Task { await reloadExpertDirectory() }
            }, selection: Binding(get: { expertSelection }, set: { value in
                guard requireLocalExpertOwner() else { return }
                let previous = expertSelection
                expertSelection = value
                expertSelectionRevisions = expertSelectionRevisions.filter { value.contains($0.key) }
                for agent in expertDirectory where value.contains(agent.id) && !previous.contains(agent.id) {
                    expertSelectionRevisions[agent.id] = agent.revision
                    let member = expertRemoteRoom?.room.room.participants.first { $0.agentID == agent.id && $0.active && $0.agentRevision == agent.revision }
                    if let source = expertRemoteSource { expertSelectionRemoteSources[agent.id] = source }
                    else { expertSelectionRemoteSources.removeValue(forKey: agent.id) }
                    expertSelectionReferences[agent.id] = .init(agentID: agent.id, agentRevision: agent.revision,
                        participantID: member?.id, contextConversationID: member?.contextConversationID)
                }
            }))
            if let expertDirectoryError { Text(expertDirectoryError).foregroundStyle(.red).padding() }
        }.task { await reloadExpertDirectory() }
    }

    private func reloadExpertDirectory() async {
        let owner = container.chatManager.currentConversationId
        let draftOwner = composerDrafts.selected
        do {
            let source = try container.chatManager.expertRoomSource(owner: owner)
            let directory: IOSAgentDirectory
            var remoteRoom: IOSConversationRoomAssociation?
            var confirmedSource: ChatManager.ExpertRoomSource?
            if let source {
                let capability = try await source.provider.expertRooms(command: .object(["type": .string("role_templates")]), expectedEpoch: source.epoch)
                let catalog = try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self, from: JSONEncoder().encode(capability.result))
                guard catalog.features.mainAgentDelegation else { throw IOSApplicationRuntimeError.rejected(String(localized: "experts.mainHostUpgrade")) }
                let read = try await source.provider.expertRooms(command: .object(["type": .string("directory")]), expectedEpoch: capability.sourceStoreEpoch)
                directory = try JSONDecoder().decode(IOSAgentDirectory.self, from: JSONEncoder().encode(read.result))
                confirmedSource = .init(owner: owner, provider: source.provider, epoch: read.sourceStoreEpoch)
                if let owner {
                    let room = try await source.provider.expertRooms(command: .object(["type": .string("for_conversation"), "conversation_id": .string(owner.uuidString)]), expectedEpoch: read.sourceStoreEpoch)
                    if room.result != .null { remoteRoom = try JSONDecoder().decode(IOSConversationRoomAssociation.self, from: JSONEncoder().encode(room.result)) }
                }
                guard try container.chatManager.matchesExpertRoomSource(source) else { throw CancellationError() }
            } else {
                let service = IOSAgentRoomService(runtime: container.applicationRuntime)
                directory = try await Task.detached { try service.directory() }.value
                if let _ = try container.chatManager.expertRoomSource(owner: owner) { throw CancellationError() }
            }
            guard IOSExpertMentionCompletionPolicy.accepts(owner: owner, draftOwner: draftOwner,
                currentOwner: container.chatManager.currentConversationId, currentDraftOwner: composerDrafts.selected) else { return }
            expertDirectory = directory.agents; expertRemoteSource = confirmedSource; expertRemoteRoom = remoteRoom
            expertDirectoryError = nil
        } catch is CancellationError { }
        catch {
            guard IOSExpertMentionCompletionPolicy.accepts(owner: owner, draftOwner: draftOwner,
                currentOwner: container.chatManager.currentConversationId, currentDraftOwner: composerDrafts.selected) else { return }
            expertDirectoryError = error.localizedDescription
        }
    }

    private func refreshExpertSourceOwner() {
        guard let id = container.chatManager.currentConversationId else {
            expertSourceOwner = .unavailable
            return
        }
        do { expertSourceOwner = try container.applicationRuntime.conversationSourceOwner(conversationID: id.uuidString) }
        catch { expertSourceOwner = .unavailable }
    }

    private var canBrowseExpertDirectory: Bool {
        container.chatManager.currentConversationId == nil || container.chatManager.getCurrentModel()?.isWebBackend == true || expertSourceOwner == .local
    }

    private func requireLocalExpertOwner() -> Bool {
        // Unsaved drafts may browse/reference the global directory without
        // creating a conversation or claiming local execution authority.
        // Mutation/admission checks provenance after canonical owner creation.
        guard container.chatManager.currentConversationId != nil else { return true }
        refreshExpertSourceOwner()
        guard canBrowseExpertDirectory else {
            showExpertPicker = false
            showExpertSourceNotice = true
            return false
        }
        return true
    }

    private func openExpertPicker() {
        guard requireLocalExpertOwner() else { return }
        composerFocused = false
        showExpertPicker = true
    }

    private var expertMentionsVisible: Bool {
        composerFocused && dismissedExpertMention != inputText && ExpertMentionQuery.atEnd(of: inputText) != nil
    }
    private var expertMentionCandidates: [ExpertMentionCandidate] {
        guard canBrowseExpertDirectory, let query = ExpertMentionQuery.atEnd(of: inputText)?.query else { return [] }
        if container.chatManager.getCurrentModel()?.isWebBackend == true { guard let _ = expertRemoteSource else { return [] } }
        return expertDirectory.filter { $0.availability == "active" && !expertSelection.contains($0.id) &&
            (query.isEmpty || $0.name.localizedCaseInsensitiveContains(query)) }
            .map { ExpertMentionCandidate(id: $0.id, name: $0.name, detail: $0.description) }
    }
    private func selectExpertMention(_ id: String) {
        guard requireLocalExpertOwner(), !joiningExpertIDs.contains(id),
              expertMentionCandidates.contains(where: { $0.id == id }),
              let candidate = expertDirectory.first(where: { $0.id == id }),
              let query = ExpertMentionQuery.atEnd(of: inputText) else { return }
        let capturedText = inputText
        let owner = container.chatManager.currentConversationId
        let draftOwner = composerDrafts.selected
        if let source = expertRemoteSource {
            do {
                guard try container.chatManager.matchesExpertRoomSource(source), source.owner == owner else { throw CancellationError() }
                let member = expertRemoteRoom?.room.room.participants.first { $0.agentID == id && $0.active }
                if let owner, let member, member.agentRevision != candidate.revision {
                    pendingExpertRejoin = .init(agent: candidate, owner: owner, draftOwner: draftOwner, text: capturedText, query: query)
                    return
                }
                expertSelectionRemoteSources[id] = source
                expertSelectionReferences[id] = .init(agentID: id, agentRevision: candidate.revision,
                    participantID: member?.id, contextConversationID: member?.contextConversationID)
                finishExpertMention(id: id, revision: candidate.revision, owner: owner, draftOwner: draftOwner, text: capturedText, query: query)
            } catch { expertDirectoryError = error.localizedDescription }
            return
        }
        if container.chatManager.getCurrentModel()?.isWebBackend == true {
            expertDirectoryError = "Reload the Host expert directory before selecting an expert."
            return
        }
        let service = IOSAgentRoomService(runtime: container.applicationRuntime)
        let operation = UUID()
        expertJoinOperations[id] = operation
        Task { @MainActor in
            defer { if expertJoinOperations[id] == operation { expertJoinOperations.removeValue(forKey: id) } }
            do {
                if let owner {
                    _ = try await Task.detached {
                        try service.joinMention(agentID: id, expectedAgentRevision: candidate.revision, conversationID: owner.uuidString)
                    }.value
                } else {
                    _ = try await Task.detached {
                        try service.validateMention(agentID: id, expectedAgentRevision: candidate.revision)
                    }.value
                }
                finishExpertMention(id: id, revision: candidate.revision, owner: owner, draftOwner: draftOwner, text: capturedText, query: query)
            } catch {
                let refreshed = try? await Task.detached { try service.directory().agents }.value
                guard IOSExpertMentionCompletionPolicy.accepts(owner: owner, draftOwner: draftOwner,
                    currentOwner: container.chatManager.currentConversationId, currentDraftOwner: composerDrafts.selected) else { return }
                if let refreshed { expertDirectory = refreshed }
                expertDirectoryError = error.localizedDescription
                if let owner, case IOSExpertMentionFailure.requiresRejoin = error {
                    pendingExpertRejoin = .init(agent: candidate, owner: owner, draftOwner: draftOwner, text: capturedText, query: query)
                }
            }
        }
    }

    private func finishExpertMention(id: String, revision: Int64, owner: UUID?, draftOwner: ConversationComposerDraftStore.Owner,
                                     text: String, query: ExpertMentionQuery) {
        guard IOSExpertMentionCompletionPolicy.accepts(owner: owner, draftOwner: draftOwner,
            currentOwner: container.chatManager.currentConversationId, currentDraftOwner: composerDrafts.selected) else { return }
        expertSelection.insert(id)
        if container.chatManager.getCurrentModel()?.isWebBackend != true { expertSelectionRemoteSources.removeValue(forKey: id) }
        expertSelectionRevisions[id] = revision
        if inputText == text { inputText = query.removing(from: text) }
        expertDirectoryError = nil
        highlightedExpertID = nil
        composerFocused = true
    }

    private func confirmExpertRejoin(_ request: IOSExpertMentionRejoinRequest) {
        pendingExpertRejoin = nil
        guard requireLocalExpertOwner(), IOSExpertMentionCompletionPolicy.accepts(owner: request.owner, draftOwner: request.draftOwner,
            currentOwner: container.chatManager.currentConversationId, currentDraftOwner: composerDrafts.selected),
              !joiningExpertIDs.contains(request.agent.id) else { return }
        if let source = expertRemoteSource {
            let remoteOperation = UUID()
            expertJoinOperations[request.agent.id] = remoteOperation
            Task { @MainActor in
                defer { if expertJoinOperations[request.agent.id] == remoteOperation { expertJoinOperations.removeValue(forKey: request.agent.id) } }
                do {
                    guard source.owner == request.owner, let epoch = source.epoch,
                          try container.chatManager.matchesExpertRoomSource(source), let association = expertRemoteRoom,
                          association.conversationID == request.owner.uuidString else { throw CancellationError() }
                    _ = try await source.provider.expertRooms(command: .object(["type": .string("rejoin"), "input": .object([
                        "room_id": .string(association.roomID), "expected_room_revision": .signedInteger(association.room.revision),
                        "agent_id": .string(request.agent.id), "expected_agent_revision": .signedInteger(request.agent.revision),
                        "at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000))])]), expectedEpoch: epoch)
                    let read = try await source.provider.expertRooms(command: .object(["type": .string("for_conversation"),
                        "conversation_id": .string(request.owner.uuidString)]), expectedEpoch: epoch)
                    let room = try JSONDecoder().decode(IOSConversationRoomAssociation.self, from: JSONEncoder().encode(read.result))
                    guard try container.chatManager.matchesExpertRoomSource(source),
                          IOSExpertMentionCompletionPolicy.accepts(owner: request.owner, draftOwner: request.draftOwner,
                            currentOwner: container.chatManager.currentConversationId, currentDraftOwner: composerDrafts.selected),
                          let member = room.room.room.participants.first(where: { $0.agentID == request.agent.id && $0.active && $0.agentRevision == request.agent.revision }) else { throw CancellationError() }
                    expertRemoteRoom = room
                    expertSelectionRemoteSources[request.agent.id] = source
                    expertSelectionReferences[request.agent.id] = .init(agentID: member.agentID, agentRevision: member.agentRevision,
                        participantID: member.id, contextConversationID: member.contextConversationID)
                    finishExpertMention(id: member.agentID, revision: member.agentRevision, owner: request.owner,
                        draftOwner: request.draftOwner, text: request.text, query: request.query)
                } catch is CancellationError { }
                catch { expertDirectoryError = error.localizedDescription }
            }
            return
        }
        let service = IOSAgentRoomService(runtime: container.applicationRuntime)
        let operation = UUID()
        expertJoinOperations[request.agent.id] = operation
        Task { @MainActor in
            defer { if expertJoinOperations[request.agent.id] == operation { expertJoinOperations.removeValue(forKey: request.agent.id) } }
            do {
                _ = try await Task.detached {
                    try service.rejoinMention(agentID: request.agent.id, expectedAgentRevision: request.agent.revision,
                        conversationID: request.owner.uuidString)
                }.value
                finishExpertMention(id: request.agent.id, revision: request.agent.revision,
                    owner: request.owner, draftOwner: request.draftOwner, text: request.text, query: request.query)
            } catch {
                let refreshed = try? await Task.detached { try service.directory().agents }.value
                guard IOSExpertMentionCompletionPolicy.accepts(owner: request.owner, draftOwner: request.draftOwner,
                    currentOwner: container.chatManager.currentConversationId, currentDraftOwner: composerDrafts.selected) else { return }
                if let refreshed { expertDirectory = refreshed }
                expertDirectoryError = error.localizedDescription
            }
        }
    }
    private func handleExpertMentionKey(_ command: String) -> Bool {
        guard expertMentionsVisible else { return false }
        if command == "dismiss" { dismissedExpertMention = inputText; return true }
        let candidates = expertMentionCandidates
        guard !candidates.isEmpty else { return command == "accept" }
        let index = candidates.firstIndex { $0.id == highlightedExpertID } ?? 0
        if command == "accept" { selectExpertMention(candidates[index].id) }
        else { highlightedExpertID = candidates[(index + (command == "up" ? -1 : 1) + candidates.count) % candidates.count].id }
        return true
    }

    private func openModelTab() {
        NotificationCenter.default.post(name: .nolaOpenModelTab, object: nil)
    }

    @discardableResult
    private func restoreUnsentConfiguration() -> Bool {
        guard case .unsent = composerDrafts.selected else { return false }
        do {
            if composerDrafts.current.configuration == nil, !composerDrafts.current.restored,
               composerDrafts.persistenceError == nil,
               let initial = ModelSelectionManager.getLastSelectedModel() {
                composerDrafts.current.configuration = .init(model: initial,
                    parameters: ModelParametersStore.shared.loadParameters(for: initial.id, isLocal: initial.isLocal))
            }
            guard let configuration = composerDrafts.current.configuration else {
                selectedModel = nil
                container.chatManager.selectedModelInfo = nil
                modelConfigurationMessage = NSLocalizedString("model.picker.title", comment: "Select model action")
                return false
            }
            var catalog = try container.applicationRuntime.listModelProfiles().compactMap(ModelSelectionManager.modelInfo(from:))
            // The Host entry represents a paired connection, not an LLM profile.
            catalog.append(contentsOf: try ModelSelectionManager.availableModels(using: container.applicationRuntime).filter(\.isWebBackend))
            let model = try IOSDraftModelResolver.resolve(configuration, catalog: catalog)
            selectedModel = model
            modelConfigurationMessage = nil
            container.chatManager.selectModel(model, persistAsDefault: false)
            return true
        } catch {
            selectedModel = nil
            container.chatManager.selectedModelInfo = nil
            attachmentImportMessage = error.localizedDescription
            return false
        }
    }

    private func selectChatModel(_ model: ModelInfo) {
        if container.chatManager.currentConversationId == nil {
            composerDrafts.current.configuration = .init(model: model,
                parameters: ModelParametersStore.shared.loadParameters(for: model.id, isLocal: model.isLocal))
        }
        do { try container.chatManager.persistSelectedModelForCurrentConversation(model) }
        catch { attachmentImportMessage = error.localizedDescription; return }
        selectedModel = model
        modelConfigurationMessage = nil
        container.chatManager.selectModel(model, persistAsDefault: false)
        refreshHostSkillsIfNeeded(for: model)
    }

    private func setConversationResponseStyle(_ style: PromptResponseStyle?) {
        if container.chatManager.currentConversationId == nil, let configuration = composerDrafts.current.configuration {
            var parameters = configuration.parameters
            parameters.responseStyle = style ?? .casual
            composerDrafts.current.configuration = .init(modelID: configuration.modelID, source: configuration.source, parameters: parameters)
            conversationResponseStyle = style
            return
        }
        do {
            try container.chatManager.setConversationPromptResponseStyle(style)
            conversationResponseStyle = style
        } catch {
            attachmentImportMessage = error.localizedDescription
        }
    }

    private func prepareExpertWorkspace() {
        if expertSubmission == nil {
            let execution = IOSRoomExecutionService(runtime: container.applicationRuntime, manager: container.chatManager)
            let submission = IOSRoomSubmissionWorkspace(service: IOSAgentRoomService(runtime: container.applicationRuntime),
                executeOwner: { try await execution.execute($0, intent: $1) })
            submission.prepareOwner = { try await execution.prepare($0) }
            submission.resumePreparedExpertOwner = { try await container.chatManager.resumePreparedExpertConversation($0) }
            submission.recoverOwner = { try await execution.recover($0, intent: $1) }
            submission.cancelPreparedOwner = { try await execution.cancelPrepared($0) }
            expertSubmission = submission
        }
    }

    /// Selection is captured as typed guidance for the ordinary main Run.
    /// Core validates revision pins and admits any missing room membership.
    private func capturedMainExperts() throws -> [AppleMainExpertSelectionEntry] {
        guard !expertSelection.isEmpty else { return [] }
        if let source = expertRemoteSource {
            guard source.owner == container.chatManager.currentConversationId,
                  try container.chatManager.matchesExpertRoomSource(source) else { throw IOSApplicationRuntimeError.rejected("The expert Host changed. Select the experts again.") }
            let entries = try expertSelection.sorted().map { id in
                guard let value = expertSelectionReferences[id], let captured = expertSelectionRemoteSources[id],
                      captured.owner == source.owner, captured.epoch == source.epoch,
                      captured.provider.sameExpertConnection(as: source.provider) else {
                    throw IOSApplicationRuntimeError.rejected("The expert Host changed. Select this expert again before sending.")
                }
                return value
            }
            try AppleMainExpertSelectionContext(selectedExperts: entries).validate()
            return entries
        }
        if let _ = try container.chatManager.expertRoomSource(owner: container.chatManager.currentConversationId) {
            throw IOSApplicationRuntimeError.rejected("Reload this Host's expert directory before sending.")
        }
        guard expertSelection.isDisjoint(with: Set(expertSelectionRemoteSources.keys)) else {
            throw IOSApplicationRuntimeError.rejected("The expert source changed. Select the experts again before sending.")
        }
        let service = IOSAgentRoomService(runtime: container.applicationRuntime)
        let room = try container.chatManager.currentConversationId.flatMap { try service.forConversation($0.uuidString) }
        let joined = (room?.room.room.participants ?? []).filter(\.active).map {
            AppleExpertParticipantReference(participant_id: $0.id, agent_id: $0.agentID,
                agent_revision: $0.agentRevision, context_conversation_id: $0.contextConversationID)
        }
        return try AppleMainExpertSelectionContext.capture(agentIDs: expertSelection,
            observedRevisions: expertSelectionRevisions, joined: joined).selectedExperts
    }

    private func sendMessage() {
        if executeComposerCommand() { return }
        guard !processingState.blocksInput else { return }
        do { try container.chatManager.validateNewMessageAdmission() }
        catch {
            errorPresentation.observe(.error(error.localizedDescription), conversationID: container.chatManager.currentConversationId)
            Task { await container.chatManager.discoverUnknownEffectRecovery() }
            return
        }
        let selectedExpertSourceEpoch = expertRemoteSource?.epoch
        let selectedExperts: [AppleMainExpertSelectionEntry]
        do { selectedExperts = try capturedMainExperts() }
        catch { expertSendError = error.localizedDescription; return }
        let typedMessage = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        let attachmentsToSend = pendingAttachments
        guard !isImportingAttachments else { return }
        guard !typedMessage.isEmpty || !attachmentsToSend.isEmpty else { return }
        if container.chatManager.currentConversationId == nil, !restoreUnsentConfiguration() { return }
        guard container.chatManager.getCurrentModel() != nil else {
            if availableModels.isEmpty {
                showNoModelWarning = true
            } else {
                openModelTab()
            }
            return
        }

        NolaHaptics.primaryAction()

        let attachmentPrompt = ChatAttachmentDraft.promptBlock(for: attachmentsToSend)
        let userMessage = composedDisplayMessage(text: typedMessage, attachmentCount: attachmentsToSend.count)
        let promptMessage = composedPromptMessage(text: typedMessage, attachmentPrompt: attachmentPrompt)
        let userAttachmentMeta = attachmentMeta(for: attachmentsToSend)

        let shouldDeriveTitleFromThisInput = currentConversationTitle == L10n.chatConversationDefaultTitle
            && !messages.contains { $0.role == .user }
        let initialTitle = shouldDeriveTitleFromThisInput
            ? smartConversationTitle(from: typedMessage.isEmpty ? userMessage : typedMessage)
            : nil
        let prepared: SendConversationTurnUseCase.Prepared
        do {
            prepared = try container.chatManager.submissionUseCase.prepare(
                title: initialTitle?.isEmpty == false ? initialTitle! : L10n.chatConversationDefaultTitle,
                initialParameters: container.chatManager.currentConversationId == nil ? composerDrafts.current.configuration?.parameters : nil,
                drafts: &composerDrafts)
        } catch { attachmentImportMessage = error.localizedDescription; return }
        let conversationId = prepared.conversationID
        let execution = prepared.execution
        activeResponseConversationID = conversationId

        let userMessageId = prepared.identity.visibleUserMessageID
        let assistantMessageId = prepared.identity.visibleAssistantMessageID
        activeAssistantMessageID = assistantMessageId
        responseAssistantIDs[conversationId] = assistantMessageId

        inputText = ""
        pendingAttachments = []
        attachmentImportMessage = nil
        messages.append(ChatMessage(
            id: userMessageId,
            role: .user,
            content: userMessage,
            includeInContext: true,
            metadata: userAttachmentMeta ?? [:]
        ))

        if let initialTitle, shouldDeriveTitleFromThisInput {
            currentConversationTitle = initialTitle
        }

        messages.append(ChatMessage(id: assistantMessageId, role: .assistant, content: "", includeInContext: true))
        submittedTurns[conversationId] = messages.filter { $0.id == userMessageId || $0.id == assistantMessageId }
        container.logger.log(.debug, "ChatView", "Submission presentation staged", metadata: [
            "conversationId": conversationId.uuidString,
            "visibleCount": String(messages.count),
            "submittedCount": String(submittedTurns[conversationId]?.count ?? 0)
        ])

        var responseStateCancellable: AnyCancellable? = nil

        let reasoningState = Publishers.CombineLatest(
            execution.$thinking.removeDuplicates(),
            execution.$thinkingFinished.removeDuplicates()
        )
        let responseState = Publishers.CombineLatest4(
            execution.$currentResponse.removeDuplicates(),
            reasoningState,
            execution.$hostToolEvents.removeDuplicates(),
            execution.$agentTraceProjection.removeDuplicates(by: { lhs, rhs in
                lhs.isVisuallyEquivalent(to: rhs)
            })
        )
        responseStateCancellable = Publishers.CombineLatest(
            responseState, execution.$agentTaskProjection.removeDuplicates()
        )
            .coalescingChatFrames()
            .sink { response, tasks in
                let (content, reasoning, events, projection) = response
                updateAssistantDraft(
                    assistantId: assistantMessageId,
                    afterUserId: userMessageId,
                    content: content,
                    thinking: reasoning.0,
                    thinkingFinished: reasoning.1,
                    hostToolEvents: events,
                    traceActivities: projection.activities,
                    tracePhase: projection.phase,
                    taskProjection: tasks.tasks.isEmpty ? nil : tasks
                )
            }

        let capabilities = Set([
            hybridFilesEnabled ? "files_read" : nil,
            hybridFilesEnabled ? "files_write" : nil,
            hybridClipboardEnabled ? "clipboard" : nil,
            hybridLocationEnabled ? "location" : nil,
            hybridCalendarEnabled ? "calendar" : nil,
        ].compactMap { $0 })

        container.chatManager.submissionUseCase.start(prepared, input: .init(
            prompt: promptMessage, displayText: userMessage, metadata: userAttachmentMeta,
            attachments: ChatAttachmentDraft.modelAttachments(for: attachmentsToSend),
            responseStyle: selectedResponseStyle, retrievalEnabled: enableRetrieval,
            preferredHostID: container.chatManager.preferredNativeAppHostId,
            hybridToolsEnabled: hybridSurfaceActive, hybridCapabilities: capabilities, selectedExperts: selectedExperts, expertSelectionSourceEpoch: selectedExpertSourceEpoch),
            observers: [responseStateCancellable].compactMap { $0 }) { result in
            // Completion below applies final state synchronously. Cancel any queued
            // intermediate frame first so it cannot overwrite the result/error.
            responseStateCancellable?.cancel()
            defer {
                if responseAssistantIDs[conversationId] == assistantMessageId {
                    responseAssistantIDs[conversationId] = nil
                }
                if activeAssistantMessageID == assistantMessageId {
                    activeAssistantMessageID = nil
                    activeResponseConversationID = nil
                }
            }

            do {
                let response = try result.get()

                updateAssistantDraft(
                    assistantId: assistantMessageId,
                    afterUserId: userMessageId,
                    content: response,
                    thinking: execution.thinking,
                    thinkingFinished: true,
                    hostToolEvents: execution.hostToolEvents,
                    traceActivities: execution.agentTraceProjection.activities,
                    tracePhase: execution.agentTraceProjection.phase,
                    taskProjection: execution.agentTaskProjection.tasks.isEmpty ? nil : execution.agentTaskProjection,
                    metadata: execution.latestResponseMeta
                )
                applyCommittedAssistantProjection(execution.committedAssistantProjection)
                if container.chatManager.currentConversationId == conversationId {
                    let usage = await execution.contextUsage(for: conversationId)
                    if container.chatManager.currentConversationId == conversationId {
                        currentContextUsage = usage
                    }
                }

                loadConversationState(for: container.chatManager.currentConversationId)
            } catch {
                if error is CancellationError {
                    return
                }
                if let rejection = ChatManager.mainExpertRejection(error), rejection.permitsDraftRestore {
                    do {
                        if try execution.confirmsUnadmittedMainSelection(error, prepared: prepared),
                           try composerDrafts.restoreRejectedSubmission(prepared.draft, conversationID: conversationId) {
                            submittedTurns.removeValue(forKey: conversationId)
                            if container.chatManager.currentConversationId == conversationId {
                                messages.removeAll { $0.id == userMessageId || $0.id == assistantMessageId }
                                expertSendError = rejection.localizedDescription
                            }
                            return
                        }
                    } catch { attachmentImportMessage = error.localizedDescription }
                }
                let displayedError = ChatManager.mainExpertRejection(error)?.localizedDescription ?? error.localizedDescription
                let renderedError = "\(L10n.chatErrorUnknown): \(displayedError)"
                updateAssistantDraft(
                    assistantId: assistantMessageId,
                    afterUserId: userMessageId,
                    content: renderedError,
                    thinking: execution.thinking,
                    thinkingFinished: true,
                    hostToolEvents: execution.hostToolEvents,
                    traceActivities: execution.agentTraceProjection.activities,
                    tracePhase: execution.agentTraceProjection.phase,
                    taskProjection: execution.agentTaskProjection.tasks.isEmpty ? nil : execution.agentTaskProjection
                )
                applyCommittedAssistantProjection(execution.committedAssistantProjection)
            }
        }
    }

    private func stopGeneration() {
        guard let conversationID = container.chatManager.currentConversationId,
              let execution = container.chatManager.executionForConversation(conversationID),
              execution.processingState.blocksInput else { return }
        NolaHaptics.warning()
        container.chatManager.submissionUseCase.cancel(conversationID: conversationID)
    }

    private func beginAttachmentImport() -> (Result<[URL], Error>) -> Void {
        guard let operation = composerDrafts.beginImport() else { return { _ in } }
        let parserHost = currentAttachmentParserHost()
        return { result in
            guard composerDrafts.accepts(operation), attachmentImportTasks[operation.id] == nil else { return }
            attachmentImportTasks[operation.id] = Task { @MainActor in
                defer { attachmentImportTasks[operation.id] = nil }
                do {
                    let importResult = try await ChatAttachmentDraft.importFiles(from: result, parserHost: parserHost)
                    guard !Task.isCancelled, composerDrafts.accepts(operation),
                          attachmentOwnerStillExists(operation.owner) else { return }
                    let imported = importResult.attachments
                    var notice: String?
                    if (composerDrafts.draft(for: operation.owner)?.attachments.count ?? 0) + imported.count > 10 {
                        notice = tr(
                            "chat.attachments.limit",
                            "Only the latest 10 attachments are kept.",
                            comment: "Hint shown when attachment list exceeds maximum size"
                        )
                    } else if imported.isEmpty {
                        notice = tr(
                            "chat.attachments.import_empty",
                            "No usable content was imported from the selected files.",
                            comment: "Hint shown when selected files produce no importable attachment content"
                        )
                    } else if importResult.failedCount > 0 {
                        let format = tr(
                            "chat.attachments.partial_failure",
                            "%d selected files couldn't be imported. Other files are ready.",
                            comment: "Attachment import partial failure message"
                        )
                        notice = String(format: format, importResult.failedCount)
                    } else {
                        let warningCount = imported.filter { $0.importWarning != nil }.count
                        if warningCount > 0 {
                            let format = tr(
                                "chat.attachments.limited_extraction",
                                "%d files were attached with limited text extraction.",
                                comment: "Attachment import limited extraction message"
                            )
                            notice = String(format: format, warningCount)
                        }
                    }
                    composerDrafts.finishImport(operation, attachments: imported, notice: notice)
                } catch {
                    guard !Task.isCancelled, attachmentOwnerStillExists(operation.owner) else { return }
                    let cancelled = (error as NSError).domain == NSCocoaErrorDomain
                        && (error as NSError).code == CocoaError.userCancelled.rawValue
                    composerDrafts.finishImport(operation, attachments: [], notice: cancelled ? nil : error.localizedDescription)
                }
            }
        }
    }

    private func attachmentOwnerStillExists(_ owner: ConversationComposerDraftStore.Owner) -> Bool {
        guard case let .conversation(id) = owner else { return true }
        // Read failures are not deletion evidence. Explicit absence retires the
        // owner; stale completion must never recreate a deleted draft.
        do {
            guard let store = container.conversationStore else { return true }
            if try store.getConversation(conversationId: id) != nil { return true }
            composerDrafts.remove(id)
            return false
        } catch { return true }
    }

    private func currentAttachmentParserHost() -> ChatAttachmentParserHost? {
        let resolvedBaseURL = (selectedModel?.baseURL ?? container.chatManager.getCurrentModel()?.baseURL ?? container.baseURLString)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let resolvedToken = (selectedModel?.resolvedAPIKey ?? container.chatManager.getCurrentModel()?.resolvedAPIKey ?? container.apiKey)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard let model = selectedModel ?? container.chatManager.getCurrentModel(),
              model.isWebBackend,
              !resolvedToken.isEmpty,
              let baseURL = URL(string: resolvedBaseURL),
              ModelSelectionManager.pairedWebHostIdentity(for: baseURL) != nil else {
            return nil
        }
        return ChatAttachmentParserHost(
            baseURL: baseURL,
            token: resolvedToken,
            pinnedCertificateSHA256: model.tlsCertificateSHA256
        )
    }

    private func composedDisplayMessage(text: String, attachmentCount: Int) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard attachmentCount > 0, trimmed.isEmpty else { return trimmed }
        let analysisDefault = tr(
            "chat.attachments.analysis_default",
            "Please analyze based on the attachments.",
            comment: "Default instruction when user sends attachments without additional text"
        )
        return analysisDefault
    }

    private func composedPromptMessage(text: String, attachmentPrompt: String) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let base = trimmed.isEmpty
            ? tr(
                "chat.attachments.analysis_default",
                "Please analyze based on the attachments.",
                comment: "Default instruction when user sends attachments without additional text"
            )
            : trimmed
        guard !attachmentPrompt.isEmpty else { return base }
        return "\(base)\n\(attachmentPrompt)"
    }

    private func smartConversationTitle(from input: String) -> String {
        let normalized = input
            .replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return L10n.chatConversationDefaultTitle }
        return normalized
    }

    private func attachmentMeta(for attachments: [ChatAttachmentDraft]) -> [String: String]? {
        guard let encoded = ChatAttachmentMessagePayload.encode(attachments) else { return nil }
        return [
            "attachmentCount": String(attachments.count),
            ChatAttachmentMessagePayload.metadataKey: encoded,
            "attachmentWarningCount": String(attachments.filter { $0.importWarning != nil }.count)
        ]
    }

    private func updateAssistantDraft(
        assistantId: UUID,
        afterUserId userId: UUID,
        content: String? = nil,
        thinking: String? = nil,
        thinkingFinished: Bool? = nil,
        hostToolEvents: [HostAgentToolEvent]? = nil,
        traceActivities: [AgentTraceActivity]? = nil,
        tracePhase: String? = nil,
        taskProjection: AgentCoreTaskProjection? = nil,
        metadata: [String: String]? = nil
    ) {
        // Keep the submission-owned draft even while another conversation is
        // selected. A later selection may arrive before its persistence event.
        if let owner = responseAssistantIDs.first(where: { $0.value == assistantId })?.key,
           var submitted = submittedTurns[owner] {
            ChatMessageDraftReducer.upsertAssistant(
                in: &submitted, assistantID: assistantId, afterUserID: userId,
                content: content, thinking: thinking, thinkingFinished: thinkingFinished,
                hostToolEvents: hostToolEvents, traceActivities: traceActivities,
                tracePhase: tracePhase, taskProjection: taskProjection, metadata: metadata
            )
            submittedTurns[owner] = submitted
        }
        guard ChatDraftPresentationOwnership.accepts(
            assistantID: assistantId,
            activeAssistantID: activeAssistantMessageID,
            ownerConversationID: activeResponseConversationID,
            selectedConversationID: container.chatManager.currentConversationId
        ) else { return }
        ChatMessageDraftReducer.upsertAssistant(
            in: &messages,
            assistantID: assistantId,
            afterUserID: userId,
            content: content,
            thinking: thinking,
            thinkingFinished: thinkingFinished,
            hostToolEvents: hostToolEvents,
            traceActivities: traceActivities,
            tracePhase: tracePhase,
            taskProjection: taskProjection,
            metadata: metadata
        )
    }

    private func applyCommittedAssistantProjection(_ snapshot: IOSCommittedAssistantProjection?) {
        guard let snapshot, container.chatManager.ownsCommittedAssistantProjection(snapshot),
              let conversationID = container.chatManager.currentConversationId else { return }
        // A queued A2UI action starts a canonical Run without the composer's
        // optimistic bubble. Hydrate its committed window before merging the
        // first assistant projection; never fabricate a local Message owner.
        let messageID = UUID(uuidString: snapshot.message.messageID)
            ?? IOSCanonicalMessageIdentity.stableUUID(for: snapshot.message.messageID)
        if !messages.contains(where: { $0.role == .assistant && $0.id == messageID }) {
            loadMessages(for: conversationID)
        }
        // A late query for a sidebar-hidden conversation cannot create or
        // mutate a bubble in the currently selected conversation.
        ChatMessageDraftReducer.applyCommitted(
            snapshot, in: &messages, conversationID: conversationID.uuidString
        )
    }

    private func loadMessages(for convId: UUID?) {
        guard convId == container.chatManager.currentConversationId else { return }
        container.logger.log(.debug, "ChatView", "Loading conversation presentation", metadata: [
            "conversationId": convId?.uuidString ?? "none",
            "visibleCount": String(messages.count),
            "submittedCount": String(convId.flatMap { submittedTurns[$0]?.count } ?? 0)
        ])
        guard let id = convId, let store = container.conversationStore else {
            conversationLoadRecovery.retire()
            messages = convId.flatMap { submittedTurns[$0] } ?? []
            historyBeforeOrdinal = nil
            hasMoreHistory = false
            currentConversationTitle = L10n.chatConversationDefaultTitle
            return
        }

        do {
            // A missing conversation is explicit retirement, not a transient
            // read failure. Never resurrect its optimistic turn from memory.
            guard let conversation = try store.getConversation(conversationId: id) else {
                conversationLoadRecovery.retire()
                composerDrafts.remove(id)
                submittedTurns[id] = ChatSubmissionPresentationPolicy.pendingSubmission(
                    submittedTurns[id], persisted: [], conversationExists: false
                )
                messages = []
                activeAssistantMessageID = nil
                activeResponseConversationID = nil
                historyBeforeOrdinal = nil
                hasMoreHistory = false
                currentConversationTitle = L10n.chatConversationDefaultTitle
                return
            }
            if let projectionStore = store as? ConversationProjectionStore {
                let window = try projectionStore.loadConversationProjectionWindow(
                    conversationId: id,
                    beforeOrdinal: nil,
                    limit: 80,
                    contentByteLimit: 512_000
                )
                let projection = window.projection
                currentConversationTitle = IOSConversationTitlePresentationPolicy.displayTitle(
                    projection.title,
                    conversationID: projection.conversationID,
                    fallback: L10n.chatConversationDefaultTitle
                )
                let presented = IOSConversationProjectionPresenter.messages(from: projection)
                messages = ChatSubmissionPresentationPolicy.merging(
                    persisted: presented, submitted: submittedTurns[id] ?? []
                )
                conversationLoadRecovery.didLoad(id)
                container.logger.log(.debug, "ChatView", "Canonical presentation applied", metadata: [
                    "conversationId": id.uuidString,
                    "persistedCount": String(presented.count),
                    "visibleCount": String(messages.count)
                ])
                submittedTurns[id] = ChatSubmissionPresentationPolicy.pendingSubmission(
                    submittedTurns[id], persisted: presented
                )
                container.chatManager.refreshJournalPresentations(window.assistantJournalHeads)
                historyBeforeOrdinal = window.beforeOrdinal
                hasMoreHistory = window.hasMore
                activeAssistantMessageID = ChatSubmissionPresentationPolicy.activeAssistant(
                    submitted: responseAssistantIDs[id], persisted: presented
                )
                return
            }
            currentConversationTitle = IOSConversationTitlePresentationPolicy.displayTitle(
                conversation.title,
                conversationID: conversation.id.uuidString,
                fallback: L10n.chatConversationDefaultTitle
            )
            let cms = try store.getMessages(conversationId: id, limit: 0)
                let mapped: [ChatMessage] = cms.map { m in
                    let role: ChatMessage.Role = (m.role == .user) ? .user : .assistant
                    let hostToolEvents = HostAgentToolEvent.decodeFromConversationMeta(m.meta)
                    return ChatMessage(
                        id: m.id,
                        role: role,
                        content: m.meta?["displayText"] ?? m.text,
                        includeInContext: true,
                        thinking: ChatAgentTraceMeta.thinking(from: m.meta),
                        thinkingFinished: !ChatAgentTraceMeta.thinking(from: m.meta).isEmpty,
                        hostToolEvents: hostToolEvents,
                        tracePhase: ChatAgentTraceMeta.phase(from: m.meta),
                        metadata: m.meta ?? [:]
                    )
                }
                messages = ChatSubmissionPresentationPolicy.merging(
                    persisted: mapped, submitted: submittedTurns[id] ?? []
                )
                submittedTurns[id] = ChatSubmissionPresentationPolicy.pendingSubmission(
                    submittedTurns[id], persisted: mapped
                )
                conversationLoadRecovery.didLoad(id)
        } catch {
            let retainedConfirmedHistory = conversationLoadRecovery.confirmedConversationID == id
            messages = conversationLoadRecovery.didFail(id, message: error.localizedDescription,
                existing: messages, submitted: submittedTurns[id] ?? [])
            if !retainedConfirmedHistory {
                historyBeforeOrdinal = nil
                hasMoreHistory = false
                currentConversationTitle = L10n.chatConversationDefaultTitle
            }
            container.logger.log(.error, "ChatView", "Failed to load messages", metadata: ["error": error.localizedDescription])
        }
    }

    @MainActor
    private func loadCompactionSource(
        _ request: IOSCompactionSourceRequest
    ) async -> ChatCompactionSourceLookupResult {
        guard let conversationID = container.chatManager.currentConversationId,
              request.conversationID == conversationID,
              let projectionStore = container.conversationStore as? ConversationProjectionStore,
              ChatCompactionSourceNavigator.accepts(
                request,
                conversationID: conversationID,
                messages: messages
              )
        else { return .failed }
        if let target = ChatCompactionSourceNavigator.targetMessageID(
            sourceItemIDs: request.sourceItemIDs,
            messages: messages
        ) {
            return .found(target)
        }

        let initialWindow: IOSConversationProjectionWindowV1
        do {
            initialWindow = try projectionStore.loadConversationProjectionWindow(
                conversationId: conversationID,
                beforeOrdinal: nil,
                limit: 80,
                contentByteLimit: 512_000
            )
        } catch {
            container.logger.log(
                .error,
                "ChatView",
                "Failed to establish compacted source history snapshot",
                metadata: ["error": error.localizedDescription]
            )
            return .failed
        }
        let initialMessages = IOSConversationProjectionPresenter.messages(from: initialWindow.projection)
        guard !Task.isCancelled,
              container.chatManager.currentConversationId == conversationID,
              ChatCompactionSourceNavigator.accepts(
                request,
                conversationID: conversationID,
                messages: messages
              ) else { return .failed }

        let expectedRevision = initialWindow.projection.revision
        let expectedEventSequence = initialWindow.eventSequence
        var requestFoundInSnapshot = ChatCompactionSourceNavigator.accepts(
            request,
            conversationID: conversationID,
            messages: initialMessages
        )
        var nextCursor = initialWindow.beforeOrdinal
        var canLoadMore = initialWindow.hasMore
        var stagedOlderPages: [[ChatMessage]] = []
        var stagedJournalHeads: [IOSAssistantJournalCursor] = []
        var stagedBeforeOrdinal = initialWindow.beforeOrdinal
        var stagedHasMore = initialWindow.hasMore
        while canLoadMore, let cursor = nextCursor {
            guard container.chatManager.currentConversationId == conversationID,
                  ChatCompactionSourceNavigator.accepts(
                    request,
                    conversationID: conversationID,
                    messages: messages
                  ) else {
                return .failed
            }
            do {
                let window = try projectionStore.loadConversationProjectionWindow(
                    conversationId: conversationID,
                    beforeOrdinal: cursor,
                    limit: 80,
                    contentByteLimit: 512_000
                )
                let older = IOSConversationProjectionPresenter.messages(from: window.projection)
                guard !Task.isCancelled,
                      container.chatManager.currentConversationId == conversationID,
                      ChatCompactionSourceNavigator.belongsToSnapshot(
                        window,
                        revision: expectedRevision,
                        eventSequence: expectedEventSequence
                      ),
                      ChatCompactionSourceNavigator.accepts(
                        request,
                        conversationID: conversationID,
                        messages: messages
                      ) else {
                    return .failed
                }
                stagedOlderPages.append(older)
                stagedJournalHeads.append(contentsOf: window.assistantJournalHeads)
                requestFoundInSnapshot = requestFoundInSnapshot
                    || ChatCompactionSourceNavigator.accepts(
                        request,
                        conversationID: conversationID,
                        messages: older
                    )
                stagedBeforeOrdinal = window.beforeOrdinal
                stagedHasMore = window.hasMore
                if let target = ChatCompactionSourceNavigator.targetMessageID(
                    sourceItemIDs: request.sourceItemIDs,
                    messages: older
                ) {
                    guard requestFoundInSnapshot else { return .failed }
                    messages = ChatCompactionSourceNavigator.mergingFetchedOlderPages(
                        stagedOlderPages,
                        into: messages
                    )
                    historyBeforeOrdinal = stagedBeforeOrdinal
                    hasMoreHistory = stagedHasMore
                    container.chatManager.refreshJournalPresentations(stagedJournalHeads)
                    return .found(target)
                }
                guard ChatCompactionSourceNavigator.canContinue(
                    previousCursor: cursor,
                    nextCursor: window.beforeOrdinal,
                    hasMore: window.hasMore
                ) else {
                    guard requestFoundInSnapshot else { return .failed }
                    messages = ChatCompactionSourceNavigator.mergingFetchedOlderPages(
                        stagedOlderPages,
                        into: messages
                    )
                    historyBeforeOrdinal = stagedBeforeOrdinal
                    hasMoreHistory = stagedHasMore
                    container.chatManager.refreshJournalPresentations(stagedJournalHeads)
                    return .exhausted
                }
                canLoadMore = window.hasMore
                nextCursor = window.beforeOrdinal
                await Task.yield()
            } catch {
                guard !Task.isCancelled,
                      container.chatManager.currentConversationId == conversationID else {
                    return .failed
                }
                container.logger.log(
                    .error,
                    "ChatView",
                    "Failed to load compacted source history",
                    metadata: ["error": error.localizedDescription]
                )
                return .failed
            }
        }
        return .exhausted
    }

    private func loadConversationState(for convId: UUID?) {
        conversationResponseStyle = convId.flatMap { container.chatManager.conversation(for: $0)?.promptResponseStyle }
        currentExecutionBackend = container.chatManager.conversation(for: convId)?.executionBackend ?? "nola"
        permissionMode = container.chatManager.conversation(for: convId)?.permissionMode ?? .cautious
        permissionChangeID = nil
        permissionApplicationStatus = container.chatManager.permissionConfigurationStatus(conversationID: convId)
        let profile = container.chatManager.chatMemoryProfile()
        if !showMoreSettingsSheet && !showContextSheet {
            memoryEditorBaseline = profile
            currentLongTermMemory = profile.longTermMemory
            currentUserPreferences = profile.userPreferences
            currentMCPServers = profile.mcpServers
            currentUploadedSkills = profile.uploadedSkills
            currentUploadedDocuments = profile.uploadedDocuments
        }
        guard let conversation = container.chatManager.conversation(for: convId) else {
            if convId != nil { selectedModel = nil }
            else { _ = restoreUnsentConfiguration() }
            currentSessionMemory = ""
            currentKnowledgePoints = ""
            currentConversationWiki = ""
            currentPromptTemplateID = nil
            currentDataSourceIDs = []
            currentSystemDataSourceTypes = []
            currentContextUsage = nil
            return
        }
        currentSessionMemory = conversation.sessionMemory
        currentKnowledgePoints = conversation.knowledgePoints
        currentConversationWiki = conversation.wiki
        currentPromptTemplateID = conversation.selectedPromptTemplateID
        currentDataSourceIDs = Set(availableContextDataSources.compactMap { source in
            source.isEnabledForConversation(
                explicitOverride: conversation.dataSourceOverrides[source.id.uuidString]
            ) ? source.id : nil
        })
        let policyStore = SystemDataSourceAccessPolicyStore.shared
        currentSystemDataSourceTypes = Set(policyStore.configurableTypes.filter { type in
            policyStore.isEnabledForConversation(
                type: type,
                explicitOverride: conversation.dataSourceOverrides[policyStore.conversationOverrideKey(for: type)]
            )
        })
        selectedModel = conversation.selectedModelInfo
        container.chatManager.selectedModelInfo = conversation.selectedModelInfo
        container.chatManager.modelFallbackNotice = conversation.modelUnavailableReason
        if let conversationModel = conversation.selectedModelInfo {
            selectedModel = conversationModel
            container.chatManager.selectModel(conversationModel, persistAsDefault: false)
            refreshHostSkillsIfNeeded(for: conversationModel)
        }
        Task { @MainActor in
            let usage = await container.chatManager.contextUsage(for: convId)
            guard !Task.isCancelled,
                  container.chatManager.currentConversationId == convId else { return }
            currentContextUsage = usage
            if let refreshedConversation = container.chatManager.conversation(for: convId) {
                currentSessionMemory = refreshedConversation.sessionMemory
                currentKnowledgePoints = refreshedConversation.knowledgePoints
                currentConversationWiki = refreshedConversation.wiki
            }
            let refreshedProfile = container.chatManager.chatMemoryProfile()
            if !showMoreSettingsSheet && !showContextSheet {
                memoryEditorBaseline = refreshedProfile
                currentLongTermMemory = refreshedProfile.longTermMemory
                currentUserPreferences = refreshedProfile.userPreferences
                currentMCPServers = refreshedProfile.mcpServers
                currentUploadedSkills = refreshedProfile.uploadedSkills
                currentUploadedDocuments = refreshedProfile.uploadedDocuments
            }
        }
    }

    private func mergedSkillDisplayList(local: [UserUploadedContextItem], host: [UserUploadedContextItem]) -> [UserUploadedContextItem] {
        var seen = Set(local.map { $0.name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() })
        var merged = local
        for skill in host {
            let key = skill.name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard !key.isEmpty, !seen.contains(key) else { continue }
            merged.append(skill)
            seen.insert(key)
        }
        return merged
    }

    private func refreshHostSkillsIfNeeded(for model: ModelInfo?, force: Bool = false) {
        let resolvedModel = model ?? selectedModel
        let resolvedBaseURL = (resolvedModel?.baseURL ?? container.baseURLString)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let resolvedToken = (resolvedModel?.resolvedAPIKey ?? container.apiKey)
            .trimmingCharacters(in: .whitespacesAndNewlines)

        guard let model = resolvedModel,
              model.isWebBackend,
              !resolvedToken.isEmpty,
              let baseURL = URL(string: resolvedBaseURL),
              let expectedHostID = ModelSelectionManager.pairedWebHostIdentity(for: baseURL) else {
            hostConnectionPhase = resolvedModel?.isWebBackend == true ? .unconfigured : .notApplicable
            hostSkillsFetchTask?.cancel()
            hostSkillsFetchTask = nil
            hostSkillsInFlightKey = nil
            hostSkillsRequestID = nil
            currentHostUploadedSkills = []
            currentHostUploadedDocuments = []
            currentHostMCPServers = []
            currentHostComputerUseStatus = nil
            currentHostNativeBrowserStatus = nil
            currentHostExecutionBackends = []
            container.chatManager.preferredNativeAppHostId = ""
            return
        }
        let requestKey = "\(baseURL.absoluteString)|\(expectedHostID)|\(model.tlsCertificateSHA256 ?? "")|\(resolvedToken)"
        let now = Date()
        if hostSkillsInFlightKey == requestKey {
            return
        }
        if !force,
           hostSkillsLastSuccessKey == requestKey,
           let lastSuccess = hostSkillsLastSuccessAt,
           now.timeIntervalSince(lastSuccess) < hostSkillsSuccessCooldownSeconds {
            hostConnectionPhase = .connected
            return
        }
        if !force,
           hostSkillsLastFailureKey == requestKey,
           let lastFailure = hostSkillsLastFailureAt,
           now.timeIntervalSince(lastFailure) < hostSkillsFailureBackoffSeconds {
            hostConnectionPhase = .unavailable
            return
        }

        hostSkillsFetchTask?.cancel()
        let requestID = UUID()
        let requestedConversationID = container.chatManager.currentConversationId
        hostSkillsRequestID = requestID
        hostSkillsInFlightKey = requestKey
        hostConnectionPhase = .checking
        hostSkillsFetchTask = Task { @MainActor in
            defer {
                if hostSkillsRequestID == requestID {
                    hostSkillsInFlightKey = nil
                    hostSkillsRequestID = nil
                }
            }
            do {
                guard let modelService = container.llmClient as? ModelService else {
                    throw IOSApplicationRuntimeError.rejected("Host provider is unavailable")
                }
                let settings = try await modelService.hostAgentSettings(config: .init(
                    baseURL: baseURL, token: resolvedToken,
                    pinnedCertificateSHA256: model.tlsCertificateSHA256,
                    expectedHostId: expectedHostID))
                guard !Task.isCancelled, hostSkillsRequestID == requestID else { return }
                guard settings.host_identity?.permission_protocol_version == WebAgentProvider.hostPermissionProtocolVersion else {
                    throw NolaError.llm(.invalidResponse(reason: "The connected host uses an unsupported permission protocol version."))
                }
                guard settings.host_identity?.id == expectedHostID else {
                    throw NolaError.llm(.invalidResponse(reason: "The connected host identity changed. Forget the existing host pairing before reconnecting."))
                }
                currentHostComputerUseStatus = settings.computer_use
                currentHostNativeBrowserStatus = settings.native_browser
                currentHostExecutionBackends = settings.execution_backends
                let availableBackendIDs = settings.execution_backends
                    .filter { $0.availabilityState == .available }
                    .map(\.id)
                if container.chatManager.currentConversationId == requestedConversationID,
                   let resolvedBackend = ExecutionBackendSelectionPolicy.resolvedSelection(
                    current: currentExecutionBackend,
                    hostPreferredBackendID: settings.execution_backend,
                    availableBackendIDs: availableBackendIDs
                ), resolvedBackend != currentExecutionBackend {
                    try container.chatManager.selectExecutionBackendForCurrentConversation(
                        resolvedBackend,
                        availableBackendIDs: Set(availableBackendIDs)
                    )
                    currentExecutionBackend = resolvedBackend
                }
                container.chatManager.preferredNativeAppHostId = settings.computer_use?
                    .hosts?
                    .first?
                    .host_id
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                currentHostUploadedSkills = settings.uploaded_skills
                    .filter { !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
                    .map { hostSkill in
                        let name = hostSkill.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? tr("chat.host_skill.default_name", "Host Skill", comment: "Fallback host skill name")
                            : hostSkill.name.trimmingCharacters(in: .whitespacesAndNewlines)
                        let key = name.lowercased()
                        let id = hostSkillIDsByName[key] ?? UUID()
                        hostSkillIDsByName[key] = id
                        return UserUploadedContextItem(
                            id: id,
                            name: name,
                            content: hostSkill.content,
                            description: "\(tr("chat.host_skill.description_prefix", "From Web host.", comment: "Prefix for host skill description")) \(hostSkill.description ?? "")".trimmingCharacters(in: .whitespacesAndNewlines),
                            enabled: hostSkill.enabled ?? true
                        )
                    }
                currentHostUploadedDocuments = settings.uploaded_documents
                    .filter { !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
                    .map { hostDocument in
                        let name = hostDocument.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? tr("chat.host_document.default_name", "Host Document", comment: "Fallback host document name")
                            : hostDocument.name.trimmingCharacters(in: .whitespacesAndNewlines)
                        let key = name.lowercased()
                        let id = hostDocumentIDsByName[key] ?? UUID()
                        hostDocumentIDsByName[key] = id
                        return UserUploadedContextItem(
                            id: id,
                            name: name,
                            content: hostDocument.content,
                            description: "\(tr("chat.host_document.description_prefix", "From Web host.", comment: "Prefix for host document description")) \(hostDocument.description ?? hostDocument.kind ?? "")".trimmingCharacters(in: .whitespacesAndNewlines),
                            enabled: hostDocument.enabled ?? true
                        )
                    }
                currentHostMCPServers = settings.mcp_servers
                    .map { hostServer in
                        let endpoint = hostServer.endpoint.trimmingCharacters(in: .whitespacesAndNewlines)
                        let name = hostServer.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? tr("chat.host_mcp.default_name", "Host MCP Server", comment: "Fallback host MCP server name")
                            : hostServer.name.trimmingCharacters(in: .whitespacesAndNewlines)
                        let key = hostServer.serverID.lowercased()
                        let id = hostMCPServerIDsByIdentity[key] ?? UUID()
                        hostMCPServerIDsByIdentity[key] = id
                        return UserMCPServer(
                            id: id,
                            name: name,
                            endpoint: endpoint,
                            description: "\(tr("chat.host_mcp.description_prefix", "From Web host.", comment: "Prefix for host MCP server description")) \(hostServer.description ?? "")".trimmingCharacters(in: .whitespacesAndNewlines),
                            enabled: hostServer.enabled ?? true,
                            serverID: hostServer.serverID,
                            availability: hostServer.availability,
                            protocolVersion: hostServer.protocolVersion,
                            tools: hostServer.tools,
                            resources: hostServer.resources,
                            resourceTemplates: hostServer.resourceTemplates,
                            prompts: hostServer.prompts
                        )
                    }
                hostSkillsLastSuccessKey = requestKey
                hostSkillsLastSuccessAt = Date()
                hostSkillsLastFailureKey = nil
                hostSkillsLastFailureAt = nil
                hostConnectionPhase = .connected
            } catch is CancellationError {
                return
            } catch {
                guard !Task.isCancelled, hostSkillsRequestID == requestID else { return }
                hostSkillsLastFailureKey = requestKey
                hostSkillsLastFailureAt = Date()
                currentHostUploadedSkills = []
                currentHostUploadedDocuments = []
                currentHostMCPServers = []
                currentHostComputerUseStatus = nil
                currentHostNativeBrowserStatus = nil
                currentHostExecutionBackends = []
                container.chatManager.preferredNativeAppHostId = ""
                hostConnectionPhase = .unavailable
                container.logger.log(.warn, "ChatView", "Failed to load host skills", metadata: ["error": error.localizedDescription])
            }
        }
    }

}
