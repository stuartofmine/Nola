// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatManager logic in the iOS service layer.
// - Interfaces:
//   - ChatManagerError
//   - ChatProcessingState
//   - ChatManager
// - APIs:
//   - none
//
//
//  ChatManager.swift
//  Nola
//
//

import Foundation
import Combine
import SwiftUI
import UIKit

/// Owns submission preparation and process-local Run jobs for the application,
/// independently of any ChatView instance or current selection.
@MainActor final class SendConversationTurnUseCase {
    struct Prepared {
        let conversationID: UUID
        let identity: IOSCanonicalAgentTurnIdentity
        let execution: ChatManager
        let draft: ConversationComposerDraftStore.Submission
        let configurationRevision: Int64?
    }
    struct Input {
        let prompt: String
        let displayText: String
        let metadata: [String: String]?
        let attachments: [ModelInput.Attachment]
        let responseStyle: PromptResponseStyle
        let retrievalEnabled: Bool
        let preferredHostID: String
        let hybridToolsEnabled: Bool
        let hybridCapabilities: Set<String>
    }
    private unowned let workspace: ChatManager
    private var runs: [UUID: (identity: UUID, task: Task<Void, Never>)] = [:]
    init(workspace: ChatManager) { self.workspace = workspace }

    func prepare(title: String, initialParameters: ModelParameters?,
                 drafts: inout ConversationComposerDraftStore) throws -> Prepared {
        try workspace.reconcileCommittedDraftSubmissions()
        try drafts.validateForSubmission()
        switch (workspace.currentConversationId, drafts.selected) {
        case (.some(let selected), .conversation(let draftOwner)) where selected == draftOwner: break
        case (.none, .unsent(_)): break
        default:
            throw IOSApplicationRuntimeError.rejected("Conversation selection changed. Your original draft is retained; reopen its conversation before sending.")
        }
        guard let conversationID = workspace.ensureCurrentConversation(title: title,
            language: nil, initialParameters: initialParameters, selectOnCreate: false) else {
            throw ChatManagerError.failedToCreateConversation("The conversation could not be opened. Your draft is retained.")
        }
        guard runs[conversationID] == nil,
              let execution = workspace.executionForConversation(conversationID, prepareForSubmission: true),
              execution.canBeginNewMessage else { throw DeviceAgentCoordinatorError.alreadyRunning }
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let configurationRevision = workspace.conversation(for: conversationID)?.configurationRevision
        let submittedDraft = try drafts.prepareSubmission(to: conversationID, identity: identity,
            configurationRevision: configurationRevision)
        workspace.currentConversationId = conversationID
        execution.prepareAgentTrace(conversationId: conversationID, canonicalAssistantMessageID: identity.assistantMessageID)
        execution.thinking = ""
        execution.thinkingFinished = false
        execution.currentResponse = ""
        execution.hostToolEvents = []
        execution.sourceReferences = []
        return Prepared(conversationID: conversationID, identity: identity, execution: execution,
            draft: submittedDraft, configurationRevision: configurationRevision)
    }

    func start(_ prepared: Prepared, input: Input, observers: [AnyCancellable],
               completed: @escaping @MainActor (Result<String, Error>) async -> Void) {
        let owner = prepared.conversationID
        guard runs[owner] == nil else { return }
        let identity = prepared.identity.visibleAssistantMessageID
        let execution = prepared.execution
        execution.beginSubmissionAttempt()
        let submittedDraft = prepared.draft
        execution.enableRetrieval = input.retrievalEnabled
        execution.preferredNativeAppHostId = input.preferredHostID
        execution.hybridDeviceToolsEnabled = input.hybridToolsEnabled
        execution.hybridDeviceCapabilities = input.hybridCapabilities
        let task = Task { [self] in
            execution.submissionCommitted = { try? submittedDraft.committed() }
            defer {
                execution.submissionCommitted = nil
                observers.forEach { $0.cancel() }
                if runs[owner]?.identity == identity { runs[owner] = nil }
                // A failed transport can still have committed the original
                // request. Query that identity; never redispatch it here.
                try? workspace.reconcileCommittedDraftSubmissions()
            }
            let result: Result<String, Error>
            do {
                guard let current = workspace.conversation(for: owner),
                      current.configurationRevision == prepared.configurationRevision else {
                    throw IOSApplicationRuntimeError.rejected("Conversation configuration changed before submission; the captured draft is retained")
                }
                result = .success(try await execution.processMessage(input.prompt,
                    displayMessage: input.displayText, userMessageMeta: input.metadata,
                    attachments: input.attachments, userMessageID: prepared.identity.visibleUserMessageID,
                    assistantMessageID: identity, responseStyle: input.responseStyle))
            } catch {
                execution.recordSubmissionFailure(error)
                result = .failure(error)
            }
            await completed(result)
        }
        runs[owner] = (identity, task)
    }

    func cancel(conversationID: UUID) {
        let submission = runs[conversationID]?.task
        guard let execution = workspace.executionForConversation(conversationID) else {
            submission?.cancel()
            return
        }
        // Cancelling the consumer first detaches its native stream and retires
        // the execution permit before Core can persist the user interrupt.
        execution.cancelCurrentGeneration(retireSubmission: { submission?.cancel() })
    }
}

@MainActor enum IOSConversationRunConfiguration {
    static func localProvider(model: ModelInfo, parameters: ModelParameters,
        availableModels: [LMModel]? = nil,
        generation: ((LocalProviderGenerationRequest) async throws -> AsyncThrowingStream<String, Error>)? = nil) throws -> LocalProvider {
        guard model.isLocal else { throw NolaError.llm(.invalidInput(reason: "Expected a local model")) }
        let catalogName = ModelDownloader.ModelType(rawValue: model.id)?.mlxModelName
        guard let selected = (availableModels ?? MLXService.availableModels).first(where: {
            $0.id == model.id || $0.name == model.id || (catalogName != nil && $0.name == catalogName)
        }) else { throw NolaError.llm(.invalidInput(reason: "The selected local model is unavailable")) }
        return try LocalProvider(model: selected, parameters: parameters, generation: generation)
    }

    static func directCloudProviderConfig(
        model: ModelInfo,
        parameters: ModelParameters,
        reasoningSelection: AgentCoreJSONValue? = nil,
        reasoningTier: String = "balanced"
    ) throws -> OpenAIProvider.Config {
        guard !model.isLocal, !model.isWebBackend,
              let baseURL = model.baseURL.flatMap(URL.init(string:)) else {
            throw NolaError.llm(.notConfigured)
        }
        let apiKey = model.apiKey ?? ""
        guard !apiKey.isEmpty || model.cloudCredential == .anonymous else {
            throw NolaError.llm(.notConfigured)
        }
        guard model.wireProtocol == nil || model.wireProtocol == .chatCompletions || model.wireProtocol == .responses else {
            throw NolaError.llm(.invalidInput(reason: "This iOS provider does not support the selected model's wire protocol."))
        }
        let stopSequences = parameters.stopSequences
            .split(whereSeparator: \.isNewline)
            .map(String.init)
        var configuration = OpenAIProvider.Config(
            baseURL: baseURL,
            apiKey: apiKey,
            model: model.effectiveUpstreamModelId,
            wireProtocol: model.wireProtocol ?? .chatCompletions,
            executionCapabilities: model.executionCapabilities ?? .init(),
            temperature: parameters.temperature,
            topP: parameters.topP,
            maxCompletionTokens: parameters.maxOutputTokens,
            presencePenalty: parameters.presencePenalty,
            frequencyPenalty: parameters.frequencyPenalty,
            seed: parameters.seed,
            stopSequences: stopSequences
        )
        configuration.reasoningSelection = reasoningSelection
        configuration.reasoningTier = reasoningTier
        try OpenAIProvider.validateGenerationConfiguration(configuration)
        return configuration
    }
}

@MainActor
public final class ChatManager: ObservableObject {
    lazy var submissionUseCase = SendConversationTurnUseCase(workspace: self)

    func reconcileCommittedDraftSubmissions() throws {
        guard let store = container.conversationStore else { return }
        let repository = FileConversationDraftRepository.scoped(to: container.applicationRuntime.databaseURL)
        for ticket in try repository.pendingSubmissions() {
            let messages = try store.getMessages(conversationId: ticket.conversationID, limit: 0)
            if messages.contains(where: { $0.id == ticket.userMessageID && $0.role == .user }) {
                try repository.commitSubmission(ticket)
            }
        }
    }
    public nonisolated static let autoCompactTriggerPercentDefault = 80
    public nonisolated static let autoCompactTriggerPercentRange = 50...95

    static func localAgentModePrompt(for prompt: String) -> String {
        """
        [Nola Agent Mode]
        \(PromptTemplates.complexAgentMode)

        [iOS Local Boundary]
        This iOS-only Agent mode may use explicitly listed device tools after user approval. Device file tools are limited to user-selected files, and clipboard tools require foreground approval. Shell commands, process execution, and computer control are unavailable on the device. Host-side tools are available only through a connected Nola Web host.
        It must not claim it executed code or used unavailable tools.

        [User Request]
        \(prompt)
        """
    }

    @Published public var processingState: ChatProcessingState = .idle
    /// Process-local submission feedback, owned by this Context's execution.
    /// Canonical journal refreshes must not erase a pre-admission failure.
    @Published private(set) var submissionFailure: String?

    func beginSubmissionAttempt() { submissionFailure = nil }

    func recordSubmissionFailure(_ error: Error) {
        guard !(error is CancellationError) else { return }
        submissionFailure = error.localizedDescription
    }
    @Published public var currentResponse: String = ""
    @Published public var thinking: String = ""
    @Published public var showThinking: Bool = true
    /// True when the model thinking stream is fully finished.
    @Published public var thinkingFinished: Bool = false
    @Published public var hostToolEvents: [HostAgentToolEvent] = []
    @Published public var sourceReferences: [ChatSourceReference] = []
    /// A read-only bounded query result, never reconstructed from trace/tokens.
    @Published private(set) var committedAssistantProjection: IOSCommittedAssistantProjection?
    @Published private(set) var committedAssistantJournalContent: IOSJournalMessageContent?
    @Published private(set) var committedJournalPresentation: IOSCommittedJournalPresentation?
    private var remoteJournalPresentationCache = IOSRemoteJournalPresentationCache()
    @Published public private(set) var agentTraceProjection = AgentRunProjection()
    @Published public private(set) var latestResponseMeta: [String: String] = [:]
    @Published public private(set) var hostPermissionSubmissionError: String?
    @Published public private(set) var submittingHostPermissionRequestID: String?
    @Published private(set) var unknownEffectRecovery: IOSUnknownEffectRecoveryPresentation?
    @Published private(set) var unknownEffectRecoveryError: String?
    @Published private(set) var unknownEffectRecoveryChecking = false
    @Published private(set) var unknownEffectPendingConversationID: UUID?
    private var frozenUnknownEffectInspection: IOSUnknownEffectRecoveryInspection?
    @Published public var pendingHostPermissionRequest: HostAgentPermissionRequest?
    /// Conversation that owns the suspended Host permission request. Keeping
    /// this separate from the request wire payload prevents a request from
    /// appearing in, or being resolved from, another conversation after the
    /// user switches the sidebar selection.
    @Published public private(set) var pendingHostPermissionConversationId: UUID?
    public var hybridDeviceToolsEnabled = false
    public var hybridDeviceCapabilities: Set<String> = []
    @Published public private(set) var agentCoreSnapshot: AgentCoreSnapshot?
    @Published private var permissionConfigurationRevisions: [UUID: Int64] = [:]
    @Published private(set) var remotePermissionConfigurations: [UUID: IOSRemotePermissionConfiguration] = [:]
    private var permissionPolicyObservationTasks: [UUID: Task<Void, Never>] = [:]
    private var remotePermissionCommitRevisions: [UUID: Int64] = [:]
    private var permissionConfigurationRunIDs: [UUID: String] = [:]
    @Published private(set) var permissionConfigurationOwner: IOSPermissionConfigurationOwner = .unknown
    private var permissionOwnerProjectionRunID: String?
    private var permissionCommittedModes: [UUID: IOSPermissionMode] = [:]
    private var permissionObservedIdentity: IOSRemoteSubmissionIdentity?
    private var permissionObservedRecovery: String?
    private var permissionHydration: [UUID: IOSPermissionHydrationState] = [:]

    private func uiProvider(conversationID: UUID) throws -> WebAgentProvider? {
        guard let conversation = try container.conversationStore?.getConversation(conversationId: conversationID) else {
            throw IOSApplicationRuntimeError.rejected("Conversation is unavailable")
        }
        guard let model = conversation.selectedModelInfo, model.isWebBackend else { return nil }
        guard let execution = ensureExecutionForConversation(conversationID),
              let provider = execution.remoteSubmissionProviders[conversationID.uuidString]
                ?? (execution.effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider,
              provider.matchesUIHost(baseURL: model.baseURL, certificateSHA256: model.tlsCertificateSHA256) else {
            throw IOSApplicationRuntimeError.rejected("请重新连接此会话绑定的 Host。")
        }
        return provider
    }

    /// A paired address is transport configuration; Core owns the durable source.
    private func uiSourceEpoch(conversationID: UUID, provider: WebAgentProvider) throws -> String? {
        switch try container.applicationRuntime.conversationSourceOwner(conversationID: conversationID.uuidString) {
        case .local:
            // Before the first remote admission there is no source-owned UI to observe.
            return nil
        case .remote(let hostID, let epoch):
            guard provider.journalSourceHostID == hostID else {
                throw IOSApplicationRuntimeError.rejected("请重新连接此会话绑定的 Host。")
            }
            return epoch
        case .unavailable:
            throw IOSApplicationRuntimeError.rejected("无法确认此会话的来源 Host。")
        }
    }

    func controlConversationGoal(conversationID: String, control: AppleGoalControl?) async throws -> Bool {
        guard let owner = UUID(uuidString: conversationID) else { throw IOSApplicationRuntimeError.invalidResponse }
        if let provider = try uiProvider(conversationID: owner) {
            return try await provider.controlConversationGoal(conversationID: conversationID, control: control)
        }
        guard let execution = ensureExecutionForConversation(owner) else { throw IOSApplicationRuntimeError.invalidResponse }
        return try await execution.controlLocalGoal(control)
    }


    private func enabledLocalSkillSnapshots() -> [IOSAgentSkillSnapshot] {
        container.chatMemoryProfileStore.loadProfile().uploadedSkills.filter(\.enabled).map {
            IOSAgentSkillSnapshot(id: $0.id.uuidString, name: $0.name, summary: $0.description,
                instructions: $0.content, version: $0.version, requiredTools: $0.requiredTools ?? [],
                validationSteps: $0.validationSteps ?? [])
        }
    }

    private func prepareLocalGoalHostConfiguration(owner: UUID, session: String,
        model: IOSPreparedRunModel, invocation: IOSPreparedHostInvocation) async throws -> AgentCoreJSONValue {
        let runtime = container.applicationRuntime
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: .init(hostId: "ios-local", clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: session),
            isForeground: UIApplication.shared.applicationState == .active,
            scopedGrants: agentCorePermissionBroker.scopedGrants)
        let application = IOSCanonicalAgentApplication(store: runtime,
            descriptor: .init(conversationID: owner.uuidString, conversationTitle: "",
                identity: .init(userMessageID: UUID()), userText: "", userContentJSON: nil, contextCheckpointID: nil))
        let client: TextAgentModelClient
        if let capability = model.frozenModelCapability {
            client = try TextAgentModelClient(client: model.client, frozenModelCapability: capability)
        } else { client = TextAgentModelClient(client: model.client) }
        let coordinator = DeviceAgentCoordinator(modelClient: client,
            delegationSettings: try model.mobileDelegationSettings(
                projectRoot: runtime.databaseURL.deletingLastPathComponent().path,
                conversationID: owner.uuidString, using: runtime),
            hostExecutor: agentCoreHostExecutor, toolPlan: plan, modelSupportsToolCalls: model.supportsToolCalls,
            canonicalApplication: application, preparedHostInvocation: invocation,
            hostDependencyIdentity: IOSPreparedHostInvocation.skillIdentity(enabledLocalSkillSnapshots()))
        let policy = try container.conversationStore?.getConversation(conversationId: owner)?.permissionMode ?? .cautious
        return try await coordinator.prepareHostConfiguration(sessionID: session,
            config: .init(platform: .ios, executionMode: .deviceLocal, hostId: "ios-local", model: model.modelName,
                maxIterations: 0, contextWindowTokens: model.parameters.maxContextLength,
                reservedOutputTokens: model.parameters.maxOutputTokens, grantedCapabilities: [], permissionProfile: policy))
    }

