// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the Mac local-agent process boundary and maps its event stream into native chat events.

import Foundation
import Darwin
import CryptoKit
import UniformTypeIdentifiers

nonisolated struct MacLocalAgentRunRequest: Sendable {
    let conversationID: String
    var editedResendOperationID: String? = nil
    var editedResendModel: AppleFrozenResendModel<MacCanonicalJSONValue>? = nil
    var goalControl: AppleGoalControl?
    var goalWakeID: String?
    var goalExecutionID: String? {
        if let goalWakeID { return goalWakeID }
        guard let goalControl else { return nil }
        return goalControl.action == .start ? "goal-\(goalControl.operationId)" : goalControl.expectedGoalId
    }
    let submissionID: String
    let queuedUserCommandID: String?
    let userMessageID: String
    let assistantMessageID: String
    let expectedConfigurationRevision: Int64?
    let expectedParentMessageID: String?
    let prompt: String
    let contextComponents: [MacCoreContextComponent]
    let projectURL: URL
    let attachmentURLs: [URL]
    let modelID: String?
    /// Canonical profile identity, never the upstream model name.
    let modelProfileID: String?
    let modelProfileRevision: Int64?
    let siwcProfile: MacSiwcProfileSelection?
    let localModelEndpoint: MacMLXOpenAIService.Endpoint?
    let modelProviderID: String?
    let endpoint: String?
    let apiKey: String?
    let contextWindowTokens: Int?
    let maxModelIterations: Int
    let modelPurpose: MacRemoteModelCapability?
    let modelTestMode: Bool
    let modelFallbacks: [MacRemoteModelFallback]
    let reasoningConfiguration: MacReasoningConfiguration?
    let reasoningPreference: String
    let reasoningTier: String?
    let modelParameters: ModelParameters?
    let modelExecutionCapabilities: MacModelExecutionCapabilities?
    let wireProtocol: MacModelWireProtocol?
    let samplingOverride: Bool
    let visionInput: Bool
    let executionBackend: String
    let externalSessionID: String?
    let inputOrigin: String
    let memoryEnabled: Bool
    let permissionMode: MacPermissionMode
    let databaseURLOverride: URL?
    let appEnvironment: MacAppEnvironment
    let applicationPort: MacApplicationPort?
    var allowedDirectoryArguments: [String]
    let authorizedWorkspaceURLs: [URL]
    var skillDirectoryArguments: [String]
    let activeSkillDescriptors: [MacBuiltInSkillDescriptor]
    let activeExternalSkillDirectories: [URL]
    let activeMCPServerConfigurationData: Data?
    var submissionTicket: MacRunSubmissionTicket?
    var submissionAccepted: (@MainActor @Sendable () async -> Void)?
    /// Admission-only material. The canonical checkpoint owns loaded context on resume.
    var automationAdmission: MacAutomationHostAdmission?

    init(
        conversationID: String,
        submissionID: String = UUID().uuidString,
        queuedUserCommandID: String? = nil,
        userMessageID: String = UUID().uuidString,
        assistantMessageID: String? = nil,
        expectedConfigurationRevision: Int64? = nil,
        expectedParentMessageID: String? = nil,
        prompt: String,
        contextComponents: [MacCoreContextComponent] = [],
        projectURL: URL,
        attachmentURLs: [URL],
        authorizedWorkspaceURLs: [URL] = [],
        modelID: String? = nil,
        modelProfileID: String? = nil,
        modelProfileRevision: Int64? = nil,
        siwcProfile: MacSiwcProfileSelection? = nil,
        localModelEndpoint: MacMLXOpenAIService.Endpoint? = nil,
        modelProviderID: String? = nil,
        endpoint: String? = nil,
        apiKey: String? = nil,
        contextWindowTokens: Int? = nil,
        maxModelIterations: Int = 0,
        modelPurpose: MacRemoteModelCapability? = nil,
        modelTestMode: Bool = false,
        modelFallbacks: [MacRemoteModelFallback] = [],
        reasoningConfiguration: MacReasoningConfiguration? = nil,
        reasoningPreference: String = "automatic",
        reasoningTier: String? = nil,
        modelParameters: ModelParameters? = nil,
        modelExecutionCapabilities: MacModelExecutionCapabilities? = nil,
        wireProtocol: MacModelWireProtocol? = nil,
        samplingOverride: Bool = false,
        executionBackend: String = "nola",
        externalSessionID: String? = nil,
        inputOrigin: String = "interactive_user",
        memoryEnabled: Bool = false,
        permissionMode: MacPermissionMode = .cautious,
        databaseURLOverride: URL? = nil,
        appEnvironment: MacAppEnvironment = .shared,
        applicationPort: MacApplicationPort? = .shared,
        submissionTicket: MacRunSubmissionTicket? = nil,
        submissionAccepted: (@MainActor @Sendable () async -> Void)? = nil
    ) throws {
        self.conversationID = conversationID
        self.automationAdmission = nil
        self.submissionTicket = submissionTicket
        self.submissionAccepted = submissionAccepted
        self.queuedUserCommandID = queuedUserCommandID
        self.submissionID = submissionID
        self.userMessageID = userMessageID
        self.expectedConfigurationRevision = expectedConfigurationRevision
        self.expectedParentMessageID = expectedParentMessageID
        let canonicalAssistantMessageID = "\(submissionID):assistant"
        if let assistantMessageID,
           assistantMessageID != canonicalAssistantMessageID {
            throw MacLocalAgentContractError.assistantMessageIdentityMismatch(
                expected: canonicalAssistantMessageID,
                actual: assistantMessageID
            )
        }
        self.assistantMessageID = canonicalAssistantMessageID
        self.prompt = prompt
        self.contextComponents = contextComponents
        self.projectURL = projectURL
        self.authorizedWorkspaceURLs = authorizedWorkspaceURLs
        self.attachmentURLs = attachmentURLs
        self.modelID = modelID
        self.modelProfileID = modelProfileID
        self.modelProfileRevision = modelProfileRevision
        self.siwcProfile = siwcProfile
        self.localModelEndpoint = localModelEndpoint
        self.modelProviderID = modelProviderID
        self.endpoint = endpoint
        self.apiKey = apiKey
        self.contextWindowTokens = contextWindowTokens
        // Match Core: zero means no arbitrary round cap. Explicit caller
        // budgets remain exact; cancellation and permission gates are separate.
        self.maxModelIterations = max(0, maxModelIterations)
        self.modelPurpose = modelPurpose
        self.modelTestMode = modelTestMode
        self.modelFallbacks = modelFallbacks
        self.reasoningConfiguration = reasoningConfiguration
        self.reasoningPreference = reasoningPreference
        self.reasoningTier = reasoningTier
        self.modelParameters = modelParameters
        self.modelExecutionCapabilities = modelExecutionCapabilities
        self.wireProtocol = wireProtocol
        self.samplingOverride = samplingOverride
        // Multimodal transport is a permanent runtime capability. Images are
        // added only when the user attaches one or an explicit tool result
        // contains image media, so text-only turns remain text-only.
        self.visionInput = true
        self.executionBackend = executionBackend
        self.externalSessionID = externalSessionID
        self.inputOrigin = inputOrigin
        self.memoryEnabled = memoryEnabled
        self.permissionMode = permissionMode
        self.databaseURLOverride = databaseURLOverride
        self.appEnvironment = appEnvironment
        self.applicationPort = applicationPort
        let skillOverridesRootURL = appEnvironment.configDirectory
            .appendingPathComponent("built-in-skill-overrides", isDirectory: true)
        let skillRegistryStore = applicationPort ?? .shared
        // Freeze the capability surface when the user starts the run. Global
        // Skill settings may change while the child Host is launching, but
        // that must not silently widen this request's filesystem authority.
        let activeExternalSkillDirectories = modelTestMode ? [] : MacLocalSkillActivationStore.activeSkillDirectories(
            workspaceURL: projectURL,
            globalSkillsURL: appEnvironment.skillsDirectory,
            store: applicationPort
        )
        self.activeExternalSkillDirectories = activeExternalSkillDirectories
        if !modelTestMode {
            try MacBuiltInSkillRegistry.synchronizeCatalog(
                overridesRootURL: skillOverridesRootURL,
                store: skillRegistryStore
            )
        }
        let activeSkills: [MacBuiltInSkillDescriptor]
        if modelTestMode {
            activeSkills = []
        } else {
            activeSkills = try MacBuiltInSkillRegistry.activeDescriptors(store: skillRegistryStore)
        }
        self.activeSkillDescriptors = activeSkills
        self.allowedDirectoryArguments = MacBuiltInSkillRegistry.allowedDirectoryArguments(
            projectURL: projectURL,
            activeSkills: activeSkills
        ) + (authorizedWorkspaceURLs + activeExternalSkillDirectories)
            .flatMap { ["--allow-dir", $0.path] }
        let builtInSkillArguments = MacBuiltInSkillRegistry.skillDirectoryArguments(
            activeSkills: activeSkills
        )
        let externalSkillArguments = activeExternalSkillDirectories.flatMap { ["--skill-dir", $0.path] }
        let enabledSkillArguments = builtInSkillArguments == ["--no-skills"]
            ? externalSkillArguments
            : builtInSkillArguments + externalSkillArguments
        self.skillDirectoryArguments = enabledSkillArguments.isEmpty
            ? ["--no-skills"]
            : enabledSkillArguments
        self.activeMCPServerConfigurationData = modelTestMode ? nil : MacLocalMCPActivationStore
            .activeServerConfigurationData(
                workspaceURL: projectURL,
                store: applicationPort
            )
    }
}

nonisolated enum MacLocalAgentContractError: LocalizedError, Equatable, Sendable {
    case assistantMessageIdentityMismatch(expected: String, actual: String)
    case visibleAssistantIdentityMismatch(canonicalID: String, expected: String, actual: String)

    var errorDescription: String? {
        switch self {
        case let .assistantMessageIdentityMismatch(expected, actual):
            return "Agent Host contract violation: assistant_message_id must be \(expected), received \(actual)."
        case let .visibleAssistantIdentityMismatch(canonicalID, expected, actual):
            return "UI identity contract violation for \(canonicalID): expected \(expected), received \(actual)."
        }
    }
}

/// Orders the user-visible stream independently of transport write timing.
/// Core terminal state is authoritative, but it must be delivered after every
/// already-produced Part/Trace frame so clients cannot finalize an empty turn.
nonisolated struct MacAgentHostEventDrainBuffer {
    private var terminal: MacLocalAgentEvent?

    static func isTerminal(_ event: MacLocalAgentEvent) -> Bool {
        switch event.kind {
        case .finished, .failed, .cancelled:
            return true
        default:
            return false
        }
    }

    mutating func receive(_ event: MacLocalAgentEvent) -> MacLocalAgentEvent? {
        guard Self.isTerminal(event) else { return event }
        if terminal == nil || event.coreTurnTerminal != nil {
            terminal = event
        }
        return nil
    }

    mutating func finish() -> MacLocalAgentEvent? {
        defer { terminal = nil }
        return terminal
    }
}

nonisolated enum MacAgentHostTransportDisposition: Equatable {
    case accepted
    case duplicate
}

/// Enforces Host journal identity before an event reaches presentation code.
/// UI reducers must never be responsible for repairing duplicate, reordered,
/// or cross-Run transport frames.
nonisolated struct MacAgentHostTransportGate {
    private let expectedTransportRunKey: String
    private let assistantMessageID: String
    private var acceptedTransportRunKey: String?
    private var lastSequenceByTraceID: [String: UInt64] = [:]
    private var eventIDs: Set<String> = []

    init(runID: String, assistantMessageID: String) {
        self.expectedTransportRunKey = Self.normalizedRunKey(runID)
        self.assistantMessageID = assistantMessageID
    }

    private static func normalizedRunKey(_ runID: String) -> String {
        let normalized = runID.lowercased()
        return normalized.hasPrefix("run_")
            ? String(normalized.dropFirst(4))
            : normalized
    }

    mutating func accept(_ event: MacLocalAgentEvent) throws -> MacAgentHostTransportDisposition {
        guard let trace = event.trace else { return .accepted }
        guard trace.schemaVersion == 1 else {
            throw MacLocalAgentRuntimeError.protocolViolation(
                "Unsupported Agent Host trace schema \(trace.schemaVersion)."
            )
        }
        let transportRunKey = Self.normalizedRunKey(trace.runID)
        guard !transportRunKey.isEmpty else {
            throw MacLocalAgentRuntimeError.protocolViolation(
                "Agent Host event \(trace.eventID) has an empty Run identity."
            )
        }
        guard transportRunKey == expectedTransportRunKey else {
            throw MacLocalAgentRuntimeError.protocolViolation(
                "Agent Host event \(trace.eventID) belongs to Run \(trace.runID), expected \(expectedTransportRunKey)."
            )
        }
        if let acceptedTransportRunKey, transportRunKey != acceptedTransportRunKey {
            throw MacLocalAgentRuntimeError.protocolViolation(
                "Agent Host changed Run identity inside one transport stream."
            )
        }
        if let messageID = trace.messageID, messageID != assistantMessageID {
            throw MacLocalAgentRuntimeError.protocolViolation(
                "Agent Host event \(trace.eventID) belongs to message \(messageID), expected \(assistantMessageID)."
            )
        }
        if eventIDs.contains(trace.eventID) { return .duplicate }
        let lastSequence = lastSequenceByTraceID[trace.traceID] ?? 0
        guard trace.sequence > lastSequence else {
            throw MacLocalAgentRuntimeError.protocolViolation(
                "Agent Host event sequence moved backward from \(lastSequence) to \(trace.sequence)."
            )
        }
        acceptedTransportRunKey = transportRunKey
        lastSequenceByTraceID[trace.traceID] = trace.sequence
        eventIDs.insert(trace.eventID)
        return .accepted
    }
}

nonisolated struct MacLocalAgentSessionSummary: Sendable {
    let id: String
    let title: String
    let updatedAt: Date
    let messageCount: Int
}

nonisolated struct MacRemoteUserInputOption: Hashable, Sendable {
    let id: String
    let label: String
    let detail: String
}

nonisolated struct MacRemoteUserInputQuestion: Identifiable, Hashable, Sendable {
    let id: String
    let prompt: String
    let options: [MacRemoteUserInputOption]
    let allowsFreeform: Bool
    let required: Bool
    let maxTextCharacters: Int
}

nonisolated struct MacUserInputAnswer: Hashable, Sendable {
    let questionID: String
    let optionID: String?
    let text: String?
    var object: [String: Any] {
        let value: [String: Any] = if let optionID {
            ["type": "option", "option_id": optionID]
        } else {
            ["type": "text", "text": text ?? ""]
        }
        return ["question_id": questionID, "value": value]
    }
}

nonisolated struct MacUserInputWaitReceipt: Sendable {
    let disposition: String
    let resumePolicy: String?
    let wait: MacRemoteUserInputRequest
}

nonisolated struct MacUserInputSubmissionIntent: Sendable {
    let commandID: String
    let request: MacRemoteUserInputRequest
    let answers: [MacUserInputAnswer]
    let decline: Bool

    func matches(request: MacRemoteUserInputRequest, answers: [MacUserInputAnswer], decline: Bool) -> Bool {
        self.request == request && self.answers == answers && self.decline == decline
    }
}

nonisolated struct MacRemoteUserInputRequest: Identifiable, Hashable, Sendable {
    let id: String
    let conversationID: String
    let runID: String
    let sessionID: String
    let revision: UInt64
    let schemaDigest: String
    let state: String
    let questions: [MacRemoteUserInputQuestion]
    var header: String { String(localized: "Answer questions") }
    var question: String { questions.map(\.prompt).joined(separator: "\n") }
    var isOpen: Bool { state == "open" }

    init?(object: [String: Any]) {
        guard let conversationID = object["conversation_id"] as? String, !conversationID.isEmpty,
              let runID = object["run_id"] as? String, !runID.isEmpty,
              let wait = object["wait"] as? [String: Any],
              let id = wait["wait_id"] as? String, !id.isEmpty,
              let sessionID = wait["core_session_id"] as? String, !sessionID.isEmpty,
              let revision = wait["revision"] as? NSNumber, revision.uint64Value > 0,
              let digest = wait["response_schema_digest"] as? String, digest.count == 64,
              let stateObject = wait["state"] as? [String: Any],
              let state = stateObject["type"] as? String, ["open", "resolved", "withdrawn"].contains(state),
              let questionSet = wait["question_set"] as? [String: Any],
              let rawQuestions = questionSet["questions"] as? [[String: Any]], !rawQuestions.isEmpty else { return nil }
        var questions: [MacRemoteUserInputQuestion] = []
        for raw in rawQuestions {
            guard let questionID = raw["id"] as? String, !questionID.isEmpty,
                  let prompt = raw["prompt"] as? String, !prompt.isEmpty,
                  let rawOptions = raw["options"] as? [[String: Any]],
                  let allowsFreeform = raw["allow_freeform"] as? Bool,
                  let required = raw["required"] as? Bool,
                  let maxCharacters = raw["max_text_chars"] as? Int, maxCharacters > 0 else { return nil }
            let options = rawOptions.compactMap { option -> MacRemoteUserInputOption? in
                guard let optionID = option["id"] as? String, !optionID.isEmpty,
                      let label = option["label"] as? String, !label.isEmpty else { return nil }
                return MacRemoteUserInputOption(id: optionID, label: label, detail: option["description"] as? String ?? "")
            }
            guard options.count == rawOptions.count, Set(options.map(\.id)).count == options.count else { return nil }
            questions.append(.init(id: questionID, prompt: prompt, options: options,
                                   allowsFreeform: allowsFreeform, required: required, maxTextCharacters: maxCharacters))
        }
        guard Set(questions.map(\.id)).count == questions.count else { return nil }
        self.id = id
        self.conversationID = conversationID
        self.runID = runID
        self.sessionID = sessionID
        self.revision = revision.uint64Value
        self.schemaDigest = digest
        self.state = state
        self.questions = questions
    }
}