    /// Core performs one cooperative scheduling step. All model/permission
    /// execution uses the same exact Run recovery path as ordinary cold starts.
    private func stepLocalGoal(_ control: AppleGoalControl? = nil, hostAvailable: Bool = true) async throws -> AgentCoreJSONValue {
        guard isConversationExecution, let owner = currentConversationId else { throw IOSApplicationRuntimeError.invalidResponse }
        let state = try? container.applicationRuntime.executeAgentCommand(.object([
            "type": .string("load_host_execution_state"), "conversation_id": .string(owner.uuidString)]))
        let host = state?.objectFields?["configuration"]?.objectFields?["host_catalog"]?.objectFields?["host_id"]?.stringScalar ?? "ios-local"
        guard ["ios-device", "ios-local"].contains(host) else { throw IOSApplicationRuntimeError.rejected("Goal belongs to another execution Host") }
        var selectedModel: AgentCoreJSONValue = .null
        var startConfiguration: AgentCoreJSONValue = .null
        if control?.action == .start {
            guard let model = try container.conversationStore?.getConversation(conversationId: owner)?.selectedModelInfo else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            let parameters = try generationParameters(for: model, conversationID: owner)
            let prepared = try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
                resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: self.container.applicationRuntime, parameters: parameters) })
            let current = try prepared.prepareHostInvocation()
            selectedModel = .object(["primary": current.route, "generation": current.generation,
                "output_admission": current.outputAdmission])
            let session = state?.objectFields?["configuration"]?.objectFields?["session_id"]?.stringScalar ?? owner.uuidString
            startConfiguration = try await prepareLocalGoalHostConfiguration(owner: owner, session: session,
                model: prepared, invocation: current)
        }
        var fields: [String: AgentCoreJSONValue] = ["type": .string("step_goal"),
            "conversation_id": .string(owner.uuidString), "source_host_id": .string(host), "control": .null, "host_available": .bool(hostAvailable), "selected_model": selectedModel,
            "start_configuration": startConfiguration]
        if let control {
            try control.validate()
            let intent: AgentCoreJSONValue = .object(["conversation_id": .string(owner.uuidString),
                "operation_id": .string(control.operationId), "action": .string(control.action.rawValue),
                "expected_goal_id": control.expectedGoalId.map(AgentCoreJSONValue.string) ?? .null,
                "expected_control_token": control.expectedControlToken.map(AgentCoreJSONValue.string) ?? .null,
                "objective": .string(control.objective ?? ""),
                "constraints": .array((control.constraints ?? []).map(AgentCoreJSONValue.string)),
                "wake_at_epoch_ms": control.wakeAtEpochMs.map(AgentCoreJSONValue.signedInteger) ?? .null])
            fields["control"] = try container.applicationRuntime.executeAgentCommand(.object(["type": .string("prepare_goal_control_intent"), "input": intent]))
        }
        let result = try container.applicationRuntime.executeAgentCommand(.object(fields))
        if let error = result.objectFields?["error"]?.stringScalar { throw IOSApplicationRuntimeError.rejected(error) }
        return result
    }

    private func controlLocalGoal(_ control: AppleGoalControl?) async throws -> Bool {
        guard UIApplication.shared.applicationState == .active else { throw CancellationError() }
        let result = try await stepLocalGoal(control)
        if result.objectFields?["interrupted_run_id"]?.stringScalar != nil {
            await deviceAgentCoordinator?.cancel(reason: "cancelled_by_user")
        }
        wakeLocalGoal(initial: result)
        return true
    }

    private func wakeLocalGoal(initial: AgentCoreJSONValue? = nil) {
        guard isConversationExecution, localGoalDriver == nil, remoteObservationForeground,
              UIApplication.shared.applicationState == .active, let owner = currentConversationId else { return }
        localGoalDriver = Task { [weak self] in
            guard let self else { return }
            defer { self.localGoalDriver = nil }
            do {
                var result: AgentCoreJSONValue
                if let initial { result = initial } else { result = try await self.stepLocalGoal() }
                while !Task.isCancelled, self.currentConversationId == owner,
                      self.remoteObservationForeground, UIApplication.shared.applicationState == .active {
                    let instruction = result.objectFields?["instruction"]?.objectFields
                    if instruction?["type"] == .string("execute_run") || instruction?["type"] == .string("needs_user"),
                       let run = instruction?["run_id"]?.stringScalar {
                        guard self.deviceAgentCoordinator == nil else { return }
                        await self.restoreActiveLocalRunIfNeeded()
                        let next = try await self.stepLocalGoal()
                        if next.objectFields?["instruction"]?.objectFields?["run_id"]?.stringScalar == run { return }
                        result = next
                    } else if instruction?["type"] == .string("wait"),
                              let wake = Self.embeddedInteger(instruction?["wake_at_epoch_ms"]) {
                        let delay = max(0, wake - Int64(Date().timeIntervalSince1970 * 1000))
                        try await Task.sleep(for: .milliseconds(delay))
                        guard self.remoteObservationForeground, UIApplication.shared.applicationState == .active else { return }
                        result = try await self.stepLocalGoal()
                    } else { return }
                }
            } catch {
                if self.currentConversationId == owner { self.processingState = .error(error.localizedDescription) }
            }
        }
    }

    private func pauseLocalGoalForLifecycle() async {
        guard isConversationExecution, let owner = currentConversationId, localGoalDriver != nil else { return }
        do {
            let result = try await stepLocalGoal(hostAvailable: false)
            if result.objectFields?["interrupted_run_id"]?.stringScalar != nil {
                await deviceAgentCoordinator?.cancel(reason: "cancelled_by_user")
            }
        } catch {
            // A pending Run remains owned and recoverable. Never report a
            // successful stop solely because this process is backgrounding.
            logger.log(.warn, "ChatManager", "Goal lifecycle pause remains unconfirmed")
        }
    }

    func loadConversationProgress(conversationID: String) async throws -> IOSInspectorProgressSnapshot {
        guard let owner = UUID(uuidString: conversationID) else { throw IOSApplicationRuntimeError.invalidResponse }
        if let provider = try uiProvider(conversationID: owner) {
            return try await provider.conversationProgress(conversationID: conversationID)
        }
        let runtime = container.applicationRuntime
        return try await Task.detached {
            var session = try runtime.executeAgentCommand(.object([
                "type": .string("load_session_projection"), "conversation_id": .string(conversationID)]))
            let observation = try runtime.executeAgentCommand(.object([
                "type": .string("load_goal_control_observation"), "conversation_id": .string(conversationID)]))
            session = try IOSInspectorProgressSnapshot.applyingGoalObservation(observation, to: session)
            let plan = try runtime.executeAgentCommand(.object([
                "type": .string("conversation_plan"), "command": .object([
                    "type": .string("get"), "conversation_id": .string(conversationID)])]))
            let decoder = JSONDecoder()
            return IOSInspectorProgressSnapshot(supportsGoalControl: true,
                facts: try decoder.decode(IOSInspectorProgressFacts?.self, from: JSONEncoder().encode(session)),
                plan: try decoder.decode(IOSInspectorProgressFacts.Plan?.self, from: JSONEncoder().encode(plan)))
        }.value
    }

    /// This user operation stays on the original parent source; child messages remain read-only.
    func inspectTaskRecovery(_ destination: AppleTaskChildDestination) throws
        -> (IOSUnknownEffectRecoveryInspection?, IOSPendingUnknownEffectRecovery?) {
        guard destination.hasValidIdentity, let parent = UUID(uuidString: destination.parentConversationID),
              try uiProvider(conversationID: parent) == nil else {
            throw IOSApplicationRuntimeError.rejected(String(localized: "task.recovery.source_host"))
        }
        let owner = AppleOwnedTaskRecoveryOwner(parent_conversation_id: destination.parentConversationID,
            parent_run_id: destination.parentRunID, task_id: destination.taskID)
        let inspection = try container.applicationRuntime.inspectOwnedTaskRecovery(owner)
        guard inspection == nil || inspection?.conversation_id == destination.childConversationID else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let pending = try container.applicationRuntime.pendingOwnedTaskRecovery(owner)
        return (inspection, pending)
    }

    func resolveTaskRecovery(_ destination: AppleTaskChildDestination,
                             inspection: IOSUnknownEffectRecoveryInspection?,
                             pending: IOSPendingUnknownEffectRecovery?, reason: String) throws -> IOSUnknownEffectRecoveryReply {
        // Recheck the original source and Core ownership before any saved intent is recovered.
        let current = try inspectTaskRecovery(destination)
        if let pending {
            guard current.1 == pending else { throw IOSApplicationRuntimeError.invalidResponse }
            return try container.applicationRuntime.recoverPendingUnknownEffectRecovery(pending)
        }
        guard let inspection, current.0 == inspection, current.1 == nil else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        return try container.applicationRuntime.resolveOwnedTaskRecovery(
            AppleOwnedTaskRecoveryOwner(parent_conversation_id: destination.parentConversationID,
                parent_run_id: destination.parentRunID, task_id: destination.taskID),
            decision: inspection.confirmed(choice: .keepUnknownBlocked, reason: reason))
    }

    /// Read-only child inspection keeps the parent's live execution and composer untouched.
    func loadTaskChildWindow(_ destination: AppleTaskChildDestination, beforeOrdinal: Int64?) async throws
        -> (window: IOSConversationProjectionWindowV1, messages: [ChatMessage]) {
        guard destination.hasValidIdentity,
              let parentID = UUID(uuidString: destination.parentConversationID) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let query = AppleTaskChildWindowRequest(destination: destination, beforeOrdinal: beforeOrdinal,
            limit: 30, contentByteLimit: 65536)
        let provider = try uiProvider(conversationID: parentID)
        let window: IOSConversationProjectionWindowV1
        if let provider {
            window = try await provider.taskChildWindow(query)
        } else {
            guard case .object(var fields) = try JSONDecoder().decode(AgentCoreJSONValue.self,
                from: JSONEncoder().encode(query)) else { throw IOSApplicationRuntimeError.invalidResponse }
            fields["type"] = .string("load_task_child_conversation_window")
            let result = try container.applicationRuntime.executeAgentCommand(.object(fields))
            window = try JSONDecoder().decode(IOSConversationProjectionWindowV1.self,
                from: JSONEncoder().encode(result))
        }
        guard window.projection.conversationID == destination.childConversationID,
              window.projection.executionOwnership == .taskChild(destination) else {
            throw IOSApplicationRuntimeError.rejected("Task result belongs to a different parent or task")
        }
        var messages = IOSConversationProjectionPresenter.messages(from: window.projection)
        for cursor in window.assistantJournalHeads {
            try Task.checkCancellation()
            guard cursor.identity.conversation_id == destination.childConversationID else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            let content: IOSJournalMessageContent
            if let provider { content = try await provider.taskChildJournalContent(at: cursor) }
            else {
                let reader = IOSAssistantJournalReader(store: container.applicationRuntime)
                content = try await reader.read(at: cursor)
            }
            ChatMessageDraftReducer.applyJournalContent(content, in: &messages,
                conversationID: destination.childConversationID)
        }
        return (window, messages)
    }

    func loadUISurfaces(conversationID: UUID) async throws -> [AppleUISnapshot<AgentCoreJSONValue>] {
        if let provider = try uiProvider(conversationID: conversationID) {
            guard let epoch = try uiSourceEpoch(conversationID: conversationID, provider: provider) else { return [] }
            return try await provider.listUISurfaces(conversationID: conversationID.uuidString, expectedSourceStoreEpoch: epoch)
        }
        let runtime = container.applicationRuntime
        return try await Task.detached { try runtime.listUISurfaces(conversationID: conversationID.uuidString) }.value
    }

    func previewUISurface(_ input: AppleUIPreviewInput<AgentCoreJSONValue>) async throws -> AppleUIPreviewResult<AgentCoreJSONValue> {
        guard let conversationID = UUID(uuidString: input.identity.owner.conversation_id) else { throw IOSApplicationRuntimeError.invalidResponse }
        if let provider = try uiProvider(conversationID: conversationID) { return try await provider.previewUISurface(input) }
        let runtime = container.applicationRuntime
        return try await Task.detached { try runtime.previewUISurface(input) }.value
    }

    func readUISurfaceChanges(identity: AppleUISurfaceIdentity, baseRevision: UInt64?) async throws -> AppleUISurfaceUpdate<AgentCoreJSONValue> {
        guard let conversationID = UUID(uuidString: identity.owner.conversation_id) else { throw IOSApplicationRuntimeError.invalidResponse }
        if let provider = try uiProvider(conversationID: conversationID) {
            return try await provider.readUISurfaceChanges(identity: identity, baseRevision: baseRevision)
        }
        let runtime = container.applicationRuntime
        return try await Task.detached { try runtime.readUISurfaceChanges(identity: identity, baseRevision: baseRevision) }.value
    }

    func readUISurfaceRevision(_ reference: AppleUISurfaceReference) async throws -> AppleUISnapshot<AgentCoreJSONValue>? {
        guard let conversationID = UUID(uuidString: reference.identity.owner.conversation_id) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        if let provider = try uiProvider(conversationID: conversationID) {
            return try await provider.readUISurfaceRevision(reference)
        }
        let runtime = container.applicationRuntime
        return try await Task.detached {
            try runtime.readUISurfaceRevision(identity: reference.identity, revision: reference.revision)
        }.value
    }

    private var uiIntentJournal: AppleUIIntentJournal<AgentCoreJSONValue> {
        let runtime = container.applicationRuntime
        return AppleUIIntentJournal(read: { key in
            try runtime.productSetting(key: key).map { ($0.valueJSON, $0.revision) }
        }, write: { key, json, revision in
            _ = try runtime.putProductSettingChecked(key: key, valueJSON: json, expectedRevision: revision,
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1000))
        })
    }

    private func uiIntentSource(identity: AppleUISurfaceIdentity) throws -> AppleUIIntentSource {
        guard let conversation = UUID(uuidString: identity.owner.conversation_id) else { throw IOSApplicationRuntimeError.invalidResponse }
        if let provider = try uiProvider(conversationID: conversation) { return provider.uiIntentSource }
        return AppleUIIntentSource(hostID: nil, address: container.applicationRuntime.databaseURL.path, certificate: nil, clientID: nil)
    }

    func pendingUIInteraction(identity: AppleUISurfaceIdentity) throws -> AppleUISubmission<AgentCoreJSONValue>? {
        try uiIntentJournal.pending(identity: identity, source: uiIntentSource(identity: identity))
    }

    func submitUIInteraction(_ input: AppleUISubmission<AgentCoreJSONValue>) async -> AppleUISubmissionOutcome<AgentCoreJSONValue> {
        do {
            let source = try uiIntentSource(identity: input.interaction.identity)
            let journal = uiIntentJournal
            try journal.stage(input, source: source)
            let outcome = await deliverUIInteraction(input, source: source)
            switch outcome {
            case .committed, .rejected: try journal.retire(input, source: source)
            case .unknown: break
            }
            return outcome
        } catch { return .unknown(error.localizedDescription) }
    }

    private func deliverUIInteraction(_ input: AppleUISubmission<AgentCoreJSONValue>, source: AppleUIIntentSource) async -> AppleUISubmissionOutcome<AgentCoreJSONValue> {
        guard let conversationID = UUID(uuidString: input.interaction.identity.owner.conversation_id) else {
            return .rejected("Invalid conversation identity")
        }
        var localCommandStarted = false
        do {
            guard try uiIntentSource(identity: input.interaction.identity) == source else {
                return .unknown("会话来源已改变，请连接原 Host 后查询提交结果。")
            }
            if let provider = try uiProvider(conversationID: conversationID) {
                let outcome = await provider.submitUIInteraction(input)
                if case .committed(let receipt) = outcome, receipt.queued_submission_id != nil {
                    let execution = ensureExecutionForConversation(conversationID)
                    execution?.remoteUIWakeSuppressed = false
                    execution?.wakeRemoteUIQueue()
                }
                return outcome
            }
            localCommandStarted = true
            let runtime = container.applicationRuntime
            let receipt = try await Task.detached { try runtime.submitUIInteraction(input) }.value
            if let commandID = receipt.queued_submission_id {
                dispatchQueuedUIInput(conversationID: conversationID, commandID: commandID)
            }
            return .committed(receipt)
        } catch IOSApplicationRuntimeError.rejected(let message) { return localCommandStarted ? .rejected(message) : .unknown(message) }
        catch IOSApplicationRuntimeError.revisionConflict { return .rejected("表单已更新，请重新确认后提交。") }
        catch { return .unknown(error.localizedDescription) }
    }

    private func readPermissionConfigurationOwner(conversationID: UUID, execution: ChatManager) throws -> IOSPermissionConfigurationOwner {
        guard let store = container.conversationStore as? any ConversationRemoteTurnStore else {
            execution.permissionConfigurationOwner = .unknown
            throw IOSApplicationRuntimeError.rejected("The canonical permission owner is unavailable")
        }
        let provider = execution.remoteSubmissionProviders[conversationID.uuidString]
            ?? (execution.effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider
        do {
            let intent = try store.remoteSubmissionIntent(conversationID: conversationID.uuidString)
            let knownRun = execution.observedRemoteRun?.run_id
                ?? (intent?.phase == "admitted" ? intent?.identity.submission_id : nil)
                ?? execution.agentCoreSnapshot?.turnId
            let facts = try IOSPermissionConfigurationOwner.read(conversationID: conversationID.uuidString,
                hostID: provider?.journalSourceHostID, clientID: provider?.observationClientID,
                unresolvedRunID: knownRun, sourceUnconfirmed: execution.remoteObservationUnresolved ||
                    intent?.phase == "prepared" || intent?.phase == "bound",
                execute: store.executeRemoteJournalCommand)
            if case .remote(let identity) = facts, let observed = execution.observedRemoteRun {
                guard observed.run_id == identity.submission_id,
                      observed.submission_receipt?.identity == identity,
                      observed.submission_receipt?.phase == "admitted" else { throw IOSApplicationRuntimeError.invalidResponse }
            }
            if let conversation = try container.conversationStore?.getConversation(conversationId: conversationID) {
                permissionCommittedModes[conversationID] = conversation.permissionMode
                execution.permissionCommittedModes[conversationID] = conversation.permissionMode
            }
            execution.permissionConfigurationOwner = facts
            return facts
        } catch {
            execution.permissionConfigurationOwner = .unknown
            throw error
        }
    }

    private func refreshPermissionOwnerProjection(conversationID: UUID) {
        guard let facts = try? readPermissionConfigurationOwner(conversationID: conversationID, execution: self),
              case .remote(let identity) = facts,
              let provider = remoteSubmissionProviders[conversationID.uuidString]
                ?? (effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider else { return }
        guard let requestID = permissionHydration[conversationID, default: .init()].begin(identity) else { return }
        Task { @MainActor [weak self] in
            guard let self else { return }
            defer { self.permissionHydration[conversationID, default: .init()].finish(identity, request: requestID, succeeded: false) }
            do {
                let saved = try await provider.permissionConfiguration(owner: identity)
                guard self.permissionConfigurationOwner == .remote(identity),
                      self.permissionHydration[conversationID, default: .init()].finish(identity, request: requestID, succeeded: true) else { return }
                self.remotePermissionConfigurations[conversationID] = saved
                self.remotePermissionCommitRevisions[conversationID] = saved.configuration_revision
                guard let mode = self.permissionCommittedModes[conversationID] else { return }
                self.observeRemotePermissionPolicy(conversationID: conversationID, execution: self,
                    provider: provider, identity: identity, saved: saved, mode: mode)
            } catch { /* Unconfirmed source remains checking, never a fabricated active ACK. */ }
        }
    }

    func permissionConfigurationStatus(conversationID: UUID?) -> String {
        guard let owner = conversationID else { return "saved" }
        let execution = executionForConversation(owner) ?? self
        guard let mode = execution.permissionCommittedModes[owner] ?? permissionCommittedModes[owner] else { return "checking" }
        switch execution.permissionConfigurationOwner {
        case .unknown: return "checking"
        case .inactive: return "saved"
        case .remote(let identity):
            guard let remote = execution.remotePermissionConfigurations[owner] ?? remotePermissionConfigurations[owner],
                  remote.active_run_id == nil || remote.active_run_id == identity.submission_id else { return "checking" }
            guard remote.configuration_revision >= (remotePermissionCommitRevisions[owner] ?? remote.configuration_revision) else { return "checking" }
            switch remote.state {
            case .inactive: return "saved"
            case .pending: return "pending"
            case .active: return "active"
            }
        case .local(let runID):
            guard execution.agentCoreSnapshot?.turnId == runID else { return "pending" }
        }
        let currentRun = execution.agentCoreSnapshot?.turnId
        let targetRevision = permissionConfigurationRunIDs[owner].flatMap {
            $0 == currentRun ? permissionConfigurationRevisions[owner] : nil
        }
        guard let snapshot = execution.agentCoreSnapshot,
              snapshot.sessionId.flatMap(UUID.init(uuidString:)) == owner,
              let policy = snapshot.permissions?.activePolicy,
              policy.runID == snapshot.turnId, policy.profile == mode,
              policy.configurationRevision >= (targetRevision ?? 0) else { return "pending" }
        return "active"
    }
    /// Transport replica state is intentionally not published. Full Core
    /// snapshots can be large and must not invalidate the entire SwiftUI chat
    /// hierarchy; views observe only derived presentation projections.
    @Published public private(set) var agentTaskProjection = AgentCoreTaskProjection()
    @Published public private(set) var agentSkillRuntimeProjection = AgentCoreSkillRuntimeProjection()
    @Published public private(set) var localAgentPermissionRequest: IOSAgentPermissionRequest?
    @Published public private(set) var localAgentScopedResourceRequest: IOSAgentPermissionRequest?
    private var localAgentPermissionQueue: [IOSAgentPermissionRequest] = []
    private var childPermissionCoordinators: [IOSChildPermissionOwner: DeviceAgentCoordinator] = [:]
    private var embeddedChildHosts: [String: IOSLocalTaskHost] = [:]
    private var embeddedChildWakeTask: Task<Void, Never>?
    private var inspectingEmbeddedChildWake = false
    private var embeddedChildWakeRequested = false
    /// Estimated token count for the current response (progressively updated during streaming)
    @Published public var currentTokenCount: Int = 0
    @Published public var selectedModelInfo: ModelInfo?
    @Published public var currentConversationId: UUID? {
        didSet {
            guard currentConversationId != oldValue else { return }
            if !isConversationExecution {
                selectConversationExecution()
                return
            }
            // Selection retires observation, not execution. The admitted Run
            // must still commit into its original conversation in background;
            // only a new submission or explicit cancellation supersedes it.
            remoteRunObserver.cancel()
            remoteTaskObservation.cancel()
            // Task revisions and Core session owners belong to this selection,
            // not to the ChatManager's lifetime or the previously selected Run.
            agentTaskProjection = AgentCoreTaskProjection()
            webAgentTaskProjectionReducer = AgentCoreTaskProjectionReducer()
            remoteResumeTask?.cancel()
            remoteResumeTask = nil
            remoteResumeID = nil
            remoteRecoveryGeneration &+= 1
            observedRemoteRun = nil
            permissionConfigurationOwner = .unknown
            remoteRunNeedsResume = false
            remoteSubmissionPending = false
            let selectedConversation = currentConversationId.flatMap {
                try? container.conversationStore?.getConversation(conversationId: $0)
            }
            remoteObservationUnresolved = currentConversationId != nil
                && selectedConversation?.selectedModelInfo?.isWebBackend == true
                && currentConversationId != knownUnsubmittedConversationID
            let selectedID = currentConversationId
            Task { [weak self] in
                guard let self, self.currentConversationId == selectedID else { return }
                if let selectedID, self.selectedModelInfo?.isWebBackend != true {
                    self.refreshPermissionOwnerProjection(conversationID: selectedID)
                    self.refreshIdleLocalTasks()
                }
                await self.restoreActiveLocalRunIfNeeded()
            }
        }
    }
    @Published public private(set) var currentExecutionBackend: String = "nola"
    @Published public var preferredNativeAppHostId: String = ""
    /// Optional warning text shown when model configuration is incomplete.
    @Published public var modelFallbackNotice: String? = nil
    /// When true, request upstream gateway to perform web retrieval before answering
    @Published public var enableRetrieval: Bool = false
    /// True while the client requested retrieval and we're waiting for the first token
    @Published public var retrievalInProgress: Bool = false
    /// Backup of last deleted conversation for possible undo (keeps Conversation metadata and messages)
    @Published public var lastDeletedConversationBackup: (conversation: Conversation, messages: [ConversationMessage], deletedAt: Date)?
    
    private var editedResendJobs: [UUID: Task<Void, Never>] = [:]
    private let container: ServiceContainer
    fileprivate var submissionCommitted: (@Sendable () -> Void)?
    private let isConversationExecution: Bool
    // Stable canonical owners, independent of selected tab and UUID-only UI models.
    private var automationExecutions: [String: ChatManager] = [:]
    private var executionClient: LLMClient?
    private var messageInFlight = false
    private var remoteQueuedUIIdentity: IOSCanonicalAgentTurnIdentity?
    private var remoteUIWakeTask: Task<Void, Never>?
    private var remoteUIWakeRequested = false
    private var remoteUIWakeSuppressed = false
    private var rejectedUIQueue: (conversationID: UUID, provider: WebAgentProvider, sourceStoreEpoch: String, receipt: IOSSourceSubmissionReceipt)?
    var canCancelRejectedUIQueue: Bool {
        if !isConversationExecution { return selectedExecution?.canCancelRejectedUIQueue ?? false }
        return rejectedUIQueue?.conversationID == currentConversationId
    }
    func cancelRejectedUIQueue() {
        if !isConversationExecution { selectedExecution?.cancelRejectedUIQueue(); return }
        guard let pending = rejectedUIQueue, pending.conversationID == currentConversationId else { return }
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                guard try self.uiProvider(conversationID: pending.conversationID)?.uiIntentSource == pending.provider.uiIntentSource,
                      try self.uiSourceEpoch(conversationID: pending.conversationID, provider: pending.provider) == pending.sourceStoreEpoch else {
                    throw IOSApplicationRuntimeError.rejected("The original queued form Host changed")
                }
                try await pending.provider.cancelQueuedUIInput(conversationID: pending.conversationID.uuidString,
                    expectedSourceStoreEpoch: pending.sourceStoreEpoch, commandID: pending.receipt.identity.submission_id, requestDigest: pending.receipt.request_digest)
                guard self.rejectedUIQueue?.receipt.identity == pending.receipt.identity,
                      self.rejectedUIQueue?.receipt.request_digest == pending.receipt.request_digest else { return }
                self.rejectedUIQueue = nil
                self.remoteUIWakeSuppressed = false
                if self.currentConversationId == pending.conversationID,
                   (try? self.uiProvider(conversationID: pending.conversationID)?.uiIntentSource) == pending.provider.uiIntentSource {
                    self.remoteRunObservationError = nil
                    self.wakeRemoteUIQueue()
                }
            } catch {
                if self.currentConversationId == pending.conversationID,
                   self.rejectedUIQueue?.receipt.identity == pending.receipt.identity,
                   self.rejectedUIQueue?.receipt.request_digest == pending.receipt.request_digest {
                    self.remoteRunObservationError = error.localizedDescription
                }
            }
        }
    }

    private func wakeRemoteUIQueue() {
        guard !remoteUIWakeSuppressed else { return }
        remoteUIWakeRequested = true
        guard isConversationExecution, !messageInFlight, remoteUIWakeTask == nil, remoteObservationForeground,
              let conversationID = currentConversationId else { return }
        remoteUIWakeTask = Task { @MainActor [weak self] in
            guard let self else { return }
            self.remoteUIWakeRequested = false
            defer {
                self.remoteUIWakeTask = nil
                if self.remoteUIWakeRequested { self.wakeRemoteUIQueue() }
            }
            do {
                var dispatchOwner: (IOSRemoteSubmissionIdentity, String)?
                while !Task.isCancelled && !self.remoteUIWakeSuppressed && self.remoteObservationForeground && self.currentConversationId == conversationID {
                    guard !self.messageInFlight, !self.processingState.blocksInput,
                          !self.remoteObservationUnresolved, self.observedRemoteRun == nil,
                          self.deviceAgentCoordinator == nil,
                          let store = self.container.conversationStore as? any ConversationRemoteTurnStore,
                          let conversation = try self.container.conversationStore?.getConversation(conversationId: conversationID),
                          conversation.executionBackend == "nola",
                          let provider = try self.uiProvider(conversationID: conversationID) else { return }
                    if let pending = try store.remoteSubmissionIntent(conversationID: conversationID.uuidString),
                       pending.phase == "prepared" || pending.phase == "bound" { return }
                    guard let sourceEpoch = try self.uiSourceEpoch(conversationID: conversationID, provider: provider) else { return }
                    let queue = try await provider.idleUIQueueHead(conversationID: conversationID.uuidString, expectedSourceStoreEpoch: sourceEpoch)
                    guard try self.uiSourceEpoch(conversationID: conversationID, provider: provider) == sourceEpoch else {
                        throw IOSApplicationRuntimeError.invalidResponse
                    }
                    guard let head = queue.head,
                          head.kind == "user_input", head.user_message_id == "ui:\(head.command_id):user" else { return }
                    if let dispatch = queue.dispatch {
                        if let owner = dispatchOwner {
                            guard owner.0 == dispatch.identity, owner.1 == dispatch.request_digest else { throw IOSApplicationRuntimeError.invalidResponse }
                        } else { dispatchOwner = (dispatch.identity, dispatch.request_digest) }
                        guard let observed = try await provider.submissionReceipt(identity: dispatch.identity,
                            digest: dispatch.request_digest, cancel: false) else {
                            throw IOSApplicationRuntimeError.rejected("The original form dispatch receipt is unavailable")
                        }
                        guard !Task.isCancelled, self.currentConversationId == conversationID,
                              !self.remoteUIWakeSuppressed, self.remoteObservationForeground,
                              try self.uiProvider(conversationID: conversationID)?.uiIntentSource == provider.uiIntentSource else { return }
                        switch observed.phase {
                        case "admitted":
                            try store.adoptObservedRemoteRun(observed)
                            self.startRemoteRunObservation(provider: provider, store: store, conversationID: conversationID)
                            return
                        case "rejected", "cancelled":
                            self.rejectedUIQueue = (conversationID, provider, sourceEpoch, observed)
                            self.remoteUIWakeSuppressed = true
                            self.remoteRunObservationError = "表单操作未执行，原请求已被拒绝或取消。可移除此排队操作。"
                            return
                        case "prepared":
                            try await Task.sleep(nanoseconds: 1_000_000_000)
                            continue
                        default: throw IOSApplicationRuntimeError.invalidResponse
                        }
                    }
                    guard dispatchOwner == nil else { throw IOSApplicationRuntimeError.invalidResponse }
                    let now = Int64(Date().timeIntervalSince1970 * 1_000)
                    if let until = head.lease_until_ms, until > now {
                        try await Task.sleep(nanoseconds: UInt64(min(until - now, 30_000)) * 1_000_000)
                        continue
                    }
                    try Task.checkCancellation()
                    guard !self.remoteUIWakeSuppressed, self.remoteObservationForeground, self.currentConversationId == conversationID,
                          try self.uiProvider(conversationID: conversationID)?.uiIntentSource == provider.uiIntentSource,
                          ((self.effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider)?.uiIntentSource == provider.uiIntentSource else { return }
                    self.remoteQueuedUIIdentity = .init(queuedSubmissionID: head.command_id,
                        userMessageID: "ui:\(head.command_id):user")
                    defer { self.remoteQueuedUIIdentity = nil }
                    _ = try await self.processMessage("表单操作", inputOrigin: .interactiveUser)
                    return
                }
            } catch is CancellationError { }
            catch {
                self.logger.log(.warn, "ChatManager", "Remote UI queue remains pending", metadata: ["error": error.localizedDescription])
            }
        }
    }

    private var queuedUIInput: (admission: IOSQueuedUserAdmission, identity: IOSCanonicalAgentTurnIdentity)?
    private var queuedSuccessorReady = false
    private var queuedSuccessorTask: Task<Void, Never>?

    /// Finish the current owner before admitting another Run. The canonical
    /// checkpoint is the queue; this task is only a foreground wake-up.
    private func scheduleQueuedSuccessorIfReady() {
        guard isConversationExecution, queuedSuccessorReady, queuedSuccessorTask == nil,
              !messageInFlight, deviceAgentCoordinator == nil,
              let conversationID = currentConversationId else { return }
        queuedSuccessorTask = Task { [weak self] in
            guard let self else { return }
            defer { self.queuedSuccessorTask = nil }
            do {
                while self.queuedSuccessorReady && !Task.isCancelled {
                    guard self.currentConversationId == conversationID,
                          !self.messageInFlight, self.deviceAgentCoordinator == nil else { return }
                    self.queuedSuccessorReady = false
                    let runtime = self.container.applicationRuntime
                    let accepted = try runtime.executeAgentCommand(.object([
                        "type": .string("promote_queued_assistant_turn"),
                        "input": .object(["conversation_id": .string(conversationID.uuidString),
                            "created_at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000))])]))
                    if accepted == .null {
                        // An action can arrive after the predecessor's last
                        // boundary poll. It remains in the external FIFO, not
                        // the checkpoint successor queue just inspected.
                        let head = try runtime.executeAgentCommand(.object([
                            "type": .string("peek_agent_command"),
                            "session_id": .string(conversationID.uuidString)]))
                        guard case .object(let headFields) = head,
                              headFields["kind"] == .string("user_input"),
                              case .string(let commandID)? = headFields["command_id"] else { return }
                        try await self.processQueuedUIInput(commandID: commandID)
                        continue
                    }
                    guard case .object(let fields) = accepted,
                          case .object(let begin)? = fields["begin"],
                          begin["conversation_id"] == .string(conversationID.uuidString),
                          case .string(let runID)? = begin["run_id"],
                          case .string(let userID)? = begin["user_message_id"],
                          begin["assistant_message_id"] == .string("\(runID):assistant") else {
                        throw DeviceAgentCoordinatorError.invalidTransition
                    }
                    let identity = IOSCanonicalAgentTurnIdentity(queuedSubmissionID: runID, userMessageID: userID)
                    let application = IOSCanonicalAgentApplication(store: runtime,
                        descriptor: .init(conversationID: conversationID.uuidString, conversationTitle: "",
                            identity: identity, userText: "", userContentJSON: nil, contextCheckpointID: nil,
                            boundRunID: runID))
                    let frozen = try application.loadExactRunInvocation()
                    guard let sessionID = frozen.coreSessionID else { throw DeviceAgentCoordinatorError.invalidTransition }
                    let prepared = try IOSPreparedRunModel.recover(frozen, using: runtime)
                    self.activeGenerationId &+= 1
                    self.observeCommittedAssistant(conversationID: conversationID.uuidString,
                        assistantMessageID: identity.assistantMessageID, generation: self.activeGenerationId)
                    try Task.checkCancellation()
                    self.messageInFlight = true
                    do {
                        _ = try await self.runDeviceAgentTurn(sessionId: sessionID, input: "", history: [],
                            conversationId: conversationID, preparedModel: prepared, turnIdentity: identity,
                            persistedUserText: "", userContentJSON: nil, inputOrigin: .interactiveUser, recovering: frozen)
                        self.messageInFlight = false
                    } catch {
                        self.messageInFlight = false
                        throw error
                    }
                }
            } catch {
                // Admission may already be committed. Preserve that exact Run;
                // never retry it as a fresh composer submission.
                self.logger.log(.warn, "ChatManager", "Queued successor requires recovery",
                    metadata: ["conversationId": conversationID.uuidString, "error": error.localizedDescription])
                self.processingState = .error(error.localizedDescription)
            }
        }
    }

    private var effectiveLLMClient: LLMClient { executionClient ?? container.llmClient }
    private var executionContextManager: ContextManager?
    private var effectiveContextManager: ContextManager? {
        guard isConversationExecution else { return container.contextManager }
        if executionContextManager == nil, let store = container.conversationStore {
            let context = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)
            context.urlFetcher = container.contextManager?.urlFetcher
            context.dataSources = container.contextManager?.dataSources
            context.localJSONStorage = container.contextManager?.localJSONStorage
            executionContextManager = context
        }
        executionContextManager?.llmClient = effectiveLLMClient
        return executionContextManager
    }
    private var conversationExecutions: [UUID: ChatManager] = [:]
    /// Creation provenance is valid only within this process and until the
    /// first submission. An empty restored transcript is NOT proof of no Run.
    private var newlyCreatedConversationID: UUID?
    private var knownUnsubmittedConversationID: UUID?
    private var selectedExecutionSubscriptions = Set<AnyCancellable>()
    private var selectedExecution: ChatManager? {
        currentConversationId.flatMap { conversationExecutions[$0] }
    }
    var canBeginNewMessage: Bool {
        if !isConversationExecution { return unknownEffectPendingConversationID == nil && (selectedExecution?.canBeginNewMessage ?? true) }
        return !messageInFlight && !processingState.blocksInput && deviceAgentCoordinator == nil
            && !remoteObservationUnresolved && observedRemoteRun == nil
            && !unknownEffectRecoveryChecking
            && (unknownEffectRecovery == nil || unknownEffectRecovery?.phase == .blocked || unknownEffectRecovery?.phase == .rejected)
    }
    func executionForConversation(_ conversationID: UUID, prepareForSubmission: Bool = false) -> ChatManager? {
        if isConversationExecution { return currentConversationId == conversationID ? self : nil }
        guard let session = conversationExecutions[conversationID] else { return nil }
        if prepareForSubmission, currentConversationId == conversationID, !session.processingState.blocksInput,
           session.deviceAgentCoordinator == nil, session.selectedModelInfo != selectedModelInfo {
            session.selectedModelInfo = selectedModelInfo
            session.executionClient = (container.llmClient as? ModelService)?.makeScopedClient()
                ?? ModelService(provider: container.llmClient)
        }
        return session
    }

    /// Opens a canonical execution owner without changing navigation or another owner's provider.
    func ensureExecutionForConversation(_ conversationID: UUID) -> ChatManager? {
        if isConversationExecution { return currentConversationId == conversationID ? self : nil }
        if let existing = conversationExecutions[conversationID] { return existing }
        guard let conversation = conversation(for: conversationID) else { return nil }
        let session = ChatManager(container: container, restorePersistedState: false, isConversationExecution: true)
        session.selectedModelInfo = conversation.selectedModelInfo
        session.currentExecutionBackend = conversation.executionBackend
        session.executionClient = ModelService(provider: PlaceholderLocalProvider.shared)
        do {
            guard let model = conversation.selectedModelInfo else { throw NolaError.llm(.notConfigured) }
            if model.isWebBackend {
                guard let url = model.baseURL.flatMap(URL.init(string:)),
                      let token = model.resolvedAPIKey, !token.isEmpty else { throw NolaError.llm(.notConfigured) }
                let config = WebAgentProvider.Config(
                    baseURL: url, token: token, model: model.effectiveUpstreamModelId,
                    pinnedCertificateSHA256: model.tlsCertificateSHA256,
                    expectedHostId: ModelSelectionManager.pairedWebHostIdentity(for: url))
                session.executionClient = (container.llmClient as? ModelService)?.makeScopedWebAgentClient(config: config)
                    ?? ModelService(provider: WebAgentProvider(config: config))
            } else {
                let parameters = try generationParameters(for: model, conversationID: conversationID)
                if model.isLocal {
                    session.executionClient = ModelService(provider: try IOSConversationRunConfiguration.localProvider(
                        model: model, parameters: parameters))
                } else {
                    let resolved = try ModelSelectionManager.resolvedCloudProfileForRun(model, using: container.applicationRuntime, parameters: parameters)
                    var configuration = try IOSConversationRunConfiguration.directCloudProviderConfig(model: resolved.model, parameters: parameters,
                        reasoningSelection: resolved.reasoningSelection, reasoningTier: resolved.reasoningTier)
                    configuration.reasoningSummarySupported = resolved.reasoningSummarySupported
                    session.executionClient = ModelService(provider: OpenAIProvider(config: configuration))
                }
            }
        } catch {
            session.modelFallbackNotice = error.localizedDescription
        }
        conversationExecutions[conversationID] = session
        if newlyCreatedConversationID == conversationID { session.knownUnsubmittedConversationID = conversationID }
        session.currentConversationId = conversationID
        return session
    }
    private let logger = Logger.shared
    private var roomPreparedModel: IOSPreparedRunModel?
    private var roomExpertEntry: AppleExpertMessageEntry<AgentCoreJSONValue>?
    private var roomCapturedInstructions: [AppleExpertPromptDefinition<AgentCoreJSONValue>]?
    private var roomCanonicalIdentity: IOSCanonicalAgentTurnIdentity?
    private var roomRemotePreparationOnly = false
    private var roomRemoteReplayIdentity: IOSRemoteSubmissionIdentity?

    func processExpertMessage(_ text: String, submissionID: String, entry: AppleExpertMessageEntry<AgentCoreJSONValue>) async throws {
        guard roomCanonicalIdentity == nil, !processingState.blocksInput,
              let uuid = UUID(uuidString: submissionID) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        guard entry.child_submission_id == submissionID, entry.participant.context_conversation_id == currentConversationId?.uuidString else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let prepared = try IOSPreparedRunModel.prepareExpert(entry, using: container.applicationRuntime)
        let captured: [AppleExpertPromptDefinition<AgentCoreJSONValue>]? = try container.applicationRuntime.expertHostOperation(
            .loadChildInstructions(conversationID: entry.participant.context_conversation_id, expectedRunID: entry.child_submission_id, authority: .local()))
        guard let captured else { throw IOSApplicationRuntimeError.rejected("The original expert instruction capture is unavailable") }
        let previousClient = executionClient
        roomPreparedModel = prepared
        roomExpertEntry = entry
        roomCapturedInstructions = captured
        executionClient = ModelService(provider: prepared.client)
        roomCanonicalIdentity = IOSCanonicalAgentTurnIdentity(queuedSubmissionID: submissionID,
            userMessageID: "\(submissionID):user")
        defer { roomCanonicalIdentity = nil; roomPreparedModel = nil; roomExpertEntry = nil; roomCapturedInstructions = nil; executionClient = previousClient }
        _ = try await processMessage(text, userMessageID: uuid)
    }

    func cancelPreparedRoomRequest(executionID: String) async throws {
        guard isConversationExecution, let context = currentConversationId?.uuidString,
              let store = container.conversationStore as? any ConversationRemoteTurnStore,
              let pending = try store.remoteSubmissionIntent(conversationID: context),
              pending.identity.submission_id == executionID,
              pending.phase == "prepared" || pending.phase == "bound" else { return }
        guard let provider = remoteSubmissionProviders[context]
                ?? (effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider else {
            throw IOSApplicationRuntimeError.rejected("The original paired Host is unavailable for cancellation")
        }
        _ = try await refreshRemoteSubmission(provider: provider, store: store,
            conversationID: context, cancel: true, expectedIdentity: pending.identity)
    }

    func prepareRemoteRoomExecution(text: String, executionID: UUID) async throws -> IOSRemoteSubmissionIntent {
        guard isConversationExecution, let context = currentConversationId,
              getCurrentModel()?.isWebBackend == true,
              let store = container.conversationStore as? any ConversationRemoteTurnStore else {
            throw IOSApplicationRuntimeError.rejected("Remote Room owner is unavailable")
        }
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: executionID)
        if let existing = try store.remoteSubmissionIntent(conversationID: context.uuidString),
           existing.identity.submission_id == identity.submissionID, existing.request_digest != nil {
            return existing
        }
        roomRemotePreparationOnly = true
        defer { roomRemotePreparationOnly = false }
        _ = try await processMessage(text, userMessageID: executionID)
        guard let bound = try store.remoteSubmissionIntent(conversationID: context.uuidString),
              bound.identity.submission_id == identity.submissionID, bound.phase == "bound", bound.request_digest != nil else {
            throw IOSApplicationRuntimeError.rejected("Room request was not durably frozen")
        }
        return bound
    }

    func executeRemoteRoomRequest(_ remote: IOSAgentRoomService.ExecutionIntent.Envelope.RemoteHost, text: String) async throws {
        guard isConversationExecution, currentConversationId?.uuidString == remote.identity.conversation_id,
              let provider = (effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider,
              provider.journalSourceHostID == remote.identity.source_host_id,
              provider.observationClientID == remote.identity.client_id,
              let store = container.conversationStore as? any ConversationRemoteTurnStore,
              let original = try store.remoteSubmissionIntent(conversationID: remote.identity.conversation_id),
              original.identity == remote.identity, original.request_digest == remote.request_digest,
              let executionID = UUID(uuidString: remote.identity.submission_id) else {
            throw IOSApplicationRuntimeError.rejected("The original paired Room request is unavailable")
        }
        let observed = try await refreshRemoteSubmission(provider: provider, store: store,
            conversationID: remote.identity.conversation_id, cancel: false)
        if observed?.phase == "admitted" {
            await restoreRemoteJournalIfNeeded()
            return
        }
        guard observed?.phase == "bound" else {
            throw IOSApplicationRuntimeError.rejected("The original Room request is no longer dispatchable")
        }
        roomRemoteReplayIdentity = remote.identity
        defer { roomRemoteReplayIdentity = nil }
        // Source observation above proved no admission. The bound original is
        // replayed below; current tools/model/context are not used on the wire.
        processingState = .idle
        _ = try await processMessage(text, userMessageID: executionID)
    }
    /// Room recovery attaches only to this owner's exact canonical Run. It
    /// never begins another turn and preserves unknown-effect decision gates.
    func recoverRoomExecution(executionID: String) async throws {
        guard isConversationExecution, let conversationID = currentConversationId else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        if getCurrentModel()?.isWebBackend == true {
            await restoreRemoteJournalIfNeeded()
            return
        }
        await discoverUnknownEffectRecovery()
        guard deviceAgentCoordinator == nil, !messageInFlight,
              !unknownEffectRecoveryChecking, unknownEffectRecovery == nil,
              unknownEffectRecoveryError == nil, unknownEffectPendingConversationID == nil else { return }
        let projection = try container.applicationRuntime.loadConversationProjectionV1(id: conversationID.uuidString)
        guard projection.activeRun?.runID == executionID else { return }
        let identity = try IOSCanonicalAgentApplication.recoveryIdentity(store: container.applicationRuntime,
            conversationID: conversationID.uuidString, expectedRunID: executionID)
        let application = IOSCanonicalAgentApplication(store: container.applicationRuntime,
            descriptor: .init(conversationID: conversationID.uuidString, conversationTitle: "",
                identity: identity, userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: executionID))
        let frozen = try application.loadExactRunInvocation()
        guard let sessionID = frozen.coreSessionID else { throw DeviceAgentCoordinatorError.invalidTransition }
        let prepared = try IOSPreparedRunModel.recover(frozen, using: container.applicationRuntime)
        observeCommittedAssistant(conversationID: conversationID.uuidString,
            assistantMessageID: identity.assistantMessageID, generation: activeGenerationId)
        messageInFlight = true
        defer {
            messageInFlight = false
            scheduleQueuedSuccessorIfReady()
        }
        _ = try await runDeviceAgentTurn(sessionId: sessionID, input: "", history: [],
            conversationId: conversationID, preparedModel: prepared, turnIdentity: identity,
            persistedUserText: "", userContentJSON: nil, inputOrigin: .systemRecovery, recovering: frozen)
    }
    private func refreshCommittedContextUsage(conversationID: UUID) {
        guard currentConversationId == conversationID else { return }
        guard let usage = restoredCoreContextUsageEvidence(conversationId: conversationID) else {
            latestResponseMeta.removeValue(forKey: "contextUsedTokens")
            latestResponseMeta.removeValue(forKey: "contextPromptLimit")
            return
        }
        latestResponseMeta["contextUsedTokens"] = String(usage.usedTokens)
        latestResponseMeta["contextPromptLimit"] = String(usage.limitTokens)
    }

    /// Monotonically incremented each time `processMessage` is invoked. Older
    /// in-flight stream loops compare against this and bail out without
    /// publishing UI state or persisting the assistant message, preventing
    /// the "two streams writing to currentResponse + double-saved reply"
    /// race when the user sends a new message before the previous one
    /// finishes streaming.
    private var activeGenerationId: UInt64 = 0
    private var committedAssistantOwner: IOSCommittedAssistantOwner?
    private lazy var committedJournalReader = IOSAssistantJournalReader(store: container.applicationRuntime)
    private var committedJournalReadTask: Task<Void, Never>?
    private var pendingJournalCursors: [IOSAssistantJournalIdentity: IOSAssistantJournalCursor] = [:]
    private let hostPermissionSession = IOSHostPermissionSession()
    private var bridgeScopedResourceContinuation: CheckedContinuation<AgentCoreJSONValue?, Never>?
    /// Pending UI choice only. Core owns and persists conversation-level permission rules.
    private var localAgentPermissionDecisions: [String: AgentCorePermissionDecision] = [:]
    private var deviceAgentCoordinator: DeviceAgentCoordinator?
    private var activeHostConversationId: String?
    private var remoteSubmissionProviders: [String: WebAgentProvider] = [:]
    private let remoteTaskObservation = IOSRemoteTaskObservation()
    private let remoteRunObserver = IOSRemoteRunObserver()
    private var remoteResumeTask: Task<Void, Never>?
    private var remoteResumeID: UUID?
    private var remoteStreamGeneration: UInt64?
    private var remoteRecoveryGeneration: UInt64 = 0
    private var observedRemoteRun: IOSRunObservation?
    private var remoteObservationUnresolved = false
    private var localGoalDriver: Task<Void, Never>?
    private var remoteObservationForeground = true
    private var remoteLifecycleSubscriptions = Set<AnyCancellable>()
    @Published public private(set) var remoteRunNeedsResume = false
    @Published public private(set) var remoteRunObservationError: String?
    @Published public private(set) var remoteSubmissionPending = false
    private var agentTraceReducer = AgentTraceReducer()
    private var agentTraceContext = AgentTraceContext(conversationId: "")
    private var webAgentTaskProjectionReducer = AgentCoreTaskProjectionReducer()
    private var webAgentSkillRuntimeReducer = AgentCoreSkillRuntimeProjectionReducer()
    private let agentCoreHostExecutor = IOSAgentHostExecutor()
    private let agentCorePermissionBroker = IOSAgentPermissionBroker()

    /// Each session owns its reducer, transports, permission continuations and
    /// generation fence. Selection changes only which published values we observe.
    private func selectConversationExecution() {
        selectedExecutionSubscriptions.removeAll()
        guard let id = currentConversationId else {
            unknownEffectRecovery = nil
            unknownEffectRecoveryError = nil
            unknownEffectRecoveryChecking = false
            unknownEffectPendingConversationID = nil
            processingState = .idle
            currentResponse = ""
            thinking = ""
            hostToolEvents = []
            pendingHostPermissionRequest = nil
            pendingHostPermissionConversationId = nil
            localAgentPermissionRequest = nil
            localAgentScopedResourceRequest = nil
            committedAssistantProjection = nil
            committedAssistantJournalContent = nil
            committedJournalPresentation = nil
            agentTraceProjection = AgentRunProjection()
            agentTaskProjection = AgentCoreTaskProjection()
            agentSkillRuntimeProjection = AgentCoreSkillRuntimeProjection()
            agentCoreSnapshot = nil
            permissionConfigurationOwner = .inactive
            latestResponseMeta = [:]
            currentTokenCount = 0
            retrievalInProgress = false
            thinkingFinished = false
            sourceReferences = []
            hostPermissionSubmissionError = nil
            submittingHostPermissionRequestID = nil
            modelFallbackNotice = nil
            remoteRunNeedsResume = false
            remoteSubmissionPending = false
            remoteRunObservationError = nil
            return
        }
        guard let session = ensureExecutionForConversation(id) else { return }
        selectedModelInfo = session.selectedModelInfo
        if let model = session.selectedModelInfo { applyModelToContainer(model) }
        session.$unknownEffectRecovery.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.unknownEffectRecovery = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$unknownEffectRecoveryError.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.unknownEffectRecoveryError = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$unknownEffectRecoveryChecking.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.unknownEffectRecoveryChecking = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$unknownEffectPendingConversationID.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.unknownEffectPendingConversationID = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$processingState.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.processingState = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$permissionConfigurationOwner.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.permissionConfigurationOwner = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$remotePermissionConfigurations.sink { [weak self, weak session] values in
            guard let self, let session, self.selectedExecution === session else { return }
            for (owner, value) in values { self.remotePermissionConfigurations[owner] = value }
        }.store(in: &selectedExecutionSubscriptions)
        session.$currentResponse.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.currentResponse = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$thinking.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.thinking = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$showThinking.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.showThinking = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$thinkingFinished.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.thinkingFinished = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$hostToolEvents.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.hostToolEvents = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$sourceReferences.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.sourceReferences = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$committedAssistantProjection.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.committedAssistantProjection = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$committedAssistantJournalContent.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.committedAssistantJournalContent = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$committedJournalPresentation.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.committedJournalPresentation = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$agentTraceProjection.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.agentTraceProjection = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$latestResponseMeta.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.latestResponseMeta = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$submittingHostPermissionRequestID.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.submittingHostPermissionRequestID = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$hostPermissionSubmissionError.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.hostPermissionSubmissionError = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$pendingHostPermissionRequest.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.pendingHostPermissionRequest = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$pendingHostPermissionConversationId.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.pendingHostPermissionConversationId = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$agentCoreSnapshot.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.agentCoreSnapshot = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$agentTaskProjection.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.agentTaskProjection = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$agentSkillRuntimeProjection.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.agentSkillRuntimeProjection = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$localAgentPermissionRequest.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.localAgentPermissionRequest = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$localAgentScopedResourceRequest.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.localAgentScopedResourceRequest = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$currentTokenCount.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.currentTokenCount = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$modelFallbackNotice.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.modelFallbackNotice = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$retrievalInProgress.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.retrievalInProgress = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$remoteRunNeedsResume.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.remoteRunNeedsResume = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$remoteRunObservationError.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.remoteRunObservationError = value
        }.store(in: &selectedExecutionSubscriptions)
        session.$remoteSubmissionPending.sink { [weak self, weak session] value in
            guard let self, let session, self.selectedExecution === session else { return }
            self.remoteSubmissionPending = value
        }.store(in: &selectedExecutionSubscriptions)
        if session.selectedModelInfo?.isWebBackend != true {
            session.refreshPermissionOwnerProjection(conversationID: id)
        }
    }

    private func isDefaultConversationTitle(_ title: String?) -> Bool {
        let normalized = (title ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.isEmpty
            || normalized == L10n.chatConversationDefaultTitle
            || normalized == L10n.chatConversationRecoveredTitle
            || normalized == "新对话"
            || normalized == "会话"
            || normalized == "Conversation"
    }

    private func firstQueryTitle(from message: String) -> String? {
        let title = ChatTextPolicy.normalizedConversationTitle(from: message)
        return title.isEmpty ? nil : title
    }

    private var isRunningUnitTests: Bool {
        NSClassFromString("XCTestCase") != nil || NSClassFromString("XCTest.XCTestCase") != nil
    }

    private func beginAgentTrace(conversationId: UUID?, assistantMessageID: String?) {
        let conversationIdentity = conversationId?.uuidString ?? ""
        let messageIdentity = assistantMessageID
        if agentTraceReducer.projection.lastSequence == 0,
           agentTraceContext.conversationId == conversationIdentity,
           agentTraceContext.messageId == messageIdentity {
            return
        }
        agentTraceReducer = AgentTraceReducer()
        webAgentTaskProjectionReducer = AgentCoreTaskProjectionReducer()
        webAgentSkillRuntimeReducer = AgentCoreSkillRuntimeProjectionReducer()
        agentTaskProjection = AgentCoreTaskProjection()
        agentSkillRuntimeProjection = AgentCoreSkillRuntimeProjection()
        agentTraceContext = AgentTraceContext(
            conversationId: conversationIdentity,
            messageId: messageIdentity
        )
        let projection = agentTraceReducer.projection
        if !projection.isVisuallyEquivalent(to: agentTraceProjection) {
            agentTraceProjection = projection
        }
        currentResponse = ""
        thinking = ""
        let projectedToolEvents = projection.hostToolEvents
        if !projectedToolEvents.isEmpty, hostToolEvents != projectedToolEvents {
            hostToolEvents = projectedToolEvents
        }
    }

    /// Creates the assistant run before work starts so SwiftUI can install one
    /// stable bubble identity before the first transport event arrives.
    public func prepareAgentTrace(
        conversationId: UUID?,
        canonicalAssistantMessageID: String
    ) {
        if !isConversationExecution {
            conversationId.flatMap { conversationExecutions[$0] }?.prepareAgentTrace(
                conversationId: conversationId, canonicalAssistantMessageID: canonicalAssistantMessageID)
            return
        }
        beginAgentTrace(
            conversationId: conversationId,
            assistantMessageID: canonicalAssistantMessageID
        )
    }

    @discardableResult
    private func applyAgentTrace(_ event: AgentTraceEvent) -> AgentTraceApplyResult {
        let result = agentTraceReducer.apply(event)
        guard result == .applied else {
            let reason: String
            switch result {
            case .gap(let expected, let received):
                reason = "gap expected=\(expected) received=\(received)"
            case .outOfOrder(let last, let received):
                reason = "out_of_order last=\(last) received=\(received)"
            case .duplicate:
                reason = "duplicate"
            case .applied:
                reason = "applied"
            }
            logger.log(.warn, "ChatManager", "Agent trace event was not applied", metadata: [
                "kind": event.kind.rawValue,
                "sequence": String(event.sequence),
                "eventId": event.eventId,
                "reason": reason,
            ])
            return result
        }
        let projection = agentTraceReducer.projection
        if !projection.isVisuallyEquivalent(to: agentTraceProjection) {
            agentTraceProjection = projection
        }
        // Both local and remote traces are diagnostics. Text and reasoning
        // come exclusively from committed journal reads, never trace deltas.
        let projectedToolEvents = projection.hostToolEvents
        if hostToolEvents != projectedToolEvents {
            hostToolEvents = projectedToolEvents
        }
        if let usage = projection.usage,
           let conversationId = currentConversationId,
           usage.promptTokenLimit > 0 || usage.contextWindowTokens > 0 {
            // Trace is only a wakeup. Request identity, diagnostics and billing
            // are read together from their committed canonical sources.
            refreshCommittedContextUsage(conversationID: conversationId)
        }
        return result
    }

    private func observeCommittedAssistant(
        conversationID: String, assistantMessageID: String, generation: UInt64
    ) {
        committedAssistantOwner = IOSCommittedAssistantOwner(
            conversationID: conversationID, assistantMessageID: assistantMessageID,
            generation: generation
        )
        committedAssistantProjection = nil
    }

    func ownsCommittedAssistantProjection(_ snapshot: IOSCommittedAssistantProjection) -> Bool {
        if !isConversationExecution { return selectedExecution?.ownsCommittedAssistantProjection(snapshot) == true }
        guard let owner = committedAssistantOwner, owner.generation == activeGenerationId else { return false }
        return snapshot.belongs(to: owner)
    }

    private func refreshCommittedAssistant(owner: IOSCommittedAssistantOwner? = nil) {
        guard let owner = owner ?? committedAssistantOwner,
              committedAssistantOwner == owner, activeGenerationId == owner.generation else { return }
        do {
            guard let snapshot = try IOSCommittedAssistantProjection.load(
                from: container.applicationRuntime, owner: owner
            ), committedAssistantOwner == owner, activeGenerationId == owner.generation,
               committedAssistantProjection.map({ snapshot.isNotOlder(than: $0) }) ?? true
            else { return }
            // Cursors fence repeat queries without deeply comparing large JSON.
            if let cursor = snapshot.journalCursor { refreshJournalPresentations([cursor]) }
            if committedAssistantProjection?.eventSequence == snapshot.eventSequence,
               committedAssistantProjection?.message.revision == snapshot.message.revision,
               committedAssistantProjection?.message.state == snapshot.message.state,
               committedAssistantProjection?.activeRun == snapshot.activeRun { return }
            committedAssistantProjection = snapshot
        } catch {
            logger.log(.warn, "ChatManager", "Failed to refresh committed assistant facts", metadata: [
                "conversationId": owner.conversationID, "error": error.localizedDescription
            ])
        }
    }

    private func refreshInitialLocalAssistantMetadata(_ snapshot: AgentCoreSnapshot, runID: String) {
        guard let owner = committedAssistantOwner,
              currentConversationId.map({ IOSCommittedAssistantProjection.sameConversation($0.uuidString, owner.conversationID) }) == true,
              owner.needsLocalMetadataRefresh(sessionID: snapshot.sessionId, runID: snapshot.turnId,
                expectedRunID: runID, currentGeneration: activeGenerationId,
                projection: committedAssistantProjection) else { return }
        refreshCommittedAssistant(owner: owner)
    }

    private func requireJournalSourceHost(_ provider: WebAgentProvider?) throws -> String {
        guard let host = provider?.journalSourceHostID, !host.isEmpty else {
            throw IOSApplicationRuntimeError.rejected("Remote Run requires the paired source Host identity")
        }
        return host
    }

    private func requireJournalClient(_ provider: WebAgentProvider?) throws -> String {
        guard let client = provider?.observationClientID, !client.isEmpty else {
            throw IOSApplicationRuntimeError.rejected("Remote submission requires paired client identity")
        }
        return client
    }

    func ownsCommittedJournalPresentation(_ update: IOSCommittedJournalPresentation) -> Bool {
        if !isConversationExecution { return selectedExecution?.ownsCommittedJournalPresentation(update) == true }
        return update.generation == activeGenerationId && currentConversationId.map {
            IOSCommittedAssistantProjection.sameConversation($0.uuidString, update.conversationID)
        } == true
    }

    private func refreshRemoteJournalPresentation(_ consumer: IOSAssistantJournalConsumer, forceWindow: Bool = false) async {
        guard let admission = consumer.admissionCursor, let conversation = currentConversationId,
              IOSCommittedAssistantProjection.sameConversation(conversation.uuidString, admission.identity.conversation_id) else { return }
        do {
            let generation = activeGenerationId
            let isCurrent = { self.activeGenerationId == generation && self.currentConversationId == conversation }
            if let window = try await remoteJournalPresentationCache.loadIfNeeded(admission: admission,
                generation: generation, cursor: consumer.receivedAssistantCursor, force: forceWindow,
                isCurrent: isCurrent, readWindow: { before in
                    try self.container.applicationRuntime.loadConversationProjectionWindowV1(
                        id: admission.identity.conversation_id, beforeOrdinal: before, limit: 80, contentByteLimit: 512_000)
                }) {
                guard isCurrent() else { return }
                committedJournalPresentation = IOSCommittedJournalPresentation(
                    conversationID: window.projection.conversationID, generation: activeGenerationId,
                    eventSequence: window.eventSequence, messages: window.projection.messages,
                    activeRun: window.projection.activeRun, surface: window.projection.surface)
                refreshJournalPresentations(window.assistantJournalHeads)
            }
            guard isCurrent() else { return }
            if let cursor = consumer.receivedAssistantCursor { refreshJournalPresentations([cursor]) }
        } catch {
            logger.log(.warn, "ChatManager", "Could not refresh committed journal owners", metadata: ["error": error.localizedDescription])
        }
    }

    private func applyCommittedJournalText() {
        guard let content = committedAssistantJournalContent,
              let owner = committedAssistantOwner,
              content.cursor.identity.assistant_message_id == owner.assistantMessageID,
              IOSCommittedAssistantProjection.sameConversation(content.cursor.identity.conversation_id, owner.conversationID)
        else { return }
        currentResponse = content.text
        thinking = content.reasoning
        hostToolEvents = content.parts.compactMap(IOSConversationProjectionPresenter.toolEvent)
        let sources = IOSConversationProjectionPresenter.sourceReferences(from: content.parts)
        if sourceReferences != sources {
            sourceReferences = sources
            NotificationCenter.default.post(name: .chatAgentSourceReferencesDidChange, object: sources)
        }
        thinkingFinished = !thinking.isEmpty && content.parts.allSatisfy {
            if case .reasoning(let part) = $0 { return part.state == .completed || part.state == .sealed || part.state == .cancelled || part.state == .failed }
            return true
        }
    }

    /// Window metadata and journal content have independent cursors. Reading
    /// content never mutates or fabricates the canonical Message metadata row.
    func refreshJournalPresentations(_ cursors: [IOSAssistantJournalCursor]) {
        if !isConversationExecution { selectedExecution?.refreshJournalPresentations(cursors); return }
        for cursor in cursors {
            guard let conversation = currentConversationId,
                  IOSCommittedAssistantProjection.sameConversation(conversation.uuidString, cursor.identity.conversation_id) else { continue }
            if let previous = pendingJournalCursors[cursor.identity],
               (UInt64(previous.sequence) ?? 0) >= (UInt64(cursor.sequence) ?? 0) { continue }
            pendingJournalCursors[cursor.identity] = cursor
        }
        guard committedJournalReadTask == nil, !pendingJournalCursors.isEmpty else { return }
        committedJournalReadTask = Task { @MainActor [weak self] in
            guard let self else { return }
            defer { self.committedJournalReadTask = nil }
            while !self.pendingJournalCursors.isEmpty {
                let next = self.pendingJournalCursors.values.sorted { $0.identity.assistant_message_id < $1.identity.assistant_message_id }
                self.pendingJournalCursors.removeAll()
                for cursor in next {
                    let generation = self.activeGenerationId
                    do {
                        let content = try await self.committedJournalReader.read(at: cursor)
                        self.presentJournalContent(content, generation: generation)
                    } catch {
                        self.logger.log(.warn, "ChatManager", "Committed journal presentation is unavailable",
                            metadata: ["conversationId": cursor.identity.conversation_id, "error": error.localizedDescription])
                    }
                }
            }
        }
    }

    private func presentJournalContent(_ content: IOSJournalMessageContent, generation: UInt64) {
        guard generation == activeGenerationId, let selected = currentConversationId,
              IOSCommittedAssistantProjection.sameConversation(selected.uuidString, content.cursor.identity.conversation_id) else { return }
        committedAssistantJournalContent = content
        applyCommittedJournalText()
    }

    private func readLocalJournalPublication(_ value: AgentCoreJSONValue, generation: UInt64) async throws -> IOSJournalMessageContent {
        let cursor = try JSONDecoder().decode(IOSAssistantJournalCursor.self, from: JSONEncoder().encode(value))
        let content = try await committedJournalReader.read(at: cursor)
        presentJournalContent(content, generation: generation)
        return content
    }

    private func consumeLocalJournalPublication(_ value: AgentCoreJSONValue,
        scope: inout IOSLocalJournalPublicationScope, generation: UInt64) async throws -> IOSJournalMessageContent? {
        guard generation == activeGenerationId, !Task.isCancelled else { throw CancellationError() }
        let cursor = try JSONDecoder().decode(IOSAssistantJournalCursor.self, from: JSONEncoder().encode(value))
        if try scope.needsAdmission(cursor) {
            if let window = try await IOSLocalJournalWindowAdmission.load(cursor: cursor, isCurrent: {
                generation == self.activeGenerationId && self.currentConversationId.map {
                    IOSCommittedAssistantProjection.sameConversation($0.uuidString, cursor.identity.conversation_id)
                } == true
            }, readWindow: { before in
                try self.container.applicationRuntime.loadConversationProjectionWindowV1(
                    id: cursor.identity.conversation_id, beforeOrdinal: before, limit: 80, contentByteLimit: 512_000)
            }) {
                applyLocalJournalWindow(window)
                try scope.didAdmit(cursor)
            }
        }
        let content = try await IOSLocalJournalWindowAdmission.read(isCurrent: { generation == self.activeGenerationId }, operation: {
            try await self.readLocalJournalPublication(value, generation: generation)
        })
        guard generation == activeGenerationId, !Task.isCancelled else { throw CancellationError() }
        return content
    }

    private func refreshTerminalJournalWindow(conversationID: String, generation: UInt64) async throws {
        _ = try await IOSLocalJournalWindowAdmission.read(isCurrent: {
            generation == self.activeGenerationId && self.currentConversationId.map {
                IOSCommittedAssistantProjection.sameConversation($0.uuidString, conversationID)
            } == true
        }, operation: { try self.refreshLocalJournalWindow(conversationID: conversationID) })
    }

    private func refreshLocalJournalWindow(conversationID: String) throws {
        guard let selected = currentConversationId,
              IOSCommittedAssistantProjection.sameConversation(selected.uuidString, conversationID) else { return }
        let window = try container.applicationRuntime.loadConversationProjectionWindowV1(
            id: conversationID, beforeOrdinal: nil, limit: 80, contentByteLimit: 512_000)
        applyLocalJournalWindow(window)
    }

    private func applyLocalJournalWindow(_ window: IOSConversationProjectionWindowV1) {
        committedJournalPresentation = IOSCommittedJournalPresentation(
            conversationID: window.projection.conversationID, generation: activeGenerationId,
            eventSequence: window.eventSequence, messages: window.projection.messages,
            activeRun: window.projection.activeRun, surface: window.projection.surface)
        refreshJournalPresentations(window.assistantJournalHeads)
    }

    private func applyAgentSkillRuntimeProjection(_ projection: AgentCoreSkillRuntimeProjection) {
        guard agentSkillRuntimeProjection != projection else { return }
        agentSkillRuntimeProjection = projection
        let projectedEvents = projection.mergingHostToolEvents(hostToolEvents)
        if hostToolEvents != projectedEvents {
            hostToolEvents = projectedEvents
        }
    }

    private func applyAgentCapabilityActivation(_ activation: AgentCoreCapabilityActivationEnvelope) {
        let projected = activation.hostToolEvent
        if let index = hostToolEvents.firstIndex(where: { $0.id == projected.id }) {
            guard hostToolEvents[index] != projected else { return }
            hostToolEvents[index] = projected
        } else {
            hostToolEvents.append(projected)
        }
    }

    private func applyAgentTrace(_ events: [AgentTraceEvent]) {
        for event in events {
            _ = applyAgentTrace(event)
        }
    }

    private var cancellationInFlight = false

    /// Persist the exact native interrupt before a UI epoch change can detach
    /// the stream and retire its physical execution permit.
    static func cancelNativeBeforeRetiringObservation(_ coordinator: DeviceAgentCoordinator,
        isCurrent: () -> Bool, retireObservation: () -> Void) async -> Bool {
        guard isCurrent() else { return false }
        await coordinator.cancel(reason: "cancelled_by_user")
        guard isCurrent() else { return false }
        retireObservation()
        return true
    }

    public func cancelCurrentGeneration(retireSubmission: (() -> Void)? = nil) {
        if !isConversationExecution { selectedExecution?.cancelCurrentGeneration(retireSubmission: retireSubmission); return }
        queuedSuccessorReady = false
        queuedSuccessorTask?.cancel()
        remoteUIWakeSuppressed = true
        remoteUIWakeRequested = false
        // The normal cancellation path owns an already-dispatched request.
        if !messageInFlight { remoteUIWakeTask?.cancel() }
        guard processingState.blocksInput, !cancellationInFlight else { return }
        cancellationInFlight = true
        let hostConversationId = activeHostConversationId
        let modelService = effectiveLLMClient as? ModelService
        let webProvider = hostConversationId.flatMap { remoteSubmissionProviders[$0] }
            ?? modelService?.currentProvider() as? WebAgentProvider
        let usesWebHost = webProvider != nil
        let remoteStore = container.conversationStore as? any ConversationRemoteTurnStore
        let pendingIntent = hostConversationId.flatMap { try? remoteStore?.remoteSubmissionIntent(conversationID: $0) }
        let cancellationReceipt = observedRemoteRun?.submission_receipt
        let cancellationIdentity = pendingIntent?.identity ?? cancellationReceipt?.identity
        let cancellationDigest = pendingIntent?.request_digest ?? cancellationReceipt?.request_digest
        let requestedGeneration = activeGenerationId
        let requestedOwner = committedAssistantOwner
        let requestedConversation = currentConversationId
        var cancelledGeneration = requestedGeneration
        func retireObservation() {
            remoteRunObserver.cancel()
            remoteResumeTask?.cancel()
            activeHostConversationId = nil
            activeGenerationId &+= 1
            cancelledGeneration = activeGenerationId
            if let owner = committedAssistantOwner {
                committedAssistantOwner = IOSCommittedAssistantOwner(
                    conversationID: owner.conversationID, assistantMessageID: owner.assistantMessageID,
                    generation: cancelledGeneration)
            }
            cancelHostPermissionInteraction()
            bridgeScopedResourceContinuation?.resume(returning: nil)
            bridgeScopedResourceContinuation = nil
            localAgentPermissionRequest = nil
            localAgentScopedResourceRequest = nil
            localAgentPermissionQueue.removeAll()
            localAgentPermissionDecisions.removeAll()
            retrievalInProgress = false
            thinkingFinished = true
            // Detaching observation is not a committed Run terminal state.
            processingState = .streaming(progress: 0)
        }
        if usesWebHost { retireObservation() }
        let trimmed = currentResponse.trimmingCharacters(in: .whitespacesAndNewlines)
        let deviceCoordinator = deviceAgentCoordinator
        if usesWebHost || deviceCoordinator == nil { retireSubmission?() }
        Task {
            defer { cancellationInFlight = false }
            if !usesWebHost, let deviceCoordinator {
                guard await Self.cancelNativeBeforeRetiringObservation(deviceCoordinator, isCurrent: {
                    activeGenerationId == requestedGeneration && committedAssistantOwner == requestedOwner
                        && currentConversationId == requestedConversation
                }, retireObservation: {
                    retireObservation()
                    retireSubmission?()
                }) else { return }
            }
            if let webProvider, let hostConversationId, let cancellationIdentity,
               cancellationIdentity.conversation_id == hostConversationId {
                if let pendingIntent, pendingIntent.phase != "admitted", let remoteStore {
                    _ = try? await refreshRemoteSubmission(provider: webProvider, store: remoteStore,
                        conversationID: hostConversationId, cancel: true, expectedIdentity: cancellationIdentity)
                } else if let cancellationDigest {
                    try? await webProvider.cancelHostGeneration(identity: cancellationIdentity, requestDigest: cancellationDigest)
                }
            }
            if usesWebHost { await deviceCoordinator?.cancel(reason: "cancelled_by_user") }
            if !usesWebHost {
                do {
                    guard let deviceCoordinator else {
                        throw DeviceAgentCoordinatorError.cancellationOutcomeUnconfirmed
                    }
                    try await deviceCoordinator.waitForCancellation()
                    try validateNewMessageAdmission()
                    guard activeGenerationId == cancelledGeneration else { return }
                    currentResponse = trimmed.isEmpty
                        ? NSLocalizedString("chat.generation.cancelled", value: "Generation stopped.", comment: "Generation cancellation message")
                        : ChatTextPolicy.trimmingTrailingWhitespaceAndNewlines(from: currentResponse) + NSLocalizedString("chat.generation.cancelled_suffix", value: "\n\n(Generation stopped)", comment: "Generation cancellation suffix")
                    processingState = .completed
                } catch {
                    guard activeGenerationId == cancelledGeneration else { return }
                    processingState = .error(IOSLocalRunAdmissionError.activeRun.localizedDescription)
                    logger.log(.error, "ChatManager", "Cancellation remains unresolved", metadata: ["error": error.localizedDescription])
                    await discoverUnknownEffectRecovery()
                }
            }
            if activeGenerationId == cancelledGeneration {
                refreshCommittedAssistant()
                if usesWebHost { await restoreRemoteJournalIfNeeded() }
            }
        }
    }

    private func taskProjectionHasSameOwners(_ projection: AgentCoreTaskProjection,
        previous: AgentCoreTaskProjection) -> Bool {
        guard Set(projection.order).count == projection.order.count,
              Set(projection.order) == Set(projection.tasks.keys) else { return false }
        let sessions = Set(previous.tasks.values.map(\.ownerSessionId))
        return projection.tasks.values.allSatisfy { task in
            guard task.isValid, sessions.isEmpty || sessions.contains(task.ownerSessionId) else { return false }
            guard let old = previous.tasks[task.id] else { return true }
            return old.ownerSessionId == task.ownerSessionId && old.ownerTurnId == task.ownerTurnId
        }
    }

    /// Cold/idle observation consumes only Core's receipt-backed terminal
    /// inbox. An active coordinator delivers the same events through its driver.
    private func refreshIdleLocalTasks() {
        guard isConversationExecution, let conversation = currentConversationId,
              selectedModelInfo?.isWebBackend != true,
              !messageInFlight, deviceAgentCoordinator == nil else { return }
        do {
            let runtime = container.applicationRuntime
            let consumer = "ios-task-view:\(UUID().uuidString.lowercased())"
            while try runtime.executeAgentCommand(.object([
                "type": .string("apply_pending_task_notice"), "conversation_id": .string(conversation.uuidString),
                "consumer_id": .string(consumer), "now_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1000)),
            ])) != .null {}
        } catch {
            logger.log(.warn, "ChatManager", "Task notice delivery remains pending; inspection is still read-only")
        }
        // Delivery can require an executable parent Host. Reading already
        // committed task facts must remain available when that Host is unavailable.
        do {
            let runtime = container.applicationRuntime
            // A newly-created conversation has no execution checkpoint yet.
            guard let read = try? runtime.executeAgentCommand(.object([
                "type": .string("load_conversation_task_projection_v1"), "conversation_id": .string(conversation.uuidString),
            ])), let fields = read.objectFields, let core = fields["core_session_id"]?.stringScalar,
                  let value = fields["task_projection"] else { return }
            let tasks = try JSONDecoder().decode(AgentCoreTaskProjection.self, from: JSONEncoder().encode(value))
            var reducer = AgentCoreTaskProjectionReducer()
            guard reducer.restore(tasks, expectedSessionId: core) == .applied,
                  taskProjectionHasSameOwners(tasks, previous: agentTaskProjection),
                  tasks.revision >= agentTaskProjection.revision else { return }
            agentTaskProjection = tasks
        } catch {
            logger.log(.warn, "ChatManager", "Local task result observation remains pending",
                metadata: ["error": error.localizedDescription])
        }
    }

    private func taskProviderIsCurrent(_ provider: WebAgentProvider, conversationID: String) -> Bool {
        guard let pinned = remoteSubmissionProviders[conversationID],
              let host = provider.journalSourceHostID else { return false }
        return pinned.journalSourceHostID == host && pinned.observationClientID == provider.observationClientID
    }

    public func cancelAgentTask(_ taskId: String, parentRunID: String) {
        if !isConversationExecution { selectedExecution?.cancelAgentTask(taskId, parentRunID: parentRunID); return }
        guard let conversation = currentConversationId, agentTaskProjection.tasks[taskId]?.isValid == true else { return }
        let conversationId = conversation.uuidString
        let generation = activeGenerationId
        if let provider = remoteSubmissionProviders[conversationId] {
            Task { [weak self] in
                guard let self, self.currentConversationId == conversation, self.activeGenerationId == generation,
                      self.taskProviderIsCurrent(provider, conversationID: conversationId) else { return }
                do {
                    _ = try await provider.cancelHostTask(conversationId: conversationId, parentRunId: parentRunID, taskId: taskId)
                    guard !Task.isCancelled, self.currentConversationId == conversation, self.activeGenerationId == generation,
                          self.taskProviderIsCurrent(provider, conversationID: conversationId) else { return }
                    // A durable cancellation request is not child termination.
                    // The owner-scoped journal subscription supplies task state.
                    self.observeRemoteAgentTasks(conversationId: conversationId)
                } catch {
                    self.logger.log(.error, "ChatManager", "Failed to cancel remote Core task",
                        metadata: ["taskId": taskId, "error": error.localizedDescription])
                }
            }
            return
        }
        // An unpinned cold remote session must recover its Host owner first.
        let saved = try? container.conversationStore?.getConversation(conversationId: conversation)
        guard saved?.selectedModelInfo?.isWebBackend != true,
              observedRemoteRun == nil, !remoteObservationUnresolved,
              let coordinator = deviceAgentCoordinator else { return }
        Task { [weak self] in
            guard let self, self.currentConversationId == conversation, self.activeGenerationId == generation,
                  self.deviceAgentCoordinator === coordinator else { return }
            do { try await coordinator.cancelTask(taskId) }
            catch { self.logger.log(.error, "ChatManager", "Failed to cancel Core task",
                metadata: ["taskId": taskId, "error": error.localizedDescription]) }
        }
    }

    public func sendAgentTaskGuidance(_ taskId: String, parentRunID: String, message: String) {
        if !isConversationExecution { selectedExecution?.sendAgentTaskGuidance(taskId, parentRunID: parentRunID, message: message); return }
        let content = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty, let conversation = currentConversationId,
              agentTaskProjection.tasks[taskId]?.isValid == true else { return }
        let conversationId = conversation.uuidString
        let generation = activeGenerationId
        if let provider = remoteSubmissionProviders[conversationId] {
            guard !parentRunID.isEmpty else { return }
            Task { [weak self] in
                guard let self, self.currentConversationId == conversation, self.activeGenerationId == generation,
                      self.taskProviderIsCurrent(provider, conversationID: conversationId) else { return }
                do {
                    _ = try await provider.submitHostTaskGuidance(
                        conversationId: conversationId, parentRunId: parentRunID, taskId: taskId, message: content)
                    guard !Task.isCancelled, self.currentConversationId == conversation, self.activeGenerationId == generation,
                          self.taskProviderIsCurrent(provider, conversationID: conversationId) else { return }
                    self.observeRemoteAgentTasks(conversationId: conversationId)
                } catch {
                    self.logger.log(.error, "ChatManager", "Failed to guide remote Core task",
                        metadata: ["taskId": taskId, "error": error.localizedDescription])
                }
            }
            return
        }
        let saved = try? container.conversationStore?.getConversation(conversationId: conversation)
        guard saved?.selectedModelInfo?.isWebBackend != true,
              observedRemoteRun == nil, !remoteObservationUnresolved,
              let coordinator = deviceAgentCoordinator else { return }
        Task { [weak self] in
            guard let self, self.currentConversationId == conversation, self.activeGenerationId == generation,
                  self.deviceAgentCoordinator === coordinator else { return }
            do { try await coordinator.sendTaskGuidance(taskId, content: content) }
            catch { self.logger.log(.error, "ChatManager", "Failed to guide Core task",
                metadata: ["taskId": taskId, "error": error.localizedDescription]) }
        }
    }

    private func observeRemoteAgentTasks(conversationId: String) {
        guard currentConversationId?.uuidString == conversationId,
              let provider = remoteSubmissionProviders[conversationId] else { return }
        let generation = activeGenerationId
        // A same-conversation submission may have a new lifecycle generation.
        // Retain its cursor, but replace the captured owner fence before attaching.
        remoteTaskObservation.cancel()
        remoteTaskObservation.start(conversationID: conversationId) { [weak self] observation in
            guard let self else { return }
            let isCurrent = {
                self.currentConversationId?.uuidString == conversationId && self.activeGenerationId == generation
                    && self.taskProviderIsCurrent(provider, conversationID: conversationId)
            }
            var retryDelayNanoseconds: UInt64 = 500_000_000
            while !Task.isCancelled {
                guard isCurrent() else { break }
                if let parentRunId = self.observedRemoteRun?.run_id {
                    do { try await provider.recoverHostTaskGuidance(conversationId: conversationId, parentRunId: parentRunId) }
                    catch { self.logger.log(.warn, "ChatManager", "Task guidance remains unconfirmed",
                        metadata: ["conversationId": conversationId, "error": error.localizedDescription]) }
                }
                let stream = provider.taskProjectionEvents(conversationId: conversationId, afterCursor: observation.cursor)
                do {
                    for try await event in stream {
                        guard !Task.isCancelled, isCurrent(), event.conversationId == conversationId else { return }
                        if let projection = event.projection {
                            guard self.taskProjectionHasSameOwners(projection, previous: self.agentTaskProjection) else {
                                throw IOSApplicationRuntimeError.rejected("Task projection owner changed")
                            }
                            if projection.revision >= self.agentTaskProjection.revision { self.agentTaskProjection = projection }
                        }
                        observation.advance(to: event.cursor, conversationID: conversationId)
                        retryDelayNanoseconds = 500_000_000
                    }
                    guard isCurrent() else { break }
                    let hasActiveTask = self.agentTaskProjection.orderedTasks.contains { !$0.state.isTerminal }
                    if !hasActiveTask && !self.processingState.blocksInput { break }
                } catch is CancellationError { break }
                catch {
                    guard isCurrent() else { break }
                    self.logger.log(.warn, "ChatManager", "Remote task observation reconnecting", metadata: [
                        "conversationId": conversationId, "cursor": String(observation.cursor),
                        "error": error.localizedDescription,
                    ])
                    try? await Task.sleep(nanoseconds: retryDelayNanoseconds)
                    retryDelayNanoseconds = min(retryDelayNanoseconds * 2, 4_000_000_000)
                }
            }
        }
    }

    public func resolveHostPermissionRequest(requestId: String, _ decision: HostAgentPermissionDecision) {
        if !isConversationExecution, let selectedExecution {
            selectedExecution.resolveHostPermissionRequest(requestId: requestId, decision); return
        }
        hostPermissionSession.resolve(requestID: requestId, decision: decision)
    }

    /// Captured when a card is rendered, not resolved through mutable selection
    /// when a delayed tap arrives. A replacement request with the same ID is
    /// still a different authorization question and is rejected.
    func permissionDecisionHandler(
        localRequest: IOSAgentPermissionRequest?, hostRequest: HostAgentPermissionRequest?
    ) -> (String, HostAgentPermissionDecision) -> Void {
        { [weak self] requestID, decision in
            guard let self, self.isConversationExecution else { return }
            if let localRequest, localRequest.presentationID == requestID,
               self.localAgentPermissionRequest == localRequest {
                switch decision {
                case .allowOnce: self.resolveLocalAgentPermission(requestId: requestID, .allowOnce)
                case .allowConversation: self.resolveLocalAgentPermission(requestId: requestID, .allowConversation)
                case .deny: self.resolveLocalAgentPermission(requestId: requestID, .deny)
                case .provideInstruction(let instruction):
                    self.provideLocalAgentPermissionInstruction(requestId: requestID, instruction)
                }
            } else if let hostRequest, hostRequest.id == requestID,
                      self.pendingHostPermissionRequest == hostRequest {
                self.resolveHostPermissionRequest(requestId: requestID, decision)
            }
        }
    }

    static func hostPermissionDecisionPayload(
        for request: HostAgentPermissionRequest,
        decision: HostAgentPermissionDecision
    ) -> [String: Any] {
        IOSHostPermissionSession.decisionPayload(for: request, decision: decision)
    }

    static func hostPermissionDecisionPayload(for request: HostAgentPermissionRequest, allow: Bool) -> [String: Any] {
        hostPermissionDecisionPayload(for: request, decision: allow ? .allowOnce : .deny)
    }

    static func canResumeHostPermission(permissionResumeCount: Int, maxPermissionResumes: Int) -> Bool {
        maxPermissionResumes <= 0 || permissionResumeCount < maxPermissionResumes
    }

    private func isHostPermissionResumeConversationAvailable(_ conversationId: UUID?) -> Bool {
        guard let conversationId else {
            return true
        }
        guard let store = container.conversationStore else {
            return true
        }
        return (try? store.getConversation(conversationId: conversationId)) != nil
    }

    private func markHostPermissionResumeUnavailable() {
        currentResponse = ""
        thinking = ""
        thinkingFinished = true
        processingState = .error(
            NSLocalizedString(
                "chat.agent.permission_resume_failed",
                value: "The conversation is no longer available, so the authorized host tool run could not resume.",
                comment: "Host tool permission resume unavailable message"
            )
        )
    }

    /// Starting a replacement ask cancels only the old interaction. The SSE consumer
    /// never awaits HTTP delivery, so a lost acknowledgement cannot hide its successor.
    @discardableResult
    func startHostPermissionInteraction(
        _ request: HostAgentPermissionRequest,
        conversationId: UUID?,
        isCurrent: @escaping () -> Bool,
        send: @escaping ([String: Any]) async throws -> Void,
        onSubmitted: @escaping (HostAgentPermissionDecision) -> Void = { _ in }
    ) -> Task<Void, Never> {
        hostPermissionSession.start(
            request,
            conversationID: conversationId,
            isCurrent: isCurrent,
            send: send,
            onSubmitted: onSubmitted
        ) { [weak self] request, error in
            self?.logger.log(
                .warn,
                "ChatManager",
                "Permission response failed; waiting for retry",
                metadata: ["requestId": request.id, "error": error.localizedDescription]
            )
        }
    }

    func cancelHostPermissionInteraction(requestId: String? = nil) {
        hostPermissionSession.cancel(requestID: requestId)
    }

    func submitHostPermissionInteraction(
        _ request: HostAgentPermissionRequest,
        conversationId: UUID?,
        isCurrent: () -> Bool,
        send: ([String: Any]) async throws -> Void
    ) async throws -> HostAgentPermissionDecision {
        try await hostPermissionSession.submit(
            request,
            conversationID: conversationId,
            isCurrent: isCurrent,
            send: send
        ) { [weak self] request, error in
            self?.logger.log(
                .warn,
                "ChatManager",
                "Permission response failed; waiting for retry",
                metadata: ["requestId": request.id, "error": error.localizedDescription]
            )
        }
    }

    public convenience init(container: ServiceContainer, restorePersistedState: Bool = true) {
        self.init(container: container, restorePersistedState: restorePersistedState, isConversationExecution: false)
    }

    private init(container: ServiceContainer, restorePersistedState: Bool, isConversationExecution: Bool) {
        self.container = container
        self.isConversationExecution = isConversationExecution
        if isConversationExecution {
            // Freeze the actual provider, not the app-wide mutable facade.
            self.executionClient = (container.llmClient as? ModelService)?.makeScopedClient()
                ?? ModelService(provider: container.llmClient)
        }
        NotificationCenter.default.publisher(for: UIApplication.didBecomeActiveNotification)
            .sink { [weak self] _ in
                Task { @MainActor in
                    guard let self else { return }
                    self.remoteObservationForeground = true
                    self.wakeLocalGoal()
                    await self.recoverEmbeddedChildTasks()
                    await self.restoreRemoteJournalIfNeeded()
                    self.wakeRemoteUIQueue()
                }
            }.store(in: &remoteLifecycleSubscriptions)
        NotificationCenter.default.publisher(for: UIApplication.willResignActiveNotification)
            .sink { [weak self] _ in
                Task { @MainActor in
                    self?.remoteObservationForeground = false
                    await self?.pauseLocalGoalForLifecycle()
                    self?.remoteRunObserver.cancel()
                    self?.remoteTaskObservation.cancel()
                    self?.remoteResumeTask?.cancel()
                    self?.remoteRecoveryGeneration &+= 1
                }
            }.store(in: &remoteLifecycleSubscriptions)
        hostPermissionSession.onStateChange = { [weak self] state in
            self?.pendingHostPermissionRequest = state.request
            self?.pendingHostPermissionConversationId = state.conversationID
            self?.hostPermissionSubmissionError = state.submissionError
            self?.submittingHostPermissionRequestID = state.isSubmitting ? state.request?.id : nil
        }
        guard restorePersistedState else { return }
        // Load the last selected model.
        self.selectedModelInfo = ModelSelectionManager.getLastSelectedModel()
        // Apply selection to container so model config takes effect after relaunch.
        if let last = self.selectedModelInfo {
            applyModelToContainer(last)
        }
        // Try to restore the last active conversation if storage is available.
        if let store = container.conversationStore {
            do {
                let convs = try store.getAllConversations()
                // A durable local permission wait takes precedence over a newer
                // idle conversation so cold launch can reattach its exact Run.
                let restored = Self.coldStartConversation(
                    from: convs,
                    fallbackModel: selectedModelInfo,
                    activeRun: { conversationID in
                        try? container.applicationRuntime.loadConversationProjectionWindowV1(
                            id: conversationID.uuidString,
                            beforeOrdinal: nil,
                            limit: 1,
                            contentByteLimit: 1_024
                        ).projection.activeRun
                    }
                )
                if let restored {
                    self.currentConversationId = restored.id
                    self.currentExecutionBackend = restored.executionBackend
                    self.selectedModelInfo = restored.selectedModelInfo
                    self.modelFallbackNotice = restored.modelUnavailableReason
                    if let conversationModel = restored.selectedModelInfo {
                        self.selectedModelInfo = conversationModel
                        applyModelToContainer(conversationModel)
                    }
                    logger.log(.info, "ChatManager", "Restored startup conversation", metadata: [
                        "conversationId": restored.id.uuidString,
                    ])
                }
            } catch {
                logger.log(.debug, "ChatManager", "No conversation restored", metadata: ["error": error.localizedDescription])
            }
        }

        // Local-only mode: keep startup behavior deterministic.
        Task { [weak self] in
            await self?.recoverEmbeddedChildTasks()
            await self?.restoreActiveLocalRunIfNeeded()
            await self?.syncConversationsFromBackend()
        }
    }

    /// Core supplies due work and the next lease/retry wake. This owns only
    /// native executors and a timer; it does not choose task states or claims.
    private func recoverEmbeddedChildTasks() async {
        guard !isConversationExecution else { return }
        guard !inspectingEmbeddedChildWake else { embeddedChildWakeRequested = true; return }
        inspectingEmbeddedChildWake = true
        defer {
            inspectingEmbeddedChildWake = false
            if embeddedChildWakeRequested {
                embeddedChildWakeRequested = false
                Task { [weak self] in await self?.recoverEmbeddedChildTasks() }
            }
        }
        embeddedChildWakeTask?.cancel()
        embeddedChildWakeTask = nil
        let runtime = container.applicationRuntime
        do {
            let now = Int64(Date().timeIntervalSince1970 * 1_000)
            let plan = try await Task.detached {
                try runtime.executeAgentCommand(.object(["type": .string("load_mobile_task_wake_plan"), "now_ms": .signedInteger(now)]))
            }.value
            guard let fields = plan.objectFields, case .array(let candidates)? = fields["candidates"] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            for candidate in candidates {
                do { try await recoverEmbeddedChild(candidate, runtime: runtime) }
                catch { logger.log(.warn, "ChatManager", "Embedded task recovery remains unconfirmed", metadata: ["error": error.localizedDescription]) }
            }
            let after = candidates.isEmpty ? plan : try await Task.detached {
                try runtime.executeAgentCommand(.object(["type": .string("load_mobile_task_wake_plan"),
                    "now_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1_000))]))
            }.value
            guard let afterFields = after.objectFields else { throw IOSApplicationRuntimeError.invalidResponse }
            for execution in conversationExecutions.values { execution.refreshIdleLocalTasks() }
            if let next = Self.embeddedInteger(afterFields["next_wake_at_ms"]) {
                let delay = max(0, next - Int64(Date().timeIntervalSince1970 * 1_000))
                embeddedChildWakeTask = Task { [weak self] in
                    do { try await Task.sleep(for: .milliseconds(delay)) } catch { return }
                    await self?.recoverEmbeddedChildTasks()
                }
            }
        } catch { logger.log(.warn, "ChatManager", "Embedded task wake plan unavailable", metadata: ["error": error.localizedDescription]) }
    }

    private static func embeddedInteger(_ value: AgentCoreJSONValue?) -> Int64? {
        guard let value, let data = try? JSONEncoder().encode(value) else { return nil }
        return try? JSONDecoder().decode(Int64.self, from: data)
    }

    private func recoverEmbeddedChild(_ candidate: AgentCoreJSONValue, runtime: IOSApplicationRuntime) async throws {
        guard let fields = candidate.objectFields, let taskID = fields["task_id"]?.stringScalar,
              let parent = fields["parent_conversation_id"]?.stringScalar,
              let child = fields["child_conversation_id"]?.stringScalar,
              let parentUUID = UUID(uuidString: parent),
              let owner = ensureExecutionForConversation(parentUUID),
              let invocationValue = fields["parent_invocation"], let exactFields = invocationValue.objectFields,
              let runID = exactFields["run_id"]?.stringScalar,
              let assistant = exactFields["assistant_message_id"]?.stringScalar,
              let config = fields["parent_config"]?.objectFields else { throw IOSApplicationRuntimeError.invalidResponse }
        let frozen = try IOSCanonicalAgentApplication.decodeExactRunInvocation(invocationValue,
            conversationID: parent, runID: runID, assistantMessageID: assistant)
        let host: IOSLocalTaskHost
        if let existing = embeddedChildHosts[runID] { host = existing }
        else {
            let model = try IOSPreparedRunModel.recover(frozen, using: runtime)
            let invocation = try model.prepareHostInvocation()
            guard let coreSessionID = frozen.coreSessionID else { throw IOSApplicationRuntimeError.invalidResponse }
            let policy = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
                expectedIdentity: .init(hostId: "ios-local", clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: coreSessionID),
                isForeground: UIApplication.shared.applicationState == .active,
                scopedGrants: agentCorePermissionBroker.scopedGrants)
            // Child authority/config remains the Core-admitted frozen candidate.
            // This small native transport config never starts a parent Run.
            let transport = AgentCoreRuntimeConfig(hostId: "ios-local", model: model.modelName,
                contextWindowTokens: Self.embeddedInteger(frozen.contextBudget.objectFields?["context_window_tokens"]).flatMap(Int.init(exactly:)),
                reservedOutputTokens: Self.embeddedInteger(frozen.contextBudget.objectFields?["reserved_output_tokens"]).flatMap(Int.init(exactly:)) ?? 0,
                grantedCapabilities: [])
            guard let dependencies = config["turn_identity"]?.objectFields?["host_dependencies"] else {
                throw IOSApplicationRuntimeError.rejected("Frozen child Host dependencies are unavailable")
            }
            _ = try IOSPreparedHostInvocation.decodeSkillIdentity(dependencies)
            host = IOSLocalTaskHost(store: runtime, modelClient: TextAgentModelClient(client: model.client),
                invocation: invocation, toolPlan: policy, hostExecutor: owner.agentCoreHostExecutor, config: transport,
                childPermissionEvents: { [weak owner] event in await owner?.applyChildPermissionEvent(event) },
                stateChanged: { [weak self] in await self?.recoverEmbeddedChildTasks() }, dependencies: dependencies)
            embeddedChildHosts[runID] = host
        }
        try await host.wake(.object(["task": .object(["id": .string(taskID),
            "owner_session_id": .string(parent), "child_session_id": .string(child)])]))
    }

    static func coldStartConversation(
        from conversations: [Conversation],
        fallbackModel: ModelInfo?,
        activeRun: (UUID) -> IOSConversationProjectionV1.RunActivity?
    ) -> Conversation? {
        let ordered = conversations.filter(\.appearsInOrdinaryHistory).sorted { $0.lastActiveAt > $1.lastActiveAt }
        let waiting = ordered.first { conversation in
            let model = conversation.selectedModelInfo
            guard model?.isWebBackend != true,
                  let run = activeRun(conversation.id),
                  run.state == .waitingForPermission else { return false }
            return !run.runID.isEmpty
        }
        return waiting ?? ordered.first
    }

    /// Reattaches the UI to the exact durable local Run after process death.
    /// Core decides recovery eligibility and successor work; selecting the
    /// conversation never creates a replacement turn.
    private func restoreActiveLocalRunIfNeeded() async {
        if !isConversationExecution { await selectedExecution?.restoreActiveLocalRunIfNeeded(); return }
        await discoverUnknownEffectRecovery()
        if unknownEffectRecoveryChecking || unknownEffectRecovery != nil || unknownEffectRecoveryError != nil
            || unknownEffectPendingConversationID != nil { return }
        await restoreRemoteJournalIfNeeded()
        guard deviceAgentCoordinator == nil,
              let conversationID = currentConversationId,
              let conversation = try? container.conversationStore?.getConversation(conversationId: conversationID),
              conversation.selectedModelInfo?.isWebBackend != true,
              let projection = try? container.applicationRuntime.loadConversationProjectionV1(
                id: conversationID.uuidString
              ) else { return }
        if projection.activeRun == nil {
            // A prior process may have ACKed a UI input into the checkpoint.
            // Use the same Core successor admission before checking external FIFO.
            queuedSuccessorReady = true
            scheduleQueuedSuccessorIfReady()
            return
        }
        guard let runID = projection.activeRun?.runID,
              let canonicalIdentity = try? IOSCanonicalAgentApplication.recoveryIdentity(
                store: container.applicationRuntime, conversationID: conversationID.uuidString,
                expectedRunID: runID) else { return }
        let recoveryFence = IOSRecoveryObservationFence(conversationID: conversationID,
            generation: activeGenerationId)
        let session = conversationID.uuidString
        do {
            let application = IOSCanonicalAgentApplication(
                store: container.applicationRuntime,
                descriptor: .init(conversationID: session, conversationTitle: conversation.title ?? "",
                    identity: canonicalIdentity, userText: "", userContentJSON: nil, contextCheckpointID: nil))
            let frozen = try application.loadExactRunInvocation()
            guard let coreSession = frozen.coreSessionID else { throw DeviceAgentCoordinatorError.invalidTransition }
            let recoveryRuntime = container.applicationRuntime
            let restoredModel = try IOSPreparedRunModel.recover(frozen, using: recoveryRuntime)
            let restoredInvocation = try restoredModel.prepareHostInvocation()
            let identity = AgentCoreExpectedIdentity(hostId: "ios-local",
                clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: coreSession)
            let toolPlan = try AgentCoreToolCatalog.prepareBoundPlan(
                for: .deviceLocal,
                expectedIdentity: identity,
                isForeground: UIApplication.shared.applicationState == .active,
                scopedGrants: agentCorePermissionBroker.scopedGrants
            )
            let frozenSkillIdentity = try application.loadFrozenSkillIdentity(frozen)
            let enabledSkills = try IOSPreparedHostInvocation.decodeSkillIdentity(frozenSkillIdentity)
            await agentCoreHostExecutor.acquireAgentSession(sessionId: coreSession)
            defer { Task { await agentCoreHostExecutor.releaseAgentSession(sessionId: coreSession) } }
            guard recoveryFence.accepts(conversationID: currentConversationId,
                generation: activeGenerationId, coordinatorIsAvailable: deviceAgentCoordinator == nil) else { return }
            await agentCoreHostExecutor.installSkillSnapshot(enabledSkills, sessionId: coreSession)
            guard recoveryFence.accepts(conversationID: currentConversationId,
                generation: activeGenerationId, coordinatorIsAvailable: deviceAgentCoordinator == nil) else { return }
            let coordinator = DeviceAgentCoordinator(
                modelClient: TextAgentModelClient(client: restoredModel.client),
                childPermissionEvents: { [weak self] event in await self?.applyChildPermissionEvent(event) },
                childTaskStateChanged: { [weak self] in await self?.container.chatManager.recoverEmbeddedChildTasks() },
                delegationSettings: try restoredModel.mobileDelegationSettings(
                    projectRoot: container.applicationRuntime.databaseURL.deletingLastPathComponent().path,
                    conversationID: frozen.conversationID, using: recoveryRuntime),
                hostExecutor: agentCoreHostExecutor,
                toolPlan: toolPlan,
                modelSupportsToolCalls: restoredModel.supportsToolCalls,
                canonicalApplication: application,
                preparedHostInvocation: restoredInvocation,
                recoveryPreparation: { current in
                    guard current.hasSameFrozenInvocation(as: frozen) else { throw DeviceAgentCoordinatorError.invalidTransition }
                    try await MainActor.run {
                        let revalidated = try IOSPreparedRunModel.recover(current, using: recoveryRuntime).prepareHostInvocation()
                        guard revalidated.route == restoredInvocation.route,
                              revalidated.generation == restoredInvocation.generation,
                              revalidated.outputAdmission == restoredInvocation.outputAdmission else {
                            throw DeviceAgentCoordinatorError.invalidTransition
                        }
                    }
                },
                hostDependencyIdentity: frozenSkillIdentity
            )
            beginAgentTrace(conversationId: conversationID,
                assistantMessageID: canonicalIdentity.assistantMessageID)
            deviceAgentCoordinator = coordinator
            processingState = .streaming(progress: 0)
            observeCommittedAssistant(
                conversationID: session, assistantMessageID: canonicalIdentity.assistantMessageID,
                generation: recoveryFence.generation
            )
            let recoveryOwner = committedAssistantOwner
            defer {
                if deviceAgentCoordinator === coordinator { deviceAgentCoordinator = nil }
            }
            let stream = await coordinator.recoverExactRun(
                sessionId: coreSession,
                config: AgentCoreRuntimeConfig(
                    platform: .ios,
                    executionMode: .deviceLocal,
                    model: restoredModel.modelName,
                    maxIterations: 0,
                    contextWindowTokens: restoredModel.parameters.maxContextLength,
                    reservedOutputTokens: restoredModel.parameters.maxOutputTokens,
                    permissionProfile: conversation.permissionMode
                )
            )
            var journalScope = IOSLocalJournalPublicationScope(conversationID: session, runID: frozen.runID)
            for try await event in stream {
                guard deviceAgentCoordinator === coordinator,
                      committedAssistantOwner == recoveryOwner,
                      recoveryOwner?.generation == activeGenerationId else { return }
                try await applyRecoveredDeviceAgentEvent(event, conversationID: session,
                    journalScope: &journalScope, generation: recoveryFence.generation)
            }
            Task { [weak self] in self?.wakeLocalGoal() }
        } catch {
            if !(error is CancellationError), currentConversationId == conversationID,
               activeGenerationId == recoveryFence.generation {
                processingState = .error(error.localizedDescription)
                if error as? IOSCanonicalAgentApplicationError == .recoveryDecisionRequired {
                    // The recovery driver has retired before this catch. Core
                    // may have classified a vanished worker after our initial
                    // inspection, so observe its decision again now. This only
                    // presents the existing user-controlled recovery surface.
                    await discoverUnknownEffectRecovery()
                }
            }
            logger.log(.warn, "ChatManager", "Could not restore active local Run", metadata: [
                "conversationId": session,
                "error": error.localizedDescription,
            ])
        }
    }

    private func restoreRemoteJournalIfNeeded() async {
        if !isConversationExecution { await selectedExecution?.restoreRemoteJournalIfNeeded(); return }
        guard remoteObservationForeground, remoteStreamGeneration == nil,
              deviceAgentCoordinator == nil, let conversationID = currentConversationId,
              conversationID != knownUnsubmittedConversationID,
              let conversation = try? container.conversationStore?.getConversation(conversationId: conversationID),
              conversation.selectedModelInfo?.isWebBackend == true,
              let provider = remoteSubmissionProviders[conversationID.uuidString]
                ?? (effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider,
              let store = container.conversationStore as? any ConversationRemoteTurnStore else { return }
        remoteRecoveryGeneration &+= 1
        let recoveryGeneration = remoteRecoveryGeneration
        remoteObservationUnresolved = true
        // A foreground/reconnect boundary can resume an ACK read that failed
        // after the configuration commit; unchanged observer ticks cannot.
        permissionHydration[conversationID, default: .init()].invalidate(forgetSuccess: true)
        refreshPermissionOwnerProjection(conversationID: conversationID)
        let fence = IOSRecoveryObservationFence(conversationID: conversationID, generation: activeGenerationId)
        defer {
            if recoveryGeneration == remoteRecoveryGeneration,
               fence.accepts(conversationID: currentConversationId, generation: activeGenerationId,
                             coordinatorIsAvailable: deviceAgentCoordinator == nil), remoteObservationForeground {
                startRemoteRunObservation(provider: provider, store: store, conversationID: conversationID)
            }
        }
        do {
            struct ActiveRun: Decodable { let run_id: String; let assistant_message_id: String? }
            let raw = try store.executeRemoteJournalCommand(.object([
                "type": .string("active_run"), "conversation_id": .string(conversationID.uuidString),
            ]))
            let active = try JSONDecoder().decode(ActiveRun?.self, from: JSONEncoder().encode(raw))
            struct Window: Decodable { let assistant_journal_heads: [IOSAssistantJournalCursor] }
            struct Terminal: Decodable { let assistant_message_id: String }
            let windowValue = try store.executeRemoteJournalCommand(.object([
                "type": .string("load_conversation_projection_window_v1"),
                "conversation_id": .string(conversationID.uuidString),
                "before_ordinal": .null, "limit": .signedInteger(32), "content_byte_limit": .signedInteger(1024),
            ]))
            let window = try JSONDecoder().decode(Window.self, from: JSONEncoder().encode(windowValue))
            var owners: [(run: String, assistant: String)] = []
            if let active, let assistant = active.assistant_message_id { owners.append((active.run_id, assistant)) }
            var observed = Set(owners.map(\.run))
            for head in window.assistant_journal_heads where observed.insert(head.identity.run_id).inserted {
                let terminalValue = try store.executeRemoteJournalCommand(.object([
                    "type": .string("canonical_run_terminal"), "conversation_id": .string(conversationID.uuidString),
                    "run_id": .string(head.identity.run_id),
                ]))
                if let terminal = try JSONDecoder().decode(Terminal?.self, from: JSONEncoder().encode(terminalValue)) {
                    owners.append((head.identity.run_id, terminal.assistant_message_id))
                }
            }
            for owner in owners {
                try Task.checkCancellation()
                guard recoveryGeneration == remoteRecoveryGeneration else { return }
                guard fence.accepts(conversationID: currentConversationId, generation: activeGenerationId,
                                    coordinatorIsAvailable: deviceAgentCoordinator == nil) else { return }
                do {
                    let consumer = try IOSAssistantJournalConsumer(provider: provider, store: store,
                        conversationID: conversationID.uuidString, runID: owner.run, assistantID: owner.assistant)
                    try await consumer.recover()
                    guard recoveryGeneration == remoteRecoveryGeneration else { return }
                    guard fence.accepts(conversationID: currentConversationId, generation: activeGenerationId,
                                        coordinatorIsAvailable: deviceAgentCoordinator == nil) else { return }
                    await refreshRemoteJournalPresentation(consumer, forceWindow: true)
                } catch is CancellationError {
                    throw CancellationError()
                } catch {
                    // A visible window may contain Runs from another execution
                    // Host. Never relabel them or block independent owner recovery.
                    logger.log(.warn, "ChatManager", "Visible journal owner remains recoverable",
                               metadata: ["runId": owner.run, "error": error.localizedDescription])
                }
            }
            guard fence.accepts(conversationID: currentConversationId, generation: activeGenerationId,
                                coordinatorIsAvailable: deviceAgentCoordinator == nil) else { return }
            if let active, let assistant = active.assistant_message_id {
                guard recoveryGeneration == remoteRecoveryGeneration else { return }
                observeCommittedAssistant(conversationID: conversationID.uuidString, assistantMessageID: assistant, generation: activeGenerationId)
                refreshCommittedAssistant()
                applyCommittedJournalText()
            }
        } catch {
            logger.log(.warn, "ChatManager", "Remote journal remains recoverable", metadata: ["error": error.localizedDescription])
        }
    }

    private func startRemoteRunObservation(provider: WebAgentProvider,
        store: any ConversationRemoteTurnStore, conversationID: UUID) {
        guard remoteObservationForeground, remoteResumeTask == nil,
              let host = provider.journalSourceHostID else { return }
        remoteSubmissionProviders[conversationID.uuidString] = provider
        // Terminal conversations have no active Run observation to deliver.
        // Task ownership is independently authenticated by the paired source.
        observeRemoteAgentTasks(conversationId: conversationID.uuidString)
        let generation = activeGenerationId
        let client = provider.observationClientID
        let isCurrent = { [weak self] in
            self?.currentConversationId == conversationID && self?.activeGenerationId == generation
                && self?.remoteObservationForeground == true
        }
        remoteObservationUnresolved = true
        remoteRunObservationError = nil
        processingState = .streaming(progress: 0)
        struct KnownRun: Decodable { let run_id: String; let assistant_message_id: String? }
        var known: KnownRun?
        do {
            let raw = try store.executeRemoteJournalCommand(.object([
                "type": .string("active_run"), "conversation_id": .string(conversationID.uuidString),
            ]))
            known = try JSONDecoder().decode(KnownRun?.self, from: JSONEncoder().encode(raw))
            if let known, known.assistant_message_id?.isEmpty != false {
                throw IOSApplicationRuntimeError.rejected("Known Host Run has no durable root assistant")
            }
        } catch {
            remoteRunObservationError = error.localizedDescription
            return
        }
        var consumer: IOSAssistantJournalConsumer?
        remoteRunObserver.start(initial: observedRemoteRun, read: { previous in
            guard isCurrent() else { throw CancellationError() }
            if let intent = try store.remoteSubmissionIntent(conversationID: conversationID.uuidString), intent.phase != "admitted" {
                self.remoteSubmissionPending = true
                let refreshed = try await self.refreshRemoteSubmission(provider: provider, store: store,
                    conversationID: conversationID.uuidString, cancel: false)
                guard isCurrent() else { throw CancellationError() }
                if refreshed?.phase == "admitted", let identity = refreshed?.identity {
                    known = KnownRun(run_id: identity.submission_id, assistant_message_id: identity.assistant_message_id)
                    self.remoteSubmissionPending = false
                } else if refreshed?.phase == "cancelled" || refreshed?.phase == "rejected" {
                    self.remoteSubmissionPending = false
                    return nil
                } else {
                    throw IOSApplicationRuntimeError.rejected("Waiting for the Host to confirm the original submission. You may cancel it without resending.")
                }
            }
            let result = try await provider.observeRun(.init(conversation: conversationID.uuidString,
                host: host, client: client, run: previous,
                expectedRunID: known?.run_id, assistantID: known?.assistant_message_id))
            if let known, previous == nil {
                guard result?.run_id == known.run_id, result?.assistant_message_id == known.assistant_message_id else {
                    throw IOSApplicationRuntimeError.rejected("Known Host Run disappeared or changed identity")
                }
            }
            try result?.validate(conversation: conversationID.uuidString, client: client, previous: previous)
            if let result {
                guard let receipt = result.submission_receipt else {
                    throw IOSApplicationRuntimeError.rejected("Observed Run has no registered source submission receipt")
                }
                guard receipt.identity.source_host_id == host, receipt.identity.client_id == client,
                      receipt.identity.conversation_id == result.conversation_id,
                      receipt.identity.submission_id == result.run_id,
                      receipt.identity.assistant_message_id == result.assistant_message_id,
                      receipt.phase == "admitted", receipt.admission?.identity == result.admission.identity else {
                    throw IOSApplicationRuntimeError.rejected("Observed submission receipt owner changed")
                }
                if try store.remoteSubmissionIntent(conversationID: conversationID.uuidString) != nil {
                    _ = try store.acceptRemoteSubmissionReceipt(receipt)
                } else if try !store.hasRemoteCanonicalRun(conversationID: conversationID.uuidString, runID: result.run_id) {
                    try store.adoptObservedRemoteRun(receipt)
                }
            }
            return result
        }, receive: { [weak self] observation in
            guard let self, isCurrent() else { throw CancellationError() }
            self.observedRemoteRun = observation
            self.remoteObservationUnresolved = true
            if self.permissionObservedIdentity != observation.submission_receipt?.identity ||
                self.permissionObservedRecovery != observation.recovery {
                self.permissionObservedIdentity = observation.submission_receipt?.identity
                self.permissionObservedRecovery = observation.recovery
                self.refreshPermissionOwnerProjection(conversationID: conversationID)
            }
            self.activeHostConversationId = conversationID.uuidString
            if consumer == nil {
                consumer = try IOSAssistantJournalConsumer(provider: provider, store: store,
                    conversationID: conversationID.uuidString, runID: observation.run_id,
                    assistantID: observation.assistant_message_id)
            }
            guard let consumer else { return }
            try await consumer.recover(contextUsage: observation.context_usage)
            guard isCurrent() else { throw CancellationError() }
            self.refreshCommittedContextUsage(conversationID: conversationID)
            await self.refreshRemoteJournalPresentation(consumer, forceWindow: true)
            self.observeCommittedAssistant(conversationID: conversationID.uuidString,
                assistantMessageID: observation.assistant_message_id, generation: generation)
            self.refreshCommittedAssistant()
            self.applyCommittedJournalText()
            let permissions = try observation.permissionRequests(host: host, client: client,
                now: Int64(Date().timeIntervalSince1970))
            if let request = permissions.first {
                if self.pendingHostPermissionRequest?.nonce != request.nonce {
                    self.startHostPermissionInteraction(request, conversationId: conversationID,
                        isCurrent: isCurrent, send: { try await provider.submitHostPermissionDecision($0) },
                        onSubmitted: { _ in })
                }
            } else if self.pendingHostPermissionConversationId == conversationID {
                self.cancelHostPermissionInteraction()
            }
            self.remoteRunNeedsResume = observation.recovery == "resume_required"
            if observation.recovery == "terminal" {
                guard consumer.terminalStatus == observation.status else {
                    throw IOSApplicationRuntimeError.rejected("Run terminal publication is not yet received")
                }
                self.remoteObservationUnresolved = !observation.can_submit
                self.observedRemoteRun = nil
                if observation.can_submit {
                    self.processingState = .completed
                    self.refreshPermissionOwnerProjection(conversationID: conversationID)
                    if let result = consumer.terminalResult,
                       result.value.kind == .answered || result.value.kind == .actionCompleted {
                        self.wakeRemoteUIQueue()
                    }
                }
                else {
                    // This exact Run is sealed, but another admitted Run may
                    // now own the conversation. Discover it without new input.
                    Task { [weak self] in
                        guard let self, isCurrent() else { return }
                        self.startRemoteRunObservation(provider: provider, store: store, conversationID: conversationID)
                    }
                }
            }
        }, idle: { [weak self] in
            guard let self, isCurrent() else { return }
            do {
                let local = try store.executeRemoteJournalCommand(.object([
                    "type": .string("active_run"), "conversation_id": .string(conversationID.uuidString),
                ]))
                guard case .null = local else {
                    throw IOSApplicationRuntimeError.rejected("The local Run still needs authoritative Host recovery")
                }
            } catch {
                self.remoteObservationUnresolved = true
                self.remoteRunObservationError = error.localizedDescription
                return
            }
            self.remoteObservationUnresolved = false
            self.observedRemoteRun = nil
            self.remoteRunNeedsResume = false
            self.processingState = .completed
            self.refreshPermissionOwnerProjection(conversationID: conversationID)
            self.wakeRemoteUIQueue()
        }, failed: { [weak self] error in
            guard let self, isCurrent() else { return }
            // Unknown/409/other failures are not permission to start a new Run.
            self.remoteObservationUnresolved = true
            self.remoteRunObservationError = error.localizedDescription
        })
    }

    public func retryRemoteRunObservation() {
        if !isConversationExecution { selectedExecution?.retryRemoteRunObservation(); return }
        permissionObservedIdentity = nil
        permissionObservedRecovery = nil
        if let owner = currentConversationId { permissionHydration[owner, default: .init()].invalidate(forgetSuccess: true) }
        Task { [weak self] in await self?.restoreRemoteJournalIfNeeded() }
    }

    private func refreshRemoteSubmission(provider: WebAgentProvider, store: any ConversationRemoteTurnStore,
        conversationID: String, cancel: Bool,
        expectedIdentity: IOSRemoteSubmissionIdentity? = nil) async throws -> IOSRemoteSubmissionIntent? {
        guard let intent = try store.remoteSubmissionIntent(conversationID: conversationID) else { return nil }
        guard !cancel || expectedIdentity == intent.identity else {
            throw IOSApplicationRuntimeError.rejected("Cancellation submission changed before dispatch")
        }
        guard intent.identity.source_host_id == provider.journalSourceHostID,
              intent.identity.client_id == provider.observationClientID else {
            throw IOSApplicationRuntimeError.rejected("Pending submission belongs to a different paired Host")
        }
        guard let digest = intent.request_digest else {
            return cancel ? try store.abandonPreparedSubmission(intent.identity) : intent
        }
        guard let receipt = try await provider.submissionReceipt(identity: intent.identity, digest: digest, cancel: cancel) else { return intent }
        let accepted = try store.acceptRemoteSubmissionReceipt(receipt)
        if cancel, accepted.phase == "admitted" {
            try await provider.cancelHostGeneration(identity: intent.identity, requestDigest: digest)
        }
        return accepted
    }

    public func cancelPendingRemoteSubmission() {
        if !isConversationExecution { selectedExecution?.cancelPendingRemoteSubmission(); return }
        guard let conversationID = currentConversationId,
              let provider = remoteSubmissionProviders[conversationID.uuidString]
                ?? (effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider,
              let store = container.conversationStore as? any ConversationRemoteTurnStore,
              let intent = try? store.remoteSubmissionIntent(conversationID: conversationID.uuidString) else { return }
        Task { [weak self] in
            guard let self else { return }
            do {
                let result = try await self.refreshRemoteSubmission(provider: provider, store: store,
                    conversationID: conversationID.uuidString, cancel: true, expectedIdentity: intent.identity)
                guard self.currentConversationId == conversationID else { return }
                if result?.phase == "cancelled" || result?.phase == "rejected" {
                    self.remoteSubmissionPending = false
                    self.remoteObservationUnresolved = false
                    self.processingState = .completed
                }
                await self.restoreRemoteJournalIfNeeded()
            } catch { if self.currentConversationId == conversationID { self.remoteRunObservationError = error.localizedDescription } }
        }
    }

    public func resumeObservedRemoteRun() {
        if !isConversationExecution { selectedExecution?.resumeObservedRemoteRun(); return }
        guard remoteResumeTask == nil, let run = observedRemoteRun, remoteRunNeedsResume,
              run.recovery == "resume_required", run.worker.state == "absent",
              let conversationID = currentConversationId, run.conversation_id == conversationID.uuidString,
              let provider = remoteSubmissionProviders[conversationID.uuidString]
                ?? (effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider,
              let store = container.conversationStore as? any ConversationRemoteTurnStore else { return }
        let generation = activeGenerationId
        remoteRunObserver.cancel()
        remoteRunNeedsResume = false
        observeRemoteAgentTasks(conversationId: conversationID.uuidString)
        let resumeID = UUID()
        remoteResumeID = resumeID
        remoteResumeTask = Task { [weak self] in
            guard let self else { return }
            defer {
                if self.remoteResumeID == resumeID {
                    self.remoteResumeTask = nil
                    self.remoteResumeID = nil
                    if self.currentConversationId == conversationID, self.activeGenerationId == generation {
                        self.startRemoteRunObservation(provider: provider, store: store, conversationID: conversationID)
                    }
                }
            }
            do {
                let consumer = try IOSAssistantJournalConsumer(provider: provider, store: store,
                    conversationID: run.conversation_id, runID: run.run_id, assistantID: run.assistant_message_id)
                for try await event in provider.resumeObservedRun(run) {
                    guard self.remoteResumeID == resumeID, self.currentConversationId == conversationID,
                          self.activeGenerationId == generation else { throw CancellationError() }
                    switch event {
                    case .assistantJournalCommitted(let raw): try await consumer.receiveHint(raw, terminal: false)
                    case .assistantRunTerminalSealed(let raw): try await consumer.receiveHint(raw, terminal: true)
                    case .permissionRequest(let request, _), .delegatedPermissionRequest(let request):
                        guard request.hostId == provider.journalSourceHostID,
                              request.clientId == provider.observationClientID, request.sessionId == run.conversation_id,
                              request.canonicalRunID == run.run_id,
                              request.protocolVersion == WebAgentProvider.hostPermissionProtocolVersion,
                              request.nonce?.isEmpty == false, let expiry = request.expiresAt,
                              expiry >= Int64(Date().timeIntervalSince1970) else {
                            throw IOSApplicationRuntimeError.rejected("Resumed permission owner mismatch")
                        }
                        self.startHostPermissionInteraction(request, conversationId: conversationID,
                            isCurrent: { self.remoteResumeID == resumeID && self.currentConversationId == conversationID && self.activeGenerationId == generation },
                            send: { try await provider.submitHostPermissionDecision($0) }, onSubmitted: { _ in })
                    case .error(let message): throw IOSApplicationRuntimeError.rejected(message)
                    default: break
                    }
                    guard self.remoteResumeID == resumeID, self.currentConversationId == conversationID,
                          self.activeGenerationId == generation else { throw CancellationError() }
                    await self.refreshRemoteJournalPresentation(consumer, forceWindow: consumer.terminalStatus != nil)
                }
                // EOF only triggers another exact observation; it never closes the Run.
            } catch {
                if self.remoteResumeID == resumeID, self.currentConversationId == conversationID, self.activeGenerationId == generation {
                    self.remoteRunObservationError = error.localizedDescription
                }
                // Includes 409: observe the winner instead of submitting new input.
            }
        }
    }

    private func applyRecoveredDeviceAgentEvent(_ event: DeviceAgentCoordinatorEvent, conversationID: String,
        journalScope: inout IOSLocalJournalPublicationScope, generation: UInt64) async throws {
        switch event {
        case .snapshot(let snapshot):
            agentCoreSnapshot = snapshot
            refreshInitialLocalAssistantMetadata(snapshot, runID: journalScope.runID)
            if permissionOwnerProjectionRunID != snapshot.turnId, let owner = currentConversationId {
                permissionOwnerProjectionRunID = snapshot.turnId
                refreshPermissionOwnerProjection(conversationID: owner)
            }
        case .assistantJournalCommitted(let cursor):
            _ = try await consumeLocalJournalPublication(cursor, scope: &journalScope, generation: generation)
        case .coreTurnTerminal:
            refreshCommittedAssistant()
            try await refreshTerminalJournalWindow(conversationID: conversationID, generation: generation)
            if let owner = currentConversationId { refreshPermissionOwnerProjection(conversationID: owner) }
        case .taskProjectionUpdated(let projection):
            if let selected = currentConversationId,
               IOSCommittedAssistantProjection.sameConversation(selected.uuidString, conversationID),
               taskProjectionHasSameOwners(projection, previous: agentTaskProjection),
               projection.revision >= agentTaskProjection.revision {
                agentTaskProjection = projection
            }
        case .skillRuntimeProjectionUpdated(let projection):
            applyAgentSkillRuntimeProjection(projection)
        case .capabilitiesActivated(let activation):
            applyAgentCapabilityActivation(activation)
        case .trace(let trace):
            agentTraceContext.adopt(trace)
            _ = applyAgentTrace(trace)
        case .permissionRequested(let request):
            enqueueLocalAgentPermission(request)
        case .permissionResolved(let requestID, _, _):
            removeLocalAgentPermission(requestId: requestID)
        case .completed:
            processingState = .completed
        }
    }

    /// Start a new conversation and persist it
    public func startConversation(title: String? = nil, language: String? = nil, initialParameters: ModelParameters? = nil,
                                  selectOnCreate: Bool = true) -> UUID? {
        guard let store = container.conversationStore else { return nil }
        do {
            let creationModel = try selectedModelInfo ?? ModelSelectionManager.lastSelectedModel(using: container.applicationRuntime)
            let conv: Conversation
            if let selectedModelInfo = creationModel {
                guard let configured = store as? any ConfiguredConversationCreationStore else {
                    throw ChatManagerError.failedToCreateConversation("Configured conversation creation unavailable")
                }
                conv = try configured.createConfiguredConversation(title: title, language: language, model: selectedModelInfo,
                    parameters: initialParameters ?? ModelParametersStore.shared.loadParameters(for: selectedModelInfo.id, isLocal: selectedModelInfo.isLocal))
            } else { conv = try store.createConversation(title: title, language: language) }
            newlyCreatedConversationID = conv.id
            defer { newlyCreatedConversationID = nil }
            if selectOnCreate {
                self.currentConversationId = conv.id
                self.currentExecutionBackend = conv.executionBackend
            } else {
                // Establish the fresh execution owner without publishing UI
                // selection; the composer must retire its draft by CAS first.
                let session = ChatManager(container: container, restorePersistedState: false, isConversationExecution: true)
                session.selectedModelInfo = conv.selectedModelInfo
                session.currentExecutionBackend = conv.executionBackend
                session.knownUnsubmittedConversationID = conv.id
                session.currentConversationId = conv.id
                conversationExecutions[conv.id] = session
            }
            Task.detached { [weak self] in
                await self?.createRemoteConversationIfNeeded(conversation: conv)
            }
            return conv.id
        } catch {
            logger.log(.error, "ChatManager", "Failed to create conversation", metadata: ["error": error.localizedDescription])
            return nil
        }
    }

    public func appendMessageToConversation(conversationId: UUID, message: ConversationMessage) throws {
        guard let store = container.conversationStore else { return }
        var shouldSyncTitle = false
        if message.role == .user,
           let conversation = try? store.getConversation(conversationId: conversationId),
           isDefaultConversationTitle(conversation.title) {
            let existingUserMessages = (try? store.getMessages(conversationId: conversationId, limit: 0)) ?? []
            let hasExistingTitleCandidate = existingUserMessages.contains {
                $0.role == .user && firstQueryTitle(from: $0.text) != nil
            }
            if !hasExistingTitleCandidate, let title = firstQueryTitle(from: message.text) {
                try store.updateConversationTitle(conversationId: conversationId, title: title)
                shouldSyncTitle = true
            }
        }
        if !isRunningUnitTests,
           shouldSyncTitle,
           canAttemptRemoteChatSync(conversationId: conversationId),
           let title = firstQueryTitle(from: message.text) {
            Task.detached { [weak self] in
                await self?.updateRemoteConversationTitleIfNeeded(conversationId: conversationId, title: title)
            }
        }
        if !isRunningUnitTests, canAttemptRemoteChatSync(conversationId: message.conversationId) {
            Task.detached { [weak self] in
                await self?.appendRemoteMessageIfNeeded(message)
            }
        }
    }
    
    public func selectModel(_ model: ModelInfo, persistAsDefault: Bool = true) {
        self.selectedModelInfo = model
        if case .error = processingState {
            processingState = .idle
        }
        if persistAsDefault { ModelSelectionManager.saveSelectedModel(model) }
        applyModelToContainer(model)
        if !isConversationExecution, let session = selectedExecution,
           !session.processingState.blocksInput, session.deviceAgentCoordinator == nil {
            session.selectedModelInfo = model
            session.executionClient = (container.llmClient as? ModelService)?.makeScopedClient()
                ?? ModelService(provider: container.llmClient)
        }
        logger.log(.info, "ChatManager", "Model selected", metadata: [
            "model": model.name,
            "isLocal": String(model.isLocal),
            "containerModel": container.model
        ])
    }

    public func persistSelectedModelForCurrentConversation(_ model: ModelInfo) throws {
        guard let store = container.conversationStore,
              let convId = currentConversationId else {
            return
        }
        do {
            guard let configured = store as? any ConfiguredConversationCreationStore else {
                throw ChatManagerError.failedToCreateConversation("Atomic conversation model selection unavailable")
            }
            try configured.selectConversationModel(conversationID: convId, model: model,
                initialParameters: ModelParametersStore.shared.loadParameters(for: model.id, isLocal: model.isLocal))
            logger.log(.debug, "ChatManager", "Persisted conversation model selection", metadata: [
                "conversationId": convId.uuidString,
                "model": model.name
            ])
        } catch {
            logger.log(.warn, "ChatManager", "Failed to persist conversation model selection", metadata: [
                "conversationId": convId.uuidString,
                "error": error.localizedDescription
            ])
            throw error
        }
    }

    public func setConversationPromptResponseStyle(_ style: PromptResponseStyle?) throws {
        guard let store = container.conversationStore,
              let conversationID = ensureCurrentConversation() else {
            throw ChatManagerError.failedToCreateConversation("conversation store unavailable")
        }
        try store.setPromptResponseStyle(conversationId: conversationID, style: style)
    }

    public func selectExecutionBackendForCurrentConversation(
        _ backendID: String,
        availableBackendIDs: Set<String>
    ) throws {
        let normalized = try ExecutionBackendSelectionPolicy.validatedSelection(
            backendID,
            availableBackendIDs: availableBackendIDs
        )
        guard let store = container.conversationStore,
              let conversationID = currentConversationId,
              var conversation = try store.getConversation(conversationId: conversationID) else {
            return
        }
        conversation.executionBackend = normalized
        conversation.lastActiveAt = Date()
        try store.upsertConversation(conversation)
        currentExecutionBackend = normalized
    }

    public func setPermissionModeForCurrentConversation(_ mode: IOSPermissionMode) async throws {
        guard let store = container.conversationStore,
              let conversationID = currentConversationId,
              let conversation = try store.getConversation(conversationId: conversationID) else {
            throw IOSApplicationRuntimeError.rejected("Select a conversation before editing permissions")
        }
        let execution = executionForConversation(conversationID) ?? self
        execution.permissionHydration[conversationID, default: .init()].invalidate()
        let policyOwner = try readPermissionConfigurationOwner(conversationID: conversationID, execution: execution)
        if case .remote(let identity) = policyOwner {
                guard let provider = execution.remoteSubmissionProviders[conversationID.uuidString]
                    ?? (execution.effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider,
                      provider.journalSourceHostID == identity.source_host_id,
                      provider.observationClientID == identity.client_id else {
                    throw IOSApplicationRuntimeError.rejected("The active Host permission owner must be reconnected before saving")
                }
                let baseline = try await provider.permissionConfiguration(owner: identity)
                guard try readPermissionConfigurationOwner(conversationID: conversationID, execution: execution) == policyOwner else {
                    throw IOSApplicationRuntimeError.rejected("The permission Run owner changed before saving")
                }
                let saved = if baseline.permission_profile == mode { baseline } else {
                    try await provider.permissionConfiguration(owner: identity,
                        expectedRevision: baseline.configuration_revision, profile: mode)
                }
                remotePermissionCommitRevisions[conversationID] = saved.configuration_revision
                remotePermissionConfigurations[conversationID] = saved
                execution.remotePermissionCommitRevisions[conversationID] = saved.configuration_revision
                execution.remotePermissionConfigurations[conversationID] = saved
                execution.permissionHydration[conversationID, default: .init()].confirm(identity)
                observeRemotePermissionPolicy(conversationID: conversationID, execution: execution,
                    provider: provider, identity: identity, saved: saved, mode: mode)
        }
        guard try readPermissionConfigurationOwner(conversationID: conversationID, execution: execution) == policyOwner else {
            throw IOSApplicationRuntimeError.rejected("The permission Run owner changed before the local mirror save")
        }
        guard let expectedRevision = conversation.configurationRevision,
              let permissionStore = store as? any ConversationPermissionStore else {
            throw IOSApplicationRuntimeError.rejected("Permission configuration store is unavailable")
        }
        let sourceHostID: String?
        if case .remote(let identity) = policyOwner { sourceHostID = identity.source_host_id }
        else { sourceHostID = nil }
        try permissionStore.setConversationPermission(.init(conversationID: conversationID.uuidString,
            expectedRevision: expectedRevision, expectedRunID: policyOwner.runID, expectedSourceHostID: sourceHostID,
            profile: mode.rawValue, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1000)))
        guard let committed = try store.getConversation(conversationId: conversationID),
              committed.permissionMode == mode, let revision = committed.configurationRevision else {
            throw IOSApplicationRuntimeError.rejected("Permission save was not confirmed; your selection is retained")
        }
        permissionConfigurationRevisions[conversationID] = revision
        permissionCommittedModes[conversationID] = committed.permissionMode
        execution.permissionCommittedModes[conversationID] = committed.permissionMode
        permissionConfigurationRunIDs[conversationID] = policyOwner.runID
        if let coordinator = executionForConversation(conversationID)?.deviceAgentCoordinator {
            do { try await coordinator.refreshPermissionPolicy() }
            catch {
                logger.log(.warn, "ChatManager", "Permissions saved; the active Run has not confirmed the update", metadata: [
                    "conversationId": conversationID.uuidString, "error": error.localizedDescription,
                ])
            }
        }
    }

    private func observeRemotePermissionPolicy(conversationID: UUID, execution: ChatManager,
                                               provider: WebAgentProvider, identity: IOSRemoteSubmissionIdentity,
                                               saved: IOSRemotePermissionConfiguration, mode: IOSPermissionMode) {
        permissionPolicyObservationTasks[conversationID]?.cancel()
        // Source ACK remains observable when a later, independent mirror save fails.
        permissionPolicyObservationTasks[conversationID] = Task { @MainActor [weak self, weak execution] in
            guard let self, let execution else { return }
            var observed = saved
            while !Task.isCancelled,
                  execution.permissionConfigurationOwner == .remote(identity),
                  execution.remotePermissionCommitRevisions[conversationID] == saved.configuration_revision,
                  observed.active_run_id == identity.submission_id,
                  observed.state == .pending {
                do {
                    try await Task.sleep(for: .milliseconds(750))
                    let next = try await provider.permissionConfiguration(owner: identity, minimumPolicyRevision: saved.configuration_revision)
                    guard !Task.isCancelled,
                          execution.permissionConfigurationOwner == .remote(identity),
                          execution.remotePermissionCommitRevisions[conversationID] == saved.configuration_revision else { return }
                    observed = next
                    self.remotePermissionConfigurations[conversationID] = next
                    execution.remotePermissionConfigurations[conversationID] = next
                } catch { return }
            }
        }
    }

    private func applyModelToContainer(_ model: ModelInfo) {
        guard !isConversationExecution else { return }
        container.useLocalModel = model.isLocal
        if model.isWebBackend {
            let baseURL = (model.baseURL ?? container.baseURLString).trimmingCharacters(in: .whitespacesAndNewlines)
            let token = (model.resolvedAPIKey ?? container.apiKey).trimmingCharacters(in: .whitespacesAndNewlines)
            let upstreamId = (model.upstreamModelId ?? container.model).trimmingCharacters(in: .whitespacesAndNewlines)
            container.baseURLString = baseURL
            container.model = upstreamId.isEmpty ? "web-host" : upstreamId
            container.apiKey = token
            if baseURL.isEmpty || token.isEmpty {
                self.modelFallbackNotice = NSLocalizedString(
                    "chat.web_host.missing_config_notice",
                    value: "The host is not configured. Pair this device from Nola on your Mac before using host tools.",
                    comment: "Incomplete host configuration notice"
                )
                logger.log(.warn, "ChatManager", "Selected Web Host model has incomplete config", metadata: [
                    "hasBaseURL": String(!baseURL.isEmpty),
                    "hasToken": String(!token.isEmpty)
                ])
            } else {
                self.modelFallbackNotice = nil
            }
        } else if !model.isLocal {
            let configured = try? ModelSelectionManager.resolvedCloudModelForRun(model)
            let baseURL = (configured?.baseURL ?? model.baseURL ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            let upstreamId = (configured ?? model).effectiveUpstreamModelId.trimmingCharacters(in: .whitespacesAndNewlines)
            let apiKey = configured?.apiKey ?? ""

            if configured == nil || baseURL.isEmpty || upstreamId.isEmpty ||
                (apiKey.isEmpty && configured?.cloudCredential != .anonymous) {
                container.baseURLString = baseURL
                container.model = upstreamId
                container.apiKey = ""
                self.modelFallbackNotice = NSLocalizedString("chat.model.missing_config_notice", value: "Model is not fully configured: add Base URL, upstream model ID, and API Key", comment: "Incomplete selected model configuration")
                logger.log(.warn, "ChatManager", "Selected cloud model has incomplete config", metadata: [
                    "modelName": model.name,
                    "modelId": model.id,
                    "hasBaseURL": String(!baseURL.isEmpty),
                    "hasUpstreamId": String(!upstreamId.isEmpty),
                    "hasApiKey": String(!apiKey.isEmpty)
                ])
            } else {
                container.baseURLString = baseURL
                container.model = upstreamId
                container.apiKey = apiKey
                self.modelFallbackNotice = nil
            }
        } else {
            self.modelFallbackNotice = nil
        }

        container.reconfigureIfPossible(selectedModel: model)
    }
    
    public func getCurrentModel() -> ModelInfo? {
        if let roomPreparedModel { return roomPreparedModel.displayModel }
        if let currentConversationId { return conversation(for: currentConversationId)?.selectedModelInfo }
        return selectedModelInfo ?? ModelSelectionManager.getLastSelectedModel()
    }

    public func currentContextTokenLimit() -> Int {
        guard let model = getCurrentModel() else { return 4_096 }
        guard let params = try? generationParameters(for: model, conversationID: currentConversationId) else { return 4_096 }
        return max(512, params.maxContextLength)
    }

    func generationOverride(for model: ModelInfo, conversationID: UUID?) -> IOSConversationGenerationOverride {
        guard let conversationID,
              let store = container.conversationStore as? any ConversationGenerationOverrideStore else { return .init() }
        return (try? store.generationOverride(
            conversationId: conversationID,
            modelKey: IOSConversationGenerationOverride.modelKey(for: model)
        )) ?? .init()
    }

    func setGenerationOverrideForCurrentConversation(
        _ value: IOSConversationGenerationOverride,
        model: ModelInfo
    ) throws {
        guard let store = container.conversationStore as? any ConversationGenerationOverrideStore,
              let conversationID = ensureCurrentConversation() else {
            throw ChatManagerError.failedToCreateConversation("conversation store unavailable")
        }
        try store.setGenerationOverride(
            conversationId: conversationID,
            modelKey: IOSConversationGenerationOverride.modelKey(for: model),
            override: value
        )
    }

    func generationParameters(for model: ModelInfo, conversationID: UUID?) throws -> ModelParameters {
        guard let conversationID else {
            return ModelParametersStore.shared.loadParameters(for: model.id, isLocal: model.isLocal)
        }
        guard let store = container.conversationStore as? any ConversationParameterSnapshotStore else {
            throw ChatManagerError.failedToCreateConversation("Conversation parameter repository unavailable")
        }
        return try store.conversationParameters(conversationId: conversationID,
            modelKey: IOSConversationGenerationOverride.modelKey(for: model))
    }

    private var resolvedAutoCompactTriggerPercent: Int {
        do {
            return try IOSChatPreferences.load().autoCompactTriggerPercent
        } catch {
            logger.log(.error, "ChatManager", "Failed to load canonical chat preferences", metadata: [
                "error": error.localizedDescription
            ])
            return Self.autoCompactTriggerPercentDefault
        }
    }

    @discardableResult
    public func ensureCurrentConversation(title: String? = nil, language: String? = nil, initialParameters: ModelParameters? = nil,
                                         selectOnCreate: Bool = true) -> UUID? {
        guard let store = container.conversationStore else { return nil }
        if let currentConversationId {
            do {
                guard try store.getConversation(conversationId: currentConversationId) != nil else {
                    logger.log(.warn, "ChatManager", "Selected conversation is missing; explicit new conversation required")
                    return nil
                }
                return currentConversationId
            } catch {
                logger.log(.error, "ChatManager", "Selected conversation could not be read", metadata: ["error": error.localizedDescription])
                return nil
            }
        }
        let resolvedTitle = title ?? L10n.chatConversationDefaultTitle
        let convId = startConversation(title: resolvedTitle, language: language, initialParameters: initialParameters, selectOnCreate: selectOnCreate)
        if selectOnCreate { currentConversationId = convId }
        return convId
    }

    public func conversation(for conversationId: UUID? = nil) -> Conversation? {
        guard let store = container.conversationStore else { return nil }
        guard let resolvedId = conversationId ?? currentConversationId else { return nil }
        return try? store.getConversation(conversationId: resolvedId)
    }

    @discardableResult
    public func updateConversationContext(
        promptTemplateID: UUID?,
        dataSourceOverrides: [String: Bool],
        for conversationId: UUID? = nil
    ) throws -> UUID {
        guard let store = container.conversationStore else {
            throw ChatManagerError.failedToCreateConversation("conversation store unavailable")
        }
        guard let convId = conversationId ?? ensureCurrentConversation() else {
            throw ChatManagerError.failedToCreateConversation("failed to initialize conversation")
        }
        try store.updateConversationMetadata(
            conversationId: convId, language: nil,
            selectedPromptTemplateID: promptTemplateID,
            dataSourceOverrides: dataSourceOverrides
        )
        if conversationId == nil {
            currentConversationId = convId
        }
        logger.log(.info, "ChatManager", "Updated conversation context", metadata: [
            "conversationId": convId.uuidString,
            "hasPromptTemplate": String(promptTemplateID != nil),
            "dataSourceOverrideCount": String(dataSourceOverrides.count)
        ])
        return convId
    }

    public func contextUsage(for conversationId: UUID? = nil) async -> ConversationContextUsage? {
        guard let resolvedId = conversationId ?? currentConversationId else { return nil }
        // Missing committed occupancy is unknown, not a history/summary token
        // estimate. In particular, never carry the previous Run's totals into
        // a newly admitted Run that has not published usage yet.
        var breakdown = ConversationContextUsage.withoutBreakdown()
        if let ctx = effectiveContextManager,
           let observed = restoredCoreContextUsageEvidence(conversationId: resolvedId), observed.hasCommittedTokenTotals {
            do {
                breakdown = try await ctx.estimateContextUsage(for: resolvedId, maxTokens: observed.limitTokens)
            } catch {
                logger.log(.warn, "ChatManager", "Context breakdown unavailable; committed totals are preserved", metadata: ["error": error.localizedDescription])
            }
        }
        // Re-read after the asynchronous breakdown: a newer Run or auxiliary
        // receipt may have committed while it was loading. Never publish the
        // earlier Run's occupancy or hide durable totals if the estimate fails.
        guard let exact = restoredCoreContextUsageEvidence(conversationId: resolvedId) else { return nil }
        return breakdown.replacingTokenTotals(
                usedTokens: exact.usedTokens,
                limitTokens: exact.limitTokens,
                cachedInputTokens: exact.cachedInputTokens,
                cacheWriteInputTokens: exact.cacheWriteInputTokens,
                promptCacheStatus: exact.promptCacheStatus,
                cacheHitRateBps: exact.cacheHitRateBps,
                cacheReportedRequests: exact.cacheReportedRequests,
                cacheUnreportedRequests: exact.cacheUnreportedRequests,
                hasCommittedTokenTotals: exact.hasCommittedTokenTotals,
                contextIsEstimated: exact.isEstimated,
                measuredRequestID: exact.measuredRequestID,
                cacheDiagnosticRequestID: exact.cacheDiagnosticRequestID,
                cacheDiagnosticAttemptID: exact.cacheDiagnosticAttemptID
            )
    }

    private func restoredCoreContextUsageEvidence(
        conversationId: UUID
    ) -> IOSContextUsageEvidence? {
        // A bounded exact-Run control read replaces full projection/history
        // scans and metadata/trace fallbacks, including after a cold reopen.
        guard let raw = try? container.applicationRuntime.executeAgentCommand(.object([
            "type": .string("load_conversation_context_usage_v1"),
            "conversation_id": .string(conversationId.uuidString),
        ])), let data = try? JSONEncoder().encode(raw),
              let usage = try? JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        else { return nil }
        return IOSContextUsageEvidence.committed(usage)
    }

    public func searchConversationRawMemory(
        _ query: String,
        for conversationId: UUID? = nil,
        limit: Int = 12
    ) async throws -> [ContextMemorySearchHit] {
        guard let ctx = effectiveContextManager else {
            throw ChatManagerError.failedToCreateConversation("context manager unavailable")
        }
        guard let resolvedId = conversationId ?? currentConversationId else {
            return []
        }
        return try await ctx.searchRawMemory(for: resolvedId, query: query, limit: limit)
    }

    public func chatMemoryProfile() -> ChatMemoryProfile {
        container.chatMemoryProfileStore.loadProfile()
    }

    @discardableResult
    public func updateContextMemoryText(
        sessionMemory: String,
        longTermMemory: String,
        userPreferences: String,
        expected: ChatMemoryProfile,
        for conversationId: UUID? = nil
    ) async throws -> ChatMemoryProfile {
        let saved = try container.chatMemoryProfileStore.saveMemoryText(
            longTermMemory: longTermMemory, userPreferences: userPreferences, expected: expected
        )
        if conversationId != nil || currentConversationId != nil || !sessionMemory.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            _ = try await updateSessionMemory(sessionMemory, for: conversationId)
        }
        return saved
    }

    @discardableResult
    public func updateSessionMemory(_ sessionMemory: String, for conversationId: UUID? = nil) async throws -> UUID {
        guard let ctx = effectiveContextManager else {
            throw ChatManagerError.failedToCreateConversation("context manager unavailable")
        }
        guard let convId = conversationId ?? ensureCurrentConversation() else {
            throw ChatManagerError.failedToCreateConversation("failed to initialize conversation")
        }
        try await ctx.updateSessionMemory(for: convId, sessionMemory: sessionMemory)
        return convId
    }

    @discardableResult
    public func updateConversationKnowledge(
        knowledgePoints: String,
        wiki: String,
        for conversationId: UUID? = nil
    ) throws -> UUID {
        guard let store = container.conversationStore else {
            throw ChatManagerError.failedToCreateConversation("conversation store unavailable")
        }
        guard let convId = conversationId ?? ensureCurrentConversation() else {
            throw ChatManagerError.failedToCreateConversation("failed to initialize conversation")
        }
        guard var conversation = try store.getConversation(conversationId: convId) else {
            throw ChatManagerError.failedToCreateConversation("conversation not found")
        }
        conversation.knowledgePoints = knowledgePoints.trimmingCharacters(in: .whitespacesAndNewlines)
        conversation.wiki = wiki.trimmingCharacters(in: .whitespacesAndNewlines)
        conversation.lastActiveAt = Date()
        try store.upsertConversation(conversation)
        return convId
    }

    @discardableResult
    public func compactConversationContext(for conversationId: UUID? = nil, preserveRecentMessages: Int = 8, force: Bool = true) async throws -> (conversationID: UUID, notice: String) {
        guard let ctx = effectiveContextManager else {
            throw ChatManagerError.failedToCreateConversation("context manager unavailable")
        }
        guard let convId = conversationId ?? ensureCurrentConversation() else {
            throw ChatManagerError.failedToCreateConversation("failed to initialize conversation")
        }
        let execution = isConversationExecution ? self : ensureExecutionForConversation(convId)
        let isRemote = try container.conversationStore?.getConversation(conversationId: convId)?.selectedModelInfo?.isWebBackend == true
        if isRemote {
            guard let execution, !execution.processingState.blocksInput,
                  let provider = execution.remoteSubmissionProviders[convId.uuidString]
                    ?? (execution.effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider,
                  let store = container.conversationStore as? any ConversationRemoteTurnStore else {
                throw IOSApplicationRuntimeError.rejected("Host context maintenance is unavailable while its execution owner is unresolved")
            }
            let snapshot = try container.applicationRuntime.loadConversation(id: convId.uuidString)
            guard let owner = try store.confirmedTerminalOwner(conversationID: convId.uuidString,
                observedMessage: snapshot?.messages.last) else {
                throw IOSApplicationRuntimeError.rejected("Host context maintenance requires a confirmed conversation admission")
            }
            let receipt = try await provider.compactContext(owner: owner)
            await execution.restoreRemoteJournalIfNeeded()
            let notice = receipt.compacted
                ? String(format: NSLocalizedString("chat.context.host.compacted", value: "Host context compacted (%lld archived items).", comment: "Authoritative Host maintenance result"), Int64(receipt.context.replaced_item_count))
                : NSLocalizedString("chat.context.host.no_history", value: "No complete historical turn is available to compact on the Host.", comment: "Host maintenance had no eligible history")
            return (convId, notice)
        }
        _ = try await ctx.compactConversation(
            for: convId,
            maxTokens: currentContextTokenLimit(),
            preserveRecentMessages: preserveRecentMessages,
            force: force
        )
        return (convId, NSLocalizedString("chat.context.compact.completed", value: "Context maintenance completed.", comment: "Local context maintenance result"))
    }
    
    /// Dispatch is owned by the conversation execution, not the selected page.
    func dispatchQueuedUIInput(conversationID: UUID, commandID: String) {
        guard let execution = ensureExecutionForConversation(conversationID) else { return }
        Task { [weak execution] in
            guard let execution else { return }
            do { try await execution.processQueuedUIInput(commandID: commandID) }
            catch {
                execution.logger.log(.error, "ChatManager", "Queued UI input dispatch failed",
                    metadata: ["error": error.localizedDescription])
                execution.processingState = .error(error.localizedDescription)
            }
        }
    }

    private func processQueuedUIInput(commandID: String) async throws {
        guard isConversationExecution, let conversationID = currentConversationId,
              !messageInFlight, !processingState.blocksInput, deviceAgentCoordinator == nil,
              getCurrentModel()?.isWebBackend != true else { return }
        let runtime = container.applicationRuntime
        let projection = try runtime.loadConversationProjectionV1(id: conversationID.uuidString)
        guard projection.activeRun == nil else { return } // Active owner consumes its boundary queue.
        let head = try runtime.executeAgentCommand(.object([
            "type": .string("peek_agent_command"), "session_id": .string(conversationID.uuidString)]))
        guard case .object(let fields) = head, fields["command_id"] == .string(commandID) else { return }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let leaseOwner = "ios-ui:\(UUID().uuidString.lowercased())"
        let claimed = try runtime.executeAgentCommand(.object([
            "type": .string("claim_next_agent_command"), "session_id": .string(conversationID.uuidString),
            "lease_owner": .string(leaseOwner), "now_ms": .signedInteger(now),
            "lease_until_ms": .signedInteger(now + 300_000)]))
        guard claimed != .null else { return }
        // Release only this dispatcher's lease on decode/admission failure or
        // after completion. A consumed command makes this an idempotent no-op.
        defer {
            if case .object(let leaseFields) = claimed,
               leaseFields["session_id"] == .string(conversationID.uuidString),
               leaseFields["lease_owner"] == .string(leaseOwner),
               case .string(let claimedID)? = leaseFields["command_id"] {
                _ = try? runtime.executeAgentCommand(.object([
                    "type": .string("release_agent_command_lease"),
                    "session_id": .string(conversationID.uuidString),
                    "command_id": .string(claimedID), "lease_owner": .string(leaseOwner)]))
            }
        }

        struct Lease: Decodable {
            let session_id: String; let command_id: String; let lease_owner: String
            let delivery_attempt: Int64; let payload_json: String
        }
        let lease = try JSONDecoder().decode(Lease.self, from: JSONEncoder().encode(claimed))
        let envelope = try JSONDecoder().decode(AppleUIQueuedSubmission.self, from: Data(lease.payload_json.utf8))
        guard lease.session_id == conversationID.uuidString, lease.command_id == commandID,
              lease.lease_owner == leaseOwner, lease.delivery_attempt > 0 else {
            throw IOSApplicationRuntimeError.rejected("Invalid queued UI input identity")
        }
        let item = try envelope.item(commandID: commandID)
        queuedUIInput = (.init(commandID: commandID, leaseOwner: leaseOwner, deliveryAttempt: lease.delivery_attempt),
            .init(queuedSubmissionID: commandID, userMessageID: item.id))
        defer { queuedUIInput = nil }
        _ = try await processMessage(item.action.summary, inputOrigin: .interactiveUser)
    }

    /// Core is authoritative after cancellation, errors, and cold recovery.
    /// Check before optimistic UI staging and again at the execution boundary.
    func validateNewMessageAdmission() throws {
        if !isConversationExecution {
            try selectedExecution?.validateNewMessageAdmission()
            return
        }
        guard let conversationID = currentConversationId,
              selectedModelInfo?.isWebBackend != true else { return }
        try Self.validateLocalRunAdmission(runtime: container.applicationRuntime, conversationID: conversationID.uuidString)
    }

    static func validateLocalRunAdmission(runtime: IOSApplicationRuntime, conversationID: String) throws {
        let active = try runtime.loadConversationProjectionWindowV1(id: conversationID,
            beforeOrdinal: nil, limit: 1, contentByteLimit: 1_024).projection.activeRun
        if active != nil { throw IOSLocalRunAdmissionError.activeRun }
    }

    public func processMessage(
        _ message: String,
        displayMessage: String? = nil,
        userMessageMeta: [String: String]? = nil,
        attachments: [ModelInput.Attachment] = [],
        userMessageID: UUID? = nil,
        assistantMessageID: UUID? = nil,
        agentMode _: Bool = false,
        responseStyle: PromptResponseStyle? = nil,
        inputOrigin: AgentCoreInputOrigin = .interactiveUser
    ) async throws -> String {
        if !isConversationExecution {
            guard let conversationID = ensureCurrentConversation(),
                  let session = executionForConversation(conversationID, prepareForSubmission: true) else {
                throw ChatManagerError.failedToCreateConversation("Missing conversation execution")
            }
            // The session and provider are captured before suspension. Another
            // selection never changes this request's owner or cancellation target.
            session.enableRetrieval = enableRetrieval
            session.preferredNativeAppHostId = preferredNativeAppHostId
            session.hybridDeviceToolsEnabled = hybridDeviceToolsEnabled
            session.hybridDeviceCapabilities = hybridDeviceCapabilities
            return try await session.processMessage(message, displayMessage: displayMessage,
                userMessageMeta: userMessageMeta, attachments: attachments,
                userMessageID: userMessageID, assistantMessageID: assistantMessageID,
                responseStyle: responseStyle, inputOrigin: inputOrigin)
        }
        guard !messageInFlight, !processingState.blocksInput, deviceAgentCoordinator == nil else {
            throw DeviceAgentCoordinatorError.alreadyRunning
        }
        try validateNewMessageAdmission()
        messageInFlight = true
        defer {
            messageInFlight = false
            scheduleQueuedSuccessorIfReady()
            if remoteUIWakeRequested { wakeRemoteUIQueue() }
        }
        guard roomRemoteReplayIdentity != nil || (!remoteObservationUnresolved && observedRemoteRun == nil) else {
            throw IOSApplicationRuntimeError.rejected("The existing Host Run must be observed or resumed before another input")
        }
        // Retire the proof before any submission can reach a remote peer. A
        // failure/interrupt after this point must use normal recovery guards.
        knownUnsubmittedConversationID = nil
        remoteRunObserver.cancel()
        // Bump generation: any older streaming loop now sees a stale id and
        // aborts before persisting / mutating @Published state.
        activeGenerationId &+= 1
        let myGeneration = activeGenerationId
        if (effectiveLLMClient as? ModelService)?.currentProvider() is WebAgentProvider {
            remoteStreamGeneration = myGeneration
        }
        defer {
            if remoteStreamGeneration == myGeneration { remoteStreamGeneration = nil }
            if !roomRemotePreparationOnly, activeGenerationId == myGeneration,
               (effectiveLLMClient as? ModelService)?.currentProvider() is WebAgentProvider {
                Task { [weak self] in await self?.restoreRemoteJournalIfNeeded() }
            }
        }
        committedAssistantOwner = nil
        committedAssistantProjection = nil
        committedJournalPresentation = nil
        remoteJournalPresentationCache = IOSRemoteJournalPresentationCache()
        let resolvedUserMessageID = userMessageID ?? UUID()
        let turnIdentity = if let roomCanonicalIdentity {
            roomCanonicalIdentity
        } else if let queuedUIInput {
            queuedUIInput.identity
        } else if let remoteQueuedUIIdentity {
            remoteQueuedUIIdentity
        } else if let assistantMessageID {
            try IOSCanonicalAgentTurnIdentity(
                userMessageID: resolvedUserMessageID,
                validatingVisibleAssistantMessageID: assistantMessageID
            )
        } else {
            IOSCanonicalAgentTurnIdentity(userMessageID: resolvedUserMessageID)
        }
        var canonicalUserMessageMeta = mergedUserMessageMeta(
            displayMessage: displayMessage,
            extra: userMessageMeta
        ) ?? [:]
        canonicalUserMessageMeta["nola.canonicalMessageID"] = turnIdentity.userMessageID
        let submittedConversation = currentConversationId.flatMap { id in
            try? container.applicationRuntime.loadConversation(id: id.uuidString)
        }
        let submittedConfigurationRevision = submittedConversation?.configurationRevision
        let submittedParentMessageID = submittedConversation?.messages.last?.id

        guard let model = getCurrentModel() else {
            let error = ChatManagerError.noModelSelected
            processingState = .error(error.errorDescription ?? L10n.chatErrorUnknown)
            throw error
        }
        guard (!roomRemotePreparationOnly && roomRemoteReplayIdentity == nil) || model.isWebBackend else {
            throw IOSApplicationRuntimeError.rejected("Room remote execution cannot switch to a device model")
        }
        // Direct local and API-backed models share the same Core Agent path.
        // A Web-backed model is already executed by its remote Agent Host and
        // must not be wrapped in a second device Agent.
        if !model.isWebBackend && !IOSApplicationRuntime.isAvailable {
            throw IOSApplicationRuntimeError.applicationUnavailable
        }
        let useAutomaticDeviceAgent = !model.isWebBackend
        hostToolEvents = []
        
        if model.isLocal {
            processingState = .loadingLocalModel
            logger.log(.info, "ChatManager", "Loading local model", metadata: ["model": model.name])
            
            do {
                try await ensureLocalModelLoaded()
            } catch {
                processingState = .error(L10n.format("chat.error.failed_to_load_local_model_format", error.localizedDescription))
                logger.log(.error, "ChatManager", "Failed to load local model", 
                          metadata: ["error": error.localizedDescription])
                throw error
            }
        }
        
        var generationConversationId = currentConversationId
        var canonicalRemoteTurnStore: (any ConversationRemoteTurnStore)?
        var remoteSubmissionRetired = false
        let journalProvider = (effectiveLLMClient as? ModelService)?.currentProvider() as? WebAgentProvider
        var journalConsumer: IOSAssistantJournalConsumer?
        if let store = container.conversationStore {
            if currentConversationId == nil {
                if let convId = startConversation(title: firstQueryTitle(from: displayMessage ?? message) ?? L10n.chatConversationDefaultTitle, language: nil) {
                    currentConversationId = convId
                    generationConversationId = convId
                } else {
                    logger.log(.warn, "ChatManager", "Failed to initialize conversation")
                }
            }
            
            if let convId = currentConversationId {
                do {
                    if let _ = try store.getConversation(conversationId: convId) {
                        if !useAutomaticDeviceAgent {
                            guard let remoteStore = store as? any ConversationRemoteTurnStore else {
                                throw IOSApplicationRuntimeError.applicationUnavailable
                            }
                            let title = try store.getConversation(conversationId: convId)?.title ?? ""
                            let existingRoomRequest = roomRemoteReplayIdentity.flatMap { identity in
                                try? remoteStore.remoteSubmissionIntent(conversationID: identity.conversation_id)
                            }
                            if existingRoomRequest == nil {
                            try remoteStore.prepareRemoteSubmission(.init(
                                sourceHostID: try requireJournalSourceHost(journalProvider),
                                clientID: try requireJournalClient(journalProvider),
                                conversationID: convId.uuidString,
                                conversationTitle: title,
                                identity: turnIdentity,
                                userText: message,
                                userContentJSON: try? String(decoding: JSONEncoder().encode(canonicalUserMessageMeta), as: UTF8.self),
                                contextCheckpointID: nil,
                                createdAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
                            ))
                            }
                            canonicalRemoteTurnStore = remoteStore
                        }
                        generationConversationId = convId
                        logger.log(.debug, "ChatManager", useAutomaticDeviceAgent ? "Canonical Run prepared" : "User message appended", metadata: ["conversationId": convId.uuidString])
                    } else {
                        throw IOSApplicationRuntimeError.rejected("The original conversation was deleted. The captured submission is retained; create a new conversation explicitly.")
                    }
                } catch {
                    logger.log(.error, "ChatManager", "Failed to append user message", metadata: ["error": error.localizedDescription])
                    throw error
                }
            }
        }

        activeHostConversationId = model.isWebBackend ? generationConversationId?.uuidString : nil
        if model.isWebBackend, let journalProvider, let canonicalRemoteTurnStore,
           let conversationID = generationConversationId?.uuidString {
            journalConsumer = try IOSAssistantJournalConsumer(provider: journalProvider, store: canonicalRemoteTurnStore,
                conversationID: conversationID, runID: turnIdentity.submissionID, assistantID: turnIdentity.assistantMessageID)
        }
        if let conversationID = generationConversationId {
            observeCommittedAssistant(
                conversationID: conversationID.uuidString,
                assistantMessageID: turnIdentity.assistantMessageID, generation: myGeneration
            )
        }
        defer {
            if activeGenerationId == myGeneration {
                activeHostConversationId = nil
            }
        }

        // Freeze the effective model parameters after resolving/creating the
        // conversation, before preparing context or starting provider work.
        let uiParams: ModelParameters
        if let roomPreparedModel { uiParams = roomPreparedModel.parameters }
        else { uiParams = try generationParameters(for: model, conversationID: generationConversationId) }
        let resolvedResponseStyle = roomPreparedModel?.parameters.responseStyle ?? responseStyle ?? conversation()?.promptResponseStyle ?? uiParams.responseStyle
        let thinkingEnabled = uiParams.showThinking
        guard ModelParameterLimits.remoteContextTokens.contains(uiParams.maxContextLength),
              ModelParameterLimits.outputTokens.contains(uiParams.maxOutputTokens) else {
            throw NolaError.llm(.invalidInput(reason: "Invalid conversation model limits"))
        }
        let preparedRunModel: IOSPreparedRunModel?
        if let roomPreparedModel { preparedRunModel = roomPreparedModel }
        else { preparedRunModel = useAutomaticDeviceAgent
            ? try IOSPreparedRunModel.prepare(model: model, parameters: uiParams,
                resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: self.container.applicationRuntime, parameters: uiParams) }) : nil }

        processingState = model.isLocal ? .processingWithLocal : .processingWithCloud
        beginAgentTrace(
            conversationId: generationConversationId,
            assistantMessageID: turnIdentity.assistantMessageID
        )
        logger.log(.info, "ChatManager", "Processing message", metadata: [
            "provider": model.isLocal ? "local" : "cloud",
            "messageLength": String(message.count)
        ])
        
        // Track whether we started streaming so catch can decide how to transition UI state
        var responseBasis: Set<String> = []
        if (Int(userMessageMeta?["attachmentCount"] ?? "") ?? 0) > 0 || !attachments.isEmpty {
            responseBasis.insert("file")
        }
        latestResponseMeta = [:]

        do {
            var promptToSend = message
            var preparedCorePayload: CorePreparedOutboundPayload?
            var preparedPromptComposition: AgentCorePromptComposition?
            if roomRemoteReplayIdentity == nil, let ctx = effectiveContextManager, let convId = generationConversationId {
                let maxTokens = uiParams.maxContextLength
                do {
                    logger.log(.debug, "ChatManager", ">>> Building context", metadata: ["conversationId": convId.uuidString])
                    let responseStyleInstruction = PromptTemplates.agentResponseStyle(resolvedResponseStyle)
                    let customInstructions = uiParams.customSystemPrompt
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    var userInstructionFragments: [AgentCorePromptFragment] = [
                        .init(
                            id: "ios:user-response-style",
                            layer: .responseStyle,
                            trust: .userInstruction,
                            cacheable: false,
                            content: responseStyleInstruction
                        )
                    ]
                    if !customInstructions.isEmpty {
                        userInstructionFragments.append(.init(
                            id: "ios:user-custom-instructions",
                            layer: .responseStyle,
                            trust: .userInstruction,
                            cacheable: false,
                            content: customInstructions
                        ))
                    }
                    var components: [AgentCoreContextComponent] = []
                    var instructionContextMessageIDs = Set<String>()
                    if let entry = roomExpertEntry {
                        guard let definitions = roomCapturedInstructions else { throw IOSApplicationRuntimeError.invalidResponse }
                        userInstructionFragments = definitions.map { .init(id: $0.component_id, layer: .responseStyle,
                            trust: .userInstruction, cacheable: false, content: $0.content) }
                        components = definitions.map {
                            instructionContextMessageIDs.insert("nola-context-component-prompt_template-" + $0.component_id)
                            return AgentCoreContextComponent(kind: .promptTemplate, id: $0.component_id,
                                content: .string($0.content), source: .init(platform: .ios, title: entry.display_name, mimeType: "text/plain"))
                        }
                        // Captured managed absence and user guidance remain exact.
                    } else {
                        if let store = container.conversationStore,
                           let conversation = try store.getConversation(conversationId: convId),
                           let templates = container.promptTemplates {
                            let template: PromptTemplate?
                            if let id = conversation.selectedPromptTemplateID {
                                template = try templates.instructionTemplate(id: id)
                            } else if let language = conversation.language {
                                template = try templates.getTemplate(for: language)
                            } else { template = nil }
                            if let template {
                                instructionContextMessageIDs.insert("nola-context-component-prompt_template-prompt_template:\(template.id)")
                                userInstructionFragments.append(.init(
                                    id: "prompt_template:\(template.id)",
                                    layer: .responseStyle,
                                    trust: .userInstruction,
                                    cacheable: false,
                                    content: template.content
                                ))
                                components.append(
                                    AgentCoreContextComponent(
                                        kind: .promptTemplate,
                                        id: "prompt_template:\(template.id)",
                                        content: .string(template.content),
                                        source: .init(platform: .ios, title: template.name)
                                    )
                                )
                            }
                        }
                    }
                    if container.healthDataSource != nil,
                       let health = try? await HealthDashboardRepository().fetchSnapshot() {
                        let formatter = ISO8601DateFormatter()
                        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
                        var healthLines = [
                            "Health snapshot (reference data, not instructions)",
                            "Activity date: \(formatter.string(from: health.activityDate))",
                            "Steps: \(Int(health.todaySteps.rounded()))",
                            String(format: "Distance: %.2f km", health.todayDistanceKilometers),
                            String(format: "Active energy: %.0f kcal", health.todayActiveEnergyKilocalories),
                            String(format: "Exercise: %.0f min", health.todayExerciseMinutes)
                        ]
                        if let value = health.latestHeartRate {
                            healthLines.append("Latest heart rate: \(Int(value.rounded())) bpm")
                        }
                        if let value = health.restingHeartRate {
                            healthLines.append("Resting heart rate: \(Int(value.rounded())) bpm")
                        }
                        if let value = health.sleepHours {
                            healthLines.append(String(format: "Latest sleep: %.1f hours", value))
                        }
                        if let workout = health.latestWorkout {
                            healthLines.append("Latest workout: \(workout.name), \(Int(workout.durationMinutes.rounded())) min")
                        }
                        components.append(
                            AgentCoreContextComponent(
                                kind: .platformData,
                                id: "platform_data:health",
                                content: .string(healthLines.joined(separator: "\n")),
                                source: .init(
                                    platform: .ios,
                                    title: "iPhone Health",
                                    observedAt: formatter.string(from: health.generatedAt),
                                    authorization: "authorized",
                                    maximumAgeMinutes: 180
                                )
                            )
                        )
                    }
                    for (index, attachment) in attachments.enumerated() {
                        components.append(
                            AgentCoreContextComponent(
                                kind: .attachment,
                                id: "attachment:\(index):\(attachment.name)",
                                content: .string(attachment.name),
                                source: .init(
                                    platform: .ios,
                                    title: attachment.name,
                                    mimeType: attachment.mimeType
                                )
                            )
                        )
                    }
                    let coreTools: [AgentCoreJSONValue] = {
                        guard useAutomaticDeviceAgent else { return [] }
                        let identity = AgentCoreExpectedIdentity(
                            hostId: "ios-local",
                            clientId: Bundle.main.bundleIdentifier ?? "nola-ios",
                            sessionId: convId.uuidString
                        )
                        return (try? AgentCoreToolCatalog.prepareBoundPlan(
                            for: .deviceLocal,
                            expectedIdentity: identity,
                            isForeground: UIApplication.shared.applicationState == .active,
                            scopedGrants: agentCorePermissionBroker.scopedGrants
                        ).schemas) ?? []
                    }()
                    let payload = try await ctx.prepareOutboundPayload(
                        for: convId,
                        maxTokens: maxTokens,
                        preserveRecentMessages: 8,
                        additionalComponents: components,
                        tools: coreTools
                    )
                    preparedCorePayload = payload
                    var promptFragments: [AgentCorePromptFragment] = [
                        .init(
                            id: "ios:host-identity",
                            layer: .identity,
                            trust: .hostFact,
                            cacheable: true,
                            content: PromptTemplates.agentIdentity
                        )
                    ]
                    promptFragments.append(contentsOf: userInstructionFragments)
                    let composition = try AgentCorePromptComposer.compose(promptFragments)
                    preparedPromptComposition = composition
                    let coreScopedContext = payload.messages
                        .filter { message in
                            message.role == "system" && !instructionContextMessageIDs.contains(message.id)
                        }
                        .map(\.content)
                    let renderedSystemPrompt = ([composition.rendered] + coreScopedContext)
                        .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
                        .joined(separator: "\n\n")
                    promptToSend = try payload.providerEnvelopeJSON(
                        systemPrompt: renderedSystemPrompt,
                        userQuery: message
                    )
                    currentTokenCount = payload.metrics.totalPromptTokens
                } catch {
                    logger.log(.error, "ChatManager", "Outbound context preparation failed; request blocked", metadata: ["error": error.localizedDescription])
                    throw error
                }
            }

            logger.log(.debug, "ChatManager", ">>> Entering Core-routed chat mode", metadata: [
                "inputPreview": String(message.prefix(320)),
                "inputLength": String(message.count)
            ])

            let preparedAgentTurn = ConversationTurnInput(
                currentUserText: message,
                historySource: .prepared((preparedCorePayload?.messages ?? []).map {
                    .init(role: $0.role, content: $0.content)
                })
            )

            func recordPreparedDataSourceUsage() async {
                guard let payload = preparedCorePayload,
                      let conversationId = generationConversationId else { return }
                await effectiveContextManager?.recordPreparedDataSourceUsage(
                    payload: payload,
                    conversationId: conversationId,
                    disclosedToCloud: !model.isLocal
                )
            }

            if useAutomaticDeviceAgent {
                guard let preparedRunModel else {
                    throw NolaError.llm(.invalidInput(reason: "Run model was not prepared"))
                }
                await recordPreparedDataSourceUsage()
                return try await runDeviceAgentTurn(
                    sessionId: generationConversationId?.uuidString,
                    input: preparedAgentTurn.input,
                    history: preparedAgentTurn.history.map { AgentCoreMessage(role: $0.role, content: $0.content) },
                    conversationId: generationConversationId,
                    preparedModel: preparedRunModel,
                    turnIdentity: turnIdentity,
                    persistedUserText: message,
                    userContentJSON: roomExpertEntry == nil ? (try? String(
                        decoding: JSONEncoder().encode(canonicalUserMessageMeta),
                        as: UTF8.self
                    )) : nil,
                    inputOrigin: inputOrigin,
                    expectedConfigurationRevision: submittedConfigurationRevision,
                    expectedParentMessageID: submittedParentMessageID,
                    contextComponents: preparedCorePayload?.contextComponents ?? [],
                    promptComposition: preparedPromptComposition,
                    queuedUserAdmission: queuedUIInput?.admission
                )
            }
            if model.isWebBackend {
            }

            await recordPreparedDataSourceUsage()

            // Use streaming API to progressively update UI (currentResponse) and expose internal reasoning via <think> blocks
            self.currentResponse = ""
            self.thinking = ""
            self.thinkingFinished = false
            self.hostToolEvents = []

            // If retrieval is enabled, mark retrievalInProgress true until the first token arrives
            if self.enableRetrieval {
                self.retrievalInProgress = true
            }

            // Load per-model parameters to decide stream display behavior
            let streamOutputEnabled = uiParams.streamOutput
            self.showThinking = thinkingEnabled

            // Build a minimal ModelInput for chat-mode streaming (providers will interpret userPrefs["message"]).
            var userPrefs: [String: String] = [
                "message": message,
                "response_style": resolvedResponseStyle.rawValue,
            ]
            if let preparedCorePayload {
                userPrefs["prepared_core_payload"] = "true"
                userPrefs["prepared_core_payload_digest"] = preparedCorePayload.payloadDigest
                userPrefs["prepared_core_payload_json"] = promptToSend
            }
            let customSystemPrompt = uiParams.customSystemPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
            if !customSystemPrompt.isEmpty,
               preparedCorePayload == nil || model.isWebBackend {
                userPrefs["custom_system_prompt"] = customSystemPrompt
            }
            if !thinkingEnabled {
                userPrefs["enable_thinking"] = "false"
            }
            if model.isWebBackend {
                // This is the committed, observed transcript parent captured
                // before optimistic staging. The local configuration revision
                // belongs to the device Store and is never a Host witness.
                userPrefs["expected_parent_message_id"] = submittedParentMessageID
                userPrefs["web_agent_max_iterations"] = "0"
                userPrefs["submission_id"] = turnIdentity.submissionID
                userPrefs["user_message_id"] = turnIdentity.userMessageID
                userPrefs["assistant_message_id"] = turnIdentity.assistantMessageID
                if let remoteQueuedUIIdentity {
                    userPrefs["queued_user_command_id"] = remoteQueuedUIIdentity.submissionID
                }
                if let convId = generationConversationId {
                    userPrefs["host_conversation_id"] = convId.uuidString
                    if let store = self.container.conversationStore,
                       let conversation = try? store.getConversation(conversationId: convId) {
                        userPrefs["execution_backend"] = conversation.executionBackend
                        userPrefs["permission_profile"] = conversation.permissionMode.rawValue
                        if let sessionID = conversation.externalExecutionSessions[conversation.executionBackend],
                           !sessionID.isEmpty {
                            userPrefs["external_session_id"] = sessionID
                        }
                    }
                }
                userPrefs["agent_mode"] = "true"
                if hybridDeviceToolsEnabled {
                    userPrefs["hybrid_device_tools_enabled"] = "true"
                    userPrefs["hybrid_device_capabilities"] = hybridDeviceCapabilities.sorted().joined(separator: ",")
                    userPrefs["hybrid_device_result_egress_enabled"] = "true"
                }
                let nativeAppHostId = preferredNativeAppHostId.trimmingCharacters(in: .whitespacesAndNewlines)
                if !nativeAppHostId.isEmpty {
                    userPrefs["native_app_host_id"] = nativeAppHostId
                }
            }

            let inputDateRange = ModelInput.DateRange(
                    start: ISO8601DateFormatter().string(from: Date().addingTimeInterval(-86400)),
                    end: ISO8601DateFormatter().string(from: Date())
            )
            func makeInput() -> ModelInput {
                return ModelInput(
                    dateRange: inputDateRange,
                    candidates: [],
                    userPrefs: userPrefs,
                    attachments: attachments
                )
            }
            let input = makeInput()

            logger.log(.debug, "ChatManager", ">>> Starting generateSummaryStream", metadata: [
                "messageLength": String(message.count),
                "promptLength": String(promptToSend.count),
                "model": model.name,
                "streamOutput": String(streamOutputEnabled),
                "thinkingEnabled": String(thinkingEnabled)
            ])
            var accumulated = ""
            var chunkIdx = 0
            var firstResponseLogged = false
            var webAgentThinkingOpen = false
            var emittedThinkingCharacterCount = 0
            var emittedResponseCharacterCount = 0

            logger.log(.debug, "ChatManager", ">>> Starting to iterate stream", metadata: [:])
            func upsertHostToolEvent(_ event: HostAgentToolEvent) {
                var updated = hostToolEvents
                if let index = updated.firstIndex(where: { $0.id == event.id }) {
                    updated[index] = event
                } else {
                    updated.append(event)
                }
                hostToolEvents = updated
            }

            func consumeStreamChunk(_ chunk: String) throws {
                if myGeneration != activeGenerationId {
                    logger.log(.info, "ChatManager", "Aborting stale stream (generation superseded)", metadata: [
                        "myGen": String(myGeneration),
                        "active": String(activeGenerationId)
                    ])
                    throw CancellationError()
                }
                chunkIdx += 1
                if chunkIdx == 1 {
                    // first token arrived — retrieval (if any) completed
                    self.retrievalInProgress = false
                }
                accumulated += chunk

                // Per-chunk debug log to help trace exactly what the client receives
                logger.log(.debug, "ChatManager", "Received chunk", metadata: [
                    "idx": String(chunkIdx),
                    "preview": String(chunk.prefix(80)),
                    "bytes": String(chunk.count)
                ])

                if chunkIdx == 1 {
                    let chunkPreview = String(chunk.prefix(80))
                    logger.log(.debug, "ChatManager", "Stream first chunk", metadata: [
                        "preview": chunkPreview,
                        "bytes": String(chunk.count)
                    ])
                }

                if chunkIdx % 10 == 1 {
                    logger.log(.debug, "ChatManager", ">>> Stream progress", metadata: [
                        "chunks": String(chunkIdx),
                        "totalBytes": String(accumulated.count)
                    ])
                }

                let parsed = ChatThinkingParser.splitThinkingBlocks(from: accumulated)
                if thinkingEnabled {
                    let thinkingDelta = try appendOnlyParsedDelta(
                        parsed.thinking,
                        emittedCharacterCount: &emittedThinkingCharacterCount,
                        channel: "thinking"
                    )
                    if !thinkingDelta.isEmpty {
                        thinking += thinkingDelta
                    }
                }
                if streamOutputEnabled {
                    let cleanedLeading = ChatTextPolicy.trimmingLeadingNewlines(from: parsed.response)
                    let responseDelta = try appendOnlyParsedDelta(
                        cleanedLeading,
                        emittedCharacterCount: &emittedResponseCharacterCount,
                        channel: "assistant"
                    )
                    if !responseDelta.isEmpty {
                        currentResponse += responseDelta
                    }
                    if !firstResponseLogged, !cleanedLeading.isEmpty {
                        firstResponseLogged = true
                        logger.log(.info, "ChatManager", "First streamed response", metadata: [
                            "preview": String(cleanedLeading.prefix(20)),
                            "bytes": String(cleanedLeading.count)
                        ])
                    }
                }

                // Heuristic progress update (best-effort, not exact)
                let prog = min(0.99, Double(chunkIdx) * 0.04)
                self.processingState = .streaming(progress: prog)
            }

            if model.isWebBackend {
                guard let journalProvider else {
                    throw NolaError.llm(.invalidResponse(reason: "The selected provider does not support Agent events."))
                }
                guard let remoteStore = canonicalRemoteTurnStore, let conversationID = generationConversationId?.uuidString,
                      let intent = try remoteStore.remoteSubmissionIntent(conversationID: conversationID) else {
                    throw IOSApplicationRuntimeError.rejected("Missing prepared remote submission")
                }
                let frozenPayload: AgentCoreJSONValue
                let bound: IOSRemoteSubmissionIntent
                if let replay = roomRemoteReplayIdentity {
                    guard intent.identity == replay, let digest = intent.request_digest else {
                        throw IOSApplicationRuntimeError.rejected("Room bound request identity changed")
                    }
                    frozenPayload = try remoteStore.remoteSubmissionRequest(identity: replay, digest: digest)
                    bound = intent
                } else {
                    let sourceWitness = try remoteStore.confirmedTerminalOwner(conversationID: conversationID,
                        observedMessage: submittedConversation?.messages.last)
                    let semanticRequest = try await journalProvider.prepareHostToolPlan(
                        journalProvider.makeHostAgentPayload(input: input), sourceRunID: sourceWitness?.run_id,
                        sourceAssistantMessageID: sourceWitness?.assistant_message_id)
                    guard myGeneration == activeGenerationId else { throw CancellationError() }
                    frozenPayload = try JSONDecoder().decode(AgentCoreJSONValue.self,
                        from: JSONSerialization.data(withJSONObject: semanticRequest))
                    bound = try remoteStore.bindRemoteSubmissionRequest(identity: intent.identity, request: semanticRequest)
                }
                guard bound.phase == "bound", let digest = bound.request_digest else {
                    throw IOSApplicationRuntimeError.rejected("Remote submission is not dispatchable")
                }
                remoteSubmissionProviders[conversationID] = journalProvider
                if roomRemotePreparationOnly {
                    processingState = .idle
                    return ""
                }
                observeRemoteAgentTasks(conversationId: conversationID)
                let eventStream = journalProvider.generateAgentEventStream(input: input, boundRequestDigest: digest, frozenPayload: frozenPayload)
                var sourceAdmitted = false
                var sourceDispatchError: String?
                defer {
                    if myGeneration == activeGenerationId { cancelHostPermissionInteraction() }
                }
                for try await event in eventStream {
                        guard myGeneration == activeGenerationId else { throw CancellationError() }
                        switch event {
                        case .submissionReceipt(let raw):
                            let receipt = try JSONDecoder().decode(IOSSourceSubmissionReceipt.self, from: JSONEncoder().encode(raw))
                            guard receipt.identity == bound.identity, receipt.request_digest == digest else {
                                throw IOSApplicationRuntimeError.rejected("Submission receipt identity changed")
                            }
                            let accepted = try remoteStore.acceptRemoteSubmissionReceipt(receipt)
                            sourceAdmitted = accepted.phase == "admitted"
                            if sourceAdmitted {
                                submissionCommitted?()
                                permissionOwnerProjectionRunID = nil
                                if let owner = generationConversationId { refreshPermissionOwnerProjection(conversationID: owner) }
                            }
                            if accepted.phase == "rejected" || accepted.phase == "cancelled" {
                                remoteSubmissionRetired = true
                                throw IOSApplicationRuntimeError.rejected(sourceDispatchError ?? receipt.rejection_code ?? "The original submission was cancelled before admission")
                            }
                        case .coreTurnTerminal(_, let trace):
                            if let trace {
                                agentTraceContext.adopt(trace)
                                _ = applyAgentTrace(trace)
                            }
                        case .coreTaskUpdated(let envelope):
                            guard currentConversationId == generationConversationId else { continue }
                            if webAgentTaskProjectionReducer.apply(
                                envelope,
                                expectedSessionId: envelope.sessionId,
                                expectedTurnId: envelope.turnId
                            ) == .applied {
                                agentTaskProjection = webAgentTaskProjectionReducer.projection
                            }
                        case .coreSkillRuntimeUpdated(let envelope):
                            if webAgentSkillRuntimeReducer.apply(
                                envelope,
                                expectedSessionID: envelope.sessionID,
                                expectedTurnID: envelope.turnID
                            ) == .applied {
                                applyAgentSkillRuntimeProjection(webAgentSkillRuntimeReducer.projection)
                            }
                        case .coreCapabilitiesActivated(let activation):
                            applyAgentCapabilityActivation(activation)
                        case .corePermissionProjectionUpdated, .coreMemoryRuntimeProjectionUpdated:
                            break
                        case .contextUsageUpdated(let usage):
                            guard UUID(uuidString: usage.sessionID) == generationConversationId,
                                  let journalConsumer else {
                                throw IOSApplicationRuntimeError.rejected("Context usage wakeup has no admitted owner")
                            }
                            _ = try await journalConsumer.recoverContextUsage()
                            guard myGeneration == activeGenerationId else { throw CancellationError() }
                            if let generationConversationId {
                                refreshCommittedContextUsage(conversationID: generationConversationId)
                            }
                        case .assistantJournalCommitted(let hint):
                            guard let journalConsumer else { throw IOSApplicationRuntimeError.applicationUnavailable }
                            try await journalConsumer.receiveHint(hint, terminal: false)
                            guard myGeneration == activeGenerationId else { throw CancellationError() }
                            if permissionOwnerProjectionRunID != bound.identity.submission_id, let owner = generationConversationId {
                                permissionOwnerProjectionRunID = bound.identity.submission_id
                                refreshPermissionOwnerProjection(conversationID: owner)
                            }
                            await refreshRemoteJournalPresentation(journalConsumer)
                            refreshCommittedAssistant()
                            applyCommittedJournalText()
                        case .assistantRunTerminalSealed(let hint):
                            guard let journalConsumer else { throw IOSApplicationRuntimeError.applicationUnavailable }
                            try await journalConsumer.receiveHint(hint, terminal: true)
                            guard myGeneration == activeGenerationId else { throw CancellationError() }
                            remoteObservationUnresolved = false
                            observedRemoteRun = nil
                            if let owner = generationConversationId { refreshPermissionOwnerProjection(conversationID: owner) }
                            await refreshRemoteJournalPresentation(journalConsumer, forceWindow: true)
                            refreshCommittedAssistant()
                            applyCommittedJournalText()
                        case .trace(let trace):
                            agentTraceContext.adopt(trace)
                            _ = applyAgentTrace(trace)
                        case .permissionRequest(let request, _), .delegatedPermissionRequest(let request):
                            if case .permissionRequest(_, let trace) = event {
                                agentTraceContext.adopt(trace)
                                _ = applyAgentTrace(trace)
                            }
                            guard sourceAdmitted,
                                  request.hostId == bound.identity.source_host_id,
                                  request.clientId == bound.identity.client_id,
                                  request.sessionId == bound.identity.conversation_id,
                                  request.canonicalRunID == bound.identity.submission_id,
                                  request.protocolVersion == WebAgentProvider.hostPermissionProtocolVersion,
                                  request.nonce?.isEmpty == false,
                                  let expiresAt = request.expiresAt,
                                  expiresAt >= Int64(Date().timeIntervalSince1970) else {
                                throw NolaError.llm(.invalidResponse(reason: "The host permission request did not match the paired host identity."))
                            }
                            self.retrievalInProgress = false
                            self.processingState = .streaming(progress: min(0.98, Double(hostToolEvents.count) * 0.10))
                            startHostPermissionInteraction(
                                request,
                                conversationId: generationConversationId,
                                isCurrent: { myGeneration == self.activeGenerationId },
                                send: { payload in
                                    try await journalProvider.submitHostPermissionDecision(payload)
                                },
                                onSubmitted: { [weak self] permissionDecision in
                                    guard let self else { return }
                                    if case .provideInstruction = permissionDecision { return }
                                    self.applyAgentTrace(AgentTraceAdapter.permissionResolved(
                                        requestId: request.id,
                                        toolName: request.toolName,
                                        allowed: permissionDecision.isAllowed,
                                        context: &self.agentTraceContext
                                    ))
                                }
                            )
                        case .deviceToolRequest(let request, let trace):
                            agentTraceContext.adopt(trace)
                            _ = applyAgentTrace(trace)
                            guard sourceAdmitted,
                                  request.hostId == bound.identity.source_host_id,
                                  request.clientId == bound.identity.client_id,
                                  request.sessionId == generationConversationId?.uuidString,
                                  request.protocolVersion == 1,
                                  !request.nonce.isEmpty,
                                  request.expiresAt >= Int64(Date().timeIntervalSince1970) else {
                                throw NolaError.llm(.invalidResponse(reason: "The device tool request did not match this paired host session."))
                            }
                            self.retrievalInProgress = false
                            guard let prepared = prepareHybridDeviceTool(request) else {
                                throw NolaError.llm(.invalidResponse(reason: "The paired host requested a device tool that is not enabled by the current Core plan."))
                            }
                            let displayRequest = HostAgentPermissionRequest(
                                id: request.id,
                                allowConversation: false,
                                toolName: prepared.resolvedTool.toolId,
                                argsPreview: request.argsPreview,
                                match: prepared.permission.capability.rawValue,
                                reason: NSLocalizedString(
                                    "chat.permission.hybrid.request_reason",
                                    value: "The paired host is requesting a device capability. The result will return only to this host session.",
                                    comment: "Hybrid device request reason"
                                ),
                                message: String(
                                    format: NSLocalizedString(
                                        "chat.permission.hybrid.run_format",
                                        value: "Run %@ on this device",
                                        comment: "Hybrid device request action format"
                                    ),
                                    prepared.resolvedTool.toolId
                                ),
                                nonce: request.nonce,
                                hostId: request.hostId,
                                clientId: request.clientId,
                                sessionId: request.sessionId,
                                expiresAt: request.expiresAt,
                                protocolVersion: request.protocolVersion,
                                deviceBridge: true
                            )
                            cancelHostPermissionInteraction()
                            let permissionDecision = try await hostPermissionSession.waitForDecision(
                                displayRequest,
                                conversationID: generationConversationId
                            )
                            guard myGeneration == activeGenerationId, !Task.isCancelled else { throw CancellationError() }
                            hostPermissionSession.clear(requestID: displayRequest.id)
                            let result: IOSAgentHostCallResult
                            if permissionDecision.isAllowed {
                                result = await executeHybridDeviceTool(request, prepared: prepared)
                            } else {
                                result = IOSAgentHostCallResult(
                                    callId: request.id,
                                    output: .object(["error": .string("Permission denied by user")]),
                                    isError: true
                                )
                            }
                            guard myGeneration == activeGenerationId, !Task.isCancelled else { throw CancellationError() }
                            try await journalProvider.submitDeviceToolResult(
                                deviceToolResultPayload(request: request, result: result)
                            )
                            guard myGeneration == activeGenerationId, !Task.isCancelled else { throw CancellationError() }
                            applyAgentTrace(AgentTraceAdapter.permissionResolved(
                                requestId: request.id,
                                toolName: request.toolName,
                                allowed: permissionDecision.isAllowed,
                                context: &agentTraceContext
                            ))
                        case .externalSessionStarted(let backendID, let sessionID):
                            guard let conversationID = generationConversationId,
                                  let store = self.container.conversationStore,
                                  var conversation = try store.getConversation(conversationId: conversationID),
                                  conversation.executionBackend == backendID else {
                                throw NolaError.llm(.invalidResponse(reason: "The host returned an external session for a different execution unit."))
                            }
                            conversation.externalExecutionSessions[backendID] = sessionID
                            conversation.lastActiveAt = Date()
                            try store.upsertConversation(conversation)
                        case .error(let message):
                            // Keep the source failure until its durable rejection
                            // arrives; the generic receipt must not erase the
                            // configuration/parent conflict that explains it.
                            if !sourceAdmitted { sourceDispatchError = message }
                            if webAgentThinkingOpen {
                                try consumeStreamChunk("</think>\n")
                                webAgentThinkingOpen = false
                            }
                            try consumeStreamChunk(WebAgentProvider.summaryStreamErrorLine(for: message))
                            applyAgentTrace(AgentTraceAdapter.web(.error(message), context: &agentTraceContext))
                            self.processingState = .streaming(progress: min(0.98, Double(hostToolEvents.count) * 0.10))
                        }
                    }
            } else {
                throw NolaError.llm(.invalidResponse(
                    reason: "The Web Agent backend must provide the canonical Agent event stream."
                ))
            }
            
            logger.log(.debug, "ChatManager", ">>> Stream iteration completed", metadata: [
                "totalChunks": String(chunkIdx),
                "accumulatedLength": String(accumulated.count)
            ])

            var finalJournalContent: IOSJournalMessageContent?
            if model.isWebBackend {
                guard let journalConsumer, try await journalConsumer.recoverTerminal() else {
                    throw IOSApplicationRuntimeError.rejected("Host stream ended without a durable terminal seal; Run remains recoverable")
                }
                guard myGeneration == activeGenerationId else { throw CancellationError() }
                await refreshRemoteJournalPresentation(journalConsumer, forceWindow: true)
                if let conversationID = generationConversationId {
                    remoteObservationUnresolved = false
                    observedRemoteRun = nil
                    refreshPermissionOwnerProjection(conversationID: conversationID)
                    refreshCommittedContextUsage(conversationID: conversationID)
                }
                refreshCommittedAssistant()
                guard let conversationID = generationConversationId else {
                    throw IOSApplicationRuntimeError.rejected("Completed Run has no conversation owner")
                }
                let owner = IOSCommittedAssistantOwner(conversationID: conversationID.uuidString,
                    assistantMessageID: turnIdentity.assistantMessageID, generation: myGeneration)
                guard let snapshot = try IOSCommittedAssistantProjection.load(from: container.applicationRuntime, owner: owner),
                      let cursor = snapshot.journalCursor else {
                    throw IOSApplicationRuntimeError.rejected("Completed Run is missing its committed assistant journal")
                }
                let content = try await committedJournalReader.read(at: cursor)
                guard myGeneration == activeGenerationId else { throw CancellationError() }
                finalJournalContent = content
                presentJournalContent(content, generation: myGeneration)
            }
            let finalParsed = ChatThinkingParser.splitThinkingBlocks(from: accumulated)
            let finalThinking = model.isWebBackend
                ? finalJournalContent?.reasoning ?? ""
                : finalParsed.thinking
            var finalClean = model.isWebBackend
                ? finalJournalContent?.text ?? ""
                : finalParsed.response
            // Remove leading blank lines to avoid empty first line in UI while preserving intentional internal spacing
            finalClean = ChatTextPolicy.trimmingLeadingNewlines(from: finalClean)

            // If a newer generation started while the loop was wrapping up,
            // do NOT publish or persist — let the newer call own UI state.
            if myGeneration != activeGenerationId {
                logger.log(.info, "ChatManager", "Stream completed but superseded; skipping publish/persist", metadata: [
                    "myGen": String(myGeneration),
                    "active": String(activeGenerationId)
                ])
                return finalClean
            }

            // Ensure retrieval flag cleared
            self.retrievalInProgress = false

            // Publish final result and preserve any thinking content collected
            if currentConversationId == generationConversationId { self.currentResponse = finalClean }
            logger.log(.info, "ChatManager", "Final response preview", metadata: [
                "preview": String(finalClean.prefix(40)),
                "bytes": String(finalClean.count)
            ])
            if currentConversationId == generationConversationId {
                self.thinking = thinkingEnabled ? finalThinking : ""
                self.thinkingFinished = thinkingEnabled && !self.thinking.isEmpty
            }


            if let convId = generationConversationId {
                recordPersonalIntelligenceTurn(
                    conversationId: convId,
                    response: finalClean,
                    model: model.name,
                    route: model.isLocal ? "local" : "cloud"
                )
                var meta = latestResponseMeta
                meta["model"] = model.name
                if let encodedToolEvents = HostAgentToolEvent.encodeForConversationMeta(hostToolEvents) {
                    meta[HostAgentToolEvent.conversationMetaKey] = encodedToolEvents
                }
                if let encodedSources = ChatSourceReference.encodeForConversationMeta(sourceReferences) {
                    meta[ChatSourceReference.conversationMetaKey] = encodedSources
                }
                if let encodedTasks = AgentCoreTaskProjectionMeta.encode(
                        agentTaskProjection,
                        assistantMessageId: turnIdentity.visibleAssistantMessageID
                   ) {
                    meta[AgentCoreTaskProjectionMeta.conversationMetaKey] = encodedTasks
                }
                meta = AgentCoreSkillProjectionMetadata.encode(agentSkillRuntimeProjection, into: meta)
                meta.merge(
                    ChatAgentTraceMeta.values(
                        thinking: thinkingEnabled ? finalThinking : "",
                        phase: agentTraceProjection.phase
                    ),
                    uniquingKeysWith: { _, latest in latest }
                )
                if !hostToolEvents.isEmpty {
                    responseBasis.insert("tools")
                }
                if !responseBasis.isEmpty {
                    meta["nola.answerBasis"] = responseBasis.sorted().joined(separator: ",")
                }
                meta["nola.canonicalMessageID"] = turnIdentity.assistantMessageID
                latestResponseMeta = meta

                // Attempt to auto-name conversation if needed (non-blocking)
                Task.detached { [weak self] in
                    guard let self = self else { return }
                    await self.autoNameConversationIfNeeded(conversationId: convId)
                }
            }

            self.processingState = .completed
            self.logger.log(.info, "ChatManager", "Message processed successfully", metadata: [
                "responseLength": String(finalClean.count)
            ])

            return finalClean
        } catch {
            if !remoteSubmissionRetired, myGeneration == activeGenerationId, let journalConsumer, !Task.isCancelled {
                if let provider = journalProvider, let remoteStore = canonicalRemoteTurnStore,
                   let conversationID = generationConversationId?.uuidString {
                    if let refreshed = try? await refreshRemoteSubmission(provider: provider, store: remoteStore,
                        conversationID: conversationID, cancel: false) {
                        remoteSubmissionRetired = refreshed.phase == "cancelled" || refreshed.phase == "rejected"
                    }
                }
                // A lost terminal notification is recovered by reading its
                // authenticated seal; an unavailable seal leaves the Run active.
                if (try? await journalConsumer.recoverTerminal()) == true, myGeneration == activeGenerationId {
                    await refreshRemoteJournalPresentation(journalConsumer, forceWindow: true)
                    if let conversationID = generationConversationId {
                        refreshCommittedContextUsage(conversationID: conversationID)
                    }
                    refreshCommittedAssistant()
                    applyCommittedJournalText()
                }
            }
            // Ensure retrieval flag cleared on error
            self.retrievalInProgress = false
            // CancellationError from generation supersession is silent — newer
            // call owns processingState.
            if error is CancellationError && myGeneration != activeGenerationId {
                throw error
            }
            // Connection/provider failures are rendered as assistant messages. Do not
            // leave the composer in a global error state that blocks retries.
            if remoteSubmissionRetired {
                remoteObservationUnresolved = false
                remoteSubmissionPending = false
                processingState = .completed
            } else if (effectiveLLMClient as? ModelService)?.currentProvider() is WebAgentProvider {
                remoteObservationUnresolved = true
                self.processingState = .streaming(progress: 0)
            } else { self.processingState = .completed }

            // Enhanced error logging with type information
            var errorMeta: [String: String] = ["error": error.localizedDescription]
            if let nolaErr = error as? NolaError {
                errorMeta["errorType"] = "NolaError"
                errorMeta["errorDetail"] = "\(nolaErr)"
            } else if let urlErr = error as? URLError {
                errorMeta["errorType"] = "URLError"
                errorMeta["errorCode"] = "\(urlErr.code.rawValue)"
            } else {
                errorMeta["errorType"] = String(describing: type(of: error))
            }
            self.logger.log(.error, "ChatManager", ">>> Message processing failed",
                           metadata: errorMeta)
            throw error
        }
    }

    private func appendOnlyParsedDelta(
        _ parsed: String,
        emittedCharacterCount: inout Int,
        channel: String
    ) throws -> String {
        guard parsed.count >= emittedCharacterCount else {
            throw NolaError.llm(.invalidResponse(
                reason: "The provider rewrote the already emitted \(channel) stream."
            ))
        }
        let start = parsed.index(parsed.startIndex, offsetBy: emittedCharacterCount)
        emittedCharacterCount = parsed.count
        return String(parsed[start...])
    }

    private func runDeviceAgentTurn(
        sessionId: String?,
        input: String,
        history: [AgentCoreMessage],
        conversationId: UUID?,
        preparedModel: IOSPreparedRunModel,
        turnIdentity: IOSCanonicalAgentTurnIdentity?,
        persistedUserText: String,
        userContentJSON: String?,
        inputOrigin: AgentCoreInputOrigin,
        expectedConfigurationRevision: Int64? = nil,
        expectedParentMessageID: String? = nil,
        contextComponents: [AgentCoreContextComponent] = [],
        promptComposition: AgentCorePromptComposition? = nil,
        automationLease: IOSAutomationExecutionLease? = nil,
        automationSpec: IOSAutomationExecutionSpec? = nil,
        recovering: IOSExactRunInvocation? = nil,
        queuedUserAdmission: IOSQueuedUserAdmission? = nil,
        freshResendTarget: String? = nil
    ) async throws -> String {
        let owningGeneration = activeGenerationId
        let automationAdmissionSource = if automationLease != nil, let conversationId {
            try container.applicationRuntime.loadConversation(id: conversationId.uuidString)
        } else { nil as IOSCanonicalConversation? }
        let mode = AgentCoreExecutionMode.deviceLocal
        let session = sessionId ?? conversationId?.uuidString ?? UUID().uuidString
        await agentCoreHostExecutor.acquireAgentSession(sessionId: session)
        defer {
            Task { await agentCoreHostExecutor.releaseAgentSession(sessionId: session) }
        }
        let enabledSkills = enabledLocalSkillSnapshots()
        await agentCoreHostExecutor.installSkillSnapshot(enabledSkills, sessionId: session)
        let identity = AgentCoreExpectedIdentity(
            hostId: "ios-local",
            clientId: Bundle.main.bundleIdentifier ?? "nola-ios",
            sessionId: session
        )
        let toolPlan = try AgentCoreToolCatalog.prepareBoundPlan(
            for: mode,
            expectedIdentity: identity,
            isForeground: UIApplication.shared.applicationState == .active,
            scopedGrants: freshResendTarget == nil ? agentCorePermissionBroker.scopedGrants : []
        )
        let applicationStore = container.applicationRuntime
        let canonicalIdentity = turnIdentity
            ?? IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let canonicalConversationID = recovering?.conversationID ?? conversationId?.uuidString ?? session
        let conversationTitle = conversationId.flatMap {
            try? container.conversationStore?.getConversation(conversationId: $0)?.title
        } ?? ""
        let canonicalApplication = IOSCanonicalAgentApplication(
            store: applicationStore,
            descriptor: IOSCanonicalAgentRunDescriptor(
                conversationID: canonicalConversationID,
                conversationTitle: conversationTitle,
                identity: canonicalIdentity,
                userText: persistedUserText,
                userContentJSON: userContentJSON,
                contextCheckpointID: nil,
                automationLease: automationLease,
                automationSpec: automationSpec,
                boundRunID: recovering?.runID,
                expectedConfigurationRevision: expectedConfigurationRevision ?? automationAdmissionSource?.configurationRevision,
                expectedParentMessageID: expectedConfigurationRevision != nil
                    ? expectedParentMessageID : automationAdmissionSource?.messages.last?.id,
                queuedUserAdmission: queuedUserAdmission
            ), submissionCommitted: submissionCommitted
        )
        let preparedInvocation = try preparedModel.prepareHostInvocation()
        let recoveryPreparation: (@Sendable (IOSExactRunInvocation) async throws -> Void)?
        if let frozen = recovering {
            recoveryPreparation = { current in
                guard current.hasSameFrozenInvocation(as: frozen) else { throw DeviceAgentCoordinatorError.invalidTransition }
                try await MainActor.run {
                    let revalidated = try IOSPreparedRunModel.recover(current, using: applicationStore).prepareHostInvocation()
                    guard revalidated.route == preparedInvocation.route,
                          revalidated.generation == preparedInvocation.generation,
                          revalidated.outputAdmission == preparedInvocation.outputAdmission else {
                        throw DeviceAgentCoordinatorError.invalidTransition
                    }
                }
            }
        } else { recoveryPreparation = nil }
        let turnModelClient: TextAgentModelClient
        if let profile = preparedModel.frozenModelCapability {
            turnModelClient = try TextAgentModelClient(client: preparedModel.client, frozenModelCapability:profile)
        } else { turnModelClient = TextAgentModelClient(client: preparedModel.client) }
        let coordinator = DeviceAgentCoordinator(
            modelClient: turnModelClient,
            childPermissionEvents: { [weak self] event in await self?.applyChildPermissionEvent(event) },
                childTaskStateChanged: { [weak self] in await self?.container.chatManager.recoverEmbeddedChildTasks() },
            delegationSettings: try preparedModel.mobileDelegationSettings(
                projectRoot: applicationStore.databaseURL.deletingLastPathComponent().path,
                conversationID: canonicalConversationID, using: applicationStore),
            hostExecutor: agentCoreHostExecutor,
            toolPlan: toolPlan,
            modelSupportsToolCalls: preparedModel.supportsToolCalls,
            canonicalApplication: canonicalApplication,
            preparedHostInvocation: preparedInvocation,
            recoveryPreparation: recoveryPreparation,
            hostDependencyIdentity: IOSPreparedHostInvocation.skillIdentity(enabledSkills),
            freshResendTarget: freshResendTarget
        )
        deviceAgentCoordinator = coordinator
        defer {
            if deviceAgentCoordinator === coordinator { deviceAgentCoordinator = nil }
        }

        let composition = try promptComposition ?? AgentCorePromptComposer.compose([
            .init(
                id: "ios:host-identity",
                layer: .identity,
                trust: .hostFact,
                cacheable: true,
                content: PromptTemplates.agentIdentity
            )
        ])
        // Conversation history and typed Turn components deliberately travel
        // through separate Core inputs. Re-inserting rendered component text
        // as history would duplicate it and erase source/authorization/audit
        // semantics.
        let effectiveHistory = freshResendTarget == nil
            ? [AgentCoreMessage(role: "system", content: composition.rendered)] + history
            : []
        let effectiveComponents: [AgentCoreContextComponent]
        if freshResendTarget != nil {
            // New Host guidance is rebuilt for this target. It is typed input,
            // never old history, resident memory, resources, or an old grant.
            effectiveComponents = [AgentCoreContextComponent(kind: .system,
                id: "ios:fresh-host-guidance", content: .string(composition.rendered),
                source: .init(platform: .ios, title: "Nola", mimeType: "text/plain"))]
                + enabledSkills.map { skill in
                    AgentCoreContextComponent(kind: .skill, id: "skill:\(skill.id)",
                        content: .string(skill.instructions),
                        source: .init(platform: .ios, title: skill.name, mimeType: "text/plain"))
                } + contextComponents
        } else { effectiveComponents = contextComponents }
        let permissionMode = conversationId.flatMap {
            try? container.conversationStore?.getConversation(conversationId: $0)?.permissionMode
        } ?? .cautious

        let runtimeConfig = AgentCoreRuntimeConfig(
                platform: .ios,
                executionMode: mode,
                model: preparedModel.modelName,
                stablePromptDigest: composition.stablePrefixSha256,
                promptFragmentIds: composition.fragmentIds,
                stablePromptFragmentDigests: composition.stableFragmentSha256,
                maxIterations: 0,
                contextWindowTokens: preparedModel.parameters.maxContextLength,
                reservedOutputTokens: preparedModel.parameters.maxOutputTokens,
                grantedCapabilities: [],
                permissionProfile: permissionMode
            )
        let stream: AsyncThrowingStream<DeviceAgentCoordinatorEvent, Error>
        if recovering != nil {
            stream = await coordinator.recoverExactRun(sessionId: session, config: runtimeConfig, inputOrigin: inputOrigin)
        } else {
            stream = await coordinator.start(sessionId: session, input: input, history: effectiveHistory,
                inputOrigin: inputOrigin, contextComponents: effectiveComponents,
                turnIdentity: canonicalIdentity, config: runtimeConfig)
        }
        var journalScope = IOSLocalJournalPublicationScope(conversationID: canonicalConversationID,
            runID: recovering?.runID ?? canonicalIdentity.submissionID)
        var authoritativeFinalContent = ""
        var receivedCommittedCompletion = false
        for try await event in stream {
            guard owningGeneration == activeGenerationId else { throw CancellationError() }
            switch event {
            case .snapshot(let snapshot):
                agentCoreSnapshot = snapshot
                refreshInitialLocalAssistantMetadata(snapshot, runID: journalScope.runID)
                if permissionOwnerProjectionRunID != snapshot.turnId, let owner = currentConversationId {
                    permissionOwnerProjectionRunID = snapshot.turnId
                    refreshPermissionOwnerProjection(conversationID: owner)
                }
            case .assistantJournalCommitted(let cursor):
                let content = try await consumeLocalJournalPublication(cursor, scope: &journalScope,
                    generation: owningGeneration)
                guard owningGeneration == activeGenerationId, !Task.isCancelled else { throw CancellationError() }
                if let content, content.cursor.identity.assistant_message_id == canonicalIdentity.assistantMessageID {
                    authoritativeFinalContent = content.text
                }
                processingState = .streaming(
                    progress: min(0.98, Double(currentResponse.count) / 4_000.0)
                )
            case .coreTurnTerminal:
                refreshCommittedAssistant()
                try await refreshTerminalJournalWindow(conversationID: canonicalConversationID, generation: owningGeneration)
                if let owner = currentConversationId { refreshPermissionOwnerProjection(conversationID: owner) }
            case .taskProjectionUpdated(let projection):
                if currentConversationId == conversationId,
                   taskProjectionHasSameOwners(projection, previous: agentTaskProjection),
                   projection.revision >= agentTaskProjection.revision {
                    agentTaskProjection = projection
                }
            case .skillRuntimeProjectionUpdated(let projection):
                applyAgentSkillRuntimeProjection(projection)
            case .capabilitiesActivated(let activation):
                applyAgentCapabilityActivation(activation)
            case .trace(let trace):
                agentTraceContext.adopt(trace)
                let result = applyAgentTrace(trace)
                guard result == .applied else { continue }
                switch trace.kind {
                case .toolStarted, .toolProgress, .toolPermissionRequested,
                     .toolPermissionResolved, .toolCompleted:
                    processingState = .streaming(
                        progress: min(0.98, Double(hostToolEvents.count) * 0.10)
                    )
                default:
                    break
                }
            case .permissionRequested(let request):
                enqueueLocalAgentPermission(request)
            case .permissionResolved(let requestId, _, _):
                removeLocalAgentPermission(requestId: requestId)
            case .completed(let result):
                receivedCommittedCompletion = true
                queuedSuccessorReady = automationLease == nil && automationSpec == nil
                    && (result.value.kind == .answered || result.value.kind == .actionCompleted)
            }
        }
        guard owningGeneration == activeGenerationId, !Task.isCancelled else { throw CancellationError() }
        guard receivedCommittedCompletion else { throw DeviceAgentCoordinatorError.missingTransition }
        localAgentPermissionQueue.removeAll { $0.childOwner == nil }
        localAgentPermissionRequest = localAgentPermissionQueue.first
        if localAgentScopedResourceRequest?.childOwner == nil { localAgentScopedResourceRequest = nil }
        let retained = Set(localAgentPermissionQueue.map(\.presentationID))
        localAgentPermissionDecisions = localAgentPermissionDecisions.filter { retained.contains($0.key) }
        authoritativeFinalContent = authoritativeFinalContent.trimmingCharacters(in: .whitespacesAndNewlines)
        if currentConversationId == conversationId {
            currentResponse = authoritativeFinalContent
            thinkingFinished = !thinking.isEmpty
        }
        processingState = .completed
        wakeLocalGoal()
        if let conversationId {
            refreshPermissionOwnerProjection(conversationID: conversationId)
            await autoNameConversationIfNeeded(conversationId: conversationId)
        }
        refreshCommittedAssistant()
        let memoryWorker = IOSCanonicalMemoryWorkerService(store: applicationStore)
        do {
            if try memoryWorker.hasClaimableJobs(conversationID: canonicalConversationID) {
                container.scheduler.scheduleMemoryWorkerWake()
            }
        } catch {
            logger.log(.warn, "ChatManager", "Unable to inspect deferred Memory Worker queue", metadata: [
                "conversationId": canonicalConversationID,
                "error": error.localizedDescription,
            ])
        }
        return authoritativeFinalContent
    }

    /// Processes Core-authored memory jobs without blocking the visible reply.
    @discardableResult
    public func processPendingMemoryWorkers(
        sessionId: String? = nil,
        maximumJobCount: Int = 2
    ) async throws -> Int {
        try await processPendingMemoryWorkerReport(
            sessionId: sessionId,
            maximumJobCount: maximumJobCount
        ).completed
    }

    func processPendingMemoryWorkerReport(
        sessionId: String? = nil,
        maximumJobCount: Int = 2
    ) async throws -> IOSMemoryWorkerRunReport {
        guard maximumJobCount > 0 else { return IOSMemoryWorkerRunReport() }
        let requestedSessionId = sessionId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let modelClient = TextAgentModelClient(client: effectiveLLMClient)
        guard let applicationStore = IOSApplicationRuntime.shared else {
            throw IOSApplicationRuntimeError.applicationUnavailable
        }
        guard let selectedModel = selectedModelInfo ?? ModelSelectionManager.getLastSelectedModel() else {
            throw ChatManagerError.noModelSelected
        }
        let memoryCoreConfig: (String) -> AgentCoreJSONValue = { sessionID in
            .object([
                "session_id": .string(sessionID),
                "model": .string(selectedModel.id),
                "context_window": .object([
                    "context_window_tokens": .number(Double(self.currentContextTokenLimit())),
                    "reserved_output_tokens": .number(16_384),
                ]),
            ])
        }
        let memoryWorker = IOSCanonicalMemoryWorkerService(store: applicationStore)
        if let requestedSessionId, !requestedSessionId.isEmpty {
            return try await memoryWorker.runReport(
                conversationID: requestedSessionId,
                coreConfig: memoryCoreConfig(requestedSessionId),
                modelClient: modelClient,
                maximumJobCount: maximumJobCount
            )
        }

        var sessionIds = try applicationStore.listConversations().map(\.id)
        if let currentSessionId = currentConversationId?.uuidString,
           !sessionIds.contains(currentSessionId) {
            sessionIds.insert(currentSessionId, at: 0)
        }
        return try await processClaimableMemoryWorkerSessions(
            conversationIDs: sessionIds,
            maximumJobCount: maximumJobCount
        ) { persistedSessionId, remainingJobCount in
            try await memoryWorker.runReport(
                conversationID: persistedSessionId,
                coreConfig: memoryCoreConfig(persistedSessionId),
                modelClient: modelClient,
                maximumJobCount: remainingJobCount
            )
        }
    }

    /// Cold-start/selection discovery observes only. It never submits a decision
    /// or presents a modal. All synchronous FFI/file-lock scopes finish on a worker.
    func discoverUnknownEffectRecovery() async {
        if !isConversationExecution {
            guard let selection = currentConversationId else { return }
            let runtime = container.applicationRuntime
            do {
                let pending = try await Task.detached { try runtime.pendingUnknownEffectRecovery(conversationID: selection.uuidString) }.value
                guard currentConversationId == selection else { return }
                if let pending, let owner = UUID(uuidString: pending.decision.conversation_id), owner != selection {
                    unknownEffectPendingConversationID = owner
                    return
                }
            } catch {
                // Child discovery presents local errors. Do not authorize from a
                // failed global intent lookup or redirect a remote conversation.
            }
            await selectedExecution?.discoverUnknownEffectRecovery()
            return
        }
        guard !unknownEffectRecoveryChecking, let conversationID = currentConversationId,
              (try? requireLocalRecoveryOwner(conversationID)) != nil else { return }
        unknownEffectRecoveryChecking = true
        unknownEffectRecoveryError = nil
        unknownEffectPendingConversationID = nil
        defer { unknownEffectRecoveryChecking = false }
        let generation = activeGenerationId
        let runtime = container.applicationRuntime
        do {
            let pending = try await Task.detached { try runtime.pendingUnknownEffectRecovery(conversationID: conversationID.uuidString) }.value
            guard generation == activeGenerationId, currentConversationId == conversationID else { return }
            if let pending {
                guard let pendingConversation = UUID(uuidString: pending.decision.conversation_id) else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                if pendingConversation != conversationID {
                    unknownEffectPendingConversationID = pendingConversation
                    return
                }
                guard pending.decision.conversation_id == conversationID.uuidString,
                      let userID = UUID(uuidString: pending.decision.run_id) else { return }
                let identity = IOSCanonicalAgentTurnIdentity(userMessageID: userID)
                guard identity.submissionID == pending.decision.run_id else { return }
                unknownEffectRecovery = .init(conversationID: conversationID, inspectionID: UUID(),
                    runID: pending.decision.run_id, assistantMessageID: identity.assistantMessageID,
                    attemptIDs: pending.decision.targets.map(\.attempt_id), canRetry: false, canKeepBlocked: false, phase: .outcomePending)
                return
            }
            let run = try await Task.detached {
                try runtime.loadConversationProjectionWindowV1(id: conversationID.uuidString,
                    beforeOrdinal: nil, limit: 1, contentByteLimit: 1_024).projection.activeRun
            }.value
            guard generation == activeGenerationId, currentConversationId == conversationID else { return }
            guard let run, let userID = UUID(uuidString: run.runID) else {
                if unknownEffectRecovery?.phase != .blocked { unknownEffectRecovery = nil }
                return
            }
            let identity = IOSCanonicalAgentTurnIdentity(userMessageID: userID)
            guard identity.submissionID == run.runID else { return }
            try await inspectUnknownEffects(conversationID: conversationID, runID: run.runID,
                assistantMessageID: identity.assistantMessageID)
        } catch {
            guard generation == activeGenerationId else { return }
            unknownEffectRecoveryError = NSLocalizedString("chat.recovery.check_failed",
                value: "Unable to check the previous operation. Try checking again; nothing was retried.", comment: "Recovery discovery failure")
        }
    }

    /// Navigation only, deliberately separate from risk authorization or resume.
    func openPendingRecoveryConversation(_ conversationID: UUID) async throws {
        guard !isConversationExecution else { throw DeviceAgentCoordinatorError.invalidTransition }
        let runtime = container.applicationRuntime
        let pending = try await Task.detached { try runtime.pendingUnknownEffectRecovery(conversationID: conversationID.uuidString) }.value
        guard pending?.decision.conversation_id == conversationID.uuidString else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        currentConversationId = conversationID
    }

    /// Explicit local A08 owner. Remote mirrors must use their source Host's
    /// control protocol and are deliberately rejected at this boundary.
    private func requireLocalRecoveryOwner(_ conversationID: UUID) throws {
        guard isConversationExecution, currentConversationId == conversationID,
              deviceAgentCoordinator == nil, !messageInFlight,
              observedRemoteRun == nil, !remoteObservationUnresolved,
              let conversation = try container.conversationStore?.getConversation(conversationId: conversationID),
              conversation.selectedModelInfo?.isWebBackend != true else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
    }

    func inspectUnknownEffects(conversationID: UUID, runID: String, assistantMessageID: String) async throws {
        if !isConversationExecution {
            guard let owner = executionForConversation(conversationID) else { throw DeviceAgentCoordinatorError.invalidTransition }
            try await owner.inspectUnknownEffects(conversationID: conversationID, runID: runID, assistantMessageID: assistantMessageID)
            return
        }
        try requireLocalRecoveryOwner(conversationID)
        let generation = activeGenerationId
        let runtime = container.applicationRuntime
        let inspection = try await Task.detached {
            try runtime.inspectUnknownEffectRecovery(conversationID: conversationID.uuidString,
                runID: runID, assistantMessageID: assistantMessageID)
        }.value
        guard generation == activeGenerationId else { throw DeviceAgentCoordinatorError.invalidTransition }
        try requireLocalRecoveryOwner(conversationID)
        frozenUnknownEffectInspection = inspection
        unknownEffectRecovery = inspection.map {
            .init(conversationID: conversationID, inspectionID: UUID(), runID: $0.run_id, assistantMessageID: $0.assistant_message_id,
                  attemptIDs: $0.targets.map(\.attempt_id), canRetry: $0.retry_allowed, canKeepBlocked: $0.independent_pending_effects == 0,
                  phase: .inspected)
        }
    }

    /// The view must capture this exact presentation in its confirmation closure.
    /// A stale click may not authorize the currently selected/newer Run.
    func confirmUnknownEffectRecovery(_ presented: IOSUnknownEffectRecoveryPresentation,
        choice: IOSUnknownEffectRecoveryDecision.Choice.Kind, attemptID: String? = nil,
        reason: String) async throws {
        if !isConversationExecution {
            guard let owner = executionForConversation(presented.conversationID) else { throw DeviceAgentCoordinatorError.invalidTransition }
            try await owner.confirmUnknownEffectRecovery(presented, choice: choice, attemptID: attemptID, reason: reason)
            return
        }
        try requireLocalRecoveryOwner(presented.conversationID)
        guard unknownEffectRecovery?.acceptsConfirmation(presented) == true,
              let inspection = frozenUnknownEffectInspection,
              inspection.run_id == presented.runID, inspection.assistant_message_id == presented.assistantMessageID else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let target = attemptID.flatMap { id in inspection.targets.first { $0.attempt_id == id } }
        guard (choice == .authorizeRiskRetry && target != nil)
                || (choice == .keepUnknownBlocked && attemptID == nil) else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let decision = try inspection.confirmed(choice: choice, reason: reason, target: target)
        let generation = activeGenerationId
        setUnknownRecoveryPhase(.outcomePending, owner: presented)
        // The Application pending envelope is durable before any transport call.
        let reply: IOSUnknownEffectRecoveryReply
        do {
            let runtime = container.applicationRuntime
            let requestID = UUID().uuidString.lowercased()
            reply = try await Task.detached {
                try runtime.resolveUnknownEffectRecovery(decision, requestID: requestID)
            }.value
        } catch {
            do {
                let runtime = container.applicationRuntime
                let hasNoPending = try await Task.detached { try runtime.pendingUnknownEffectRecovery(conversationID: presented.conversationID.uuidString) == nil }.value
                if hasNoPending, generation == activeGenerationId {
                    frozenUnknownEffectInspection = nil
                    setUnknownRecoveryPhase(.rejected, owner: presented)
                }
            } catch { /* Unreadable intent is still uncertain, never rejection evidence. */ }
            throw error
        }
        guard generation == activeGenerationId else { throw DeviceAgentCoordinatorError.invalidTransition }
        try requireLocalRecoveryOwner(presented.conversationID)
        try await consumeUnknownRecoveryReply(reply, owner: presented)
    }

    /// May be called after relaunch; uses the stored input verbatim. No new time,
    /// authorization, or resolution is generated by this retry action.
    func recoverPendingUnknownEffectRecovery(conversationID: UUID, runID: String,
                                            assistantMessageID: String) async throws {
        if !isConversationExecution {
            guard let owner = executionForConversation(conversationID) else { throw DeviceAgentCoordinatorError.invalidTransition }
            try await owner.recoverPendingUnknownEffectRecovery(conversationID: conversationID,
                runID: runID, assistantMessageID: assistantMessageID)
            return
        }
        try requireLocalRecoveryOwner(conversationID)
        let runtime = container.applicationRuntime
        let generation = activeGenerationId
        let storedPending = try await Task.detached { try runtime.pendingUnknownEffectRecovery(conversationID: conversationID.uuidString) }.value
        guard let pending = storedPending,
              pending.decision.conversation_id == conversationID.uuidString, pending.decision.run_id == runID,
              let userID = UUID(uuidString: runID),
              IOSCanonicalAgentTurnIdentity(userMessageID: userID).assistantMessageID == assistantMessageID else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let owner = IOSUnknownEffectRecoveryPresentation(conversationID: conversationID, inspectionID: UUID(), runID: runID,
            assistantMessageID: assistantMessageID, attemptIDs: pending.decision.targets.map(\.attempt_id),
            canRetry: false, canKeepBlocked: false, phase: .outcomePending)
        unknownEffectRecovery = owner
        let reply = try await Task.detached { try runtime.recoverPendingUnknownEffectRecovery(pending) }.value
        guard generation == activeGenerationId else { throw DeviceAgentCoordinatorError.invalidTransition }
        try requireLocalRecoveryOwner(conversationID)
        try await consumeUnknownRecoveryReply(reply, owner: owner)
    }

    private func setUnknownRecoveryPhase(_ phase: IOSUnknownEffectRecoveryPresentation.Phase,
                                         owner: IOSUnknownEffectRecoveryPresentation) {
        unknownEffectRecovery = .init(conversationID: owner.conversationID, inspectionID: owner.inspectionID, runID: owner.runID,
            assistantMessageID: owner.assistantMessageID, attemptIDs: owner.attemptIDs,
            canRetry: owner.canRetry, canKeepBlocked: owner.canKeepBlocked, phase: phase)
    }

    private func consumeUnknownRecoveryReply(_ reply: IOSUnknownEffectRecoveryReply,
                                            owner: IOSUnknownEffectRecoveryPresentation) async throws {
        guard reply.outcome != .rejected, reply.outcome != .conflict else {
            frozenUnknownEffectInspection = nil
            setUnknownRecoveryPhase(.rejected, owner: owner)
            return
        }
        let identity = try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: owner.runID,
            assistantMessageID: owner.assistantMessageID)
        let application = IOSCanonicalAgentApplication(store: container.applicationRuntime,
            descriptor: .init(conversationID: owner.conversationID.uuidString, conversationTitle: "",
                identity: identity, userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: owner.runID))
        let generation = activeGenerationId
        let frozen = try await Task.detached { try application.loadExactRunInvocation() }.value
        guard generation == activeGenerationId else { throw DeviceAgentCoordinatorError.invalidTransition }
        guard let sessionID = frozen.coreSessionID else { throw DeviceAgentCoordinatorError.invalidTransition }
        frozenUnknownEffectInspection = nil
        if reply.outcome == .unknownPreservedBlocked {
            let terminal = try await Task.detached { try application.committedTerminalSeal(coreSessionID: sessionID) }.value
            guard generation == activeGenerationId, let seal = terminal,
                  seal.result.value.kind == .blocked else { throw DeviceAgentCoordinatorError.invalidTransition }
            setUnknownRecoveryPhase(.blocked, owner: owner)
            return
        }
        setUnknownRecoveryPhase(.retryAuthorized, owner: owner)
        let model = try IOSPreparedRunModel.recover(frozen, using: container.applicationRuntime)
        observeCommittedAssistant(conversationID: owner.conversationID.uuidString,
            assistantMessageID: owner.assistantMessageID, generation: activeGenerationId)
        messageInFlight = true
        defer {
            messageInFlight = false
            scheduleQueuedSuccessorIfReady()
        }
        _ = try await runDeviceAgentTurn(sessionId: sessionID, input: "", history: [],
            conversationId: owner.conversationID, preparedModel: model, turnIdentity: identity,
            persistedUserText: "", userContentJSON: nil, inputOrigin: .systemRecovery, recovering: frozen)
        unknownEffectRecovery = nil
    }

    private func automationExecution(for conversationID: String) -> ChatManager {
        if let worker = automationExecutions[conversationID] { return worker }
        let worker = ChatManager(container: container, restorePersistedState: false, isConversationExecution: true)
        automationExecutions[conversationID] = worker
        return worker
    }

    /// Restores the bound invocation; never calls Begin or samples a replacement model.
    func resumeScheduledAction(execution: IOSAutomationExecutionIdentity,
                               binding: IOSAutomationExecutionBinding) async throws -> String {
        if !isConversationExecution {
            return try await automationExecution(for: execution.conversationID)
                .resumeScheduledAction(execution: execution, binding: binding)
        }
        guard deviceAgentCoordinator == nil,
              binding.conversationID == execution.conversationID,
              binding.submissionID == execution.submissionID else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let identity = IOSCanonicalAgentTurnIdentity(automation: execution)
        let application = IOSCanonicalAgentApplication(store: container.applicationRuntime,
            descriptor: .init(conversationID: execution.conversationID, conversationTitle: "",
                identity: identity, userText: "", userContentJSON: nil, contextCheckpointID: nil,
                boundRunID: binding.runID))
        let frozen = try application.loadExactRunInvocation()
        guard frozen.invocationConfigDigest == binding.invocationConfigDigest,
              let coreSessionID = frozen.coreSessionID else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let model = try IOSPreparedRunModel.recover(frozen, using: container.applicationRuntime)
        return try await runDeviceAgentTurn(sessionId: coreSessionID, input: "", history: [],
            conversationId: nil, preparedModel: model, turnIdentity: identity,
            persistedUserText: "", userContentJSON: nil, inputOrigin: .scheduledAction,
            recovering: frozen)
    }

    func processScheduledAction(
        _ prompt: String,
        execution: IOSAutomationExecutionIdentity,
        lease: IOSAutomationExecutionLease,
        spec: IOSAutomationExecutionSpec,
        referenceComponents: [AgentCoreContextComponent]
    ) async throws -> String {
        if !isConversationExecution {
            let worker = automationExecution(for: execution.conversationID)
            return try await worker.processScheduledAction(prompt, execution: execution,
                lease: lease, spec: spec, referenceComponents: referenceComponents)
        }
        guard IOSApplicationRuntime.isAvailable else {
            throw AgentCoreRuntimeError.libraryUnavailable
        }
        guard deviceAgentCoordinator == nil else {
            throw DeviceAgentCoordinatorError.alreadyRunning
        }
        guard spec.prompt == prompt, spec.executionScope == .deviceLocal(hostID: "ios-device"),
              spec.permissionProfile == "cautious", spec.modelSelection == "use_current_default_at_admission",
              spec.skillRefs.isEmpty,
              Set(spec.contextRefs.map(\.id)) == Set(referenceComponents.map(\.id)),
              spec.contextRefs.allSatisfy({ $0.kind == "web_source" }),
              referenceComponents.allSatisfy({ $0.kind == .document && $0.source?.authorization == "available" }) else {
            throw AgentCoreRuntimeError.core("Scheduled action input does not match its authorized execution definition")
        }
        guard let model = selectedModelInfo ?? ModelSelectionManager.getLastSelectedModel() else {
            throw ChatManagerError.noModelSelected
        }
        guard !model.isWebBackend else {
            throw NolaError.llm(.notConfigured)
        }
        beginAgentTrace(conversationId: nil, assistantMessageID: nil)
        currentResponse = ""
        thinking = ""
        thinkingFinished = false
        hostToolEvents = []
        let parameters = try generationParameters(for: model, conversationID: nil)
        let preparedModel = try IOSPreparedRunModel.prepare(model: model, parameters: parameters,
            resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: self.container.applicationRuntime, parameters: parameters) })
        return try await runDeviceAgentTurn(
            sessionId: execution.conversationID,
            input: prompt,
            history: [],
            conversationId: nil,
            preparedModel: preparedModel,
            turnIdentity: IOSCanonicalAgentTurnIdentity(automation: execution),
            persistedUserText: prompt,
            userContentJSON: nil,
            inputOrigin: .scheduledAction,
            contextComponents: referenceComponents,
            automationLease: lease,
            automationSpec: spec
        )
    }

    private func applyChildPermissionEvent(_ event: IOSChildPermissionEvent) async {
        switch event {
        case .requested(let request, let coordinator):
            guard let fresh = try? await coordinator.currentPermissionRequest(requestId: request.requestId),
                  fresh.childOwner == request.childOwner, fresh.presentationID == request.presentationID else { return }
            guard isConversationExecution, let owner = request.childOwner,
                  owner.parentConversationID == currentConversationId?.uuidString,
                  childPermissionCoordinators[owner] == nil || childPermissionCoordinators[owner] === coordinator else { return }
            childPermissionCoordinators[owner] = coordinator
            enqueueLocalAgentPermission(fresh)
        case .removed(let owner, let coreRequestID, let coordinator):
            guard childPermissionCoordinators[owner] === coordinator,
                  let request = localAgentPermissionQueue.first(where: { $0.childOwner == owner && $0.requestId == coreRequestID }) else { return }
            removeLocalAgentPermission(requestId: request.presentationID)
            if localAgentScopedResourceRequest?.presentationID == request.presentationID { localAgentScopedResourceRequest = nil }
            if !localAgentPermissionQueue.contains(where: { $0.childOwner == owner }) {
                childPermissionCoordinators.removeValue(forKey: owner)
            }
        }
    }

    private func permissionCoordinator(for request: IOSAgentPermissionRequest) -> DeviceAgentCoordinator? {
        if let owner = request.childOwner { return childPermissionCoordinators[owner] }
        return deviceAgentCoordinator
    }

    private func ownsPermission(_ request: IOSAgentPermissionRequest, coordinator: DeviceAgentCoordinator) -> Bool {
        permissionCoordinator(for: request) === coordinator
            && localAgentPermissionRequest?.presentationID == request.presentationID
    }

    public func provideLocalAgentPermissionInstruction(requestId: String, _ instruction: String) {
        if !isConversationExecution { selectedExecution?.provideLocalAgentPermissionInstruction(requestId: requestId, instruction); return }
        guard let request = localAgentPermissionRequest, request.presentationID == requestId,
              case .provideInstruction(let text)? = HostAgentPermissionDecision.validatedInstruction(instruction, requestID: request.requestId, existing: request.permissionInstructions),
              let coordinator = permissionCoordinator(for: request) else { return }
        Task { [weak self] in
            do {
                try await coordinator.providePermissionInstruction(requestId: request.requestId, instruction: text)
                guard let self, self.permissionCoordinator(for: request) === coordinator else { return }
                self.removeLocalAgentPermission(requestId: request.presentationID)
            } catch { self?.logger.log(.warn, "ChatManager", "Permission instruction failed", metadata: ["error": error.localizedDescription]) }
        }
    }

    public func resolveLocalAgentPermission(requestId: String, _ decision: AgentCorePermissionDecision) {
        if !isConversationExecution { selectedExecution?.resolveLocalAgentPermission(requestId: requestId, decision); return }
        guard let request = localAgentPermissionRequest, request.presentationID == requestId,
              let coordinator = permissionCoordinator(for: request) else { return }
        localAgentPermissionDecisions[request.presentationID] = decision
        if pendingHostPermissionRequest == nil { hostPermissionSubmissionError = nil }
        Task { [weak self] in
            guard let self, self.ownsPermission(request, coordinator: coordinator) else { return }
            if decision == .deny {
                await self.dispatchLocalAgentPermission(request: request, decision: .deny, grant: nil, coordinator: coordinator)
                return
            }
            do {
                let fresh = try await coordinator.currentPermissionRequest(requestId: request.requestId)
                guard self.ownsPermission(request, coordinator: coordinator), fresh.childOwner == request.childOwner else { return }
                let outcome = await self.agentCorePermissionBroker.resolve(fresh, userApproved: true)
                guard self.ownsPermission(request, coordinator: coordinator) else { return }
                await self.applyLocalAgentPermissionOutcome(outcome, request: fresh, coordinator: coordinator)
            } catch {
                // A stale/unknown receiver is not a denial. Keep the pending
                // question available for exact recovery rather than inventing consent.
                if self.ownsPermission(request, coordinator: coordinator), self.pendingHostPermissionRequest == nil {
                    self.hostPermissionSubmissionError = error.localizedDescription
                }
                self.logger.log(.warn, "ChatManager", "Permission refresh failed", metadata: ["error": error.localizedDescription])
            }
        }
    }

    public func resolveLocalAgentScopedResource(url: URL?, request: IOSAgentPermissionRequest) {
        if !isConversationExecution { selectedExecution?.resolveLocalAgentScopedResource(url: url, request: request); return }
        guard localAgentScopedResourceRequest == request else { return }
        if let continuation = bridgeScopedResourceContinuation {
            let outcome = agentCorePermissionBroker.scopedResourceWasSelected(for: request, url: url)
            bridgeScopedResourceContinuation = nil
            localAgentScopedResourceRequest = nil
            if case .scopedResourceGranted(_, let grant) = outcome { continuation.resume(returning: grant) }
            else { continuation.resume(returning: nil) }
            return
        }
        guard let coordinator = permissionCoordinator(for: request) else { return }
        Task { [weak self] in
            guard let self, self.ownsPermission(request, coordinator: coordinator) else { return }
            do {
                let fresh = try await coordinator.currentPermissionRequest(requestId: request.requestId)
                guard self.ownsPermission(request, coordinator: coordinator), fresh.childOwner == request.childOwner,
                      self.localAgentScopedResourceRequest == request else { return }
                let outcome = self.agentCorePermissionBroker.scopedResourceWasSelected(for: fresh, url: url)
                await self.applyLocalAgentPermissionOutcome(outcome, request: fresh, coordinator: coordinator)
            } catch { self.logger.log(.warn, "ChatManager", "Permission scope refresh failed", metadata: ["error": error.localizedDescription]) }
        }
    }

    private struct PreparedHybridDeviceTool {
        let plan: AgentCorePreparedToolPlan
        let resolvedTool: AgentCoreResolvedTool
        let permission: IOSAgentPermissionRequest
    }

    private func prepareHybridDeviceTool(_ request: HostDeviceToolRequest) -> PreparedHybridDeviceTool? {
        let identity = AgentCoreExpectedIdentity(
            hostId: request.hostId,
            clientId: request.clientId,
            sessionId: request.sessionId
        )
        let resultEgressAuthorization = IOSAgentResultEgressAuthorization.explicitUserApproval(
            identity: identity,
            userApproved: hybridDeviceToolsEnabled
        )
        guard let plan = try? AgentCoreToolCatalog.prepareBoundPlan(
            for: .hybrid,
            expectedIdentity: identity,
            isForeground: UIApplication.shared.applicationState == .active,
            allowedCapabilities: hybridDeviceCapabilities,
            resultEgressAuthorization: resultEgressAuthorization,
            scopedGrants: agentCorePermissionBroker.scopedGrants
        ),
        let resolved = plan.resolvedTool(
            id: request.toolName,
            route: "device",
            hostId: request.hostId,
            clientId: request.clientId,
            sessionId: request.sessionId
        ),
        let permission = IOSAgentPermissionRequest(
            requestId: request.id,
            allowConversation: false,
            callId: request.id,
            resolvedTool: resolved,
            plan: plan,
            rationale: "Requested by paired host \(request.hostId)",
            scope: request.argsPreview
        ) else { return nil }
        return PreparedHybridDeviceTool(plan: plan, resolvedTool: resolved, permission: permission)
    }

    private func executeHybridDeviceTool(
        _ request: HostDeviceToolRequest,
        prepared: PreparedHybridDeviceTool
    ) async -> IOSAgentHostCallResult {
        let permission = prepared.permission
        let outcome = await agentCorePermissionBroker.resolve(permission, userApproved: true)
        let grant: AgentCoreJSONValue?
        switch outcome {
        case .decision(_, let decision):
            guard decision != .deny else {
                return IOSAgentHostCallResult(
                    callId: request.id,
                    output: .object(["error": .string("System permission denied")]),
                    isError: true
                )
            }
            grant = nil
        case .scopedResourceGranted(_, let value):
            grant = value
        case .requiresScopedResource:
            localAgentScopedResourceRequest = permission
            grant = await withCheckedContinuation { continuation in
                bridgeScopedResourceContinuation = continuation
            }
            guard grant != nil else {
                return IOSAgentHostCallResult(
                    callId: request.id,
                    output: .object(["error": .string("No document was selected")]),
                    isError: true
                )
            }
        case .unavailable(_, let reason):
            return IOSAgentHostCallResult(
                callId: request.id,
                output: .object(["error": .string(reason)]),
                isError: true
            )
        }
        return await agentCoreHostExecutor.executeBridge(
            callId: request.id,
            arguments: request.arguments,
            grant: grant,
            resolvedTool: prepared.resolvedTool,
            plan: prepared.plan
        )
    }

    private func deviceToolResultPayload(
        request: HostDeviceToolRequest,
        result: IOSAgentHostCallResult
    ) -> [String: Any] {
        let output: Any = (try? JSONEncoder().encode(result.output))
            .flatMap { try? JSONSerialization.jsonObject(with: $0) } ?? NSNull()
        return [
            "protocol_version": request.protocolVersion,
            "nonce": request.nonce,
            "request_id": request.id,
            "host_id": request.hostId,
            "client_id": request.clientId,
            "session_id": request.sessionId,
            "tool_name": request.toolName,
            "output": output,
            "error": result.isError,
        ]
    }

    private func applyLocalAgentPermissionOutcome(_ outcome: IOSAgentPermissionOutcome,
        request: IOSAgentPermissionRequest, coordinator: DeviceAgentCoordinator) async {
        guard ownsPermission(request, coordinator: coordinator) else { return }
        switch outcome {
        case .decision(let requestID, let decision):
            guard requestID == request.requestId else { return }
            await dispatchLocalAgentPermission(request: request,
                decision: decision == .deny ? .deny : (localAgentPermissionDecisions[request.presentationID] ?? .deny),
                grant: nil, coordinator: coordinator)
        case .scopedResourceGranted(let requestID, let grant):
            guard requestID == request.requestId else { return }
            await dispatchLocalAgentPermission(request: request,
                decision: localAgentPermissionDecisions[request.presentationID] ?? .deny, grant: grant, coordinator: coordinator)
        case .requiresScopedResource: localAgentScopedResourceRequest = request
        case .unavailable(_, let reason):
            if pendingHostPermissionRequest == nil { hostPermissionSubmissionError = reason }
            logger.log(.warn, "ChatManager", "Local agent capability unavailable", metadata: ["reason": reason])
            // Broker unavailability is not a user denial and must remain retryable.
        }
    }

    private func dispatchLocalAgentPermission(request: IOSAgentPermissionRequest,
        decision: AgentCorePermissionDecision, grant: AgentCoreJSONValue?, coordinator: DeviceAgentCoordinator) async {
        guard ownsPermission(request, coordinator: coordinator) else { return }
        do {
            try await coordinator.resolvePermission(requestId: request.requestId, decision: decision, grant: grant)
            guard permissionCoordinator(for: request) === coordinator else { return }
            removeLocalAgentPermission(requestId: request.presentationID)
            if localAgentScopedResourceRequest?.presentationID == request.presentationID { localAgentScopedResourceRequest = nil }
        } catch {
            guard permissionCoordinator(for: request) === coordinator else { return }
            if pendingHostPermissionRequest == nil { hostPermissionSubmissionError = error.localizedDescription }
            logger.log(.warn, "ChatManager", "Device agent permission resolution failed", metadata: ["error": error.localizedDescription])
        }
    }

    private func enqueueLocalAgentPermission(_ request: IOSAgentPermissionRequest) {
        guard !localAgentPermissionQueue.contains(where: { $0.presentationID == request.presentationID }) else { return }
        localAgentPermissionQueue.append(request)
        if localAgentPermissionRequest == nil { localAgentPermissionRequest = request }
    }

    private func removeLocalAgentPermission(requestId: String) {
        localAgentPermissionDecisions.removeValue(forKey: requestId)
        localAgentPermissionQueue.removeAll { $0.presentationID == requestId }
        if localAgentPermissionRequest?.presentationID == requestId { localAgentPermissionRequest = localAgentPermissionQueue.first }
    }

    private func mergedUserMessageMeta(displayMessage: String?, extra: [String: String]?) -> [String: String]? {
        var meta = extra ?? [:]
        if let displayMessage,
           !displayMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
           displayMessage != "" {
            meta["displayText"] = displayMessage
        }
        return meta.isEmpty ? nil : meta
    }

    private func persistAssistantMessageIfNeeded(_ text: String, modelName: String, conversationId: UUID? = nil, messageID: UUID? = nil) {
        guard let convId = conversationId ?? currentConversationId else { return }
        var meta = ["model": modelName]
        if let encodedSources = ChatSourceReference.encodeForConversationMeta(sourceReferences) {
            meta[ChatSourceReference.conversationMetaKey] = encodedSources
        }
        if let messageID,
           let encodedTasks = AgentCoreTaskProjectionMeta.encode(
                agentTaskProjection,
                assistantMessageId: messageID
           ) {
            meta[AgentCoreTaskProjectionMeta.conversationMetaKey] = encodedTasks
        }
        meta = AgentCoreSkillProjectionMetadata.encode(agentSkillRuntimeProjection, into: meta)
        let assistantMsg = ConversationMessage(
            id: messageID ?? UUID(),
            conversationId: convId,
            role: .assistant,
            text: text,
            createdAt: Date(),
            meta: meta
        )
        do {
            try appendMessageToConversation(conversationId: convId, message: assistantMsg)
        } catch {
            logger.log(.error, "ChatManager", "Failed to append assistant task acknowledgement", metadata: [
                "error": error.localizedDescription
            ])
        }

        Task.detached { [weak self] in
            guard let self = self else { return }
            await self.autoNameConversationIfNeeded(conversationId: convId)
        }
    }

    private func recordPersonalIntelligenceTurn(
        conversationId: UUID,
        response: String,
        model: String,
        route: String
    ) {
        let eventID = UUID().uuidString
        let summary = String(response.trimmingCharacters(in: .whitespacesAndNewlines).prefix(280))
        let event = IntelligenceEvent(
            id: eventID,
            source: "conversation",
            kind: "turn.completed",
            subject: conversationId.uuidString,
            summary: summary.isEmpty ? "Conversation turn completed" : summary,
            importance: 0.35,
            confidence: 1,
            tags: ["conversation", route],
            dedupeKey: "conversation:\(conversationId.uuidString):\(eventID)"
        )
        _ = PersonalIntelligenceCore.ingest(
            IntelligenceIngestRequest(
                events: [event],
                outcomes: [
                    IntelligenceTaskOutcome(
                        taskType: "conversation",
                        route: route,
                        model: model,
                        success: !summary.isEmpty
                    )
                ]
            )
        )
    }

    private static func mergedMemoryProfile(
        existing: ChatMemoryProfile,
        newLongTermMemory: [String],
        newUserPreferences: [String],
        provenance: ChatMemoryProvenance,
        observedAt: Date
    ) -> ChatMemoryProfile {
        existing.addingMemory(
            longTerm: newLongTermMemory,
            preferences: newUserPreferences,
            provenance: provenance,
            observedAt: observedAt
        )
    }

    // Keep default-titled conversations named after the first user query.
    private func autoNameConversationIfNeeded(conversationId: UUID? = nil) async {
        guard let store = container.conversationStore, let convId = conversationId ?? currentConversationId else { return }
        do {
            if let conv = try store.getConversation(conversationId: convId) {
                if !isDefaultConversationTitle(conv.title) { return }

                let msgs = try store.getMessages(conversationId: convId, limit: 0)
                guard let title = msgs.lazy.compactMap({ $0.role == .user ? self.firstQueryTitle(from: $0.text) : nil }).first else { return }
                try store.updateConversationTitle(conversationId: convId, title: title)
                Task.detached { [weak self] in
                    await self?.updateRemoteConversationTitleIfNeeded(conversationId: convId, title: title)
                }
                logger.log(.info, "ChatManager", "Named conversation from first user query", metadata: ["conversationId": convId.uuidString])
            }
        } catch {
            logger.log(.debug, "ChatManager", "Auto-name conversation failed", metadata: ["error": error.localizedDescription])
        }
    }
    
    public func reset() {
        if !isConversationExecution { selectedExecution?.reset() }
        currentResponse = ""
        processingState = .idle
        currentTokenCount = 0
    }

    /// Clear a conversation and keep a backup for possible undo within a short window.
    /// If conversationId is nil, will clear currentConversationId if any.
    public func clearConversation(_ conversationId: UUID? = nil) async {
        guard let store = container.conversationStore else {
            // no persistent store; just clear in-memory id
            await MainActor.run {
                self.currentConversationId = nil
                self.lastDeletedConversationBackup = nil
            }
            return
        }

        let cid = conversationId ?? currentConversationId
        guard let convId = cid else {
            return
        }

        do {
            // Fetch conversation and messages for backup
            if let conv = try store.getConversation(conversationId: convId) {
                let msgs = try store.getMessages(conversationId: convId, limit: 0)
                // store backup
                await MainActor.run {
                    self.lastDeletedConversationBackup = (conversation: conv, messages: msgs, deletedAt: Date())
                }
            }

            // perform deletion
            try store.deleteConversation(conversationId: convId)
            // Only retire the session after canonical deletion succeeds. An
            // active Run rejected by Store keeps its execution and observers.
            conversationExecutions.removeValue(forKey: convId)
            Task.detached { [weak self] in
                await self?.deleteRemoteConversationIfNeeded(conversationId: convId)
            }
            await MainActor.run {
                if self.currentConversationId == convId {
                    self.currentConversationId = nil
                }
            }

            logger.log(.info, "ChatManager", "Cleared conversation", metadata: ["conversationId": convId.uuidString])
        } catch {
            logger.log(.error, "ChatManager", "Failed to clear conversation", metadata: ["error": error.localizedDescription])
        }
    }

    /// Undo the last clear if within 5 minutes. Restores conversation metadata and messages (with a new conversation id).
    public func undoClearConversation() async throws {
        guard let store = container.conversationStore else { throw ChatManagerError.failedToCreateConversation("no store") }
        guard let backup = lastDeletedConversationBackup else { throw ChatManagerError.failedToCreateConversation("no backup available") }

        // limit undo window to 5 minutes
        if Date().timeIntervalSince(backup.deletedAt) > 300 {
            // expire backup
            await MainActor.run { self.lastDeletedConversationBackup = nil }
            throw ChatManagerError.failedToCreateConversation("undo window expired")
        }

        // create new conversation preserving title/language/persistence flag
        let conv = try store.createConversation(title: backup.conversation.title, language: backup.conversation.language)
        let restoredConversation = Conversation(
            id: conv.id,
            title: backup.conversation.title,
            createdAt: conv.createdAt,
            lastActiveAt: backup.conversation.lastActiveAt,
            language: backup.conversation.language,
            sessionMemory: backup.conversation.sessionMemory,
            lastCompactedAt: backup.conversation.lastCompactedAt,
            selectedPromptTemplateID: backup.conversation.selectedPromptTemplateID,
            dataSourceOverrides: backup.conversation.dataSourceOverrides,
            selectedModelInfo: backup.conversation.selectedModelInfo,
            persistenceEnabled: backup.conversation.persistenceEnabled
        )
        try store.upsertConversation(restoredConversation)

        // Restore finalized history through the explicit import contract.
        try store.importMessages(
            conversationId: conv.id,
            messages: backup.messages.map { original in
                ConversationMessage(
                    conversationId: conv.id,
                    role: original.role,
                    text: original.text,
                    createdAt: original.createdAt,
                    meta: original.meta
                )
            },
            source: .backupRestore
        )

        await MainActor.run {
            self.currentConversationId = conv.id
            self.lastDeletedConversationBackup = nil
        }

        logger.log(.info, "ChatManager", "Undo clear conversation succeeded", metadata: ["restoredConversationId": conv.id.uuidString])
    }

    // MARK: - Local-only sync stubs

    public func syncNow() {}

    private func syncConversationsFromBackend() async {}

    private func createRemoteConversationIfNeeded(conversation: Conversation) async { _ = conversation }

    private func appendRemoteMessageIfNeeded(_ message: ConversationMessage) async { _ = message }

    private func updateRemoteConversationTitleIfNeeded(conversationId: UUID, title: String) async {
        _ = (conversationId, title)
    }

    private func deleteRemoteConversationIfNeeded(conversationId: UUID) async { _ = conversationId }

    private func canAttemptRemoteChatSync(conversationId: UUID? = nil) -> Bool {
        _ = conversationId
        return false
    }

    private func parseISODate(_ raw: String?) -> Date? {
        guard let raw, !raw.isEmpty else { return nil }
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = iso.date(from: raw) {
            return d
        }
        let fallback = ISO8601DateFormatter()
        fallback.formatOptions = [.withInternetDateTime]
        return fallback.date(from: raw)
    }
    
    public func getStateDescription() -> String {
        if retrievalInProgress {
            return L10n.chatRetrievalInProgress
        }
        switch processingState {
        case .idle:
            return L10n.chatStateIdle
        case .loadingLocalModel:
            return L10n.chatStateLoadingLocal
        case .processingWithLocal:
            return L10n.chatStateProcessingLocal
        case .processingWithCloud:
            return L10n.chatStateProcessingCloud
        case .streaming(let progress):
            return String(format: L10n.chatStateStreamingFormat, Int(progress * 100))
        case .error(let message):
            return String(format: L10n.chatStateErrorFormat, message)
        case .completed:
            return L10n.chatStateCompleted
        }
    }
    
    public func getStateIcon() -> String {
        if retrievalInProgress {
            return "magnifyingglass"
        }
        switch processingState {
        case .idle:
            return "checkmark.circle"
        case .loadingLocalModel, .processingWithLocal, .processingWithCloud, .streaming:
            // Use a more widely-available symbol to avoid missing symbol issues
            return "hourglass"
        case .error:
            return "exclamationmark.circle.fill"
        case .completed:
            return "checkmark.circle.fill"
        }
    }
    
    public func getStateColor() -> Color {
        if retrievalInProgress {
            return Color.purple
        }
        switch processingState {
        case .idle:
            return .gray
        case .loadingLocalModel, .processingWithLocal, .processingWithCloud, .streaming:
            return .blue
        case .error:
            return .red
        case .completed:
            return .green
        }
    }
    
    // MARK: - Private Methods
    
    private func ensureLocalModelLoaded() async throws {
        // Log current selection and container flags for diagnostics
        let lastSelected = self.selectedModelInfo ?? ModelSelectionManager.getLastSelectedModel()
        logger.log(.debug, "ChatManager", "ensureLocalModelLoaded start", metadata: ["selected": lastSelected?.id ?? "(none)", "useLocalModel": String(container.useLocalModel)])

        // Determine desired LMModel for the selected model id (if any)
        var desiredLMModel: LMModel? = nil
        if let sel = lastSelected, sel.isLocal {
            if let mtype = ModelDownloader.ModelType(rawValue: sel.id), let mlxName = mtype.mlxModelName {
                desiredLMModel = MLXService.availableModels.first { $0.name == mlxName }
            }
            if desiredLMModel == nil {
                desiredLMModel = MLXService.availableModels.first { $0.id == sel.id || $0.name == sel.id }
            }
        }

        guard let desiredLMModel else {
            throw NSError(
                domain: "Nola.LocalModel",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: NSLocalizedString("model.local.unavailable.error", value: "The selected local model is unavailable.", comment: "Selected local model cannot be resolved")]
            )
        }

        guard MLXService.shared.isRunning(desiredLMModel) else {
            throw NSError(
                domain: "Nola.LocalModel",
                code: 1,
                userInfo: [
                    NSLocalizedDescriptionKey: NSLocalizedString(
                        "model.local.not_started.error",
                        value: "Start the selected local model in Model Center before chatting.",
                        comment: "Local model is selected but its runtime is stopped"
                    )
                ]
            )
        }

        // Determine model-specific parameters and apply them when switching to a local provider
        let modelIDForParameters = lastSelected?.id ?? "default"
        let params = ModelParametersStore.shared.loadParameters(for: modelIDForParameters, isLocal: lastSelected?.isLocal ?? true)
        guard ModelParameterLimits.outputTokens.contains(params.maxOutputTokens) else {
            throw NolaError.llm(.invalidInput(reason: "Invalid model output token limit"))
        }
        let maxTokensToUse = params.maxOutputTokens
        let tempToUse = Float(params.temperature)
        let topPToUse = Float(params.topP)
        let topKToUse = params.topK
        // Apply showThinking setting to ChatManager
        self.showThinking = params.showThinking

        if isConversationExecution {
            executionClient = ModelService(provider: LocalProvider(model: desiredLMModel,
                maxTokens: maxTokensToUse, temperature: tempToUse, topP: topPToUse,
                topK: topKToUse, enableThinking: params.showThinking))
            return
        }

        // If we have a ModelService, switch to local provider with desired model and parameters.
        if let modelService = effectiveLLMClient as? ModelService {
            if !container.useLocalModel {
                // Apply the explicit local selection together with the mode change.
                container.useLocalModel = true
                container.reconfigureIfPossible(selectedModel: getCurrentModel())
            } else {
                // Already in local mode - directly apply the desired model to ensure the provider uses the selected model and params
                modelService.useLocalProvider(model: desiredLMModel, maxTokens: maxTokensToUse, temperature: tempToUse, topP: topPToUse, topK: topKToUse, enableThinking: params.showThinking)
            }
        } else {
            // Replace client with a ModelService backed by a LocalProvider using desired model and params
            container.llmClient = ModelService(provider: LocalProvider(model: desiredLMModel, maxTokens: maxTokensToUse, temperature: tempToUse, topP: topPToUse, topK: topKToUse, enableThinking: params.showThinking))
        }

        logger.log(
            .info,
            "ChatManager",
            "Local model initialized",
            metadata: ["appliedModel": desiredLMModel.id]
        )
    }
}
enum ExecutionBackendSelectionPolicy {
    static func validatedSelection(
        _ backendID: String,
        availableBackendIDs: Set<String>
    ) throws -> String {
        let normalized = backendID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty, availableBackendIDs.contains(normalized) else {
            throw NolaError.llm(.invalidResponse(
                reason: "The selected execution unit is not available on the connected Host."
            ))
        }
        return normalized
    }

    /// Reconciles persisted conversation state with the latest Host catalog.
    /// A persisted available choice wins, followed by the Host preference,
    /// Nola, and finally the first available catalog entry.
    static func resolvedSelection(
        current backendID: String,
        hostPreferredBackendID: String?,
        availableBackendIDs: [String]
    ) -> String? {
        let available = availableBackendIDs.filter { !$0.isEmpty }
        guard !available.isEmpty else { return nil }
        let normalizedCurrent = backendID.trimmingCharacters(in: .whitespacesAndNewlines)
        if available.contains(normalizedCurrent) {
            return normalizedCurrent
        }
        let normalizedPreferred = hostPreferredBackendID?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if available.contains(normalizedPreferred) {
            return normalizedPreferred
        }
        if available.contains("nola") {
            return "nola"
        }
        return available.first
    }
}

enum HostPermissionPresentationPolicy {
    static func shouldPresent(
        pendingConversationID: UUID?,
        currentConversationID: UUID?
    ) -> Bool {
        pendingConversationID == currentConversationID
    }
}

extension ChatManager {
    /// Edited resend is a fresh ordinary Run, never a replay of the source owner.
    /// Returns after durable admission; the workspace retains the normal Host job.
    func sendEditedMessage(_ request: AppleMessageEditSubmission) async throws -> UUID {
        guard !isConversationExecution else { return try await container.chatManager.sendEditedMessage(request) }
        let runtime = container.applicationRuntime
        let title = String(request.text.unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) }
            .map(String.init).joined().prefix(80))
        let input = request.preparation(title: title)
        let sourceManifest: AppleExpertMessageManifest<AgentCoreJSONValue>? = try runtime.expertHostOperation(
            .loadManifest(conversationID: input.source_conversation_id, messageID: input.source_message_id, authority: .local()))
        let encoded = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(input))
        if let sourceManifest {
            return try await sendEditedExperts(input: input, sourceManifest: sourceManifest)
        }
        let result = try runtime.executeAgentCommand(.object(["type": .string("prepare_edited_resend"), "input": encoded]))
        let prepared = try JSONDecoder().decode(ApplePreparedEditedResend<AgentCoreJSONValue>.self,
            from: JSONEncoder().encode(result))
        guard prepared.input == input else { throw IOSApplicationRuntimeError.invalidResponse }
        return try await dispatchPreparedEditedMessage(prepared)
    }

    /// Explicit cold recovery reads the durable original edited intent. No new
    /// editor IDs, current model selection, or source composer are substituted.
    func resumePreparedExpertConversation(_ conversationID: String) async throws -> Bool {
        guard !isConversationExecution else { return try await container.chatManager.resumePreparedExpertConversation(conversationID) }
        let runtime = container.applicationRuntime
        let loaded: ApplePreparedExpertResend<AgentCoreJSONValue>? = try runtime.expertHostOperation(
            .loadPreparedResend(conversationID: conversationID, authority: .local()))
        guard let prepared = loaded else { return false }
        guard prepared.input.target_conversation_id == conversationID else { throw IOSApplicationRuntimeError.invalidResponse }
        let source: AppleExpertMessageManifest<AgentCoreJSONValue>? = try runtime.expertHostOperation(
            .loadManifest(conversationID: prepared.input.source_conversation_id, messageID: prepared.input.source_message_id, authority: .local()))
        guard let manifest = source else { throw IOSApplicationRuntimeError.invalidResponse }
        _ = try await sendEditedExperts(input: prepared.input, sourceManifest: manifest)
        return true
    }

    /// Dispatch a complete Core expert plan. Admitted peers are observed on retry;
    /// a partial admission keeps the editor intent and never claims full success.
    private func sendEditedExperts(input: ApplePrepareEditedResend,
        sourceManifest: AppleExpertMessageManifest<AgentCoreJSONValue>) async throws -> UUID {
        let runtime = container.applicationRuntime
        let plan: ApplePreparedExpertResend<AgentCoreJSONValue> = try runtime.expertHostOperation(
            .prepareResend(input: input, authority: .local()))
        guard plan.input == input, !plan.entries.isEmpty,
              plan.source_submission_id == sourceManifest.input.submission_id,
              plan.entries.count == sourceManifest.entries.count,
              sourceManifest.entries.map(\.participant) == sourceManifest.input.selected_participants,
              zip(plan.entries, sourceManifest.entries).allSatisfy({ entry, source in
                  entry.original.participant == source.participant && entry.original.child_submission_id == source.child_submission_id
                    && entry.original.child_user_message_id == source.child_user_message_id
                    && entry.participant.agent_id == source.participant.agent_id
                    && entry.participant.agent_revision == source.participant.agent_revision
                    && entry.prepared.source.run_id == source.child_submission_id
                    && entry.prepared.source.message_id == source.child_user_message_id
              }),
              let target = UUID(uuidString: input.target_conversation_id) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        var admitted = 0
        do {
            for entry in plan.entries {
                guard entry.prepared.input.target_conversation_id == entry.participant.context_conversation_id else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                _ = try await dispatchPreparedEditedMessage(entry.prepared, expertRoomID: plan.room_id,
                    expertParentMessageID: plan.parent_message_id, expertParticipant: entry.participant)
                admitted += 1
            }
            let observed: ApplePreparedExpertResend<AgentCoreJSONValue>? = try runtime.expertHostOperation(
                .loadPreparedResend(conversationID: input.target_conversation_id, authority: .local()))
            guard let committed = observed else { throw IOSApplicationRuntimeError.invalidResponse }
            guard committed.input == input, committed.entries.count == plan.entries.count,
                  zip(committed.entries, plan.entries).allSatisfy({ current, original in
                      current.participant == original.participant &&
                      current.prepared.input == original.prepared.input &&
                      current.prepared.run_id == original.prepared.run_id &&
                      current.prepared.admitted_run_id == original.prepared.run_id
                  }) else { throw IOSApplicationRuntimeError.invalidResponse }
            return target
        } catch {
            if case IOSApplicationRuntimeError.outcomePending = error { throw error }
            throw IOSApplicationRuntimeError.rejected("Expert admission is partial (\(admitted)/\(plan.entries.count)). Retry this edit to resume the same intent. \(error.localizedDescription)")
        }
    }

    private func dispatchPreparedEditedMessage(_ prepared: ApplePreparedEditedResend<AgentCoreJSONValue>,
        expertRoomID: String? = nil, expertParentMessageID: String? = nil,
        expertParticipant: AppleExpertParticipantReference? = nil) async throws -> UUID {
        let runtime = container.applicationRuntime
        guard let target = UUID(uuidString: prepared.input.target_conversation_id) else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let roomService = IOSAgentRoomService(runtime: runtime)
        var roomReceipt: IOSRoomSubmission?
        var existingRoomIntent: IOSAgentRoomService.ExecutionIntent?
        if let expertRoomID, let expertParticipant, let expertParentMessageID {
            guard let receipt = try roomService.submission(roomID: expertRoomID, submissionID: prepared.run_id),
                  receipt.id == prepared.run_id, receipt.input.parentMessageID == expertParentMessageID,
                  receipt.contextConversationID == prepared.input.target_conversation_id,
                  receipt.selectedParticipantID == expertParticipant.participant_id else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            roomReceipt = receipt
            existingRoomIntent = try roomService.executionIntent(receipt)
        }
        let identity = try IOSCanonicalAgentTurnIdentity(editedResend: prepared)
        if let admitted = prepared.admitted_run_id {
            guard admitted == prepared.run_id,
                  roomReceipt == nil || roomReceipt?.executionID == prepared.run_id else { throw IOSApplicationRuntimeError.invalidResponse }
            if let roomReceipt { _ = try roomService.observeExecution(roomReceipt) }
            return target // Lost acknowledgement: observe the existing admission, never resubmit.
        }
        guard editedResendJobs[target] == nil else { throw DeviceAgentCoordinatorError.alreadyRunning }
        let capturedDefinitions: [AppleExpertPromptDefinition<AgentCoreJSONValue>]? = try runtime.expertHostOperation(
            .loadChildInstructions(conversationID: prepared.input.target_conversation_id, expectedRunID: prepared.run_id, authority: .local()))
        let definitions = capturedDefinitions ?? []
        let contextComponents = definitions.map {
            AgentCoreContextComponent(kind: .promptTemplate, id: $0.component_id,
                content: .string($0.content),
                source: .init(platform: .ios, title: "Expert instruction", mimeType: "text/plain"))
        }
        let model = try IOSPreparedRunModel.reconstitute(prepared.source.model, using: runtime)
        guard let conversation = try runtime.loadConversation(id: prepared.input.target_conversation_id),
              conversation.messages.isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        guard let configured = container.conversationStore as? any ConfiguredConversationCreationStore else {
            throw IOSApplicationRuntimeError.applicationUnavailable
        }
        if existingRoomIntent == nil {
            try configured.selectConversationModel(conversationID: target, model: model.displayModel, initialParameters: model.parameters)
        }
        guard let configuredTarget = try runtime.loadConversation(id: prepared.input.target_conversation_id),
              configuredTarget.messages.isEmpty else { throw IOSApplicationRuntimeError.invalidResponse }
        if let receipt = roomReceipt {
            guard let revision = configuredTarget.configurationRevision,
                  let profile = try runtime.modelProfile(id: model.displayModel.id),
                  profile.enabled, profile.deletedAtMs == nil else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            let intent = IOSAgentRoomService.ExecutionIntent(execution_id: prepared.run_id,
                envelope: .init(kind: "native", platform: "ios", conversation_configuration_revision: revision,
                    model_profile_id: profile.id, model_profile_revision: profile.revision,
                    expected_parent_message_id: nil, text: prepared.input.edited_text))
            if let existingRoomIntent, existingRoomIntent != intent {
                throw IOSApplicationRuntimeError.rejected("The prepared expert target changed; no replacement request was sent")
            }
            let ready: IOSRoomSubmission
            if existingRoomIntent == nil { ready = try roomService.prepareExecution(receipt, intent: intent) }
            else { ready = receipt }
            let claim = try roomService.claimExecution(ready)
            guard claim.submission.executionID == prepared.run_id, claim.submission.state == "executing",
                  claim.submission.input == receipt.input else { throw IOSApplicationRuntimeError.invalidResponse }
            roomReceipt = claim.submission
        }
        let claimedRoomReceipt = roomReceipt
        let execution = ChatManager(container: container, restorePersistedState: false, isConversationExecution: true)
        execution.selectedModelInfo = model.displayModel
        execution.executionClient = ModelService(provider: model.client)
        execution.currentConversationId = target
        execution.activeGenerationId &+= 1
        execution.processingState = model.displayModel.isLocal ? .processingWithLocal : .processingWithCloud
        execution.beginSubmissionAttempt()
        execution.prepareAgentTrace(conversationId: target, canonicalAssistantMessageID: prepared.assistant_message_id)
        conversationExecutions[target] = execution
        let admission = AsyncThrowingStream<UUID, Error>.makeStream()
        execution.submissionCommitted = {
            do {
                let result = try runtime.executeAgentCommand(.object([
                    "type": .string("load_prepared_edited_resend"), "conversation_id": .string(prepared.input.target_conversation_id),
                ]))
                let committed = try JSONDecoder().decode(ApplePreparedEditedResend<AgentCoreJSONValue>.self,
                    from: JSONEncoder().encode(result))
                guard committed.input == prepared.input, committed.run_id == prepared.run_id,
                      committed.admitted_run_id == prepared.run_id,
                      committed.user_message_id == prepared.user_message_id,
                      committed.assistant_message_id == prepared.assistant_message_id else { throw IOSApplicationRuntimeError.invalidResponse }
                admission.continuation.yield(target)
                admission.continuation.finish()
            } catch { admission.continuation.finish(throwing: error) }
        }
        editedResendJobs[target] = Task { @MainActor [self, execution] in
            defer {
                execution.submissionCommitted = nil
                editedResendJobs[target] = nil
                if let claimedRoomReceipt { _ = try? roomService.observeExecution(claimedRoomReceipt) }
            }
            do {
                _ = try await execution.runDeviceAgentTurn(sessionId: prepared.input.target_conversation_id,
                    input: prepared.input.edited_text, history: [], conversationId: target, preparedModel: model,
                    turnIdentity: identity, persistedUserText: prepared.input.edited_text, userContentJSON: nil,
                    inputOrigin: .interactiveUser, expectedConfigurationRevision: configuredTarget.configurationRevision,
                    expectedParentMessageID: nil, contextComponents: contextComponents,
                    freshResendTarget: prepared.input.target_conversation_id)
                execution.processingState = .idle
                execution.thinkingFinished = true
                admission.continuation.finish(throwing: DeviceAgentCoordinatorError.missingTransition)
            } catch {
                execution.recordSubmissionFailure(error)
                execution.processingState = error is CancellationError ? .idle : .error(error.localizedDescription)
                admission.continuation.finish(throwing: error)
            }
        }
        for try await admitted in admission.stream { return admitted }
        throw DeviceAgentCoordinatorError.missingTransition
    }
}