nonisolated struct MacLocalAgentEvent: Sendable {
    enum Kind: Sendable, Equatable {
        case toolStarted
        case toolFinished
        case planUpdated
        case permissionRequested
        case userInputRequested
        case status
        case finished
        case failed
        case cancelled
    }

    let kind: Kind
    let text: String
    let title: String?
    let detail: String?
    let isError: Bool
    let terminalReason: String?
    let eventID: String
    let toolCallID: String?
    let permissionRequest: MacRemotePermissionRequest?
    let userInputRequest: MacRemoteUserInputRequest?
    let coreTurnTerminal: MacCoreTurnTerminalEnvelope?
    let coreTaskUpdate: MacCoreTaskUpdateEnvelope?
    let coreSkillRuntimeUpdate: MacCoreSkillRuntimeUpdateEnvelope?
    let coreCapabilityActivation: MacCoreCapabilityActivationEnvelope?
    let corePermissionProjection: MacCorePermissionProjectionEnvelope?
    let assistantJournalCursor: MacAssistantJournalCursor?
    let assistantRunTerminalSealDigest: String?
    let contextUsedTokens: Int?
    let contextWindowTokens: Int?
    let contextPromptLimit: Int?
    let artifactURLs: [URL]
    let trace: MacAgentTraceEnvelope?
    private(set) var canonicalRunID: String? = nil
    private(set) var remoteSubmissionIntent: MacRemoteSubmissionIntent? = nil
    private(set) var validatedCoreSessionID: String? = nil
    private(set) var validatedCoreConversationID: String? = nil
    private(set) var validatedControlSessionID: String? = nil

    init(
        kind: Kind,
        text: String,
        title: String?,
        detail: String?,
        isError: Bool,
        terminalReason: String? = nil,
        eventID: String = UUID().uuidString,
        toolCallID: String? = nil,
        permissionRequest: MacRemotePermissionRequest? = nil,
        userInputRequest: MacRemoteUserInputRequest? = nil,
        coreTurnTerminal: MacCoreTurnTerminalEnvelope? = nil,
        coreTaskUpdate: MacCoreTaskUpdateEnvelope? = nil,
        coreSkillRuntimeUpdate: MacCoreSkillRuntimeUpdateEnvelope? = nil,
        coreCapabilityActivation: MacCoreCapabilityActivationEnvelope? = nil,
        corePermissionProjection: MacCorePermissionProjectionEnvelope? = nil,
        assistantJournalCursor: MacAssistantJournalCursor? = nil,
        assistantRunTerminalSealDigest: String? = nil,
        contextUsedTokens: Int? = nil,
        contextWindowTokens: Int? = nil,
        contextPromptLimit: Int? = nil,
        artifactURLs: [URL] = [],
        trace: MacAgentTraceEnvelope? = nil
    ) {
        self.kind = kind
        self.text = text
        self.title = title
        self.detail = detail
        self.isError = isError
        self.terminalReason = terminalReason
        self.eventID = eventID
        self.toolCallID = toolCallID
        self.permissionRequest = permissionRequest
        self.userInputRequest = userInputRequest
        self.coreTurnTerminal = coreTurnTerminal
        self.coreTaskUpdate = coreTaskUpdate
        self.coreSkillRuntimeUpdate = coreSkillRuntimeUpdate
        self.coreCapabilityActivation = coreCapabilityActivation
        self.corePermissionProjection = corePermissionProjection
        self.assistantJournalCursor = assistantJournalCursor
        self.assistantRunTerminalSealDigest = assistantRunTerminalSealDigest
        self.contextUsedTokens = contextUsedTokens
        self.contextWindowTokens = contextWindowTokens
        self.contextPromptLimit = contextPromptLimit
        self.artifactURLs = artifactURLs
        self.trace = trace
    }

    func bindingCanonicalRun(_ runID: String?) -> Self {
        var event = self
        event.canonicalRunID = runID
        return event
    }

    func bindingSubmissionIntent(_ intent: MacRemoteSubmissionIntent?) -> Self {
        var event = self
        event.remoteSubmissionIntent = intent
        return event
    }

    fileprivate func bindingCoreSession(conversationID: String?, coreSessionID: String?) -> Self {
        var event = self
        event.validatedCoreConversationID = conversationID
        event.validatedCoreSessionID = coreSessionID
        return event
    }

    fileprivate func bindingControlSession(_ sessionID: String?) -> Self {
        var event = self
        event.validatedControlSessionID = sessionID
        return event
    }

    func addingArtifactURLs(_ urls: [URL]) -> MacLocalAgentEvent {
        let merged = Array(NSOrderedSet(array: artifactURLs + urls))
            .compactMap { $0 as? URL }
        return MacLocalAgentEvent(
            kind: kind,
            text: text,
            title: title,
            detail: detail,
            isError: isError,
            terminalReason: terminalReason,
            eventID: eventID,
            toolCallID: toolCallID,
            permissionRequest: permissionRequest,
            userInputRequest: userInputRequest,
            coreTurnTerminal: coreTurnTerminal,
            coreTaskUpdate: coreTaskUpdate,
            coreSkillRuntimeUpdate: coreSkillRuntimeUpdate,
            coreCapabilityActivation: coreCapabilityActivation,
            corePermissionProjection: corePermissionProjection,
            assistantJournalCursor: assistantJournalCursor,
            assistantRunTerminalSealDigest: assistantRunTerminalSealDigest,
            contextUsedTokens: contextUsedTokens,
            contextWindowTokens: contextWindowTokens,
            contextPromptLimit: contextPromptLimit,
            artifactURLs: merged,
            trace: trace
        ).bindingCanonicalRun(canonicalRunID)
            .bindingSubmissionIntent(remoteSubmissionIntent)
            .bindingCoreSession(conversationID: validatedCoreConversationID,
                                coreSessionID: validatedCoreSessionID)
            .bindingControlSession(validatedControlSessionID)
    }
}

nonisolated struct MacCorePendingPermissionIdentity: Codable, Sendable, Equatable {
    let allowConversation: Bool
    let requestID: String
    let turnID: String
    let callID: String
    let capabilityID: String
    let resource: String
    let permissionDomain: String
    let rationale: String

    enum CodingKeys: String, CodingKey {
        case allowConversation = "allow_conversation"
        case resource, rationale
        case requestID = "request_id"
        case turnID = "turn_id"
        case callID = "call_id"
        case capabilityID = "capability_id"
        case permissionDomain = "permission_domain"
    }
}

nonisolated struct MacActivePermissionPolicy: Codable, Sendable, Equatable {
    let runID: String
    let configurationRevision: Int64
    let profile: MacPermissionMode

    enum CodingKeys: String, CodingKey {
        case runID = "run_id"
        case configurationRevision = "configuration_revision"
        case profile
    }
}

nonisolated struct MacCorePermissionProjection: Codable, Sendable, Equatable {
    let revision: UInt64
    let pending: [MacCorePendingPermissionIdentity]
    let activePolicy: MacActivePermissionPolicy?

    enum CodingKeys: String, CodingKey {
        case revision, pending
        case activePolicy = "active_policy"
    }
}

nonisolated struct MacCorePermissionProjectionEnvelope: Codable, Sendable, Equatable {
    let sessionID: String
    let turnID: String?
    let permissionProjectionRevision: UInt64
    let permissionProjection: MacCorePermissionProjection

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case turnID = "turn_id"
        case permissionProjectionRevision = "permission_projection_revision"
        case permissionProjection = "permission_projection"
    }
}

nonisolated private struct MacComputerBridgeRequest {
    let id: String
    let sessionID: String
    let toolName: String
    let arguments: [String: Any]
}

nonisolated enum MacAgentArtifactPayload {
    static func decode(_ object: [String: Any]) -> [URL] {
        guard (object["sensitive"] as? Bool) != true else { return [] }
        let meta = object["meta"] as? [String: Any] ?? [:]
        var paths: [String] = []
        for key in ["screenshot_path", "download_path", "artifact_path", "output_path"] {
            if let path = meta[key] as? String {
                paths.append(path)
            }
        }
        if let download = meta["download"] as? [String: Any],
           let path = download["path"] as? String {
            paths.append(path)
        }
        for key in ["artifact_paths", "output_paths", "generated_files"] {
            if let values = meta[key] as? [String] {
                paths.append(contentsOf: values)
            }
        }

        func collectArtifacts(_ value: Any?) {
            if let path = value as? String {
                paths.append(path)
                return
            }
            if let values = value as? [Any] {
                values.forEach(collectArtifacts)
                return
            }
            guard let artifact = value as? [String: Any] else { return }
            for key in ["path", "uri", "url", "artifact_path", "output_path"] {
                if let path = artifact[key] as? String {
                    paths.append(path)
                }
            }
        }

        // Agent Host protocol events wrap canonical tool output inside the
        // trace envelope.
        if let trace = object["trace"] as? [String: Any],
           let payload = trace["payload"] as? [String: Any] {
            collectArtifacts(payload["artifacts"])
            if let result = payload["result"] as? [String: Any] {
                collectArtifacts(result["artifacts"])
            }
        }

        var seen: Set<String> = []
        return paths.compactMap { rawPath in
            let raw = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
            let path = raw.hasPrefix("file://")
                ? (URL(string: raw)?.path ?? raw)
                : raw
            guard path.hasPrefix("/"),
                  !path.contains("\0"),
                  !path.contains("/../") else {
                return nil
            }
            let url = URL(fileURLWithPath: path).standardizedFileURL
            guard seen.insert(url.path).inserted else { return nil }
            return url
        }
    }
}

nonisolated struct MacHostSubmissionDecision: Decodable, Equatable, Sendable {
    enum Kind: String, Decodable, Sendable { case admitted = "submission_admitted", rejected = "submission_rejected" }
    let type: Kind
    let conversation_id: String
    let run_id: String
    let assistant_message_id: String
    let code: String?
    let message: String?

    static func decode(_ line: String, conversationID: String, runID: String,
                       assistantMessageID: String) throws -> Self? {
        let data = Data(line.utf8)
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let kind = object["type"] as? String,
              Kind(rawValue: kind) != nil else { return nil }
        let decision = try JSONDecoder().decode(Self.self, from: data)
        guard decision.conversation_id == conversationID, decision.run_id == runID,
              decision.assistant_message_id == assistantMessageID else {
            throw MacLocalAgentRuntimeError.protocolViolation("Host submission decision has a different owner.")
        }
        if decision.type == .rejected {
            guard let code = decision.code, !code.isEmpty,
                  let message = decision.message, !message.isEmpty else {
                throw MacLocalAgentRuntimeError.protocolViolation("Host submission rejection is missing its reason.")
            }
        }
        return decision
    }
}

/// A transport has yielded its execution slot at a committed question boundary.
/// This is neither a terminal result nor a disconnected/failed Run.
nonisolated struct MacLocalAgentWaitSuspension: Error, Sendable {
    let conversationID: String
    let runID: String
    let waits: [MacRemoteUserInputRequest]
    let readyToResume: Bool

    static func validated(conversationID: String, runID: String, activeRunID: String?,
                          openWaits: [MacRemoteUserInputRequest], observedWaitIDs: Set<String>) -> Self? {
        guard activeRunID == runID else { return nil }
        let ownedWaits = openWaits.filter { $0.conversationID == conversationID && $0.runID == runID && $0.isOpen }
        guard !ownedWaits.isEmpty || !observedWaitIDs.isEmpty else { return nil }
        return Self(conversationID: conversationID, runID: runID, waits: ownedWaits,
                    readyToResume: ownedWaits.isEmpty)
    }
}

nonisolated enum MacLocalAgentRuntimeError: LocalizedError {
    case executableUnavailable
    case alreadyRunning
    case launchFailed(String)
    case processFailed(Int32, String)
    case protocolViolation(String)
    case submissionRejected(code: String, message: String)

    var errorDescription: String? {
        switch self {
        case .executableUnavailable:
            return String(localized: "The local agent runtime is part of Nola for Mac. Rebuild the Mac target or configure NOLA_AGENT_HOST_PATH.")
        case .alreadyRunning:
            return String(localized: "This conversation already has an active local run.")
        case .launchFailed(let detail):
            return String(
                format: String(localized: "The local agent could not start: %@"),
                detail
            )
        case .processFailed(let code, let detail):
            let trimmed = detail.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty {
                return String(
                    format: String(localized: "The local agent exited with status %d."),
                    code
                )
            }
            return String(
                format: String(localized: "The local agent exited with status %d."),
                code
            ) + "\n" + String(trimmed.prefix(4_000))
        case .protocolViolation(let detail):
            return String(localized: "The Agent Host returned an invalid event stream.")
                + "\n" + detail
        case .submissionRejected(_, let message):
            return message
        }
    }
}

nonisolated enum MacAgentHostExecutableLocator {
    static func resolve(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        bundle: Bundle = .main,
        fileManager: FileManager = .default
    ) -> URL? {
        if let configured = environment["NOLA_AGENT_HOST_PATH"]?
            .trimmingCharacters(in: .whitespacesAndNewlines),
           !configured.isEmpty {
            let url = URL(fileURLWithPath: configured)
            if fileManager.isExecutableFile(atPath: url.path) {
                return url
            }
        }
        if let bundled = bundle.url(forAuxiliaryExecutable: "nola-agent-host"),
           fileManager.isExecutableFile(atPath: bundled.path) {
            return bundled
        }
        return nil
    }
}

nonisolated enum MacLocalExecutionBackendCatalog {
    static func fetch() async throws -> [MacRemoteExecutionBackend] {
        guard let executableURL = MacAgentHostExecutableLocator.resolve() else {
            throw MacLocalAgentRuntimeError.executableUnavailable
        }
        let process = Process()
        let output = Pipe()
        let errors = Pipe()
        process.executableURL = executableURL
          process.arguments = ["--list-execution-backends"]
          process.standardOutput = output
          process.standardError = errors
        let appEnvironment = MacAppEnvironment.shared
        let projectURL = appEnvironment.scratchWorkspaceURL
        var environment = appEnvironment.processEnvironment(projectURL: projectURL)
        for (key, value) in appEnvironment.launchAgentEnvironment(projectURL: projectURL) {
            environment[key] = value
          }
          process.environment = environment
          // Drain both pipes while the catalog process is running. Waiting for
          // termination before reading can deadlock once a backend (for
          // example OpenCode) reports enough models to fill the stdout pipe.
          let outputReader = Task.detached(priority: .utility) {
              output.fileHandleForReading.readDataToEndOfFile()
          }
          let errorReader = Task.detached(priority: .utility) {
              errors.fileHandleForReading.readDataToEndOfFile()
          }
          do {
              try await withCheckedThrowingContinuation { continuation in
                  process.terminationHandler = { _ in continuation.resume() }
                  do {
                      try process.run()
                      try? output.fileHandleForWriting.close()
                      try? errors.fileHandleForWriting.close()
                  } catch {
                      continuation.resume(throwing: error)
                  }
              }
          } catch {
              try? output.fileHandleForWriting.close()
              try? errors.fileHandleForWriting.close()
              _ = await outputReader.value
              _ = await errorReader.value
              throw error
          }
          let data = await outputReader.value
          let errorData = await errorReader.value
          guard process.terminationStatus == 0 else {
              let detail = String(
                  decoding: errorData.prefix(1_000),
                  as: UTF8.self
              )
            throw MacLocalAgentRuntimeError.launchFailed(detail)
        }
        return try JSONDecoder().decode([MacRemoteExecutionBackend].self, from: data)
    }
}

protocol MacLocalAgentRuntime: Actor {
    func prepareColdResume(conversationID: String, expectedRunID: String, assistantMessageID: String,
                           applicationPort: MacApplicationPort, authorization: MacLocalAgentResumeAuthorization,
                           appEnvironment: MacAppEnvironment) async throws
    func interruptOwnedTransport(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool
    func deliverCancellation(_ request: MacRunCancellationRequest, databaseURL: URL) async throws
    func releaseCompletedRunSnapshot(conversationID: String, expectedRunID: String, assistantMessageID: String) throws
    func canResume(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool
    func resume(
        conversationID: String, expectedRunID: String, assistantMessageID: String,
        revalidate: @escaping @MainActor @Sendable () async throws -> MacLocalAgentResumeAuthorization,
        transportReady: (@MainActor @Sendable () async -> Void)?
    ) -> AsyncThrowingStream<MacLocalAgentEvent, Error>
    func run(
        _ request: MacLocalAgentRunRequest
    ) -> AsyncThrowingStream<MacLocalAgentEvent, Error>
    func cancel(conversationID: String)
    func resolvePermission(
        conversationID: String,
        request: MacRemotePermissionRequest,
        allow: Bool,
        scope: MacPermissionGrantScope
    ) throws
    func providePermissionInstruction(conversationID: String, request: MacRemotePermissionRequest, instruction: String) throws
    func activePermissionRequest(
        conversationID: String,
        requestID: String
    ) -> MacRemotePermissionRequest?
    func listSessions() async throws -> [MacLocalAgentSessionSummary]
    func deleteSession(id: String) async throws
}

extension MacLocalAgentRuntime {
    func prepareColdResume(conversationID: String, expectedRunID: String, assistantMessageID: String,
                           applicationPort: MacApplicationPort, authorization: MacLocalAgentResumeAuthorization,
                           appEnvironment: MacAppEnvironment) async throws {
        throw MacLocalAgentResumeError.snapshotUnavailable
    }
    func interruptOwnedTransport(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool { false }
    func deliverCancellation(_ request: MacRunCancellationRequest, databaseURL: URL) async throws {
        throw MacApplicationPortError.serviceUnavailable
    }
    func releaseCompletedRunSnapshot(conversationID: String, expectedRunID: String, assistantMessageID: String) throws {}
    func canResume(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool { false }

    func resume(
        conversationID: String, expectedRunID: String, assistantMessageID: String,
        revalidate: @escaping @MainActor @Sendable () async throws -> MacLocalAgentResumeAuthorization,
        transportReady: (@MainActor @Sendable () async -> Void)?
    ) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { $0.finish(throwing: MacLocalAgentResumeError.snapshotUnavailable) }
    }
    func providePermissionInstruction(conversationID: String, request: MacRemotePermissionRequest, instruction: String) throws {
        throw MacPermissionInstructionError.unsupported
    }



    func activePermissionRequest(
        conversationID: String,
        requestID: String
    ) -> MacRemotePermissionRequest? {
        nil
    }

}

actor MacAgentHostRuntime: MacLocalAgentRuntime {
    static let shared = MacAgentHostRuntime()
    private let credentialStore: MacAppleChatConfigurationStore

    init(credentialStore: MacAppleChatConfigurationStore = .shared) {
        self.credentialStore = credentialStore
    }

    func prepareColdResume(conversationID: String, expectedRunID: String, assistantMessageID: String,
                           applicationPort: MacApplicationPort, authorization: MacLocalAgentResumeAuthorization,
                           appEnvironment: MacAppEnvironment) async throws {
        guard processes[conversationID] == nil, !startingConversations.contains(conversationID),
              let active = try applicationPort.activeRunIdentity(conversationID: conversationID),
              active.runID == expectedRunID, active.assistantMessageID == assistantMessageID,
              let exact = try applicationPort.loadExactRunInvocation(conversationID: conversationID,
                expectedRunID: expectedRunID, assistantMessageID: assistantMessageID),
              let coreSessionID = exact.coreSessionID,
              let binding = try JSONSerialization.jsonObject(with: JSONEncoder().encode(exact.hostInvocation)) as? [String: Any],
              let primary = binding["primary"] as? [String: Any],
              let generation = binding["generation"] as? [String: Any],
              let fallbackRoutes = binding["fallbacks"] as? [[String: Any]],
              let executable = locateExecutable() else { throw MacLocalAgentResumeError.snapshotUnavailable }
        let descriptor = try MacFrozenLaunchDescriptor.load(exact)
        try descriptor.validate(authorization)
        guard let mode = MacPermissionMode(rawValue: descriptor.permission_mode),
              fallbackRoutes.count == descriptor.fallback_profile_ids.count else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
        func resolve(_ route: [String: Any], profileID: String) throws -> (MacRunModelCredential, String) {
            let (credential, secret) = try credentialStore.resolveColdRunCredential(profileID: profileID)
            guard route["kind"] as? String == "http",
                  route["model"] as? String == credential.model,
                  route["protocol"] as? String == credential.wireProtocol.rawValue,
                  route["credential_sha256"] as? String == credential.secretDigest,
                  var components = URLComponents(string: credential.endpoint) else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            let query = components.percentEncodedQuery
            components.query = nil
            components.fragment = nil
            guard components.user == nil, components.password == nil else { throw MacLocalAgentResumeError.credentialUnavailable }
            if components.path.isEmpty { components.path = "/" }
            guard components.url?.absoluteString == route["endpoint"] as? String,
                  query.map(MacRunModelCredential.digest) == route["endpoint_query_sha256"] as? String else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            return (credential, secret)
        }
        let external = try MacFrozenExternalExecutorIdentity.resolve(primary: primary,
            modelProfileID: descriptor.model_profile_id, fallbackCount: fallbackRoutes.count)
        let primaryProfile = descriptor.model_profile_id
        let siwc: MacSiwcProfileSelection?
        if primary["protocol"] as? String == MacModelWireProtocol.siwcResponses.rawValue {
            guard let primaryProfile, let revision = descriptor.model_profile_revision, revision > 0,
                  primary["siwc_account"] is [String: Any], fallbackRoutes.isEmpty else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            siwc = MacSiwcProfileSelection(profileID: primaryProfile, revision: revision)
            try credentialStore.validateSiwcProfile(siwc!)
        } else { siwc = nil }
        let localEndpoint: MacMLXOpenAIService.Endpoint?
        let first: (MacRunModelCredential, String)?
        var credentials: [MacRunModelCredential] = []
        if external != nil || siwc != nil {
            localEndpoint = nil
            first = nil
        } else if primary["kind"] as? String == "native" {
            guard let primaryProfile, primary["adapter"] as? String == "mlx",
                  primary["adapter_contract_revision"] as? Int == 1,
                  primary["model_id"] as? String == primaryProfile,
                  let reference = primary["profile_ref"] as? [String: Any],
                  reference["profile_id"] as? String == primaryProfile,
                  let revision = reference["admitted_revision"] as? Int64,
                  let profile = MacLocalModelProfile.resolve(primaryProfile) else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            let endpoint = try await MacMLXOpenAIService.shared.start(profile,
                expectedRevision: revision, applicationPort: applicationPort)
            localEndpoint = endpoint
            first = (.init(profileID: primaryProfile, endpointID: "", account: "",
                endpoint: endpoint.baseURL, model: endpoint.modelID, wireProtocol: .chatCompletions,
                secretDigest: MacRunModelCredential.digest(endpoint.apiKey)), endpoint.apiKey)
        } else {
            localEndpoint = nil
            guard let primaryProfile else { throw MacLocalAgentResumeError.credentialUnavailable }
            let resolved = try resolve(primary, profileID: primaryProfile)
            first = resolved
            credentials.append(resolved.0)
        }
        var fallbacks: [[String: Any]] = []
        for (route, profile) in zip(fallbackRoutes, descriptor.fallback_profile_ids) {
            let resolved = try resolve(route, profileID: profile)
            credentials.append(resolved.0)
            fallbacks.append(["endpoint": resolved.0.endpoint, "model": resolved.0.model, "api_key": resolved.1])
        }
        let attachmentObjects = try JSONSerialization.jsonObject(with: JSONEncoder().encode(descriptor.host_attachments)) as? [[String: Any]] ?? []
        let attachments = try attachmentObjects.map { value -> URL in
            guard let path = value["path"] as? String, path.hasPrefix("/") else { throw MacLocalAgentResumeError.invalidFrame }
            return URL(fileURLWithPath: path)
        }
        var request = try MacLocalAgentRunRequest(conversationID: conversationID,
            submissionID: expectedRunID, assistantMessageID: assistantMessageID, prompt: "",
            projectURL: authorization.projectURL, attachmentURLs: attachments,
            authorizedWorkspaceURLs: authorization.authorizedWorkspaceURLs,
            modelID: external?.model ?? first?.0.model ?? (siwc == nil ? nil : primary["model"] as? String), modelProfileID: primaryProfile,
            modelProfileRevision: descriptor.model_profile_revision,
            siwcProfile: siwc, localModelEndpoint: localEndpoint,
            modelProviderID: external?.provider, endpoint: first?.0.endpoint ?? (siwc == nil ? nil : primary["endpoint"] as? String), apiKey: first?.1,
            contextWindowTokens: descriptor.context_window_tokens, maxModelIterations: descriptor.max_iterations,
            wireProtocol: siwc == nil ? first?.0.wireProtocol : .siwcResponses, executionBackend: external?.backend ?? "nola",
            inputOrigin: descriptor.input_origin, memoryEnabled: descriptor.memory_enabled,
            permissionMode: mode, databaseURLOverride: applicationPort.databaseURL,
            appEnvironment: appEnvironment, applicationPort: applicationPort)
        if descriptor.input_origin == "scheduled_action" {
            // Skill catalog directories are read-only discovery roots, not
            // additional automation workspace grants.
            request.allowedDirectoryArguments = ["--allow-dir", request.projectURL.standardizedFileURL.path]
        }
        guard request.activeSkillDescriptors.map({ $0.id.rawValue }) == descriptor.active_skill_ids,
              request.allowedDirectoryArguments == descriptor.allowed_directory_arguments,
              request.skillDirectoryArguments == descriptor.skill_directory_arguments else {
            throw MacLocalAgentResumeError.authorizationChanged
        }
        request.allowedDirectoryArguments = descriptor.allowed_directory_arguments
        request.skillDirectoryArguments = descriptor.skill_directory_arguments
        let rules = try applicationPort.effectivePermissionRules(conversationID: conversationID, hostID: "mac-local")
        let currentRules = try JSONDecoder().decode([MacCanonicalJSONValue].self, from: JSONEncoder().encode(rules.rules))
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: descriptor.permission_rules, current: currentRules)
        var configuration: [String: Any] = [
            "submission_id": expectedRunID, "assistant_message_id": assistantMessageID,
            "execution_backend": external?.backend ?? "nola", "permission_profile": mode.permissionProfile,
            "llm_fallbacks": fallbacks, "max_iterations": descriptor.max_iterations,
            "model_capabilities": descriptor.model_capabilities, "vision_input": descriptor.vision_input,
            "permission_rules": try JSONSerialization.jsonObject(with: JSONEncoder().encode(descriptor.permission_rules)),
            "host_attachments": try JSONSerialization.jsonObject(with: JSONEncoder().encode(descriptor.host_attachments)),
            "mac_launch": binding["mac_launch"]!,
            "frozen_output_admission": binding["output_admission"]!,
        ]
        if let first {
            configuration["model_protocol"] = first.0.wireProtocol.rawValue
            configuration["api_key"] = first.1
        }
        if let siwc {
            configuration["model_protocol"] = MacModelWireProtocol.siwcResponses.rawValue
            configuration["model_profile_id"] = siwc.profileID
            configuration["expected_profile_revision"] = siwc.revision
        }
        if let external {
            configuration["execution_model"] = external.model
            configuration["execution_model_provider"] = external.provider
        }
        if let localEndpoint {
            configuration["native_model_route"] = try localEndpoint.nativeRoute()
        }
        for key in ["max_output_tokens", "temperature", "top_p", "sampling_override", "generation_options", "reasoning_selection", "reasoning_tier",
                    "max_retries", "parallel_tool_calls", "thinking_enabled", "reasoning_summary_supported", "prompt_cache", "prompt_cache_retention"] {
            configuration[key] = generation[key]
        }
        // Cache key preimages are deliberately not persisted. A Run using one
        // needs an independently authorized resolver; never substitute a new key.
        guard generation["prompt_cache_key_sha256"] is NSNull,
              generation["prompt_cache_key_prefix_sha256"] is NSNull else {
            throw MacLocalAgentResumeError.snapshotUnavailable
        }
        configuration["context_window_tokens"] = descriptor.context_window_tokens
        guard let outputAdmission = binding["output_admission"] as? [String: Any] else {
            throw MacLocalAgentResumeError.invalidFrame
        }
        if outputAdmission["preference"] as? String == "default" {
            configuration.removeValue(forKey: "max_output_tokens")
        }
        if let first, first.0.wireProtocol != .responses {
            configuration["chat_output_token_field"] = primary["output_token_field"]
        }
        if let mcp = request.activeMCPServerConfigurationData {
            configuration["mcp_servers"] = try JSONSerialization.jsonObject(with: mcp)
        }
        let toolPlan = try MacCoreToolPolicy.localPlan(executableURL: executable,
            projectURL: request.projectURL, sessionId: coreSessionID)
        guard let receipt = toolPlan.receiptBase64 else { throw MacLocalAgentResumeError.invalidFrame }
        var environment = runtimeEnvironment(projectURL: request.projectURL, permissionMode: mode, appEnvironment: appEnvironment)
        environment["NOLA_CORE_TOOL_POLICY_RECEIPT"] = receipt
        environment["NOLA_CORE_TOOL_POLICY_DIGEST"] = toolPlan.output.policyDigest
        environment["NOLA_CORE_TOOL_MANIFEST_DIGEST"] = toolPlan.output.manifestDigest
        environment["NOLA_MAC_NATIVE_COMPUTER_BRIDGE"] = "1"
        environment["NOLA_LONG_TERM_MEMORY_ENABLED"] = descriptor.memory_enabled ? "1" : "0"
        environment["NOLA_DB_PATH"] = applicationPort.databaseURL.path
        let manifest: [String: Any] = ["schema_version": 1, "memory_enabled": descriptor.memory_enabled,
            "skills_enabled": !request.activeSkillDescriptors.isEmpty || !request.activeExternalSkillDirectories.isEmpty,
            "enabled_skill_ids": descriptor.active_skill_ids,
            "enabled_external_skill_paths": request.activeExternalSkillDirectories.map(\.path),
            "native_computer_enabled": true, "native_browser_enabled": true, "managed_browser_enabled": true]
        environment["NOLA_HOST_CAPABILITY_MANIFEST"] = String(decoding: try JSONSerialization.data(withJSONObject: manifest, options: [.sortedKeys]), as: UTF8.self)
        // No claim, effect dispatch, or new submission occurs here. The existing
        // resume admission still checks the complete Host policy hash before takeover.
        guard let stillActive = try applicationPort.activeRunIdentity(conversationID: conversationID),
              stillActive.runID == expectedRunID, stillActive.assistantMessageID == assistantMessageID else {
            throw MacLocalAgentResumeError.snapshotUnavailable
        }
        ownedLaunches[conversationID] = MacLocalAgentOwnedLaunchSnapshot(request: request,
            executableURL: executable, arguments: arguments(for: request, localEndpoint: localEndpoint),
            environment: environment, initialRequestData: try JSONSerialization.data(withJSONObject: configuration, options: [.sortedKeys]),
            toolPlan: toolPlan, credentialSelection: .init(credentials: credentials, recoverable: true,
                nativeEndpoint: localEndpoint, applicationPort: localEndpoint == nil ? nil : applicationPort, siwcProfile: siwc))
    }

    private var processes: [String: Process] = [:]
    private var ownedLaunches: [String: MacLocalAgentOwnedLaunchSnapshot] = [:]
    private var startingConversations: Set<String> = []
    private var processSessions: [String: MacLocalAgentProcessSession] = [:]
    private var controlInputs: [String: FileHandle] = [:]
    private var toolPlans: [String: MacCoreToolPlanReceipt] = [:]
    private var projectURLs: [String: URL] = [:]
    private var executableURLs: [String: URL] = [:]
    // A turn may have more than one suspended tool call. Conversation-only
    // storage lets a later challenge overwrite an earlier one, leaving the
    // visible permission card impossible to resolve. Request identity is part
    // of the Core contract and must remain the second key.
    private var pendingPermissionRequests: [String: [String: MacRemotePermissionRequest]] = [:]
    private var admittedCoreSessions: [String: (runID: String, sessionID: String, controlSessionID: String)] = [:]
    private var cancellationRequestedConversationIDs: Set<String> = []
    private var continuations: [
        String: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation
    ] = [:]

    func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        return AsyncThrowingStream { continuation in
            Task {
                do {
                    try await self.start(request, continuation: continuation)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    func interruptOwnedTransport(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool {
        guard processes[conversationID] != nil, let launch = ownedLaunches[conversationID],
              launch.request.submissionID == expectedRunID,
              launch.request.assistantMessageID == assistantMessageID else { return false }
        cancel(conversationID: conversationID)
        return true
    }

    func deliverCancellation(_ request: MacRunCancellationRequest, databaseURL: URL) async throws {
        guard !startingConversations.contains(request.conversationID) else {
            throw MacLocalAgentRuntimeError.alreadyRunning
        }
        if processes[request.conversationID] != nil {
            guard let launch = ownedLaunches[request.conversationID],
                  launch.request.submissionID == request.expectedRunID,
                  launch.request.assistantMessageID == request.assistantMessageID else {
                throw MacRunCancellationContractError.identityMismatch
            }
            cancel(conversationID: request.conversationID)
            return
        }
        guard let executable = locateExecutable() else {
            throw MacLocalAgentRuntimeError.executableUnavailable
        }
        startingConversations.insert(request.conversationID)
        defer { startingConversations.remove(request.conversationID) }
        try await MacCancellationDeliveryProcess.run(executable: executable,
            databaseURL: databaseURL, request: request)
    }

    func canResume(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool {
        guard processes[conversationID] == nil, !startingConversations.contains(conversationID),
              let snapshot = ownedLaunches[conversationID] else { return false }
        return snapshot.request.submissionID == expectedRunID
            && snapshot.request.assistantMessageID == assistantMessageID
            && !snapshot.request.modelTestMode
    }

    /// Read-only process ownership for diagnostics and transport fault testing.
    func ownedProcessIdentifier(conversationID: String, expectedRunID: String) -> Int32? {
        guard ownedLaunches[conversationID]?.request.submissionID == expectedRunID,
              let process = processes[conversationID], process.isRunning else { return nil }
        return process.processIdentifier
    }

    func releaseCompletedRunSnapshot(conversationID: String, expectedRunID: String, assistantMessageID: String) throws {
        guard let snapshot = ownedLaunches[conversationID],
              snapshot.request.submissionID == expectedRunID,
              snapshot.request.assistantMessageID == assistantMessageID else { return }
        guard let port = snapshot.request.applicationPort else {
            throw MacApplicationPortError.serviceUnavailable
        }
        guard let terminal = try port.canonicalRunTerminal(conversationID: conversationID, runID: expectedRunID),
              terminal.conversationID == conversationID, terminal.runID == expectedRunID,
              terminal.assistantMessageID == assistantMessageID else { return }
        // No suspension between the exact canonical read and owner-matched
        // retirement. A stale completion cannot clear a newer launch.
        ownedLaunches.removeValue(forKey: conversationID)
    }

    func resume(
        conversationID: String, expectedRunID: String, assistantMessageID: String,
        revalidate: @escaping @MainActor @Sendable () async throws -> MacLocalAgentResumeAuthorization,
        transportReady: (@MainActor @Sendable () async -> Void)? = nil
    ) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    guard self.canResume(conversationID: conversationID, expectedRunID: expectedRunID,
                                         assistantMessageID: assistantMessageID),
                          let snapshot = self.ownedLaunches[conversationID] else {
                        throw MacLocalAgentResumeError.snapshotUnavailable
                    }
                    try await self.start(snapshot.request, continuation: continuation,
                        resumeSnapshot: snapshot, revalidate: revalidate, transportReady: transportReady)
                } catch { continuation.finish(throwing: error) }
            }
        }
    }

    func cancel(conversationID: String) {
        guard let process = processes[conversationID] else { return }
        if process.isRunning,
           cancellationRequestedConversationIDs.insert(conversationID).inserted {
            process.interrupt()
            // Keep stdout and the continuation alive until Core publishes its
            // canonical TurnCancelled event. A local synthetic terminal races
            // the final journal commit and leaves persisted and visible state
            // disagreeing after reload.
            Task {
                try? await Task.sleep(for: .seconds(5))
                guard self.processes[conversationID] === process,
                      process.isRunning else { return }
                Darwin.kill(process.processIdentifier, SIGKILL)
            }
        }
    }


    private func recoverTaskGuidance(conversationID: String) async throws {
        guard let launch = ownedLaunches[conversationID],
              let admitted = admittedCoreSessions[conversationID],
              let port = launch.request.applicationPort,
              try launchOwnsRun(launch, conversationID: conversationID, runID: admitted.runID) else {
            throw AppleTaskGuidanceOutbox.Failure.invalidOwner
        }
        try await AppleTaskGuidanceOutbox.shared.recover(
            hostId: "local-store:" + SHA256.hash(data: Data(port.databaseURL.standardizedFileURL.path.utf8))
                .map { String(format: "%02x", $0) }.joined(), conversationId: conversationID,
            parentRunId: admitted.runID
        ) { path, payload in
            try port.taskGuidanceTransport(path: path, payload: payload,
                conversationID: conversationID, parentRunID: admitted.runID,
                coreSessionID: admitted.sessionID)
        }
    }


    func sendGoalControl(conversationID: String, control: AppleGoalControl) throws -> Bool {
        try control.validate()
        guard let process = processes[conversationID], process.isRunning,
              let input = controlInputs[conversationID] else { return false }
        let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(control))
        var frame = try JSONSerialization.data(withJSONObject: [
            "type": "goal_control", "session_id": conversationID, "control": value,
        ], options: [.sortedKeys])
        frame.append(0x0A)
        try input.write(contentsOf: frame)
        return true
    }


    private func writeControlFrame(_ payload: [String: Any], to input: FileHandle) throws {
        var data = try JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
        data.append(0x0A)
        try input.write(contentsOf: data)
    }

    func activePermissionRequest(
        conversationID: String,
        requestID: String
    ) -> MacRemotePermissionRequest? {
        guard let process = processes[conversationID], process.isRunning,
              controlInputs[conversationID] != nil,
              toolPlans[conversationID] != nil,
              let request = pendingPermissionRequests[conversationID]?[requestID],
              request.sessionID == admittedControlSessionID(conversationID: conversationID) else {
            return nil
        }
        return request
    }

    func resolvePermission(
        conversationID: String,
        request: MacRemotePermissionRequest,
        allow: Bool,
        scope: MacPermissionGrantScope = .once
    ) throws {
        try sendPermissionDecision(conversationID: conversationID, request: request, decision: request.decision(allow: allow, scope: scope))
    }

    func providePermissionInstruction(conversationID: String, request: MacRemotePermissionRequest, instruction: String) throws {
        try sendPermissionDecision(conversationID: conversationID, request: request, decision: request.instructionDecision(instruction))
    }

    private func sendPermissionDecision(conversationID: String, request: MacRemotePermissionRequest, decision: MacRemotePermissionDecision) throws {
        guard request.sessionID == admittedControlSessionID(conversationID: conversationID) else {
            throw NSError(
                domain: "NolaMac.AgentHost.Permission",
                code: 1,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        String(localized: "The permission request belongs to a different conversation.")
                ]
            )
        }
        guard let pendingRequest = pendingPermissionRequests[conversationID]?[request.id],
              pendingRequest == request else {
            throw NSError(
                domain: "NolaMac.AgentHost.Permission",
                code: 5,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        String(localized: "The permission request is no longer pending.")
                ]
            )
        }
        guard let input = controlInputs[conversationID],
              let toolPlan = toolPlans[conversationID],
              let projectURL = projectURLs[conversationID],
              let executableURL = executableURLs[conversationID] else {
            throw NSError(
                domain: "NolaMac.AgentHost.Permission",
                code: 2,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        String(localized: "The agent permission input channel is no longer active.")
                ]
            )
        }
        guard toolPlan.matchesPermissionChallenge(
            toolId: request.toolName,
            hostId: request.hostID,
            clientId: request.clientID,
            sessionId: request.sessionID,
            policyDigest: request.policyDigest,
            manifestDigest: request.manifestDigest,
            mcpDescriptorData: request.mcpDescriptorData
        ) else {
            throw NSError(
                domain: "NolaMac.AgentHost.Permission",
                code: 3,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        String(localized: "The permission request does not match the active tool plan.")
                ]
            )
        }
        let freshPlan = try MacCoreToolPolicy.localPlan(
            executableURL: executableURL,
            projectURL: projectURL,
            sessionId: conversationID
        )
        guard freshPlan.sourceManifestId == toolPlan.sourceManifestId,
              freshPlan.matchesPermissionChallenge(
                toolId: request.toolName,
                hostId: request.hostID,
                clientId: request.clientID,
                sessionId: request.sessionID,
                policyDigest: freshPlan.output.policyDigest,
                manifestDigest: freshPlan.output.manifestDigest,
                mcpDescriptorData: request.mcpDescriptorData
              ) else {
            if case .authorization = decision.resolution {
                cancel(conversationID: conversationID)
            }
            throw NSError(
                domain: "NolaMac.AgentHost.Permission",
                code: 4,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        String(localized: "The local agent tool manifest changed. Please retry the request.")
                ]
            )
        }
        if request.mcpDescriptorData != nil {
            guard let launch = ownedLaunches[conversationID],
                  let captured = launch.request.activeMCPServerConfigurationData,
                  captured == MacLocalMCPActivationStore.activeServerConfigurationData(
                    workspaceURL: projectURL, store: launch.request.applicationPort) else {
                throw MacCoreToolPolicyError.stalePlan
            }
        }
        let payload = decision.payload
        try writeControlFrame(payload, to: input)
        pendingPermissionRequests[conversationID]?.removeValue(forKey: request.id)
        if pendingPermissionRequests[conversationID]?.isEmpty == true {
            pendingPermissionRequests.removeValue(forKey: conversationID)
        }
    }

    func listSessions() async throws -> [MacLocalAgentSessionSummary] {
        let data = try await runCapture(arguments: ["sessions", "--json"])
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let sessions = root["sessions"] as? [[String: Any]] else {
            return []
        }
        return sessions.compactMap { item in
            guard let id = item["id"] as? String, !id.isEmpty else { return nil }
            let title = item["title"] as? String ?? ""
            let updated = (item["updated_at"] as? String).flatMap(Self.parseDate) ?? .distantPast
            let count = item["message_count"] as? Int
                ?? (item["messages"] as? Int)
                ?? 0
            return MacLocalAgentSessionSummary(
                id: id,
                title: title,
                updatedAt: updated,
                messageCount: count
            )
        }
    }

    func deleteSession(id: String) async throws {
        guard processes[id] == nil, startingConversations.insert(id).inserted else {
            throw MacLocalAgentRuntimeError.alreadyRunning
        }
        defer { startingConversations.remove(id) }
        _ = try await runCapture(arguments: ["session", "delete", id, "--json"],
                                 ownedLaunch: ownedLaunches[id])
        ownedLaunches.removeValue(forKey: id)
    }

    private func start(
        _ request: MacLocalAgentRunRequest,
        continuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation,
        resumeSnapshot: MacLocalAgentOwnedLaunchSnapshot? = nil,
        revalidate: (@MainActor @Sendable () async throws -> MacLocalAgentResumeAuthorization)? = nil,
        transportReady: (@MainActor @Sendable () async -> Void)? = nil
    ) async throws {
        let runStartedAt = ContinuousClock.now
        let ticket = resumeSnapshot == nil ? request.submissionTicket : nil
        if let ticket {
            guard ticket.identity.conversationID == request.conversationID,
                  ticket.identity.submissionID == request.submissionID else {
                throw MacRunSubmissionTicket.SubmissionError.identityMismatch
            }
        }
        try ticket?.checkPreparation()
        func milliseconds(from start: ContinuousClock.Instant, to end: ContinuousClock.Instant) -> String {
            let components = start.duration(to: end).components
            return String(
                components.seconds * 1_000
                    + components.attoseconds / 1_000_000_000_000_000
            )
        }
        var didRecoverTaskGuidance = false
        var didLogFirstProtocolEvent = false
        var didLogFirstAssistantDelta = false
        guard processes[request.conversationID] == nil,
              startingConversations.insert(request.conversationID).inserted else {
            throw MacLocalAgentRuntimeError.alreadyRunning
        }
        defer { startingConversations.remove(request.conversationID) }
        guard let executableURL = resumeSnapshot?.executableURL ?? locateExecutable() else {
            throw MacLocalAgentRuntimeError.executableUnavailable
        }
        let executableReadyAt = ContinuousClock.now
        let initialDeliverables = Self.deliverableSnapshot(at: request.projectURL)
        let deliverablesReadyAt = ContinuousClock.now
        let toolPlan = try resumeSnapshot?.toolPlan ?? MacCoreToolPolicy.localPlan(
            executableURL: executableURL,
            projectURL: request.projectURL,
            sessionId: request.conversationID
        )
        let toolPlanReadyAt = ContinuousClock.now
        let chromeConfigurationURL = request.appEnvironment.supportRoot
            .appendingPathComponent("Chrome", isDirectory: true)
            .appendingPathComponent("native_browser_host.json")
        let chromePort = MacChromeMessagingConfiguration.runtimePort()
        let chromeConfiguration = try resumeSnapshot == nil ? MacChromeMessagingService.shared.ensureStarted(
            configuration: .loadOrCreate(
                fileURL: chromeConfigurationURL,
                port: chromePort
            )
        ) : nil
        let chromeReadyAt = ContinuousClock.now
        guard let receipt = toolPlan.receiptBase64 else {
            throw MacLocalAgentRuntimeError.launchFailed(
                String(localized: "Unable to encode the Core tool policy receipt.")
            )
        }

        let process = Process()
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        let inputPipe = Pipe()
        process.executableURL = executableURL
        process.currentDirectoryURL = request.projectURL
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        process.standardInput = inputPipe
        if resumeSnapshot == nil { try MacBuiltInSkillRegistry.installTrustedRouter(
            configurationDirectory: request.appEnvironment.configDirectory,
            activeSkills: request.activeSkillDescriptors
        ) }
        let skillRouterReadyAt = ContinuousClock.now
        // Resolve the running local instance once for both wire surfaces. An
        // unavailable local model must fail, not silently become a cloud route.
        let localEndpoint = try resumeSnapshot == nil ? resolvedLocalEndpoint(for: request) : nil
        let credentialSelection = try resumeSnapshot?.credentialSelection
            ?? MacLocalAgentCredentialGate.capture(request, localEndpoint: localEndpoint,
                store: credentialStore)
        process.arguments = resumeSnapshot?.arguments ?? arguments(for: request, localEndpoint: localEndpoint)
        let initialRequestData: Data
        if let resumeSnapshot {
            initialRequestData = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(
                original: resumeSnapshot.initialRequestData, conversationID: request.conversationID,
                runID: request.submissionID, assistantMessageID: request.assistantMessageID)
        } else {
            initialRequestData = try self.initialRequestData(for: request, localEndpoint: localEndpoint)
        }
        let requestReadyAt = ContinuousClock.now

        var environment = resumeSnapshot?.environment ?? runtimeEnvironment(
            projectURL: request.projectURL,
            permissionMode: request.permissionMode,
            appEnvironment: request.appEnvironment
        )
        if let chromeConfiguration {
            chromeConfiguration.apply(to: &environment)
            MacPlaywrightRuntimeService.shared.apply(to: &environment, broker: chromeConfiguration)
        }
        let browserRuntimeReadyAt = ContinuousClock.now
        if resumeSnapshot == nil {
            environment["NOLA_CORE_TOOL_POLICY_RECEIPT"] = receipt
            environment["NOLA_CORE_TOOL_POLICY_DIGEST"] = toolPlan.output.policyDigest
            environment["NOLA_CORE_TOOL_MANIFEST_DIGEST"] = toolPlan.output.manifestDigest
            environment["NOLA_LONG_TERM_MEMORY_ENABLED"] = request.memoryEnabled ? "1" : "0"
            environment["NOLA_MAC_NATIVE_COMPUTER_BRIDGE"] = "1"
            // Deterministic mock tests use short deadlines. Real-provider
            // acceptance must exercise production timing, including cold prefill.
            if MacProcessRuntimePolicy.isRunningTests,
               ProcessInfo.processInfo.environment["NOLA_REAL_LLM_E2E"] != "1" {
                environment["NOLA_DEVICE_TOOL_TIMEOUT_SECONDS"] = "5"
                environment["NOLA_LLM_HTTP_TIMEOUT_SEC"] = "15"
                environment["NOLA_LLM_STREAM_IDLE_TIMEOUT_SEC"] = "5"
                environment["NOLA_LLM_STREAM_FIRST_EVENT_TIMEOUT_SEC"] = "5"
                environment["NOLA_LLM_STREAM_TAIL_IDLE_TIMEOUT_SEC"] = "5"
            }
            environment["NOLA_MODELS_ROOT"] = FileManager.default.urls(
                for: .cachesDirectory,
                in: .userDomainMask
            )[0].appendingPathComponent("models", isDirectory: true).path
            let capabilityManifest: [String: Any] = [
                "schema_version": 1,
                "memory_enabled": request.memoryEnabled,
                "skills_enabled": !request.activeSkillDescriptors.isEmpty
                    || !request.activeExternalSkillDirectories.isEmpty,
                "enabled_skill_ids": request.activeSkillDescriptors.map { $0.id.rawValue },
                "enabled_external_skill_paths": request.activeExternalSkillDirectories.map(\.path),
                "native_computer_enabled": true,
                "native_browser_enabled": true,
                "managed_browser_enabled": true,
            ]
            let capabilityData = try JSONSerialization.data(
                withJSONObject: capabilityManifest,
                options: [.sortedKeys]
            )
            guard let capabilityJSON = String(data: capabilityData, encoding: .utf8) else {
                throw MacLocalAgentRuntimeError.launchFailed(
                    String(localized: "Unable to encode the Host capability manifest.")
                )
            }
            environment["NOLA_HOST_CAPABILITY_MANIFEST"] = capabilityJSON
            if let databaseURLOverride = request.databaseURLOverride {
                environment["NOLA_DB_PATH"] = databaseURLOverride.path
            }
        }
        process.environment = environment

        if let resumeSnapshot, let revalidate {
            try resumeSnapshot.validate(try await revalidate())
            guard let port = request.applicationPort,
                  let exact = try port.loadExactRunInvocation(conversationID: request.conversationID,
                    expectedRunID: request.submissionID, assistantMessageID: request.assistantMessageID) else {
                throw MacLocalAgentResumeError.snapshotUnavailable
            }
            let descriptor = try MacFrozenLaunchDescriptor.load(exact)
            let rules = try port.effectivePermissionRules(conversationID: request.conversationID, hostID: "mac-local")
            let currentRules = try JSONDecoder().decode([MacCanonicalJSONValue].self, from: JSONEncoder().encode(rules.rules))
            try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: descriptor.permission_rules, current: currentRules)
        }
        try Task.checkCancellation()

        let processSession = MacLocalAgentProcessSession(process: process, inputPipe: inputPipe,
            outputPipe: outputPipe, errorPipe: errorPipe)
        do {
            try MacLocalAgentCredentialGate.launch(selection: credentialSelection,
                isResume: resumeSnapshot != nil, store: credentialStore) {
                try processSession.launch(ticket: ticket)
            }
        } catch {
            throw MacLocalAgentRuntimeError.launchFailed(error.localizedDescription)
        }
        // Register the owned launch before any post-launch throwing operation
        // or actor suspension. A failed initial write must remain cancellable
        // and must pass through the same cleanup as a later transport failure.
        processSessions[request.conversationID] = processSession
        processes[request.conversationID] = process
        if resumeSnapshot == nil {
            // A retained recovery snapshot must not retain the completed UI
            // submission generation or its callback/Workspace ownership.
            var retainedRequest = request
            retainedRequest.submissionTicket = nil
            retainedRequest.submissionAccepted = nil
            ownedLaunches[request.conversationID] = MacLocalAgentOwnedLaunchSnapshot(
                request: retainedRequest, executableURL: executableURL, arguments: process.arguments ?? [],
                environment: environment, initialRequestData: initialRequestData, toolPlan: toolPlan,
                credentialSelection: credentialSelection)
        }
        continuations[request.conversationID] = continuation
        controlInputs[request.conversationID] = inputPipe.fileHandleForWriting
        toolPlans[request.conversationID] = toolPlan
        projectURLs[request.conversationID] = request.projectURL
        executableURLs[request.conversationID] = executableURL
        let outputHandle = processSession.output
        let errorHandle = processSession.error
        let stderrTask = Task.detached {
            var retainedLines: [String] = []
            var retainedBytes = 0
            do {
                for try await line in Self.lineStream(
                    from: errorHandle,
                    processID: process.processIdentifier
                ) {
                    await Logger.shared.log(
                        line,
                        level: .debug,
                        category: "MacAgentHostProcess",
                        metadata: ["conversation": String(request.conversationID.prefix(12))]
                    )
                    let lineBytes = line.utf8.count + 1
                    if retainedBytes + lineBytes <= 64 * 1_024 {
                        retainedLines.append(line)
                        retainedBytes += lineBytes
                    }
                }
            } catch {}
            return retainedLines.joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }

        var currentNativeEndpoint = credentialSelection.nativeEndpoint
        var observedGoalRuns = Set<String>()
        var currentRunID = request.submissionID
        var currentAssistantMessageID = request.assistantMessageID
        var receivedTerminalEvent = false
        var observedUserInputWaitIDs = Set<String>()
        var submissionAdmitted = resumeSnapshot != nil
        var admittedCoreSessionID: String?
        var transportGate = MacAgentHostTransportGate(
            runID: request.submissionID,
            assistantMessageID: request.assistantMessageID
        )
        var undecodedOutput: [String] = []
        // Start the independent pipe reader before writing the request. A Host
        // may emit output while consuming a large stdin frame; waiting until
        // after write would let both pipe buffers fill and deadlock each other.
        let outputLines = Self.lineStream(from: outputHandle, processID: process.processIdentifier)
        do {
            try processSession.writeInitialRequest(initialRequestData)
            let processReadyAt = ContinuousClock.now
            if resumeSnapshot != nil { await transportReady?() }
            for try await line in outputLines {
                guard !Task.isCancelled else { break }
                if request.goalExecutionID == nil, let decision = try MacHostSubmissionDecision.decode(line,
                    conversationID: request.conversationID, runID: request.submissionID,
                    assistantMessageID: request.assistantMessageID) {
                    guard resumeSnapshot == nil else {
                        throw MacLocalAgentRuntimeError.protocolViolation("A resume cannot decide a new submission.")
                    }
                    switch decision.type {
                    case .admitted:
                        if !submissionAdmitted {
                            guard ticket?.markAccepted() != false else {
                                throw MacLocalAgentRuntimeError.protocolViolation("Host admitted a closed submission.")
                            }
                            submissionAdmitted = true
                            await request.submissionAccepted?()
                        }
                    case .rejected:
                        guard !submissionAdmitted, ticket?.rejectBeforeAdmission() != false else {
                            throw MacLocalAgentRuntimeError.protocolViolation("Host rejected an already admitted submission.")
                        }
                        throw MacLocalAgentRuntimeError.submissionRejected(
                            code: decision.code!, message: decision.message!)
                    }
                    continue
                }
                if let bridgeRequest = Self.decodeComputerBridgeRequest(line),
                   bridgeRequest.sessionID == request.conversationID {
                    let response: Data
                    if bridgeRequest.toolName == "__nola_resolve_goal_model_channel" {
                        do {
                            guard let goalID = request.goalExecutionID,
                                  bridgeRequest.arguments["goal_id"] as? String == goalID,
                                  let proposed = bridgeRequest.arguments["native_model_route"] as? [String: Any],
                                  let port = request.applicationPort,
                                  let frozen = try JSONSerialization.jsonObject(with: port.preparedGoalNativeRoute(
                                    conversationID: request.conversationID, goalID: goalID)) as? [String: Any],
                                  NSDictionary(dictionary: frozen).isEqual(to: proposed),
                                  proposed["adapter"] as? String == "mlx",
                                  proposed["adapter_contract_revision"] as? Int == 1,
                                  let modelID = proposed["model_id"] as? String,
                                  let reference = proposed["profile_ref"] as? [String: Any],
                                  reference["profile_id"] as? String == modelID,
                                  let revision = reference["admitted_revision"] as? Int64,
                                  let profile = MacLocalModelProfile.resolve(modelID) else {
                                throw MacLocalAgentResumeError.credentialUnavailable
                            }
                            let endpoint = try await MacMLXOpenAIService.shared.start(profile,
                                expectedRevision: revision, applicationPort: port)
                            try MacMLXOpenAIService.shared.validate(endpoint, applicationPort: port)
                            guard NSDictionary(dictionary: try endpoint.nativeRoute()).isEqual(to: proposed) else {
                                throw MacLocalAgentResumeError.credentialUnavailable
                            }
                            currentNativeEndpoint = endpoint
                            response = try JSONSerialization.data(withJSONObject: [
                                "type":"device_tool_result", "request_id":bridgeRequest.id,
                                "session_id":request.conversationID,
                                "result":["authorized":true,"native_model_route":proposed,
                                    "endpoint":endpoint.baseURL,"model":endpoint.modelID,"api_key":endpoint.apiKey],
                            ])
                        } catch {
                            response = try JSONSerialization.data(withJSONObject: [
                                "type":"device_tool_result", "request_id":bridgeRequest.id,
                                "session_id":request.conversationID,"result":["authorized":false],
                            ])
                        }
                    } else if bridgeRequest.toolName == "__nola_validate_native_model_channel" {
                        let authorized: Bool
                        do {
                            guard let endpoint = currentNativeEndpoint,
                                  let port = request.applicationPort,
                                  let active = try port.activeRunIdentity(conversationID: request.conversationID) else {
                                throw MacLocalAgentResumeError.credentialUnavailable
                            }
                            let matchingGoal = try request.goalExecutionID.map { goal in
                                try port.goalExecutionAssistantIdentity(conversationID: request.conversationID,
                                    goalID: goal, runID: active.runID) == active.assistantMessageID
                            } == true
                            guard (active.runID == request.submissionID && active.assistantMessageID == request.assistantMessageID)
                                    || matchingGoal else {
                                throw MacLocalAgentResumeError.credentialUnavailable
                            }
                            try Self.validateNativeModelChannel(bridgeRequest.arguments, endpoint: endpoint,
                                applicationPort: port)
                            authorized = true
                        } catch { authorized = false }
                        response = try JSONSerialization.data(withJSONObject: [
                            "type": "device_tool_result", "request_id": bridgeRequest.id,
                            "session_id": request.conversationID, "result": ["authorized": authorized]
                        ], options: [.sortedKeys])
                    } else if bridgeRequest.toolName == MacAgentOrchestrationService.toolName {
                        response = await MacAgentOrchestrationService.responseData(
                            requestID: bridgeRequest.id,
                            arguments: bridgeRequest.arguments,
                            workspaceURL: request.projectURL,
                            appEnvironment: request.appEnvironment
                        )
                    } else if bridgeRequest.toolName == "conversation_recall" {
                        response = MacConversationRecallBridge.responseData(
                            requestID: bridgeRequest.id,
                            conversationID: bridgeRequest.sessionID,
                            arguments: bridgeRequest.arguments,
                            store: request.applicationPort
                        )
                    } else if MacProductivityToolService.supports(bridgeRequest.toolName) {
                        response = await MacProductivityToolService.responseData(
                            requestID: bridgeRequest.id,
                            toolName: bridgeRequest.toolName,
                            arguments: bridgeRequest.arguments
                        )
                    } else {
                        response = await MacComputerUseService.responseData(
                            requestID: bridgeRequest.id,
                            toolName: bridgeRequest.toolName,
                            arguments: bridgeRequest.arguments
                        )
                    }
                    var responseFrame = response
                    responseFrame.append(0x0A)
                    try inputPipe.fileHandleForWriting.write(contentsOf: responseFrame)
                    continue
                }
                if let goalID = request.goalExecutionID,
                   let runID = Self.goalEventRunID(line: line),
                   runID != currentRunID {
                    guard observedGoalRuns.insert(runID).inserted,
                          let port = request.applicationPort,
                          let assistant = try port.goalExecutionAssistantIdentity(conversationID: request.conversationID,
                              goalID: goalID, runID: runID) else {
                        throw MacLocalAgentRuntimeError.protocolViolation("Goal event has no exact Core execution owner.")
                    }
                    currentRunID = runID
                    currentAssistantMessageID = assistant
                    admittedCoreSessionID = nil
                    didRecoverTaskGuidance = false
                    transportGate = MacAgentHostTransportGate(runID: currentRunID,
                        assistantMessageID: currentAssistantMessageID)
                }
                if Self.requiresRunIdentityWitness(line: line), admittedCoreSessionID == nil {
                    guard let port = request.applicationPort,
                          let exact = try port.loadExactRunInvocation(
                            conversationID: request.conversationID,
                            expectedRunID: currentRunID,
                            assistantMessageID: currentAssistantMessageID
                          ), let coreSessionID = exact.coreSessionID else {
                        throw MacLocalAgentRuntimeError.protocolViolation(
                            "The Agent Host emitted Core events before an owned Run checkpoint was available."
                        )
                    }
                    admittedCoreSessionID = coreSessionID
                    if !submissionAdmitted {
                        guard ticket?.markAccepted() != false else {
                            throw MacLocalAgentRuntimeError.protocolViolation("Owned Run evidence arrived for a closed submission.")
                        }
                        submissionAdmitted = true
                        await request.submissionAccepted?()
                    }
                    admittedCoreSessions[request.conversationID] = (
                        runID: currentRunID, sessionID: coreSessionID,
                        controlSessionID: exact.conversationID
                    )
                }
                if var event = Self.decode(line: line, conversationID: request.conversationID,
                                           coreSessionID: admittedCoreSessionID,
                                           controlSessionID: admittedControlSessionID(conversationID: request.conversationID)) {
                    if let eventRunID = event.canonicalRunID, eventRunID != currentRunID {
                        throw MacLocalAgentRuntimeError.protocolViolation(
                            "The Agent Host event belongs to a different canonical Run."
                        )
                    }
                    if let wait = event.userInputRequest {
                        guard wait.runID == currentRunID,
                              wait.conversationID == request.conversationID,
                              wait.sessionID == admittedCoreSessionID else {
                            throw MacLocalAgentRuntimeError.protocolViolation("The user question belongs to a different canonical Run.")
                        }
                        observedUserInputWaitIDs.insert(wait.id)
                    }
                    if try transportGate.accept(event) == .duplicate {
                        continue
                    }
                    if !didRecoverTaskGuidance, admittedCoreSessions[request.conversationID] != nil {
                        didRecoverTaskGuidance = true
                        do { try await recoverTaskGuidance(conversationID: request.conversationID) }
                        catch {
                            continuation.yield(MacLocalAgentEvent(kind: .status, text: "",
                                title: String(localized: "Task guidance", table: "Chat"),
                                detail: error.localizedDescription, isError: true)
                                .bindingCanonicalRun(currentRunID))
                        }
                    }
                    // Unscoped diagnostics/control frames are owned by this
                    // exact transport; never make UI infer the current Run.
                    event = event.bindingCanonicalRun(currentRunID)
                    if !didLogFirstProtocolEvent {
                        didLogFirstProtocolEvent = true
                        let elapsed = runStartedAt.duration(to: .now)
                        let elapsedComponents = elapsed.components
                        let latencyMilliseconds = elapsedComponents.seconds * 1_000
                            + elapsedComponents.attoseconds / 1_000_000_000_000_000
                        await Logger.shared.log(
                            "Agent Host first protocol event",
                            level: .info,
                            category: "MacAgentHost",
                            metadata: [
                                "conversation": String(request.conversationID.prefix(12)),
                                "backend": request.executionBackend,
                                "event": String(describing: event.kind),
                                "latency_ms": String(latencyMilliseconds),
                                "locate_ms": milliseconds(from: runStartedAt, to: executableReadyAt),
                                "deliverables_ms": milliseconds(from: executableReadyAt, to: deliverablesReadyAt),
                                "tool_policy_ms": milliseconds(from: deliverablesReadyAt, to: toolPlanReadyAt),
                                "chrome_ms": milliseconds(from: toolPlanReadyAt, to: chromeReadyAt),
                                "skill_router_ms": milliseconds(from: chromeReadyAt, to: skillRouterReadyAt),
                                "request_encode_ms": milliseconds(from: skillRouterReadyAt, to: requestReadyAt),
                                "browser_runtime_ms": milliseconds(from: requestReadyAt, to: browserRuntimeReadyAt),
                                "process_launch_ms": milliseconds(from: browserRuntimeReadyAt, to: processReadyAt),
                                "process_to_event_ms": milliseconds(from: processReadyAt, to: .now),
                            ]
                        )
                    }
                    if event.assistantJournalCursor != nil, !didLogFirstAssistantDelta {
                        didLogFirstAssistantDelta = true
                        let elapsed = runStartedAt.duration(to: .now)
                        let elapsedComponents = elapsed.components
                        let latencyMilliseconds = elapsedComponents.seconds * 1_000
                            + elapsedComponents.attoseconds / 1_000_000_000_000_000
                        await Logger.shared.log(
                            "Agent Host first committed assistant publication",
                            level: .info,
                            category: "MacAgentHost",
                            metadata: [
                                "conversation": String(request.conversationID.prefix(12)),
                                "backend": request.executionBackend,
                                "latency_ms": String(latencyMilliseconds),
                            ]
                        )
                    }
                    if event.kind == .permissionRequested,
                       let permissionRequest = event.permissionRequest {
                        pendingPermissionRequests[request.conversationID, default: [:]][permissionRequest.id]
                            = permissionRequest
                    }
                    Self.persistCompactionCheckpointIfNeeded(
                        event,
                        store: request.applicationPort
                    )
                    if MacAgentHostEventDrainBuffer.isTerminal(event) {
                        receivedTerminalEvent = true
                        let changedDeliverables = Self.changedDeliverables(
                            at: request.projectURL,
                            comparedWith: initialDeliverables
                        )
                        event = event.addingArtifactURLs(changedDeliverables)
                        // The semantic Run is complete, but the Runtime
                        // Supervisor owns process cleanup and the session lock.
                        // Keep draining until normal EOF; presentation must not
                        // kill the Host process from a terminal UI event.
                        continuation.yield(event)
                        continue
                    }
                    continuation.yield(event)
                } else {
                    let diagnostic = line.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !diagnostic.isEmpty {
                        undecodedOutput.append(String(diagnostic.prefix(2_000)))
                        if undecodedOutput.count > 8 { undecodedOutput.removeFirst() }
                    }
                }
            }
        } catch {
            await processSession.shutdownAndDrain()
            stderrTask.cancel()
            _ = await stderrTask.value
            releaseRunRegistration(
                conversationID: request.conversationID,
                process: process
            )
            throw error
        }

        let termination = await processSession.waitForExit()
        let terminationStatus = termination.status
        let cancellationWasRequested = cancellationRequestedConversationIDs.remove(
            request.conversationID
        ) != nil
        stderrTask.cancel()
        let stderr = await stderrTask.value
        let processDiagnostic = [stderr, undecodedOutput.joined(separator: "\n")]
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        releaseRunRegistration(
            conversationID: request.conversationID,
            process: process
        )

        if request.goalExecutionID != nil {
            // This process can finish between rounds or after queueing a
            // control. Its exit is not a Goal/Run terminal fact; callers read
            // the Core ledger and projection. A later-round transport failure
            // must not be hidden by an earlier round's terminal event.
            guard terminationStatus == 0 else {
                throw MacLocalAgentRuntimeError.processFailed(terminationStatus, processDiagnostic)
            }
            continuation.finish()
            return
        }
        if !receivedTerminalEvent, cancellationWasRequested {
            throw CancellationError()
        } else if !receivedTerminalEvent, terminationStatus == 0 {
            if let port = request.applicationPort {
                // Re-read durable facts after EOF. A user can answer while the
                // old process is exiting: the Run remains recoverable even if
                // its last open wait has already been resolved in SQLite.
                let openWaits = try port.openUserInputWaits(conversationID: request.conversationID)
                let activeRun = try port.activeRunIdentity(conversationID: request.conversationID)
                if activeRun?.assistantMessageID == request.assistantMessageID,
                   let suspension = MacLocalAgentWaitSuspension.validated(
                    conversationID: request.conversationID, runID: request.submissionID,
                    activeRunID: activeRun?.runID, openWaits: openWaits,
                    observedWaitIDs: observedUserInputWaitIDs
                   ) {
                    throw suspension
                }
            }
            throw MacLocalAgentRuntimeError.launchFailed(
                String(localized: "The agent host exited without a terminal event.")
            )
        } else if !receivedTerminalEvent, termination.signaled {
            throw MacLocalAgentRuntimeError.processFailed(
                terminationStatus,
                String(
                    format: String(localized: "The local agent was terminated by macOS (signal %d). Rebuild the Mac app so its embedded Agent Host is signed correctly."),
                    terminationStatus
                )
            )
        } else if !receivedTerminalEvent {
            throw MacLocalAgentRuntimeError.processFailed(terminationStatus, processDiagnostic)
        }
        continuation.finish()
    }

    private func releaseRunRegistration(
        conversationID: String,
        process: Process
    ) {
        guard processes[conversationID] === process else { return }
        if let session = processSessions[conversationID], session.process === process {
            session.releaseResources()
            processSessions.removeValue(forKey: conversationID)
        }
        processes.removeValue(forKey: conversationID)
        controlInputs.removeValue(forKey: conversationID)
        toolPlans.removeValue(forKey: conversationID)
        projectURLs.removeValue(forKey: conversationID)
        executableURLs.removeValue(forKey: conversationID)
        pendingPermissionRequests.removeValue(forKey: conversationID)
        admittedCoreSessions.removeValue(forKey: conversationID)
        cancellationRequestedConversationIDs.remove(conversationID)
        continuations.removeValue(forKey: conversationID)
    }

    private func launchOwnsRun(_ launch: MacLocalAgentOwnedLaunchSnapshot, conversationID: String, runID: String) throws -> Bool {
        if launch.request.submissionID == runID { return true }
        guard let goal = launch.request.goalExecutionID, let port = launch.request.applicationPort else { return false }
        return try port.goalExecutionAssistantIdentity(conversationID: conversationID, goalID: goal, runID: runID) != nil
    }

    private func admittedControlSessionID(conversationID: String) -> String? {
        guard let admitted = admittedCoreSessions[conversationID], let launch = ownedLaunches[conversationID],
              (try? launchOwnsRun(launch, conversationID: conversationID, runID: admitted.runID)) == true else { return nil }
        return admitted.controlSessionID
    }

    private static let deliverableExtensions: Set<String> = [
        "docx", "xlsx", "pptx", "pdf", "csv", "tsv",
        "png", "jpg", "jpeg", "gif", "svg", "zip"
    ]

    private static func deliverableSnapshot(at root: URL) -> [String: String] {
        let keys: Set<URLResourceKey> = [
            .isRegularFileKey, .contentModificationDateKey, .fileSizeKey
        ]
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: Array(keys),
            options: [.skipsHiddenFiles, .skipsPackageDescendants]
        ) else { return [:] }

        var snapshot: [String: String] = [:]
        var visited = 0
        while let url = enumerator.nextObject() as? URL, visited < 10_000 {
            visited += 1
            guard deliverableExtensions.contains(url.pathExtension.lowercased()),
                  let values = try? url.resourceValues(forKeys: keys),
                  values.isRegularFile == true else { continue }
            let modified = values.contentModificationDate?.timeIntervalSince1970 ?? 0
            snapshot[url.standardizedFileURL.path] = "\(values.fileSize ?? 0):\(modified)"
        }
        return snapshot
    }

    private static func changedDeliverables(
        at root: URL,
        comparedWith baseline: [String: String]
    ) -> [URL] {
        deliverableSnapshot(at: root).compactMap { path, signature in
            guard baseline[path] != signature else { return nil }
            return URL(fileURLWithPath: path).standardizedFileURL
        }
        .sorted { $0.path < $1.path }
    }

    private func resolvedLocalEndpoint(for request: MacLocalAgentRunRequest) throws -> MacMLXOpenAIService.Endpoint? {
        guard MacExecutionBackendPolicy.requiresNolaModel(request.executionBackend) else { return nil }
        if request.wireProtocol == .siwcResponses { return nil }
        return try MacLocalModelEndpointSelection.resolve(modelID: request.modelID, captured: request.localModelEndpoint) {
            try MacMLXOpenAIService.shared.endpoint(for: $0)
        }
    }

    func arguments(
        for request: MacLocalAgentRunRequest,
        localEndpoint: MacMLXOpenAIService.Endpoint?
    ) -> [String] {
        var arguments = [
            "run",
            "--json",
            "--host-request-stdin",
            "--session",
            request.conversationID,
            "--project",
            request.projectURL.path,
        ]
        arguments.append(contentsOf: request.allowedDirectoryArguments)
        arguments.append(contentsOf: request.skillDirectoryArguments)
        // Permission intent travels once, as the typed stdin permission_profile.
        // Launch arguments select transport/resources, never a second policy.
        if request.executionBackend == "nola", request.wireProtocol != .siwcResponses,
           let endpoint = localEndpoint?.baseURL ?? request.endpoint, !endpoint.isEmpty {
            arguments.append(contentsOf: ["--endpoint", endpoint])
        }
        // Local endpoint credentials are serialized by initialRequestData into
        // stdin as well; argv is visible to process inspection and diagnostics.
        if request.wireProtocol != .siwcResponses, let modelID = localEndpoint?.modelID ?? request.modelID, !modelID.isEmpty {
            arguments.append(contentsOf: ["--model", modelID])
        }
        return arguments
    }

    func initialRequestData(for request: MacLocalAgentRunRequest) throws -> Data {
        try initialRequestData(for: request, localEndpoint: resolvedLocalEndpoint(for: request))
    }

    func initialRequestData(
        for request: MacLocalAgentRunRequest,
        localEndpoint: MacMLXOpenAIService.Endpoint?
    ) throws -> Data {
        let usesNolaModel = MacExecutionBackendPolicy.requiresNolaModel(request.executionBackend)
        let remoteCapabilities = usesNolaModel && localEndpoint == nil ? request.modelExecutionCapabilities : nil
        guard let applicationPort = request.applicationPort else {
            throw MacApplicationPortError.serviceUnavailable
        }
        let outputLimit = request.modelParameters?.maxOutputTokens
            ?? request.reasoningConfiguration?.outputLimit ?? ModelParameters().maxOutputTokens
        let requestedSelection: MacCanonicalJSONValue
        if !usesNolaModel || request.reasoningTier != nil {
            // Explicit agent tier intent is resolved by Application and does
            // not submit a second provider selection alongside it.
            requestedSelection = .null
        } else {
            let selection = request.reasoningConfiguration?.selection(request.reasoningPreference)
            guard request.reasoningPreference == "automatic" || selection != nil else {
                throw MacApplicationPortError.rejected("The reasoning preference has no declared transport selection.")
            }
            requestedSelection = try selection.map { try MacCanonicalJSONValue(foundationValue: $0) } ?? .null
        }
        let canonicalSelection: MacCanonicalJSONValue = try usesNolaModel ? applicationPort.resolveReasoning(
            declaration: request.reasoningConfiguration,
            wireProtocol: request.wireProtocol ?? .chatCompletions,
            outputLimit: outputLimit,
            supportsReasoning: remoteCapabilities?.reasoning ?? false,
            requested: requestedSelection,
            tier: request.reasoningTier
        ) : .null
        let reasoningSelection: [String: Any]?
        if canonicalSelection == .null {
            reasoningSelection = nil
        } else {
            guard let object = try JSONSerialization.jsonObject(
                with: JSONEncoder().encode(canonicalSelection)
            ) as? [String: Any] else {
                throw MacApplicationPortError.invalidResponse
            }
            reasoningSelection = object
        }
        if let remoteCapabilities {
            if request.samplingOverride && !remoteCapabilities.samplingParameters {
                throw NSError(domain: "NolaMac.LocalAgentRequest", code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "The selected model does not support sampling parameters."])
            }
            if request.samplingOverride && remoteCapabilities.samplingRequiresNoReasoning
                && reasoningSelection?["value"] as? String != "none" {
                throw NSError(domain: "NolaMac.LocalAgentRequest", code: 4,
                    userInfo: [NSLocalizedDescriptionKey: "This model accepts sampling parameters only with explicit reasoning effort none."])
            }
        }
        // Resolve conditional sampling against the canonical selection, not
        // the pre-resolution UI hint. Defaults are still frozen request values.
        let declaredSamplingAllowed = remoteCapabilities?.samplingParameters == true
            && (remoteCapabilities?.samplingRequiresNoReasoning != true
                || (reasoningSelection?["kind"] as? String == "effort"
                    && reasoningSelection?["value"] as? String == "none"))
        let effectiveSamplingOverride = request.samplingOverride
            || localEndpoint != nil || declaredSamplingAllowed
        // Mac's canonical run command loads history itself. Share the same
        // captured-current-turn rule as iOS without injecting duplicate history.
        let turnInput = ConversationTurnInput(currentUserText: request.prompt, historySource: .coreManaged)
        var contextComponents: [[String: Any]] = try request.contextComponents.map { component in
            let data = try JSONEncoder().encode(component)
            guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                throw NSError(
                    domain: "NolaMac.LocalAgentRequest",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Context component did not encode as an object"]
                )
            }
            return object
        }
        if request.automationAdmission == nil && request.editedResendOperationID == nil { contextComponents.append([
            "kind": "platform_data",
            "id": "nola-mac-active-workspace",
            "content": "Default working directory: \(request.projectURL.standardizedFileURL.path). This is an execution location, not necessarily the user's task target. When the task identifies a specific directory, resolve and request that directory directly; do not enumerate its parent or unrelated sibling directories merely to rediscover the supplied target. For a directory outside the allowed roots, submit workspace_access with the exact absolute target directory, then retry the intended operation with an explicit root/cwd after approval. An empty session directory or a boundary denial is not evidence about the target repository. Folder selection is in the chat's Working Environment menu; do not invent settings paths or ask the user to copy the repository into the sandbox.",
            "source": [
                "platform": "macos",
                "title": "Active workspace",
            ],
        ]) }
        var payload: [String: Any] = [
            "prompt": turnInput.input,
            "permission_profile": request.permissionMode.permissionProfile,
            "submission_id": request.submissionID,
            "user_message_id": request.userMessageID,
            "assistant_message_id": request.assistantMessageID,
            "expected_configuration_revision": request.expectedConfigurationRevision as Any? ?? NSNull(),
            "expected_parent_message_id": request.expectedParentMessageID as Any? ?? NSNull(),
            "context_components": contextComponents,
            "execution_backend": request.executionBackend,
            "model_test_mode": request.modelTestMode,
            "input_origin": request.inputOrigin,
            "vision_input": request.visionInput && (remoteCapabilities?.imageInput ?? (localEndpoint != nil)),
            "max_iterations": request.maxModelIterations,
            "attachments": request.attachmentURLs.compactMap { url -> [String: Any]? in
                let normalized = url.standardizedFileURL
                guard normalized.isFileURL else { return nil }
                let values = try? normalized.resourceValues(forKeys: [
                    .isRegularFileKey,
                    .fileSizeKey,
                    .contentTypeKey,
                ])
                guard values?.isRegularFile == true else { return nil }
                let contentType = values?.contentType
                    ?? UTType(filenameExtension: normalized.pathExtension)
                    ?? .data
                return [
                    "path": normalized.path,
                    "name": normalized.lastPathComponent,
                    "mime_type": contentType.preferredMIMEType ?? "application/octet-stream",
                    "byte_count": values?.fileSize ?? 0,
                ]
            },
        ]
        if let reasoningTier = request.reasoningTier {
            payload["reasoning_tier"] = reasoningTier
        }
        if let control = request.goalControl {
            payload["goal_control"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(control))
        }
        if let goalID = request.goalWakeID { payload["goal_wake_id"] = goalID }

        if let localEndpoint {
            payload["native_model_route"] = try localEndpoint.nativeRoute()
        }
        if let remoteCapabilities {
            payload["chat_output_token_field"] = remoteCapabilities.chatOutputTokenField?.rawValue ?? "auto"
            payload["model_capabilities"] = [
                "native_tool_calls": remoteCapabilities.toolCalls,
                "stream_tool_calls": remoteCapabilities.toolCalls,
                "vision_input": remoteCapabilities.imageInput,
                "reasoning": remoteCapabilities.reasoning,
                // The Host decodes public reasoning output independently of
                // whether this model supports requesting reasoning generation.
                "stream_reasoning": true,
                "reasoning_continuity": request.wireProtocol?.usesResponsesTransport == true && remoteCapabilities.reasoning,
            ]
            if let summarySupported = remoteCapabilities.reasoningSummarySupported {
                payload["reasoning_summary_supported"] = summarySupported
            }
        }
        if let selection = reasoningSelection {
            payload["reasoning_selection"] = selection
            payload["max_output_tokens"] = outputLimit
        }
        if usesNolaModel, let parameters = request.modelParameters {
            payload["max_output_tokens"] = parameters.maxOutputTokens
            payload["temperature"] = parameters.temperature
            payload["top_p"] = parameters.topP
            // A local runtime keeps startup defaults for its lifetime. Always
            // send the already-resolved request values, even when they happen
            // to equal the UI defaults; its startup defaults may differ.
            payload["sampling_override"] = effectiveSamplingOverride
            var options: [String: Any] = [:]
            if parameters.presencePenalty != 0 { options["presence_penalty"] = parameters.presencePenalty }
            if parameters.frequencyPenalty != 0 { options["frequency_penalty"] = parameters.frequencyPenalty }
            if let seed = parameters.seed { options["seed"] = seed }
            let stop = parameters.stopSequences.components(separatedBy: .newlines).filter { !$0.isEmpty }
            if !stop.isEmpty { options["stop"] = stop }
            if let topK = parameters.topK { options["top_k"] = topK }
            if let penalty = parameters.repetitionPenalty { options["repetition_penalty"] = penalty }
            payload["generation_options"] = options
        }
        // Only the endpoint's explicit wire contract chooses the route. Legacy
        // reasoning metadata must not silently switch a Chat endpoint to Responses.
        if usesNolaModel {
            payload["model_protocol"] = (request.wireProtocol ?? .chatCompletions).rawValue
            if request.wireProtocol == .siwcResponses {
                guard let selected = request.siwcProfile, selected.profileID == request.modelProfileID,
                      selected.revision > 0, request.apiKey?.isEmpty != false, request.modelFallbacks.isEmpty else {
                    throw MacLocalAgentResumeError.credentialUnavailable
                }
                payload["model_profile_id"] = selected.profileID
                payload["expected_profile_revision"] = selected.revision
            }
        }
        if let modelPurpose = request.modelPurpose {
            payload["model_purpose"] = modelPurpose.rawValue
        }
        // A selected reasoning contract belongs to this concrete route. Do not
        // send its parameters to an unqualified fallback model.
        if request.executionBackend == "nola", !request.modelFallbacks.isEmpty,
           request.wireProtocol != .responses, request.wireProtocol != .siwcResponses,
           request.reasoningConfiguration?.control == nil || request.reasoningConfiguration?.control == .automatic {
            payload["llm_fallbacks"] = request.modelFallbacks.map { fallback in
                var value: [String: Any] = [
                    "endpoint": fallback.endpoint,
                    "model": fallback.model,
                ]
                if !fallback.apiKey.isEmpty { value["api_key"] = fallback.apiKey }
                return value
            }
        }
        if request.executionBackend != "nola", let modelID = request.modelID, !modelID.isEmpty {
            payload["execution_model"] = modelID
            if let provider = request.modelProviderID, !provider.isEmpty {
                payload["execution_model_provider"] = provider
            }
        }
        if let externalSessionID = request.externalSessionID, !externalSessionID.isEmpty {
            payload["external_session_id"] = externalSessionID
        }
        if usesNolaModel, let apiKey = (localEndpoint?.apiKey ?? request.apiKey)?
            .trimmingCharacters(in: .whitespacesAndNewlines),
           !apiKey.isEmpty {
            payload["api_key"] = apiKey
        }
        if let contextWindowTokens = request.contextWindowTokens
            ?? request.modelParameters?.maxContextLength
            ?? localEndpoint?.contextWindowTokens {
            payload["context_window_tokens"] = contextWindowTokens
        }
        if let mcpData = request.activeMCPServerConfigurationData {
            payload["mcp_servers"] = try JSONSerialization.jsonObject(with: mcpData)
        }
        let rules = try applicationPort.effectivePermissionRules(
            conversationID: request.conversationID,
            hostID: "mac-local"
        )
        let rulesData = try JSONEncoder().encode(rules.rules)
        payload["permission_rules"] = try JSONSerialization.jsonObject(with: rulesData)
        if let operation = request.editedResendOperationID, let frozen = request.editedResendModel {
            guard request.submissionID == operation, request.contextComponents.isEmpty,
                  request.attachmentURLs.isEmpty, request.externalSessionID == nil,
                  request.permissionMode == .cautious,
                  case .object(let generation) = frozen.generation,
                  case .object(let primary) = frozen.primary else { throw MacEditedResendError.sourceUnavailable }
            payload["edited_resend_operation_id"] = operation
            payload["expert_template_authority"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertTemplateAuthority.local()))
            for key in ["max_output_tokens", "temperature", "top_p", "sampling_override", "generation_options", "reasoning_selection", "reasoning_tier",
                        "max_retries", "parallel_tool_calls", "thinking_enabled", "reasoning_summary_supported", "prompt_cache", "prompt_cache_retention"] {
                if let value = generation[key] { payload[key] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(value), options: [.fragmentsAllowed]) }
                else { payload.removeValue(forKey: key) }
            }
            payload["frozen_output_admission"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(frozen.output_admission))
            if case .object(let admission) = frozen.output_admission {
                if admission["preference"] == .string("default") { payload.removeValue(forKey: "max_output_tokens") }
                if let window = admission["configured_context_window"], window != .null {
                    payload["context_window_tokens"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(window), options: [.fragmentsAllowed])
                }
            }

            if let field = primary["output_token_field"] {
                payload["chat_output_token_field"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(field), options: [.fragmentsAllowed])
            }
            // All routes were resolved against current private credentials. The
            // Host still compares its actual binding with durable Prepared.
            payload["llm_fallbacks"] = request.modelFallbacks.map { ["endpoint": $0.endpoint, "model": $0.model, "api_key": $0.apiKey] }
        }
        if !request.modelTestMode {
            // This closed, non-secret descriptor is committed inside the same
            // immutable Host invocation as the Run, not a separate UI archive.
            let profileID: String? = request.executionBackend == "nola"
                ? (localEndpoint?.modelID ?? request.modelProfileID) : nil
            let fallbackCount: Int = (payload["llm_fallbacks"] as? [[String: Any]])?.count ?? 0
            let fallbackProfileIDs: [String] = request.modelFallbacks.prefix(fallbackCount).map(\.profileID)
            let authorizedPaths: [String] = request.authorizedWorkspaceURLs.map {
                $0.standardizedFileURL.resolvingSymlinksInPath().path
            }
            var descriptor: [String: Any] = [:]
            descriptor["version"] = 1
            descriptor["input_origin"] = request.inputOrigin
            descriptor["project_path"] = request.projectURL.standardizedFileURL.resolvingSymlinksInPath().path
            descriptor["authorized_workspace_paths"] = authorizedPaths
            descriptor["allowed_directory_arguments"] = request.allowedDirectoryArguments
            descriptor["skill_directory_arguments"] = request.skillDirectoryArguments
            descriptor["active_skill_ids"] = request.activeSkillDescriptors.map { $0.id.rawValue }
            descriptor["model_profile_id"] = profileID.map { $0 as Any } ?? NSNull()
            descriptor["model_profile_revision"] = (request.modelProfileRevision ?? request.siwcProfile?.revision).map { $0 as Any } ?? NSNull()
            descriptor["fallback_profile_ids"] = fallbackProfileIDs
            descriptor["permission_mode"] = request.permissionMode.rawValue
            descriptor["memory_enabled"] = request.memoryEnabled
            descriptor["context_window_tokens"] = payload["context_window_tokens"] ?? NSNull()
            descriptor["max_iterations"] = request.maxModelIterations
            descriptor["model_capabilities"] = payload["model_capabilities"] ?? [String: Bool]()
            descriptor["vision_input"] = payload["vision_input"] ?? false
            descriptor["permission_rules"] = payload["permission_rules"] ?? []
            descriptor["host_attachments"] = payload["attachments"] ?? []
            payload["mac_launch"] = descriptor
        }
        if let commandID = request.queuedUserCommandID { payload["queued_user_command_id"] = commandID }
        if let admission = request.automationAdmission {
            guard request.inputOrigin == "scheduled_action", request.contextComponents.isEmpty,
                  request.attachmentURLs.isEmpty, request.prompt == admission.spec.prompt else {
                throw MacLocalAgentResumeError.invalidFrame
            }
            payload["automation_execution_lease"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(admission.lease))
            payload["automation_execution_spec"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(admission.spec))
            payload["automation_artifacts"] = try JSONSerialization.jsonObject(with: admission.artifacts)
        }
        var data = try JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
        data.append(0x0A)
        return data
    }

    func runCapture(arguments: [String], input: Data? = nil,
                    ownedLaunch: MacLocalAgentOwnedLaunchSnapshot? = nil) async throws -> Data {
        guard let executableURL = ownedLaunch?.executableURL ?? locateExecutable() else {
            throw MacLocalAgentRuntimeError.executableUnavailable
        }
        let process = Process()
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        let projectURL = ownedLaunch?.request.projectURL ?? MacAppEnvironment.shared.scratchWorkspaceURL
        process.executableURL = executableURL
        process.currentDirectoryURL = projectURL
        process.arguments = arguments
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        let inputPipe = input.map { _ in Pipe() }
        if let inputPipe {
            process.standardInput = inputPipe
        } else {
            process.standardInput = FileHandle.nullDevice
        }
        process.environment = ownedLaunch?.environment ?? runtimeEnvironment(projectURL: projectURL)
        do {
            try process.run()
            try? outputPipe.fileHandleForWriting.close()
            try? errorPipe.fileHandleForWriting.close()
            try? inputPipe?.fileHandleForReading.close()
        } catch {
            try? outputPipe.fileHandleForWriting.close()
            try? errorPipe.fileHandleForWriting.close()
            try? inputPipe?.fileHandleForReading.close()
            try? inputPipe?.fileHandleForWriting.close()
            throw MacLocalAgentRuntimeError.launchFailed(error.localizedDescription)
        }
        if let input, let inputPipe {
            inputPipe.fileHandleForWriting.write(input)
            try? inputPipe.fileHandleForWriting.close()
        }

        let outputHandle = outputPipe.fileHandleForReading
        let errorHandle = errorPipe.fileHandleForReading
        let outputTask = Task.detached {
            try outputHandle.readToEnd() ?? Data()
        }
        let errorTask = Task.detached {
            try errorHandle.readToEnd() ?? Data()
        }
        process.waitUntilExit()
        let output = try await outputTask.value
        let errorData = try await errorTask.value
        guard process.terminationStatus == 0 else {
            throw MacLocalAgentRuntimeError.processFailed(
                process.terminationStatus,
                String(decoding: errorData, as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            )
        }
        return output
    }

    private func runtimeEnvironment(
        projectURL: URL,
        permissionMode: MacPermissionMode = .cautious,
        appEnvironment: MacAppEnvironment = .shared
    ) -> [String: String] {
        var environment = appEnvironment.processEnvironment(projectURL: projectURL)
        for (key, value) in appEnvironment.launchAgentEnvironment(projectURL: projectURL) {
            environment[key] = value
        }
        // The native Mac app owns its permission UX. Make that identity an
        // explicit Host contract instead of relying on the embedded binary's
        // filename or on inherited shell/IDE environment. Core's recommended
        // profile lets reversible work run without interruption and asks only
        // for the concrete irreversible invocation.
        environment["NOLA_CLIENT_SURFACE"] = "mac"
        environment["NOLA_DESKTOP_HOST"] = "1"
        environment["NOLA_PERMISSION_PROFILE"] = permissionMode.permissionProfile
        environment["NO_COLOR"] = "1"
        MacDebugLoggingConfiguration.apply(to: &environment)
        return environment
    }

    private func locateExecutable() -> URL? {
        MacAgentHostExecutableLocator.resolve()
    }

    static func decode(line: String, conversationID: String = "local",
                       coreSessionID: String? = nil,
                       controlSessionID: String? = nil) -> MacLocalAgentEvent? {
        var event = decodeUnbound(line: line, conversationID: conversationID,
                                  coreSessionID: coreSessionID,
                                  controlSessionID: controlSessionID)
        if let coreSessionID, event?.isError == false || event?.coreTurnTerminal != nil {
            event = event?.bindingCoreSession(conversationID: conversationID, coreSessionID: coreSessionID)
        }
        if let controlSessionID, event?.isError == false {
            event = event?.bindingControlSession(controlSessionID)
        }
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let runID = object["canonical_run_id"] as? String,
              !runID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return event }
        if let boundRunID = event?.canonicalRunID, boundRunID != runID {
            return protocolFailure("Conflicting canonical Run identities in Host event.",
                                   conversationID: conversationID, eventID: nil)
        }
        return event?.bindingCanonicalRun(runID)
    }

    @inline(never)
    private static func decodeUnbound(line: String, conversationID: String, coreSessionID: String?,
                                      controlSessionID: String?) -> MacLocalAgentEvent? {
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            let text = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { return nil }
            return MacLocalAgentEvent(kind: .status, text: text, title: nil, detail: nil, isError: false)
        }

        let nestedEvent = object["event"] as? [String: Any]
        let type = optionalString(in: nestedEvent ?? object, keys: ["type"])
            ?? optionalString(in: object, keys: ["type"])
            ?? "unknown"
        if requiresCoreSessionWitness(type: type),
           coreSessionID?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false {
            return protocolFailure(
                "The Agent Host Core event has no exact Run checkpoint identity witness.",
                conversationID: conversationID, eventID: type
            )
        }
        if requiresControlSessionWitness(type: type),
           controlSessionID?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false {
            return protocolFailure(
                "The Agent Host control event has no authenticated session identity witness.",
                conversationID: conversationID, eventID: type
            )
        }
        // Keep large typed envelope temporaries in separate, non-inlined frames.
        // The monolithic decoder exceeded cooperative executor thread stacks.
        switch type {
        case "assistant_run_terminal_committed_v1":
            guard object["session_id"] as? String == conversationID,
                  let sourceHostID = object["source_host_id"] as? String,
                  !sourceHostID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  let runID = object["run_id"] as? String,
                  !runID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  let digest = object["seal_digest"] as? String, digest.utf8.count == 64,
                  digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
                return protocolFailure("Invalid assistant Run terminal notification.", conversationID: conversationID, eventID: nil)
            }
            return MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
                assistantRunTerminalSealDigest: digest).bindingCanonicalRun(runID)
        case "assistant_journal_committed_v1":
            guard let notification = try? JSONDecoder().decode(MacAssistantJournalNotification.self, from: data),
                  !notification.source_host_id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  notification.session_id == conversationID,
                  notification.through.identity.conversation_id == conversationID else {
                return protocolFailure("Invalid assistant journal notification.", conversationID: conversationID, eventID: nil)
            }
            return MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
                assistantJournalCursor: notification.through).bindingCanonicalRun(notification.through.identity.run_id)
        case "turn_completed", "turn_failed", "turn_cancelled":
            return decodeCoreTurnTerminal(object: object, conversationID: conversationID, coreSessionID: coreSessionID, type: type)
        case "core_task_updated":
            return decodeCoreTask(object: object, conversationID: conversationID, coreSessionID: coreSessionID, data: data)
        case "core_skill_runtime_updated":
            return decodeCoreSkillRuntime(object: object, conversationID: conversationID, coreSessionID: coreSessionID, data: data)
        case "core_capabilities_activated":
            return decodeCoreCapabilityActivation(object: object, conversationID: conversationID, coreSessionID: coreSessionID, data: data)
        case "core_permission_projection_updated":
            return decodeCorePermissionProjection(object: object, conversationID: conversationID, coreSessionID: coreSessionID, data: data)
        case "permission_request":
            return decodePermissionRequest(object: object, conversationID: conversationID, coreSessionID: controlSessionID)
        case "user_input_wait_changed":
            return decodeUserInputRequest(object: object, conversationID: conversationID, coreSessionID: coreSessionID)
        case "context_usage":
            return decodeContextUsage(object: object, conversationID: conversationID)
        default:
            return decodeTraceEvent(object: object, conversationID: conversationID, type: type)
        }
    }

    private static func requiresCoreSessionWitness(type: String) -> Bool {
        switch type {
        case "core_task_updated", "core_skill_runtime_updated",
             "core_capabilities_activated", "core_permission_projection_updated",
             "turn_completed", "turn_failed", "turn_cancelled", "user_input_wait_changed":
            return true
        default:
            return false
        }
    }

    private static func requiresControlSessionWitness(type: String) -> Bool {
        type == "permission_request"
    }

    static func goalEventRunID(line: String) -> String? {
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return object["canonical_run_id"] as? String ?? MacAgentTraceEnvelope.decode(object["trace"])?.runID
    }

    private static func requiresRunIdentityWitness(line: String) -> Bool {
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return false }
        let nested = object["event"] as? [String: Any]
        let type = optionalString(in: nested ?? object, keys: ["type"])
            ?? optionalString(in: object, keys: ["type"]) ?? "unknown"
        return requiresCoreSessionWitness(type: type) || requiresControlSessionWitness(type: type)
    }

    @inline(never)
    private static func decodeCoreTurnTerminal(object: [String: Any], conversationID: String, coreSessionID: String?, type: String) -> MacLocalAgentEvent? {
        let nestedEvent = object["event"] as? [String: Any]
        let sessionID = optionalString(in: object, keys: ["session_id"])
        let turnID = optionalString(in: nestedEvent ?? object, keys: ["turn_id"])
            ?? optionalString(in: object, keys: ["turn_id"])
        func unsignedInteger(_ value: Any?) -> UInt64? {
            guard let value = value as? NSNumber,
                  CFGetTypeID(value) != CFBooleanGetTypeID(),
                  value.doubleValue.isFinite, value.doubleValue >= 0,
                  value.doubleValue.rounded(.towardZero) == value.doubleValue,
                  value.doubleValue < 18_446_744_073_709_551_616 else { return nil }
            return value.uint64Value
        }
        let rawCursor = (object["cursor"] as? [String: Any])?["sequence"] ?? object["cursor"]
        let status: MacCoreTurnTerminalEnvelope.Status = switch type {
        case "turn_failed": .failed
        case "turn_cancelled": .cancelled
        default: .completed
        }
        guard let coreSessionID, sessionID == coreSessionID,
              nestedEvent?["session_id"] == nil || nestedEvent?["session_id"] as? String == coreSessionID,
              object["turn_id"] == nil || object["turn_id"] as? String == turnID,
              let turnID, let cursor = unsignedInteger(rawCursor),
              let revision = unsignedInteger(object["revision"]),
              let terminal = MacCoreTurnTerminalEnvelope.validated(
            sessionID: coreSessionID,
            turnID: turnID,
            cursor: cursor,
            revision: revision,
            status: status,
            reason: optionalString(in: nestedEvent ?? object, keys: ["reason", "error"])
                ?? optionalString(in: object, keys: ["reason", "error"])
              ) else {
            return protocolFailure(
                "The Agent Host emitted an invalid Core Turn terminal event.",
                conversationID: conversationID,
                eventID: type
            )
        }
        return MacLocalAgentEvent(
            kind: status == .failed ? .failed : (status == .cancelled ? .cancelled : .finished),
            text: "",
            title: nil,
            detail: terminal.reason,
            isError: status == .failed,
            terminalReason: terminal.reason,
            eventID: "core-turn:\(cursor):\(turnID)",
            coreTurnTerminal: terminal
        )
    }

    @inline(never)
    private static func decodeCoreTask(object: [String: Any], conversationID: String, coreSessionID: String?, data: Data) -> MacLocalAgentEvent? {
        guard let update = try? JSONDecoder().decode(MacCoreTaskUpdateEnvelope.self, from: data),
              update.sessionID == coreSessionID,
              update.task.ownerSessionID == coreSessionID else {
            return protocolFailure(
                "The Agent Host emitted an invalid Core Task update.",
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["task_id"])
            )
        }
        return MacLocalAgentEvent(
            kind: .status,
            text: "",
            title: update.task.currentStep,
            detail: nil,
            isError: false,
            eventID: "core-task:\(update.taskProjectionRevision):\(update.task.id)",
            coreTaskUpdate: update
        )
    }

    @inline(never)
    private static func decodeCoreSkillRuntime(object: [String: Any], conversationID: String, coreSessionID: String?, data: Data) -> MacLocalAgentEvent? {
        guard let update = try? JSONDecoder().decode(MacCoreSkillRuntimeUpdateEnvelope.self, from: data),
              update.sessionID == coreSessionID else {
            return protocolFailure(
                "The Agent Host emitted an invalid Core Skill runtime update.",
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["cursor"])
            )
        }
        return MacLocalAgentEvent(
            kind: .status,
            text: "",
            title: nil,
            detail: nil,
            isError: false,
            eventID: "core-skill:\(update.cursor):\(update.skillRuntimeRevision)",
            coreSkillRuntimeUpdate: update
        )
    }

    @inline(never)
    private static func decodeCoreCapabilityActivation(object: [String: Any], conversationID: String, coreSessionID: String?, data: Data) -> MacLocalAgentEvent? {
        guard let update = try? JSONDecoder().decode(
            MacCoreCapabilityActivationEnvelope.self,
            from: data
        ), update.sessionID == coreSessionID else {
            return protocolFailure(
                "The Agent Host emitted an invalid Core capability activation.",
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["cursor"])
            )
        }
        return MacLocalAgentEvent(
            kind: .status,
            text: "",
            title: nil,
            detail: nil,
            isError: false,
            eventID: "core-capability:\(update.cursor):\(update.source)",
            coreCapabilityActivation: update
        )
    }

    @inline(never)
    private static func decodeCorePermissionProjection(object: [String: Any], conversationID: String, coreSessionID: String?, data: Data) -> MacLocalAgentEvent? {
        let update: MacCorePermissionProjectionEnvelope
        do {
            update = try JSONDecoder().decode(MacCorePermissionProjectionEnvelope.self, from: data)
        } catch {
            return protocolFailure(
                "The Agent Host emitted an invalid Core permission projection: \(error)",
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["permission_projection_revision"])
            )
        }
        guard update.sessionID == coreSessionID,
              update.permissionProjection.revision == update.permissionProjectionRevision else {
            return protocolFailure(
                "The Agent Host emitted a Core permission projection with mismatched identity or revision.",
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["permission_projection_revision"])
            )
        }
        return MacLocalAgentEvent(
            kind: .status,
            text: "",
            title: nil,
            detail: nil,
            isError: false,
            eventID: "core-permission:\(update.permissionProjectionRevision)",
            corePermissionProjection: update
        )
    }

    @inline(never)
    private static func decodePermissionRequest(object: [String: Any], conversationID: String, coreSessionID: String?) -> MacLocalAgentEvent? {
        guard let instructions = MacPermissionInstruction.decode(object["permission_instructions"]) else {
            return protocolFailure("Invalid permission instruction metadata.", conversationID: conversationID, eventID: nil)
        }
        let trace = MacAgentTraceEnvelope.decode(object["trace"])
        if let trace, trace.conversationID != conversationID {
            return protocolFailure(
                "The Agent Host permission request belongs to a different conversation.",
                conversationID: conversationID,
                eventID: trace.eventID
            )
        }
        guard let requestID = optionalString(in: object, keys: ["request_id", "id"]),
              let allowConversation = object["allow_conversation"] as? Bool,
              let protocolVersion = (object["protocol_version"] as? NSNumber)?.intValue,
              protocolVersion == MacRemotePermissionRequest.supportedProtocolVersion,
              let nonce = optionalString(in: object, keys: ["nonce"]),
              let hostID = optionalString(in: object, keys: ["host_id"]),
              let clientID = optionalString(in: object, keys: ["client_id"]),
              let sessionID = optionalString(in: object, keys: ["session_id"]), sessionID == coreSessionID,
              let toolName = optionalString(in: object, keys: ["tool_name"]) else {
            return protocolFailure(
                "The Agent Host emitted an incomplete permission request envelope.",
                conversationID: conversationID,
                eventID: trace?.eventID
            )
        }
        let reason = optionalString(in: object, keys: ["reason"])
            ?? String(localized: "This operation needs your approval.")
        let request = MacRemotePermissionRequest(
            id: requestID,
            allowConversation: allowConversation,
            protocolVersion: protocolVersion,
            nonce: nonce,
            hostID: hostID,
            clientID: clientID,
            sessionID: sessionID,
            toolName: toolName,
            arguments: jsonString(object["args"] ?? object["arguments"]) ?? "",
            permissionInstructions: instructions,
            callID: optionalString(in: object, keys: ["call_id"]) ?? trace?.payload.callID,
            reason: reason,
            message: optionalString(in: object, keys: ["message"])
                ?? String(localized: "Allow this operation?"),
            expiresAt: (object["expires_at"] as? NSNumber)?.int64Value,
            policyDigest: optionalString(in: object, keys: ["policy_digest"]),
            manifestDigest: optionalString(in: object, keys: ["manifest_digest"]),
            manifestValidUntilMs: (object["manifest_valid_until_ms"] as? NSNumber)?.int64Value,
            toolCatalogEvidence: MacRemotePermissionToolEvidence.decode(object["tool_catalog_evidence"], toolName: toolName),
            mcpDescriptorData: (object["mcp_descriptor"] as? [String: Any]).flatMap {
                try? JSONSerialization.data(withJSONObject: $0, options: [.sortedKeys])
            }
        )
        return MacLocalAgentEvent(
            kind: .permissionRequested,
            text: "",
            title: toolName,
            detail: reason,
            isError: false,
            eventID: trace?.eventID ?? "permission:\(requestID)",
            toolCallID: request.callID ?? requestID,
            permissionRequest: request,
            trace: trace
        )
    }

    @inline(never)
    private static func decodeUserInputRequest(object: [String: Any], conversationID: String, coreSessionID: String?) -> MacLocalAgentEvent? {
        guard let record = object["record"] as? [String: Any],
              let request = MacRemoteUserInputRequest(object: record),
              request.conversationID == conversationID,
              request.sessionID == coreSessionID else {
            return protocolFailure(
                "The Agent Host emitted an invalid user input request.",
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["request_id"])
            )
        }
        return MacLocalAgentEvent(
            kind: .userInputRequested,
            text: "",
            title: request.header,
            detail: request.question,
            isError: false,
            eventID: "user-input:\(request.id):\(request.revision)",
            toolCallID: request.id,
            userInputRequest: request
        )
    }

    @inline(never)
    private static func decodeContextUsage(object: [String: Any], conversationID: String) -> MacLocalAgentEvent? {
        let trace = MacAgentTraceEnvelope.decode(object["trace"])
        if let trace, trace.conversationID != conversationID {
            return protocolFailure(
                "The Agent Host context usage belongs to a different conversation.",
                conversationID: conversationID,
                eventID: trace.eventID
            )
        }
        guard let data = try? JSONSerialization.data(withJSONObject: object),
              let usage = try? JSONDecoder().decode(MacAgentTraceUsage.self, from: data),
              usage.usedTokens >= 0,
              usage.contextWindowTokens > 0,
              usage.promptTokenLimit > 0,
              usage.promptTokenLimit <= usage.contextWindowTokens else {
            return protocolFailure(
                "The Agent Host emitted invalid context usage.",
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["event_id", "eventId", "id"])
            )
        }
        return MacLocalAgentEvent(
            kind: .status,
            text: "",
            title: nil,
            detail: nil,
            isError: false,
            eventID: "context-usage:\(usage.usedTokens):\(usage.contextWindowTokens)",
            contextUsedTokens: usage.usedTokens,
            contextWindowTokens: usage.contextWindowTokens,
            contextPromptLimit: usage.promptTokenLimit,
            trace: trace
        )
    }

    @inline(never)
    private static func decodeTraceEvent(object: [String: Any], conversationID: String, type: String) -> MacLocalAgentEvent? {
        guard let trace = MacAgentTraceEnvelope.decode(object["trace"]) else {
            let keys = object.keys.sorted().joined(separator: ",")
            let diagnostic = type == "error"
                ? String((object["detail"] as? String ?? "").prefix(2000)) : ""
            return protocolFailure(
                "The Agent Host emitted an event without the required trace envelope. "
                    + "type=\(type) keys=\(keys)"
                    + (diagnostic.isEmpty ? "" : " detail=\(diagnostic)"),
                conversationID: conversationID,
                eventID: optionalString(in: object, keys: ["event_id", "eventId", "id"])
            )
        }
        guard trace.conversationID == conversationID else {
            return protocolFailure(
                "The Agent Host event belongs to a different conversation.",
                conversationID: conversationID,
                eventID: trace.eventID
            )
        }

        let text = trace.payload.summary
            ?? trace.payload.detail
            ?? trace.payload.result?.displayString
            ?? ""
        // Host-level model retries and protocol recoveries are emitted as
        // tool.progress traces without a concrete tool name. Preserve their
        // summary as the event title so the chat surface can render the
        // structured failure/recovery disclosure.
        let title = trace.payload.toolName
            ?? (trace.kind == "tool.progress" ? trace.payload.summary : nil)
        let detail = trace.payload.detail
            ?? trace.payload.summary
            ?? trace.payload.arguments?.displayString
            ?? trace.payload.result?.displayString
        let toolCallID = trace.payload.callID ?? trace.spanID
        let terminalReason = trace.payload.reason
        let contextUsage = trace.payload.usage

        let kind: MacLocalAgentEvent.Kind
        switch trace.kind {
        case "tool.started":
            kind = .toolStarted
        case "tool.progress", "tool.permission.requested", "tool.permission.resolved":
            // Progress and permission transitions update the canonical trace
            // item in place. They are not additional tool executions.
            kind = .status
        case "tool.completed":
            kind = .toolFinished
        case "plan.updated":
            kind = .planUpdated
        case "run.completed", "run.failed", "run.cancelled":
            // Diagnostic traces can accompany the canonical Core terminal.
            // Preserve their detail without emitting a second lifecycle event.
            kind = .status
        case "run.started", "phase.changed", "prompt.composed", "usage.snapshot", "memory.used",
             "validation.updated", "subagent.updated", "context.compacted":
            kind = .status
        default:
            return protocolFailure(
                "The Agent Host emitted an unsupported trace event: \(trace.kind).",
                conversationID: conversationID,
                eventID: trace.eventID
            )
        }

        // Only explicit artifacts belong under the assistant message.
        // Source files touched by a tool stay in the tool timeline as inline
        // code references; promoting them here creates misleading attachments.
        let artifactURLs = Array(NSOrderedSet(
            array: MacAgentArtifactPayload.decode(object)
        )).compactMap { $0 as? URL }
        return MacLocalAgentEvent(
            kind: kind,
            text: text,
            title: title,
            detail: detail,
            isError: trace.isError,
            terminalReason: terminalReason,
            eventID: trace.eventID,
            toolCallID: toolCallID,
            contextUsedTokens: contextUsage?.usedTokens,
            contextWindowTokens: contextUsage?.contextWindowTokens,
            contextPromptLimit: contextUsage?.promptTokenLimit,
            artifactURLs: artifactURLs,
            trace: trace
        )
    }

    @discardableResult
    static func persistCompactionCheckpointIfNeeded(
        _ event: MacLocalAgentEvent,
        store: MacApplicationPort?
    ) -> MacCanonicalContextAggregate? {
        guard let trace = event.trace,
              trace.kind == "context.compacted",
              let summary = trace.payload.summary,
              let store else {
            return nil
        }
        let normalizedSummary = summary.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedSummary.isEmpty else { return nil }
        let digest = SHA256.hash(data: Data(normalizedSummary.utf8))
            .map { String(format: "%02x", $0) }
            .joined()
        let checkpoint = MacCanonicalContextCheckpoint(
            id: trace.eventID,
            conversationID: trace.conversationID,
            runID: trace.runID,
            modelProfileID: "default",
            policyVersion: "agent-core-v1",
            coveredThroughMessageID: nil,
            coveredThroughSequence: nil,
            summaryPartID: nil,
            retainedTailStartMessageID: nil,
            tokenUsage: MacCanonicalContextTokenUsage(
                estimated: Int64(max(1, normalizedSummary.utf8.count / 4)),
                measured: nil,
                window: 0,
                promptLimit: 0
            ),
            payloadDigest: digest,
            createdAtMs: Int64(
                ((parseDate(trace.emittedAt) ?? .now).timeIntervalSince1970 * 1_000).rounded()
            )
        )
        let component = MacCanonicalContextComponent(
            id: "\(trace.eventID):summary",
            checkpointID: trace.eventID,
            kind: "conversation_summary",
            sourceID: nil,
            sourceURI: nil,
            title: nil,
            mimeType: "text/plain",
            authorization: "not_required",
            observedAtMs: checkpoint.createdAtMs,
            maximumAgeMinutes: nil,
            freshness: "fresh",
            contentReference: MacCanonicalContextContentReference(
                type: "inline_preview",
                text: normalizedSummary,
                truncated: false,
                artifactID: nil,
                blobID: nil
            ),
            contentDigest: digest,
            estimatedTokens: checkpoint.tokenUsage.estimated,
            provenance: MacCanonicalContextProvenance(
                providerID: "agent-core",
                collectionMethod: "compaction",
                sourceRevision: nil,
                disclosedToModel: true
            )
        )
        return try? store.commitContextCheckpoint(
            MacCanonicalContextCommit(checkpoint: checkpoint, components: [component])
        )
    }

    /// Internal transport attestation, never exposed in the model's tool catalog.
    /// The process still must present its exact captured channel, and the
    /// supervisor and canonical profile are rechecked before every physical call.
    static func validateNativeModelChannel(_ arguments: [String: Any],
                                          endpoint: MacMLXOpenAIService.Endpoint,
                                          applicationPort: MacApplicationPort) throws {
        guard let route = arguments["native_model_route"] as? [String: Any],
              NSDictionary(dictionary: route).isEqual(to: try endpoint.nativeRoute()),
              arguments["endpoint"] as? String == endpoint.baseURL,
              arguments["model"] as? String == endpoint.modelID,
              arguments["credential_sha256"] as? String == MacRunModelCredential.digest(endpoint.apiKey) else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
        try MacMLXOpenAIService.shared.validate(endpoint, applicationPort: applicationPort)
    }

    private static func decodeComputerBridgeRequest(_ line: String) -> MacComputerBridgeRequest? {
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              object["type"] as? String == "device_tool_request",
              let id = object["request_id"] as? String,
              let sessionID = object["session_id"] as? String,
              let toolName = object["tool_name"] as? String else { return nil }
        return MacComputerBridgeRequest(
            id: id,
            sessionID: sessionID,
            toolName: toolName,
            arguments: object["arguments"] as? [String: Any] ?? [:]
        )
    }

    /// `FileHandle.AsyncBytes.lines` can stop delivering after an initial burst
    /// from a long-lived child process. Native tool requests share stdout with
    /// ordinary Agent Host events, so frame the single stream explicitly.
    private static func lineStream(
        from handle: FileHandle,
        processID _: Int32
    ) -> AsyncThrowingStream<String, Error> {
        // Each stream owns a duplicate descriptor and an event-driven reader.
        // Detached polling readers can accumulate across many sequential Host
        // runs and delay protocol delivery under a full XCTest bundle.
        let descriptor = Darwin.dup(handle.fileDescriptor)
        if descriptor >= 0 {
            let flags = Darwin.fcntl(descriptor, F_GETFL)
            if flags >= 0 {
                _ = Darwin.fcntl(descriptor, F_SETFL, flags | O_NONBLOCK)
            }
        }
        return AsyncThrowingStream { continuation in
            guard descriptor >= 0 else {
                continuation.finish(
                    throwing: POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
                )
                return
            }
            let queue = DispatchQueue(
                label: "com.gaojc.Nola.agent-host-pipe.\(descriptor)",
                qos: .utility
            )
            let source = DispatchSource.makeReadSource(
                fileDescriptor: descriptor,
                queue: queue
            )
            var pending = Data()
            source.setEventHandler {
                var buffer = [UInt8](repeating: 0, count: 64 * 1_024)
                while true {
                    let count = buffer.withUnsafeMutableBytes { bytes in
                        Darwin.read(descriptor, bytes.baseAddress, bytes.count)
                    }
                    if count == 0 {
                        if !pending.isEmpty {
                            continuation.yield(String(decoding: pending, as: UTF8.self))
                            pending.removeAll()
                        }
                        source.cancel()
                        return
                    }
                    if count < 0 {
                        if errno == EINTR { continue }
                        if errno == EAGAIN || errno == EWOULDBLOCK { return }
                        if errno != EBADF && errno != EINVAL {
                            continuation.finish(
                                throwing: POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
                            )
                        }
                        source.cancel()
                        return
                    }
                    pending.append(buffer, count: count)
                    while let newline = pending.firstIndex(of: 0x0A) {
                        let line = pending[..<newline]
                        pending.removeSubrange(...newline)
                        continuation.yield(String(decoding: line, as: UTF8.self))
                    }
                    if !pending.isEmpty,
                       (try? JSONSerialization.jsonObject(with: pending)) is [String: Any] {
                        continuation.yield(String(decoding: pending, as: UTF8.self))
                        pending.removeAll(keepingCapacity: true)
                    }
                }
            }
            source.setCancelHandler {
                Darwin.close(descriptor)
                continuation.finish()
            }
            source.resume()
            continuation.onTermination = { _ in
                source.cancel()
                try? handle.close()
            }
        }
    }

    private static func protocolFailure(
        _ message: String,
        conversationID: String,
        eventID: String?
    ) -> MacLocalAgentEvent {
        MacLocalAgentEvent(
            kind: .failed,
            text: message,
            title: String(localized: "Agent Host protocol error"),
            detail: message,
            isError: true,
            terminalReason: "protocol_error",
            eventID: eventID ?? "protocol-error:\(conversationID)"
        )
    }

    private static func optionalString(in object: [String: Any], keys: [String]) -> String? {
        for key in keys {
            if let value = object[key] as? String, !value.isEmpty { return value }
            if let value = object[key],
               JSONSerialization.isValidJSONObject(value),
               let data = try? JSONSerialization.data(withJSONObject: value),
               let text = String(data: data, encoding: .utf8) {
                return text
            }
        }
        return nil
    }

    private static func jsonString(_ value: Any?) -> String? {
        guard let value, JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    private static func parseDate(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) { return date }
        let standard = ISO8601DateFormatter()
        standard.formatOptions = [.withInternetDateTime]
        return standard.date(from: value)
    }
}
