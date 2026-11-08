// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Makes the canonical Nola Store the durable source for macOS chat transcripts.

import Foundation
import Observation

nonisolated enum MacPermissionConfigurationOwnerFacts: Sendable {
    case inactive
    case local(runID: String)
    case remote(MacRemoteSubmissionIdentity)
}

/// Presentation reads inherit cancellation without cancelling the Agent Run.
/// A synchronous service call already in progress must drain; cancellation
/// prevents subsequent read stages and publication of its now-unwanted result.
nonisolated enum MacConversationReadTask {
    static func run<Value: Sendable>(
        priority: TaskPriority = .userInitiated,
        _ operation: @escaping @Sendable () throws -> Value
    ) async throws -> Value {
        try Task.checkCancellation()
        let worker = Task.detached(priority: priority) {
            try Task.checkCancellation()
            let value = try operation()
            try Task.checkCancellation()
            return value
        }
        return try await withTaskCancellationHandler {
            let value = try await worker.value
            try Task.checkCancellation()
            return value
        } onCancel: {
            worker.cancel()
        }
    }
}

nonisolated enum MacConversationProjectionPresentationPolicy {
    /// Canonical revisions remain individually durable. The view waits one or
    /// two display frames before materializing the newest snapshot so bursts of
    /// adjacent revisions produce one SwiftUI invalidation.
    static func coalescingMilliseconds(isActive: Bool) -> Int {
        isActive ? 32 : 0
    }
}

/// Subscription failures do not terminate a Run. Back off reads while preserving
/// cancellation, and report repeated identical failures only once per streak.
nonisolated struct MacProjectionSubscriptionRetryPolicy {
    private(set) var failureCount = 0
    private var lastFailure: String?

    mutating func reset() {
        failureCount = 0
        lastFailure = nil
    }

    mutating func failed(_ signature: String) -> (milliseconds: Int, shouldLog: Bool) {
        let shouldLog = lastFailure != signature
        lastFailure = signature
        failureCount = min(failureCount + 1, 5)
        return (min(250 * (1 << failureCount), 5_000), shouldLog)
    }
}

/// View-local history state. Canonical content and membership come exclusively
/// from Application window queries, never from filtering whole-session events.
@MainActor
@Observable
final class MacConversationWindowModel {
    private let store: MacApplicationPort
    private var generation: UInt64 = 0
    private typealias PreparedWindow = (window: MacCanonicalConversationWindow, messages: [MacChatMessage])
    private struct PendingLoad {
        let conversationID: String
        let beforeOrdinal: Int64?
        let generation: UInt64
        let task: Task<PreparedWindow, Error>
    }
    @ObservationIgnored private var pendingLoad: PendingLoad?
    private var requestedBeforeOrdinal: Int64?
    private(set) var conversationID: String?
    private(set) var window: MacCanonicalConversationWindow? {
        didSet {
            let hasMore = window?.hasMore == true
            if hasEarlierMessages != hasMore { hasEarlierMessages = hasMore }
            let ordinal = window?.beforeOrdinal
            if oldestMessageOrdinal != ordinal { oldestMessageOrdinal = ordinal }
        }
    }
    private(set) var messages: [MacChatMessage] = []
    private(set) var isLoading = false

    // Navigation observes only page boundaries, not every canonical cursor,
    // metadata or context update of this potentially large window.
    private(set) var hasEarlierMessages = false
    private(set) var oldestMessageOrdinal: Int64?
    var isViewingHistory: Bool { requestedBeforeOrdinal != nil }

    /// Diagnostic correlation only; not a new revision or UI-visible frame.
    var projectionMeasurementCursor: MacProjectionApplyMeasurement.Cursor? {
        guard let window else { return nil }
        return .init(generation: generation, eventSequence: window.eventSequence,
            projectionRevision: window.projection.revision)
    }

    init(store: MacApplicationPort) {
        self.store = store
    }

    func select(conversationID: String) async throws {
        try await load(conversationID: conversationID, beforeOrdinal: nil)
    }

    func loadEarlier() async throws {
        guard !isLoading, let conversationID,
              let window, window.hasMore, let beforeOrdinal = window.beforeOrdinal else { return }
        try await load(conversationID: conversationID, beforeOrdinal: beforeOrdinal)
    }

    func returnToLatest() async throws {
        guard let conversationID else { return }
        try await load(conversationID: conversationID, beforeOrdinal: nil)
    }

    func locateCompactionSource(
        request: MacCompactionSourceRequest
    ) async -> MacCompactionSourceNavigator.Outcome {
        guard !isLoading, !request.sourceItemIDs.isEmpty,
              let conversationID, request.conversationID == conversationID,
              request.matches(conversationID: conversationID, messages: messages),
              let initialWindow = window else {
            return .readError
        }
        let expectedGeneration = generation
        let expectedRevision = initialWindow.projection.revision
        let expectedEventSequence = initialWindow.eventSequence
        isLoading = true
        defer {
            if generation == expectedGeneration { isLoading = false }
        }
        let initialRequest = requestedBeforeOrdinal
        var loadedPages: [Int64: PreparedWindow] = [:]
        let initialPage = MacCompactionSourceNavigator.Page(
            requestedBeforeOrdinal: initialRequest,
            beforeOrdinal: initialWindow.beforeOrdinal,
            hasMore: initialWindow.hasMore,
            messages: messages
        )
        let store = store
        let outcome = await MacCompactionSourceNavigator.locate(
            sourceItemIDs: request.sourceItemIDs,
            initialPage: initialPage
        ) { beforeOrdinal in
            let prepared = try await Task.detached(priority: .userInitiated) {
                try Task.checkCancellation()
                let loaded = try store.loadConversationWindow(
                    conversationID, beforeOrdinal: beforeOrdinal
                )
                try Task.checkCancellation()
                guard loaded.projection.revision == expectedRevision,
                      loaded.eventSequence == expectedEventSequence else {
                    throw MacCompactionSourceNavigator.LoadError.snapshotChanged
                }
                let projected = try MacCanonicalConversationRepository.projectMessages(loaded, databaseURL: store.databaseURL)
                return (window: loaded, messages: projected)
            }.value
            loadedPages[beforeOrdinal] = prepared
            return MacCompactionSourceNavigator.Page(
                requestedBeforeOrdinal: beforeOrdinal,
                beforeOrdinal: prepared.window.beforeOrdinal,
                hasMore: prepared.window.hasMore,
                messages: prepared.messages
            )
        }
        guard generation == expectedGeneration, self.conversationID == conversationID else {
            return .cancelled
        }
        if case let .found(_, page) = outcome,
           let request = page.requestedBeforeOrdinal,
           let prepared = loadedPages[request] {
            requestedBeforeOrdinal = request
            window = prepared.window
            if messages != prepared.messages { messages = prepared.messages }
        }
        return outcome
    }

    /// Called by the presentation lifecycle; this object starts no polling loop.
    @discardableResult
    func refreshIfChanged(
        waitMilliseconds: Int = 0,
        coalescingMilliseconds: Int = 0
    ) async throws -> Bool {
        guard !isLoading, let conversationID, let window else { return false }
        let expectedGeneration = generation
        let measurement = MacProjectionApplyMeasurement.begin(session: conversationID,
            stage: .windowRefresh, generation: expectedGeneration)
        var measurementOutcome = MacProjectionApplyMeasurement.Outcome.failed
        defer {
            MacProjectionApplyMeasurement.finish(measurement,
                outcome: generation != expectedGeneration ? .discarded : (Task.isCancelled ? .cancelled : measurementOutcome),
                cursor: projectionMeasurementCursor)
        }
        let beforeOrdinal = requestedBeforeOrdinal
        let store = store
        let queryCancellation = MacApplicationQueryCancellation()
        let query = Task.detached(priority: .utility) { () throws -> PreparedWindow? in
            if waitMilliseconds > 0 {
                let sequence = try store.waitForConversationProjectionCursor(
                    conversationID, afterSequence: window.eventSequence,
                    waitMilliseconds: waitMilliseconds, cancellation: queryCancellation
                )
                guard sequence > window.eventSequence else { return nil }
            }
            let delay = min(max(0, coalescingMilliseconds), 100)
            if delay > 0 {
                try await Task.sleep(for: .milliseconds(delay))
            }
            try Task.checkCancellation()
            guard let updated = try store.loadConversationWindowUpdate(
                conversationID, afterSequence: window.eventSequence,
                beforeOrdinal: beforeOrdinal
            ) else { return nil }
            try Task.checkCancellation()
            let messages = try MacCanonicalConversationRepository.projectMessages(updated, databaseURL: store.databaseURL)
            try Task.checkCancellation()
            return (window: updated, messages: messages)
        }
        let updated = try await withTaskCancellationHandler {
            try await query.value
        } onCancel: {
            queryCancellation.cancel()
            query.cancel()
        }
        try Task.checkCancellation()
        guard generation == expectedGeneration,
              self.window?.eventSequence == window.eventSequence else {
            measurementOutcome = .discarded
            return false
        }
        guard let updated else {
            measurementOutcome = .unchanged
            return false
        }
        self.window = updated.window
        if messages != updated.messages { messages = updated.messages }
        measurementOutcome = .applied
        return true
    }

    func readOriginalText(
        _ reference: MacCanonicalConversationWindow.DeferredTextPart, offsetBytes: UInt64
    ) async throws -> MacCanonicalTextRange {
        guard let conversationID, window?.deferredTextParts.contains(reference) == true else {
            throw MacApplicationServiceError.invalidResponse
        }
        let expectedGeneration = generation
        let store = store
        let queryCancellation = MacApplicationQueryCancellation()
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            return try store.readConversationTextRange(conversationID, messageID: reference.messageID,
                partID: reference.partID, offsetBytes: offsetBytes, cancellation: queryCancellation)
        }
        let range = try await withTaskCancellationHandler { try await task.value } onCancel: {
            queryCancellation.cancel()
            task.cancel()
        }
        try Task.checkCancellation()
        guard generation == expectedGeneration, self.conversationID == conversationID,
              window?.deferredTextParts.contains(reference) == true else { throw CancellationError() }
        guard range.totalUTF8Bytes == reference.totalUTF8Bytes else { throw MacApplicationServiceError.invalidResponse }
        return range
    }

    private func load(conversationID: String, beforeOrdinal: Int64?) async throws {
        let request: PendingLoad
        if let pendingLoad,
           pendingLoad.conversationID == conversationID,
           pendingLoad.beforeOrdinal == beforeOrdinal {
            request = pendingLoad
        } else {
            pendingLoad?.task.cancel()
            generation &+= 1
            let store = store
            request = PendingLoad(
                conversationID: conversationID, beforeOrdinal: beforeOrdinal,
                generation: generation,
                task: Task.detached(priority: .userInitiated) {
                    try Task.checkCancellation()
                    let window = try store.loadConversationWindow(conversationID, beforeOrdinal: beforeOrdinal)
                    try Task.checkCancellation()
                    let messages = try MacCanonicalConversationRepository.projectMessages(window, databaseURL: store.databaseURL)
                    try Task.checkCancellation()
                    return (window: window, messages: messages)
                }
            )
            pendingLoad = request
        }
        let expectedGeneration = request.generation
        let measurement = MacProjectionApplyMeasurement.begin(session: conversationID,
            stage: .windowLoad, generation: expectedGeneration)
        var measurementOutcome = MacProjectionApplyMeasurement.Outcome.failed
        defer {
            MacProjectionApplyMeasurement.finish(measurement,
                outcome: generation != expectedGeneration ? .discarded : (Task.isCancelled ? .cancelled : measurementOutcome),
                cursor: projectionMeasurementCursor)
        }
        if self.conversationID != conversationID {
            self.conversationID = conversationID
            window = nil
            messages = []
            requestedBeforeOrdinal = nil
        }
        isLoading = true
        defer {
            if generation == expectedGeneration {
                isLoading = false
                pendingLoad = nil
            }
        }
        let loaded = try await request.task.value
        try Task.checkCancellation()
        guard generation == expectedGeneration else {
            measurementOutcome = .discarded
            throw CancellationError()
        }
        requestedBeforeOrdinal = beforeOrdinal
        window = loaded.window
        if messages != loaded.messages { messages = loaded.messages }
        measurementOutcome = .applied
    }
}

struct MacStoredConversation: Codable, Sendable {
    let conversation: MacConversation
    let messages: [MacChatMessage]
    let knowledge: MacConversationKnowledge?
    let estimatedUsedTokens: Int?
    let contextUsage: MacCanonicalConversationProjectionV1.ContextUsage?

    init(
        conversation: MacConversation,
        messages: [MacChatMessage],
        knowledge: MacConversationKnowledge? = nil,
        estimatedUsedTokens: Int? = nil,
        contextUsage: MacCanonicalConversationProjectionV1.ContextUsage? = nil
    ) {
        self.conversation = conversation
        self.messages = messages
        self.knowledge = knowledge
        self.estimatedUsedTokens = estimatedUsedTokens
        self.contextUsage = contextUsage
    }
}

@MainActor
final class MacCanonicalConversationRepository {
    private struct StoredMetadata: Codable {
        enum CodingKeys: String, CodingKey, CaseIterable {
            case version, isPinned, promptTemplateID, promptTemplateSelectionIsExplicit
            case modelSelectionID, executionHostRawValue, responseStyleRawValue
            case modelOverrides, contextLimitOverride, memoryEnabledOverride, retrievalEnabledOverride
            case executionBackendRawValue, permissionModeRawValue, externalExecutionSessionIDs
            case externalExecutionModelIDs, externalExecutionModelProviderIDs, browserSnapshot, knowledge
        }
        let version: Int
        var isPinned: Bool
        var promptTemplateID: UUID?
        var promptTemplateSelectionIsExplicit: Bool?
        var modelSelectionID: String?
        var executionHostRawValue: String?
        var responseStyleRawValue: String?
        var modelOverrides: [String: MacConversationModelOverride]?
        var contextLimitOverride: Int?
        var memoryEnabledOverride: Bool?
        var retrievalEnabledOverride: Bool?
        var executionBackendRawValue: String?
        var permissionModeRawValue: String?
        var externalExecutionSessionIDs: [String: String]?
        var externalExecutionModelIDs: [String: String]?
        var externalExecutionModelProviderIDs: [String: String]?
        var browserSnapshot: MacConversationBrowserSnapshot?
        var knowledge: MacConversationKnowledge?

        init(conversation: MacConversation, knowledge: MacConversationKnowledge?) {
            version = 2
            isPinned = conversation.isPinned
            promptTemplateID = conversation.promptTemplateID
            promptTemplateSelectionIsExplicit = conversation.promptTemplateSelectionIsExplicit
            modelSelectionID = conversation.modelSelectionID
            executionHostRawValue = conversation.executionHostRawValue
            responseStyleRawValue = conversation.responseStyleRawValue
            modelOverrides = conversation.modelOverrides
            contextLimitOverride = conversation.contextLimitOverride
            memoryEnabledOverride = conversation.memoryEnabledOverride
            retrievalEnabledOverride = conversation.retrievalEnabledOverride
            executionBackendRawValue = conversation.executionBackendRawValue
            permissionModeRawValue = conversation.permissionModeRawValue
            externalExecutionSessionIDs = conversation.externalExecutionSessionIDs
            externalExecutionModelIDs = conversation.externalExecutionModelIDs
            externalExecutionModelProviderIDs = conversation.externalExecutionModelProviderIDs
            browserSnapshot = conversation.browserSnapshot
            self.knowledge = knowledge
        }

        func applying(to canonical: MacConversation) -> MacConversation {
            var result = canonical
            result.isPinned = isPinned
            result.promptTemplateID = promptTemplateID
            result.promptTemplateSelectionIsExplicit = promptTemplateSelectionIsExplicit
            result.modelSelectionID = modelSelectionID
            result.executionHostRawValue = executionHostRawValue
            result.responseStyleRawValue = responseStyleRawValue
            result.modelOverrides = modelOverrides
            result.contextLimitOverride = contextLimitOverride
            result.memoryEnabledOverride = memoryEnabledOverride
            result.retrievalEnabledOverride = retrievalEnabledOverride
            result.executionBackendRawValue = executionBackendRawValue
            result.permissionModeRawValue = permissionModeRawValue
            result.externalExecutionSessionIDs = externalExecutionSessionIDs
            result.externalExecutionModelIDs = externalExecutionModelIDs
            result.externalExecutionModelProviderIDs = externalExecutionModelProviderIDs
            result.browserSnapshot = browserSnapshot
            return result
        }
    }

    private let store: MacApplicationPort
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private let metadataQueue: DispatchQueue
    // Historical reads return their payload to the caller, but must not retain
    // a second full transcript solely to reject an older asynchronous result.
    private var projectionSequences: [String: UInt64] = [:]
    private var projectionSequenceOrder: [String] = []
    private let maximumTrackedProjectionSequences = 2

    func makeDraftRepository() -> MacConversationDraftRepository {
        MacConversationDraftRepository(store: store)
    }

    func makeAgentRoomService() -> MacAgentRoomService {
        MacAgentRoomService(runtime: store)
    }

    @MainActor func makeRoomRoutingService() -> MacRoomRoutingService { MacRoomRoutingService(application: store) }
    @MainActor func agentModelDefaults() throws -> MacCanonicalJSONValue {
        let catalog = try MacChatModelCatalog.load(application: store)
        let resolution = try MacDefaultChatModelStore(application: store).resolve(candidates: catalog.candidates)
        guard case .resolved(let id, _) = resolution,
              let entry = catalog.entries.first(where: { $0.candidate.profileID == id }) else {
            throw MacApplicationPortError.rejected("Choose an available default chat model before creating an assistant")
        }
        return .object(["modelSelectionID": .string(entry.option.id)])
    }

    @MainActor func agentModelOptions() throws -> [MacChatModelOption] {
        try MacChatModelCatalog.load(application: store).entries.filter {
            $0.candidate.allowedForChat && $0.candidate.availability == .available
        }.map(\.option)
    }

    @MainActor func makeRoomExecutionOwner(contextID: String,
        localRuntime: any MacLocalAgentRuntime, remoteHost: any MacRemoteAgentRunning) -> MacWorkspaceModel {
        MacWorkspaceModel(initialConversationID: contextID, canonicalRepository: self,
            localAgentRuntime: localRuntime, remoteHost: remoteHost, modelApplication: store)
    }

    init(store: MacApplicationPort,
         metadataQueue: DispatchQueue = DispatchQueue(label: "nola.conversation-metadata", qos: .utility)) {
        self.store = store
        self.metadataQueue = metadataQueue
    }

    private func decodeStoredMetadata(_ data: Data) throws -> StoredMetadata {
        guard var object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacApplicationPortError.invalidResponse
        }
        // Sparse cross-client settings need only presentation defaults. Invalid
        // execution intent must propagate, never become an empty configuration.
        if object["version"] == nil { object["version"] = 2 }
        if object["isPinned"] == nil { object["isPinned"] = false }
        object.removeValue(forKey: "browserSnapshot")
        for (canonical, local) in Self.configurationKeyAliases {
            if canonical != "permissionProfile" {
                object.removeValue(forKey: local)
                if let value = object[canonical] { object[local] = value }
            }
        }
        if let storedProfile = object["permissionModeRawValue"] {
            guard let value = storedProfile as? String else {
                throw MacApplicationPortError.invalidResponse
            }
            _ = try MacPermissionMode.configured(value)
        }
        let sanitized = try JSONSerialization.data(withJSONObject: object)
        return try decoder.decode(StoredMetadata.self, from: sanitized)
    }

    var databaseURL: URL {
        store.databaseURL
    }

    func submissionParentMessageID(conversationID: String) throws -> String? {
        try store.loadConversationWindow(conversationID, limit: 1, contentByteLimit: 1).projection.messages.last?.messageID
    }

    var memoryEnabledDefault: Bool {
        MacAssistantPreferences.memoryEnabled(in: store)
    }

    func memoryEnabledDefaultAsync() async -> Bool {
        let store = store
        return await Task.detached(priority: .userInitiated) {
            MacAssistantPreferences.memoryEnabled(in: store)
        }.value
    }

    func retrievalEnabledDefaultAsync() async -> Bool {
        let store = store
        return await Task.detached(priority: .userInitiated) {
            MacAssistantPreferences.retrievalEnabled(in: store)
        }.value
    }

    var userPreferences: String {
        MacAssistantPreferences.userPreferences(in: store)
    }

    func activeCanonicalRunID(conversationID: String) throws -> String? {
        // Execution identity is independent of whether a queued assistant is
        // represented as a running activity in the visible transcript window.
        guard let run = try store.activeRunIdentity(conversationID: conversationID),
              run.kind == "conversation", run.assistantMessageID != nil,
              !run.runID.isEmpty else { return nil }
        return run.runID
    }

    func permissionConfigurationSnapshot(conversationID: String, unresolvedRunIDs: [String],
                                         sourceUnconfirmed: Bool) throws -> MacApplicationPort.PermissionConfigurationSnapshot {
        try store.permissionConfigurationSnapshot(conversationID: conversationID,
            unresolvedRunIDs: unresolvedRunIDs, sourceUnconfirmed: sourceUnconfirmed)
    }

    func activeCanonicalRunOwner(conversationID: String) throws -> (runID: String, assistantMessageID: String)? {
        guard let run = try store.activeRunIdentity(conversationID: conversationID), run.kind == "conversation",
              let assistant = run.assistantMessageID, !assistant.isEmpty else { return nil }
        return (run.runID, assistant)
    }

    func activeCanonicalRunOwnerAsync(conversationID: String) async throws -> (runID: String, assistantMessageID: String)? {
        let store = store
        return try await MacConversationReadTask.run { () throws -> (runID: String, assistantMessageID: String)? in
            guard let run = try store.activeRunIdentity(conversationID: conversationID), run.kind == "conversation",
                  let assistant = run.assistantMessageID, !assistant.isEmpty else { return nil }
            return (run.runID, assistant)
        }
    }

    func permissionConfigurationOwnerFactsAsync(conversationID: String, unresolvedRunIDs: [String],
                                                sourceUnconfirmed: Bool = false) async throws
        -> MacPermissionConfigurationOwnerFacts {
        let store = store
        return try await MacConversationReadTask.run {
            let unresolved = Set(unresolvedRunIDs)
            let snapshot = try store.permissionConfigurationSnapshot(conversationID: conversationID,
                unresolvedRunIDs: unresolved.sorted(), sourceUnconfirmed: sourceUnconfirmed)
            switch snapshot.owner.kind {
            case "inactive": return .inactive
            case "local":
                guard let runID = snapshot.owner.run_id, !runID.isEmpty else {
                    throw MacApplicationPortError.invalidResponse
                }
                return .local(runID: runID)
            case "remote":
                guard let identity = snapshot.owner.identity,
                      identity.conversation_id == conversationID else {
                    throw MacApplicationPortError.invalidResponse
                }
                return .remote(identity)
            default: throw MacApplicationPortError.invalidResponse
            }
        }
    }

    func remoteSubmissionIntent(conversationID: String) throws -> MacRemoteSubmissionIntent? {
        try store.remoteSubmissionIntent(conversationID: conversationID)
    }

    func canonicalRunTerminal(conversationID: String, runID: String) throws -> MacCanonicalRunTerminal? {
        try store.canonicalRunTerminal(conversationID: conversationID, runID: runID)
    }

    func requestRunCancellation(_ request: MacRunCancellationRequest) throws -> MacRunCancellationReceipt {
        try store.requestRunCancellation(request)
    }

    func inspectRunRecovery(conversationID: String, expectedRunID: String,
                            assistantMessageID: String) throws -> MacRunRecoveryInspection {
        try store.inspectRunRecovery(conversationID: conversationID, expectedRunID: expectedRunID,
                                     assistantMessageID: assistantMessageID)
    }

    func inspectRunRecoveryAsync(conversationID: String, expectedRunID: String,
                                 assistantMessageID: String) async throws -> MacRunRecoveryInspection {
        let store = store
        return try await MacConversationReadTask.run {
            try store.inspectRunRecovery(conversationID: conversationID, expectedRunID: expectedRunID,
                                         assistantMessageID: assistantMessageID)
        }
    }

    func makeWindowModel() -> MacConversationWindowModel {
        MacConversationWindowModel(store: store)
    }

    func displayProjection(conversationID: String) throws -> MacCanonicalConversationProjectionV1 {
        try store.loadConversationWindow(conversationID).projection
    }

    func displayProjectionAsync(conversationID: String) async throws -> MacCanonicalConversationProjectionV1 {
        let backgroundStore = store
        let projection = try await MacConversationReadTask.run {
            try backgroundStore.loadConversationWindow(conversationID).projection
        }
        try Task.checkCancellation()
        return projection
    }

    func conversationIDs() throws -> [String] {
        try store.listConversations().map(\.id)
    }

    func loadCanonicalConversationAsync(
        conversationID: String
    ) async throws -> MacCanonicalConversation? {
        let backgroundStore = store
        return try await MacConversationReadTask.run(priority: .utility) {
            try backgroundStore.loadConversation(conversationID)
        }
    }

    @discardableResult
    func upsertPermissionGrant(
        _ input: MacCanonicalPermissionGrantInput
    ) throws -> MacCanonicalPermissionGrant {
        try store.upsertPermissionGrant(input)
    }

    /// Loads lightweight sidebar rows and eagerly projects only the explicitly
    /// selected transcript. A long-lived local-first archive must not retain a
    /// historical message merely because its sidebar row is visible.
    func loadSidebar(eagerConversationID: String? = nil) throws -> [MacStoredConversation] {
        try loadArchives(
            eagerConversationID: eagerConversationID,
            loadsAllTranscripts: false
        )
    }

    /// Loads complete canonical archives for export and backup. This operation
    /// is intentionally separate from sidebar restoration so a caller cannot
    /// accidentally export metadata-only conversations.
    func loadAll() throws -> [MacStoredConversation] {
        try loadArchives(eagerConversationID: nil, loadsAllTranscripts: true)
    }

    private func loadArchives(
        eagerConversationID: String?,
        loadsAllTranscripts: Bool
    ) throws -> [MacStoredConversation] {
        var archives: [MacStoredConversation] = []
        for summary in try store.listConversations() {
            do {
                let configuration = try store.loadConversationMetadata(summary.id)
                if !loadsAllTranscripts, eagerConversationID != summary.id,
                   let settings = configuration?.settingsJSON,
                   let object = try JSONSerialization.jsonObject(with: Data(settings.utf8)) as? [String: Any],
                   object["agentRoomOwner"] != nil {
                    continue
                }
                let metadata = try configuration?.settingsJSON
                    .map { try decodeStoredMetadata(Data($0.utf8)) }
                let canonical = MacConversation(
                    id: summary.id,
                    title: summary.title,
                    preview: "",
                    updatedAt: Self.date(milliseconds: summary.updatedAtMs),
                    isPinned: false,
                    isPersisted: true
                )
                var conversation = metadata?.applying(to: canonical) ?? canonical
                conversation.configurationRevision = configuration?.configurationRevision
                conversation.title = summary.title
                conversation.updatedAt = Self.date(milliseconds: summary.updatedAtMs)
                conversation.isPersisted = true
                let shouldLoadTranscript = loadsAllTranscripts
                    || eagerConversationID == summary.id
                let messages: [MacChatMessage]
                let contextUsage: MacCanonicalConversationProjectionV1.ContextUsage?
                if shouldLoadTranscript {
                    let loaded = try Self.loadJournalTranscript(store: store, conversationID: summary.id)
                    recordProjectionSequence(loaded.snapshot.sequence, conversationID: summary.id)
                    messages = loaded.messages
                    contextUsage = loaded.snapshot.projection.contextUsage
                } else {
                    messages = []
                    contextUsage = nil
                }
                if let latestUserText = messages.last(where: { $0.role == .user })?.text {
                    conversation.preview = String(latestUserText.prefix(120))
                }
                archives.append(
                    MacStoredConversation(
                        conversation: conversation,
                        messages: messages,
                        knowledge: metadata?.knowledge,
                        contextUsage: contextUsage
                    )
                )
            } catch {
                Logger.shared.log(
                    "Skipped an unreadable canonical conversation",
                    level: .error,
                    category: "MacConversationStore",
                    metadata: [
                        "conversation": String(summary.id.prefix(12)),
                        "error": error.localizedDescription,
                    ]
                )
            }
        }
        return archives
    }

    func load(conversationID: String, windowed: Bool = false) throws -> MacStoredConversation? {
        guard let summary = try store.listConversations().first(where: { $0.id == conversationID }) else {
            return nil
        }
        let loaded = try Self.loadJournalTranscript(store: store, conversationID: conversationID, windowed: windowed)
        let projection = loaded.snapshot.projection
        recordProjectionSequence(loaded.snapshot.sequence, conversationID: conversationID)
        let configuration = try store.loadConversationMetadata(conversationID)
        let metadata = try configuration?.settingsJSON
            .map { try decodeStoredMetadata(Data($0.utf8)) }
        let canonical = MacConversation(
            id: projection.conversationID,
            title: projection.title,
            preview: "",
            updatedAt: Self.date(milliseconds: summary.updatedAtMs),
            isPinned: false,
            isPersisted: true
        )
        var conversation = metadata?.applying(to: canonical) ?? canonical
        conversation.configurationRevision = configuration?.configurationRevision
        conversation.title = projection.title
        conversation.updatedAt = Self.date(milliseconds: summary.updatedAtMs)
        conversation.isPersisted = true
        let messages = loaded.messages
        if let latestUserText = messages.last(where: { $0.role == .user })?.text {
            conversation.preview = String(latestUserText.prefix(120))
        }
        return MacStoredConversation(
            conversation: conversation,
            messages: messages,
            knowledge: metadata?.knowledge,
            contextUsage: projection.contextUsage
        )
    }

    func loadAsync(conversationID: String, windowed: Bool = false) async throws -> MacStoredConversation? {
        let backgroundStore = store
        while true {
            try Task.checkCancellation()
            let (snapshot, messages, estimatedUsedTokens, summary, metadataJSON) = try await MacConversationReadTask.run {
                let loaded = try Self.loadJournalTranscript(store: backgroundStore, conversationID: conversationID, windowed: windowed)
                try Task.checkCancellation()
                let snapshot = loaded.snapshot
                let messages = loaded.messages
                let transcriptBytes = messages.reduce(into: 0) { total, message in
                    total += message.text.utf8.count
                }
                // Precise estimation is derived state and cannot block transcript
                // publication. For very large histories use a deterministic byte
                // estimate; the next canonical Core usage snapshot replaces it.
                let estimate = if transcriptBytes > 1_000_000 {
                    max(1, transcriptBytes / 4)
                } else {
                    MacContextTokenEstimator.estimate(
                        messages: messages,
                        draft: "",
                        promptTemplate: "",
                        userPreferences: ""
                    )
                }
                try Task.checkCancellation()
                let summary = try backgroundStore.listConversations().first { $0.id == conversationID }
                try Task.checkCancellation()
                let metadataJSON = try backgroundStore.loadConversationMetadata(conversationID)
                return (snapshot, messages, estimate, summary, metadataJSON)
            }
            try Task.checkCancellation()
            // Do not replace a newer subscription state with an older cold load.
            if let current = projectionSequences[conversationID], current > snapshot.sequence {
                // Retry off-main rather than turning a subscription race into a
                // synchronous full-history read on the UI actor.
                continue
            }
            recordProjectionSequence(snapshot.sequence, conversationID: conversationID)
            let projection = snapshot.projection
            guard let summary else { return nil }
            let metadata = try metadataJSON?.settingsJSON
                .map { try decodeStoredMetadata(Data($0.utf8)) }
            let canonical = MacConversation(
                id: projection.conversationID,
                title: projection.title,
                preview: "",
                updatedAt: Self.date(milliseconds: summary.updatedAtMs),
                isPinned: false,
                isPersisted: true
            )
            var conversation = metadata?.applying(to: canonical) ?? canonical
            conversation.configurationRevision = metadataJSON?.configurationRevision
            conversation.title = projection.title
            conversation.updatedAt = Self.date(milliseconds: summary.updatedAtMs)
            conversation.isPersisted = true
            if let latestUserText = messages.last(where: { $0.role == .user })?.text {
                conversation.preview = String(latestUserText.prefix(120))
            }
            return MacStoredConversation(
                conversation: conversation,
                messages: messages,
                knowledge: metadata?.knowledge,
                estimatedUsedTokens: estimatedUsedTokens,
                contextUsage: projection.contextUsage
            )
        }
    }

    func prepareRemoteSubmission(
        conversation: MacConversation,
        userMessage: MacChatMessage,
        assistantMessage: MacChatMessage,
        runID: String,
        sourceHostID: String,
        clientID: String,
        queuedUserCommandID: String? = nil
    ) throws {
        let canonicalUserMessageID = queuedUserCommandID.map { "ui:\($0):user" } ?? userMessage.id.uuidString
        if let command = queuedUserCommandID {
            guard command == runID, MacCanonicalMessageIdentity.stableID(canonicalUserMessageID) == userMessage.id else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
        let userPartID = "\(canonicalUserMessageID):body"
        let canonicalAssistantMessageID = "\(runID):assistant"
        let expectedVisibleID = MacCanonicalMessageIdentity.stableID(
            canonicalAssistantMessageID
        )
        guard assistantMessage.id == expectedVisibleID else {
            throw MacLocalAgentContractError.visibleAssistantIdentityMismatch(
                canonicalID: canonicalAssistantMessageID,
                expected: expectedVisibleID?.uuidString ?? "",
                actual: assistantMessage.id.uuidString
            )
        }
        let assistantPartID = "\(canonicalAssistantMessageID):text"
        let plainUserText = userMessage.attachmentNames.isEmpty
            && userMessage.attachmentURLs.isEmpty
            && userMessage.presentation == nil
        _ = try store.prepareRemoteSubmission(
            MacPrepareRemoteSubmissionInput(
                conversationID: conversation.id,
                conversationTitle: conversation.title,
                submissionID: runID,
                sourceHostID: sourceHostID,
                clientID: clientID,
                userMessageID: canonicalUserMessageID,
                userPartID: userPartID,
                userText: userMessage.text,
                userContentJSON: plainUserText ? nil : try encodedMessage(userMessage),
                assistantMessageID: canonicalAssistantMessageID,
                assistantPartID: assistantPartID,
                contextCheckpointID: nil,
                createdAtMs: Self.milliseconds(userMessage.createdAt)
            )
        )
    }

    func rename(conversationID: String, title: String) throws {
        _ = try store.renameConversation(
            conversationID,
            title: title,
            updatedAtMs: Self.milliseconds(.now)
        )
    }

    /// A nil knowledge argument means this operation does not own a knowledge
    /// edit. Use an explicit `.empty` document to clear knowledge.
    func saveMetadata(
        conversation: MacConversation,
        knowledge: MacConversationKnowledge?
    ) throws {
        let data = try encoder.encode(StoredMetadata(conversation: conversation, knowledge: knowledge))
        try metadataQueue.sync {
            _ = try Self.writeMetadata(data, conversationID: conversation.id,
                expectedRevision: conversation.configurationRevision, store: store)
        }
    }

    /// Capture values on the main actor, enqueue immediately, then return
    /// without waiting for service I/O. Submission order is also shared with
    /// synchronous imports and deletion; callbacks never mutate another save.
    func enqueueMetadataSave(
        conversation: MacConversation,
        knowledge: MacConversationKnowledge?,
        configurationCommitted: @escaping @MainActor @Sendable (Int64) -> Void = { _ in },
        completion: @escaping @MainActor @Sendable (Result<Void, Error>) -> Void
    ) throws {
        let data = try encoder.encode(StoredMetadata(conversation: conversation, knowledge: knowledge))
        let conversationID = conversation.id
        let backgroundStore = store
        metadataQueue.async {
            let result = Result { try Self.writeMetadata(data, conversationID: conversationID,
                expectedRevision: conversation.configurationRevision, store: backgroundStore) }
            Task { @MainActor in
                if case .success(let receipt) = result, receipt.previous == conversation.configurationRevision {
                    configurationCommitted(receipt.current)
                }
                completion(result.map { _ in () })
            }
        }
    }

    nonisolated private static let configurationKeyAliases = [
        "selectedPromptTemplateID": "promptTemplateID", "promptResponseStyle": "responseStyleRawValue",
        "executionBackend": "executionBackendRawValue", "permissionProfile": "permissionModeRawValue",
        "externalExecutionSessions": "externalExecutionSessionIDs"
    ]

    nonisolated private static func writeMetadata(_ data: Data, conversationID: String, expectedRevision: Int64?,
                                                  store: MacApplicationPort) throws -> (previous: Int64, current: Int64) {
        guard let owned = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MacApplicationPortError.rejected("Invalid Mac conversation metadata.")
        }
        var fields = owned
        if let knowledge = fields["knowledge"] as? [String: Any] {
            fields["knowledge"] = knowledge.filter { ["keyPointsMarkdown", "wikiMarkdown"].contains($0.key) }
        }
        fields.removeValue(forKey: "modelSelectionID")
        fields.removeValue(forKey: "modelOverrides")
        fields.removeValue(forKey: "executionHostRawValue")
        // Permission edits have their own sparse CAS command. Archiving stale
        // composer metadata must never submit an unconfirmed permission draft.
        fields.removeValue(forKey: "permissionModeRawValue")
        fields.removeValue(forKey: "version")
        // Browser snapshots are a device presentation, never configuration.
        fields.removeValue(forKey: "browserSnapshot")
        // Knowledge is shared with other clients. A metadata-only save must
        // not replace it with a cached document or interpret omission as clear.
        // Other absent optional Mac fields retain their explicit reset meaning.
        for key in StoredMetadata.CodingKeys.allCases {
            if key == .modelSelectionID || key == .modelOverrides || key == .executionHostRawValue || key == .permissionModeRawValue { continue }
            if key == .version || key == .browserSnapshot { continue }
            if key == .knowledge && owned[key.rawValue] == nil { continue }
            if owned[key.rawValue] == nil { fields[key.rawValue] = NSNull() }
        }
        guard let expectedRevision, let conversation = try store.loadConversationMetadata(conversationID),
              conversation.configurationRevision == expectedRevision else {
            throw MacApplicationPortError.rejected("Conversation is unavailable for metadata editing.")
        }
        for (canonical, local) in configurationKeyAliases {
            if let value = fields.removeValue(forKey: local) { fields[canonical] = value }
        }
        if fields["executionBackend"] is NSNull { fields["executionBackend"] = "nola" }
        let stored = try conversation.settingsJSON.flatMap { $0.data(using: .utf8) }
            .map { try JSONSerialization.jsonObject(with: $0) as? [String: Any] ?? [:] } ?? [:]
        fields = fields.filter { key, value in
            !NSDictionary(dictionary: ["v": value]).isEqual(to: ["v": stored[key] ?? NSNull()])
        }
        guard !fields.isEmpty else { return (expectedRevision, expectedRevision) }
        let patch = String(decoding: try JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys]), as: UTF8.self)
        let result = try store.patchConversationConfiguration(
            MacCanonicalPatchConversationMetadataInput(
                conversationID: conversationID,
                expectedRevision: expectedRevision,
                patchJSON: patch,
                updatedAtMs: Self.milliseconds(.now)
            )
        )
        let snapshot = try result.committedSnapshot()
        guard let revisionValue = snapshot["configurationRevision"],
              let revision = try JSONSerialization.jsonObject(with: JSONEncoder().encode(["v": revisionValue])) as? [String: NSNumber],
              let current = revision["v"]?.int64Value else { throw MacApplicationPortError.invalidResponse }
        return (expectedRevision, current)
    }

    nonisolated struct PermissionConfigurationReceipt: Sendable {
        let mode: MacPermissionMode
        let revision: Int64
    }

    func enqueuePermissionConfiguration(conversationID: String, expectedRevision: Int64, expectedRunID: String?, expectedSourceHostID: String?, mode: MacPermissionMode,
        completion: @escaping @MainActor @Sendable (Result<PermissionConfigurationReceipt, Error>) -> Void) {
        let backgroundStore = store
        metadataQueue.async {
            let result = Result<PermissionConfigurationReceipt, Error> {
                let committed = try backgroundStore.setConversationPermission(.init(conversationID: conversationID,
                    expectedRevision: expectedRevision, expectedRunID: expectedRunID,
                    expectedSourceHostID: expectedSourceHostID, profile: mode.rawValue,
                    updatedAtMs: Self.milliseconds(.now))).committedSnapshot()
                guard expectedRevision < Int64.max,
                      committed["conversationId"] == .string(conversationID),
                      committed["permissionProfile"] == .string(mode.rawValue),
                      let value = committed["configurationRevision"],
                      let decoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(["revision": value])) as? [String: NSNumber],
                      let revision = decoded["revision"]?.int64Value, revision == expectedRevision + 1 else {
                    throw MacApplicationPortError.invalidResponse
                }
                return .init(mode: mode, revision: revision)
            }
            Task { @MainActor in completion(result) }
        }
    }

    func reloadPermissionConfiguration(conversationID: String,
        completion: @escaping @MainActor @Sendable (Result<PermissionConfigurationReceipt, Error>) -> Void) {
        let backgroundStore = store
        metadataQueue.async {
            let result = Result<PermissionConfigurationReceipt, Error> {
                guard let stored = try backgroundStore.loadConversationMetadata(conversationID) else {
                    throw MacApplicationPortError.rejected("Conversation is unavailable for permission editing.")
                }
                let fields = try Self.modelConfigurationFields(stored)
                if let raw = fields["permissionModeRawValue"], !(raw is String) {
                    throw MacApplicationPortError.invalidResponse
                }
                let mode = try MacPermissionMode.configured(fields["permissionModeRawValue"] as? String)
                return .init(mode: mode, revision: try Self.requireConfigurationRevision(stored))
            }
            Task { @MainActor in completion(result) }
        }
    }

    nonisolated struct ModelConfigurationReceipt: Sendable {
        let selectionID: String?
        let overrides: [String: MacConversationModelOverride]?
        let executionHostRawValue: String?
        let revision: Int64
    }

    nonisolated enum ModelConfigurationConflict: Error, LocalizedError {
        case staleSnapshot
        var errorDescription: String? { "Conversation configuration changed. Reload before editing." }
    }

    nonisolated private static func modelConfigurationFields(_ stored: MacCanonicalConversationMetadata) throws -> [String: Any] {
        _ = try requireConfigurationRevision(stored)
        // No settings row is a current owner inheriting defaults, not a
        // malformed document. Present documents must be JSON objects.
        guard let json = stored.settingsJSON else { return [:] }
        guard let fields = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else {
            throw MacApplicationPortError.rejected("Invalid conversation configuration.")
        }
        return fields
    }

    func reloadModelConfiguration(conversationID: String,
        completion: @escaping @MainActor @Sendable (Result<ModelConfigurationReceipt, Error>) -> Void) {
        let backgroundStore = store
        metadataQueue.async {
            let result = Result<ModelConfigurationReceipt, Error> {
                guard let stored = try backgroundStore.loadConversationMetadata(conversationID) else {
                    throw MacApplicationPortError.rejected("Conversation configuration is unavailable.")
                }
                let fields = try Self.modelConfigurationFields(stored)
                let overrides = try (fields["modelOverrides"] as? [String: Any]).map {
                    try JSONDecoder().decode([String: MacConversationModelOverride].self,
                        from: JSONSerialization.data(withJSONObject: $0))
                }
                return ModelConfigurationReceipt(selectionID: fields["modelSelectionID"] as? String,
                    overrides: overrides, executionHostRawValue: fields["executionHostRawValue"] as? String,
                    revision: try Self.requireConfigurationRevision(stored))
            }
            Task { @MainActor in completion(result) }
        }
    }

    /// One ordered, owner-bound configuration edit. UI changes only after the
    /// canonical transaction succeeds; metadata saves never own these fields.
    func enqueueModelConfiguration(
        conversation: MacConversation, selectionID: String?, host: MacExecutionHost? = nil,
        changesSelection: Bool = true,
        overrideEdit: (key: String, baseline: MacConversationModelOverride, value: MacConversationModelOverride)? = nil,
        completion: @escaping @MainActor @Sendable (Result<ModelConfigurationReceipt, Error>) -> Void
    ) {
        let backgroundStore = store
        metadataQueue.async {
            let result = Result<ModelConfigurationReceipt, Error> {
                guard let expectedRevision = conversation.configurationRevision else {
                    throw ModelConfigurationConflict.staleSnapshot
                }
                guard let stored = try backgroundStore.loadConversationMetadata(conversation.id) else {
                    throw MacApplicationPortError.rejected("Conversation configuration is unavailable.")
                }
                let fields = try Self.modelConfigurationFields(stored)
                guard stored.configurationRevision == expectedRevision,
                      fields["modelSelectionID"] as? String == conversation.modelSelectionID else {
                    throw ModelConfigurationConflict.staleSnapshot
                }
                let receipt: [String: MacCanonicalJSONValue]
                if let edit = overrideEdit {
                    let models = fields["modelOverrides"] as? [String: Any] ?? [:]
                    let current = models[edit.key] as? [String: Any] ?? [:]
                    let decoded = try JSONDecoder().decode(MacConversationModelOverride.self,
                        from: JSONSerialization.data(withJSONObject: current))
                    guard decoded == edit.baseline else {
                        throw ModelConfigurationConflict.staleSnapshot
                    }
                    guard let before = try JSONSerialization.jsonObject(with: JSONEncoder().encode(edit.baseline)) as? [String: Any],
                          let after = try JSONSerialization.jsonObject(with: JSONEncoder().encode(edit.value)) as? [String: Any] else {
                        throw MacApplicationPortError.rejected("Invalid model parameter edit.")
                    }
                    var changes: [String: Any] = [:]
                    for key in Set(before.keys).union(after.keys) {
                        if !NSDictionary(dictionary: ["v": before[key] ?? NSNull()]).isEqual(to: ["v": after[key] ?? NSNull()]) {
                            changes[key] = after[key] ?? NSNull()
                        }
                    }
                    let json = String(decoding: try JSONSerialization.data(withJSONObject: ["modelOverrides": [edit.key: changes]]), as: UTF8.self)
                    receipt = try backgroundStore.patchConversationConfiguration(.init(conversationID: conversation.id,
                        expectedRevision: expectedRevision, patchJSON: json, updatedAtMs: Self.milliseconds(.now))).committedSnapshot()
                } else {
                    let selection: SelectConversationExecutionCommand<MacCanonicalJSONValue>.Selection
                    if changesSelection {
                        selection = selectionID.map { .model(id: $0, parameters: [:], presentation: [:]) } ?? .inherit
                    } else if let host {
                        selection = .location(executionHost: host.rawValue)
                    } else {
                        throw MacApplicationPortError.rejected("Missing execution selection")
                    }
                    receipt = try backgroundStore.selectConversationExecution(.init(input: .init(
                        conversation_id: conversation.id, expected_revision: expectedRevision, selection: selection,
                        updated_at_ms: Self.milliseconds(.now), execution_host: changesSelection ? host?.rawValue : nil))).committedSnapshot()
                }
                guard let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(receipt)) as? [String: Any] else {
                    throw MacApplicationPortError.invalidResponse
                }
                let overrides = try (value["modelOverrides"] as? [String: Any]).map {
                    try JSONDecoder().decode([String: MacConversationModelOverride].self,
                        from: JSONSerialization.data(withJSONObject: $0))
                }
                return ModelConfigurationReceipt(selectionID: value["modelSelectionID"] as? String, overrides: overrides,
                    executionHostRawValue: value["executionHostRawValue"] as? String,
                    revision: (value["configurationRevision"] as? NSNumber)?.int64Value ?? expectedRevision + 1)
            }
            Task { @MainActor in completion(result) }
        }
    }

    nonisolated private static func requireConfigurationRevision(_ metadata: MacCanonicalConversationMetadata) throws -> Int64 {
        guard let revision = metadata.configurationRevision, revision > 0 else { throw MacApplicationPortError.invalidResponse }
        return revision
    }

    func delete(conversationID: String) throws {
        try metadataQueue.sync {
            _ = try store.deleteConversation(conversationID)
        }
    }

    func restoreBackupSnapshot(_ conversations: [MacStoredConversation]) throws {
        let writer = try MacArchiveRestoreWriter(port: store)
        for conversation in conversations {
            try writer.appendProfile(creationInput(for: conversation))
            for (ordinal, message) in conversation.messages.enumerated() {
                guard message.deferredContent == nil else {
                    throw MacApplicationPortError.rejected("Restore requires original message content, not a deferred preview.")
                }
                let plainText = message.attachmentNames.isEmpty && message.attachmentURLs.isEmpty && message.presentation == nil
                try writer.appendMessage(
                    messageInput(message, ordinal: ordinal, includePresentationJSON: !plainText),
                    conversationID: conversation.conversation.id
                )
            }
        }
        try writer.commit()
    }

    func restoreCanonicalBackupSnapshot(
        _ conversations: [MacBackupConversation]
    ) throws {
        try MacArchiveRestoreWriter.restore(conversations, port: store)
    }

    func restoreCanonicalBackupSnapshotAsync(_ conversations: [MacBackupConversation]) async throws {
        try Task.checkCancellation()
        let backgroundStore = store
        let worker = Task.detached(priority: .utility) {
            try MacArchiveRestoreWriter.restore(conversations, port: backgroundStore)
        }
        try await withTaskCancellationHandler {
            try await worker.value
        } onCancel: {
            worker.cancel()
        }
    }

    @discardableResult
    func ensureConversation(_ conversation: MacConversation) throws -> Int64? {
        guard try store.loadConversation(conversation.id) == nil else { return conversation.configurationRevision }
        return try createSnapshot(
            MacStoredConversation(conversation: conversation, messages: [])
        )
    }

    func clear() throws {
        for conversation in try store.listConversations() {
            _ = try store.deleteConversation(conversation.id)
        }
    }

    func contextCheckpointCountAsync(conversationID: String) async throws -> Int {
        let backgroundStore = store
        let worker = Task.detached(priority: .utility) {
            try Task.checkCancellation()
            let count = try backgroundStore.countContextCheckpoints(conversationID)
            try Task.checkCancellation()
            return count
        }
        return try await withTaskCancellationHandler {
            try await worker.value
        } onCancel: {
            worker.cancel()
        }
    }

    @discardableResult
    func commitContextCheckpoint(
        _ input: MacCanonicalContextCommit
    ) throws -> MacCanonicalContextAggregate {
        try store.commitContextCheckpoint(input)
    }

    func conversationSourceOwner(conversationID: String) throws -> AppleConversationSourceOwner {
        try store.conversationSourceOwner(conversationID: conversationID)
    }

    func sessionProjection(conversationID: String) throws -> MacCanonicalSessionProjection? {
        try store.loadSessionProjection(conversationID)
    }

    func sessionProjectionAsync(conversationID: String) async throws -> MacCanonicalSessionProjection? {
        let store = store
        return try await MacConversationReadTask.run { try store.loadSessionProjection(conversationID) }
    }

    func conversationPlanAsync(conversationID: String) async throws -> MacCanonicalConversationPlan? {
        let store = store
        return try await MacConversationReadTask.run { try store.conversationPlan(conversationID) }
    }

    func permissionProjection(conversationID: String) throws -> MacCorePermissionProjection? {
        try Self.permissionProjection(store: store, conversationID: conversationID)
    }

    func permissionProjectionAsync(conversationID: String) async throws -> MacCorePermissionProjection? {
        let store = store
        return try await MacConversationReadTask.run {
            try Self.permissionProjection(store: store, conversationID: conversationID)
        }
    }

    nonisolated private static func permissionProjection(
        store: MacApplicationPort, conversationID: String
    ) throws -> MacCorePermissionProjection? {
        func activeRunID() throws -> String? {
            guard let run = try store.activeRunIdentity(conversationID: conversationID),
                  run.kind == "conversation", run.assistantMessageID != nil,
                  !run.runID.isEmpty else { return nil }
            return run.runID
        }
        guard let runID = try activeRunID() else { return nil }
        try Task.checkCancellation()
        let projection = try store.loadPermissionProjection(conversationID, runID: runID)
        try Task.checkCancellation()
        guard try activeRunID() == runID else {
            throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "Permission Run changed during cold read"))
        }
        return projection
    }

    func conversationProjection(
        conversationID: String
    ) throws -> MacCanonicalConversationProjectionV1 {
        try loadProjection(conversationID: conversationID)
    }

     func conversationContextUsage(
        conversationID: String
    ) throws -> MacCanonicalConversationProjectionV1.ContextUsage {
        try store.loadConversationContextUsage(conversationID)
    }

    private func loadProjection(
        conversationID: String
    ) throws -> MacCanonicalConversationProjectionV1 {
        let snapshot = try Self.loadProjection(store: store, conversationID: conversationID)
        recordProjectionSequence(snapshot.sequence, conversationID: conversationID)
        return snapshot.projection
    }

    private func recordProjectionSequence(
        _ sequence: UInt64,
        conversationID: String
    ) {
        projectionSequences[conversationID] = max(projectionSequences[conversationID] ?? 0, sequence)
        projectionSequenceOrder.removeAll { $0 == conversationID }
        projectionSequenceOrder.append(conversationID)
        while projectionSequenceOrder.count > maximumTrackedProjectionSequences {
            let evicted = projectionSequenceOrder.removeFirst()
            projectionSequences[evicted] = nil
        }
    }

    nonisolated private static func loadProjection(
        store: MacApplicationPort,
        conversationID: String
    ) throws -> MacCanonicalConversationSnapshot {
        try store.loadConversationProjectionSnapshot(conversationID)
    }

    /// Archive reads page canonical rows and retain lazy journal references.
    /// Never silently truncate an export to the selected UI window or hydrate
    /// cumulative answer bodies merely to construct message identities.
    nonisolated private static func loadJournalTranscript(
        store: MacApplicationPort, conversationID: String, windowed: Bool = false
    ) throws -> (snapshot: MacCanonicalConversationSnapshot, messages: [MacChatMessage]) {
        try Task.checkCancellation()
        let newest = try store.loadConversationWindow(conversationID)
        try Task.checkCancellation()
        var page = newest
        var messages = try projectMessages(page, databaseURL: store.databaseURL)
        var visited = Set<Int64>()
        while !windowed && page.hasMore {
            try Task.checkCancellation()
            guard let before = page.beforeOrdinal, visited.insert(before).inserted else {
                throw MacApplicationServiceError.invalidResponse
            }
            page = try store.loadConversationWindow(conversationID, beforeOrdinal: before)
            try Task.checkCancellation()
            guard page.eventSequence == newest.eventSequence,
                  page.projection.revision == newest.projection.revision else {
                throw MacCompactionSourceNavigator.LoadError.snapshotChanged
            }
            messages = try projectMessages(page, databaseURL: store.databaseURL) + messages
        }
        return (.init(projection: newest.projection, sequence: newest.eventSequence), messages)
    }

    func searchConversationAsync(
        conversationID: String,
        query: String,
        limit: Int = 12
    ) async throws -> [MacConversationRecallResult] {
        let backgroundStore = store
        return try await MacConversationReadTask.run {
            try Task.checkCancellation()
            let matches = try backgroundStore.searchConversationText(
                conversationID,
                query: query.trimmingCharacters(in: .whitespacesAndNewlines),
                limit: min(20, max(1, limit))
            )
            try Task.checkCancellation()
            return matches.map { match in
                MacConversationRecallResult(
                    id: match.partID,
                    source: .originalMessage,
                    role: MacChatMessage.Role(rawValue: match.role),
                    content: match.snippet,
                    snippet: match.snippet,
                    messageID: match.messageID,
                    partID: match.partID,
                    messageOrdinal: match.messageOrdinal,
                    startCharacter: match.startCharacter
                )
            }
        }
    }

    private func createSnapshot(_ storedConversation: MacStoredConversation) throws -> Int64? {
        let messages = try messageInputs(for: storedConversation)
        let batches = try importBatches(messages)
        let result = try store.createConversation(creationInput(for: storedConversation))
        guard result.created else { return nil }
        guard !messages.isEmpty else { return result.initialConfigurationRevision }
        var expectedMessageCount = 0
        for (batchIndex, batch) in batches.enumerated() {
            let importResult: MacCanonicalTranscriptImportResult
            do {
                importResult = try store.importConversationTranscript(
                    MacCanonicalImportConversationTranscriptInput(
                        importID: "backup-restore:\(storedConversation.conversation.id):\(batchIndex)",
                        source: "backup_restore",
                        conversationID: storedConversation.conversation.id,
                        expectedMessageCount: expectedMessageCount,
                        messages: batch,
                        importedAtMs: Self.milliseconds(storedConversation.conversation.updatedAt)
                    )
                )
            } catch {
                throw MacApplicationPortError.rejected(
                    "Transcript import batch \(batchIndex) at message \(expectedMessageCount) failed: \(error.localizedDescription)"
                )
            }
            expectedMessageCount = importResult.messageCount
        }
        return result.initialConfigurationRevision
    }

    private func importBatches(
        _ messages: [MacCanonicalMessageInput]
    ) throws -> [[MacCanonicalMessageInput]] {
        let maximumBatchBytes = 12 * 1_024 * 1_024
        var batches: [[MacCanonicalMessageInput]] = []
        var batch: [MacCanonicalMessageInput] = []
        var batchBytes = 1_024
        for message in messages {
            let messageBytes = try encoder.encode(message).count
            guard messageBytes + 1_024 < maximumBatchBytes else {
                throw MacApplicationPortError.rejected(
                    "Backup message \(message.id) exceeds the Application Service import frame limit."
                )
            }
            if !batch.isEmpty, batchBytes + messageBytes >= maximumBatchBytes {
                batches.append(batch)
                batch = []
                batchBytes = 1_024
            }
            batch.append(message)
            batchBytes += messageBytes
        }
        if !batch.isEmpty {
            batches.append(batch)
        }
        return batches
    }

    private func creationInput(for storedConversation: MacStoredConversation) throws -> MacCanonicalCreateConversationInput {
        let localSettings = try String(
            decoding: encoder.encode(
                StoredMetadata(
                    conversation: storedConversation.conversation,
                    knowledge: storedConversation.knowledge
                )
            ),
            as: UTF8.self
        )
        guard var settingsObject = try JSONSerialization.jsonObject(with: Data(localSettings.utf8)) as? [String: Any] else {
            throw MacApplicationPortError.invalidResponse
        }
        for (canonical, local) in Self.configurationKeyAliases where canonical != "permissionProfile" {
            if let value = settingsObject.removeValue(forKey: local) { settingsObject[canonical] = value }
        }
        settingsObject.removeValue(forKey: "browserSnapshot")
        let settings = String(decoding: try JSONSerialization.data(withJSONObject: settingsObject, options: [.sortedKeys]), as: UTF8.self)
        let createdAtMs = storedConversation.messages.first
            .map { Self.milliseconds($0.createdAt) }
            ?? Self.milliseconds(storedConversation.conversation.updatedAt)
        let updatedAtMs = max(
            Self.milliseconds(storedConversation.conversation.updatedAt),
            storedConversation.messages.last.map { Self.milliseconds($0.createdAt) } ?? createdAtMs
        )
        return MacCanonicalCreateConversationInput(
            id: storedConversation.conversation.id,
            title: storedConversation.conversation.title,
            settingsJSON: settings,
            createdAtMs: createdAtMs,
            updatedAtMs: updatedAtMs
        )
    }

    private func messageInputs(for storedConversation: MacStoredConversation) throws -> [MacCanonicalMessageInput] {
        try storedConversation.messages.enumerated().map { ordinal, message in
            let plainText = message.attachmentNames.isEmpty
                && message.attachmentURLs.isEmpty
                && message.presentation == nil
            return try messageInput(
                message,
                ordinal: ordinal,
                includePresentationJSON: !plainText
            )
        }
    }

    private func messageInput(
        _ message: MacChatMessage, ordinal: Int, includePresentationJSON: Bool
    ) throws -> MacCanonicalMessageInput {
        let timestamp = Self.milliseconds(message.createdAt)
        return MacCanonicalMessageInput(
            id: message.id.uuidString, runID: nil, role: message.role.rawValue,
            status: "completed", ordinal: Int64(ordinal + 1),
            createdAtMs: timestamp, completedAtMs: timestamp,
            parts: [MacCanonicalMessagePartInput(
                id: "\(message.id.uuidString):text", kind: "text", ordinal: 1, state: "completed",
                contentText: message.text,
                contentJSON: includePresentationJSON ? try encodedMessage(message) : nil,
                contentDigest: nil, createdAtMs: timestamp, updatedAtMs: timestamp
            )]
        )
    }

    private func encodedMessage(_ message: MacChatMessage) throws -> String {
        String(decoding: try encoder.encode(message), as: UTF8.self)
    }

    nonisolated static func projectMessages(_ window: MacCanonicalConversationWindow, databaseURL: URL? = nil) throws -> [MacChatMessage] {
        let deferredIDs = Set(window.deferredTextParts.map(\.messageID))
        // A journal-owned row intentionally has no inline body. Validate its
        // owner before applying inline visibility rules, not after discarding it.
        var journalIDs = Set<String>()
        for cursor in window.assistantJournalHeads {
            guard cursor.identity.conversation_id == window.projection.conversationID,
                  journalIDs.insert(cursor.identity.assistant_message_id).inserted,
                  window.projection.messages.contains(where: {
                      $0.messageID == cursor.identity.assistant_message_id && $0.role == .assistant
                  }) else { throw MacApplicationServiceError.invalidResponse }
        }
        guard journalIDs.isEmpty || databaseURL != nil else {
            throw MacApplicationServiceError.invalidResponse
        }
        var messages = try projectMessages(window.projection, deferredMessageIDs: deferredIDs,
                                           journalMessageIDs: journalIDs)
        if let databaseURL {
            // Presentation residency only; source/Host access remains checked
            // by the original operation port when a reader/action is used.
            let source = MacCanonicalPresentationSource(databaseURL: databaseURL,
                sourceOwner: try MacApplicationPort(databaseURL: databaseURL)
                    .conversationSourceOwner(conversationID: window.projection.conversationID))
            for index in messages.indices { messages[index].canonicalPresentation?.source = source }
            var seen = Set<UUID>()
            for cursor in window.assistantJournalHeads {
                guard cursor.identity.conversation_id == window.projection.conversationID,
                      let id = MacCanonicalMessageIdentity.stableID(cursor.identity.assistant_message_id),
                      seen.insert(id).inserted,
                      let index = messages.firstIndex(where: { $0.id == id && $0.role == .assistant }) else {
                    throw MacApplicationServiceError.invalidResponse
                }
                messages[index].journalReference = MacJournalMessageReference(databaseURL: databaseURL, cursor: cursor)
            }
        }
        // Store independently reports Part lifecycle and current original-text
        // eligibility. A final Part alone does not authorize an unversioned read.
        let readableParts = Dictionary(uniqueKeysWithValues: window.projection.messages.map { message in
            (message.messageID, Set(message.parts.compactMap { part -> String? in
                switch part {
                case .text(let text): return text.isFinal ? text.partID : nil
                case .historical(let history):
                    return history.projection.canonicalKind == "text" ? history.partID : nil
                default: return nil
                }
            }))
        })
        for index in messages.indices {
            if let canonical = window.projection.messages.first(where: {
                MacCanonicalMessageIdentity.stableID($0.messageID) == messages[index].id
            }) {
                messages[index].historicalDetails = canonical.parts.compactMap { part in
                    guard case .historical(let history) = part else { return nil }
                    return MacHistoricalPartSelection(owner: .init(
                        conversationID: window.projection.conversationID,
                        messageID: canonical.messageID, partID: history.partID),
                        canonicalRevision: history.projection.canonicalRevision,
                        windowSequence: window.eventSequence)
                }
            }
            let parts = window.deferredTextParts.filter {
                MacCanonicalMessageIdentity.stableID($0.messageID) == messages[index].id
                    && $0.isFinal
                    && $0.canReadOriginal
                    && readableParts[$0.messageID]?.contains($0.partID) == true
            }
            if let first = parts.first {
                messages[index].deferredContent = MacDeferredMessageContent(
                    conversationID: window.projection.conversationID, messageID: first.messageID, parts: parts
                )
            }
        }
        return messages
    }

    nonisolated static func projectMessages(
        _ projection: MacCanonicalConversationProjectionV1,
        deferredMessageIDs: Set<String> = [],
        journalMessageIDs: Set<String> = []
    ) throws -> [MacChatMessage] {
        var messagesByID: [String: MacCanonicalConversationProjectionV1.Message] = [:]
        for message in projection.messages {
            guard messagesByID.updateValue(message, forKey: message.messageID) == nil else {
                throw DecodingError.dataCorrupted(.init(
                    codingPath: [],
                    debugDescription: "Conversation projection contains duplicate message identity \(message.messageID)."
                ))
            }
        }
        var orderedIDs = projection.surface.committedMessageIDs
        if let liveMessageID = projection.surface.liveMessageID {
            orderedIDs.append(liveMessageID)
        }
        guard Set(orderedIDs).count == orderedIDs.count,
              Set(orderedIDs) == Set(messagesByID.keys) else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: [],
                debugDescription: "Conversation surface does not cover the projected message set exactly."
            ))
        }
        return try orderedIDs.compactMap { messageID in
            guard let message = messagesByID[messageID] else { return nil }
            // System messages remain available in the canonical projection for
            // audit and replay, but are not user-visible chat timeline rows.
            guard message.role != .system else { return nil }
            let hasVisiblePart = message.parts.contains { part in
                switch part {
                case let .text(value):
                    return value.isDisclosed && !value.markdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                case let .historical(value):
                    if value.projection.canonicalKind == "text" {
                        return !(value.projection.visibleText ?? "")
                            .trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    }
                    if value.projection.canonicalKind == "reasoning" {
                        return !(value.projection.decodedText ?? "")
                            .trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    }
                    if value.projection.canonicalKind == "tool" {
                        return historicalTool(value) != nil
                    }
                    // Unsupported envelopes and legacy tool-shaped metadata
                    // remain available through the explicit history reader.
                    // They do not create empty rows in the transcript.
                    return false
                case let .tool(value):
                    return !isProtocolPlumbingTool(value.capabilityID)
                default:
                    return true
                }
            }
            let showsCanonicalPlaceholder = message.messageID == projection.surface.liveMessageID
                && projection.surface.showAssistantPlaceholder
            guard message.role != .assistant
                    || hasVisiblePart
                    || showsCanonicalPlaceholder
                    || message.state == .failed
                    || message.state == .cancelled
                    || journalMessageIDs.contains(messageID)
                    || deferredMessageIDs.contains(messageID) else {
                return nil
            }
            return try projectMessage(
                message,
                showAssistantPlaceholder: showsCanonicalPlaceholder
            )
        }
    }

    nonisolated static func projectMessage(
        _ message: MacCanonicalConversationProjectionV1.Message,
        showAssistantPlaceholder: Bool = false
    ) throws -> MacChatMessage {
            for case .tool(let tool) in message.parts {
                if let fullOutput = tool.fullOutput,
                   fullOutput.messageID != message.messageID || fullOutput.partID != tool.partID || fullOutput.toolCallID != tool.toolCallID {
                    throw MacApplicationPortError.rejected("Full result belongs to another canonical Tool Part.")
                }
            }
            let role: MacChatMessage.Role
            switch message.role {
            case .user: role = .user
            case .assistant: role = .assistant
            case .system: role = .system
            }

            let text = message.parts.compactMap { part -> String? in
                switch part {
                case let .text(value): return value.isDisclosed ? value.markdown : nil
                case let .uiAction(value): return value.summary
                // Historical protocol status belongs in the explicit details
                // reader. Only recovered canonical prose belongs in a chat
                // bubble; otherwise status/schema fields leak into user text.
                case let .historical(value):
                    return ["text", "ui_action"].contains(value.projection.canonicalKind)
                        ? value.projection.visibleText
                        : nil
                default: return nil
                }
            }.joined()
            let attachments = message.parts.compactMap { part -> (String, URL, String)? in
                guard case let .attachment(value) = part,
                      let url = URL(string: value.uri) else { return nil }
                return (value.name, url, part.partID)
            }
            let attachmentMetadata = message.parts.compactMap { part -> MacCorePartArtifact? in
                guard case let .attachment(value) = part else { return nil }
                return MacCorePartArtifact(uri: value.uri, name: value.name,
                    mimeType: value.mimeType, operation: value.operation)
            }
            let hasSemanticAssistantParts = message.parts.contains { part in
                switch part {
                case .text, .attachment:
                    false
                case let .historical(value):
                    (value.projection.canonicalKind == "reasoning"
                        && value.projection.decodedText != nil)
                        || historicalTool(value) != nil
                default:
                    true
                }
            }
            // Streaming overlays keep their semantic node tree until the turn
            // completes. A cold, completed text-only message has no mutable
            // tail, so rebuilding thousands of reveal nodes is wasted work and
            // makes large conversation switching proportional to every phrase.
            // Render that immutable document directly; retain a presentation
            // only when typed reasoning/tool/source Parts require one.
            let hasAssistantPresentation = hasSemanticAssistantParts
            let presentation: MacAssistantPresentation?
            if role == .assistant && hasAssistantPresentation {
                presentation = projectAssistantPresentation(message)
            } else if role == .assistant && showAssistantPlaceholder {
                presentation = .processing()
            } else {
                presentation = nil
            }
            return MacChatMessage(
                id: MacCanonicalMessageIdentity.stableID(message.messageID) ?? UUID(),
                role: role,
                text: text,
                createdAt: date(milliseconds: message.createdAtMs),
                attachmentNames: attachments.map(\.0),
                attachmentURLs: attachments.map(\.1),
                attachmentMetadata: attachmentMetadata,
                attachmentPartIDs: attachments.map(\.2),
                presentation: presentation,
                deliveryState: deliveryState(message.state),
                canonicalItemIDs: Set([message.messageID] + message.parts.map(\.partID)),
                canonicalMessageID: message.messageID,
                canonicalPresentation: .init(message: message),
                uiSurfaceParts: message.parts.compactMap { if case .uiSurface(let value) = $0 { return value }; return nil },
                uiHistoryReferences: message.parts.compactMap {
                    if case .historical(let value) = $0, let reference = value.projection.uiSurfaceReference,
                       reference.identity.owner.message_id == message.messageID { return reference }
                    return nil
                }
            )
    }

    nonisolated private static func deliveryState(
        _ state: MacCanonicalConversationProjectionV1.MessageState
    ) -> MacChatMessage.DeliveryState {
        switch state {
        case .pending, .running: .running
        case .completed: .completed
        case .failed: .failed
        case .cancelled: .cancelled
        }
    }

    nonisolated private static func projectAssistantPresentation(
        _ message: MacCanonicalConversationProjectionV1.Message
    ) -> MacAssistantPresentation {
        var presentation = MacAssistantPresentation.processing()
        var assembler = MacSemanticStreamAssembler(
            identitySeed: MacCanonicalMessageIdentity.stableID(message.messageID)?.uuidString
        )
        var evidence: [MacCoreEvidencePart] = []
        var hasAssistantText = false

        for part in message.parts {
            switch part {
            case let .historical(value):
                if let text = value.projection.visibleText, value.projection.canonicalKind == "text" {
                    hasAssistantText = true
                    presentation.append(assembler.append(text))
                } else if let text = value.projection.decodedText, value.projection.canonicalKind == "reasoning" {
                    presentation.upsertCoreThinking(partID: value.partID,
                        title: String(localized: "Thinking Process"), detail: text, isComplete: true)
                } else if let tool = historicalTool(value) {
                    presentation.upsertTool(
                        externalID: value.partID,
                        title: tool.capabilityID,
                        detail: tool.output ?? tool.input,
                        state: tool.state,
                        arguments: tool.input,
                        result: tool.output,
                        affectedFiles: tool.affectedFiles
                    )
                }
            case let .text(value):
                guard value.isDisclosed, !value.markdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                    continue
                }
                hasAssistantText = true
                presentation.append(assembler.append(value.markdown))
            case let .reasoning(value):
                let state = traceState(value.state)
                presentation.upsertCoreThinking(
                    partID: value.partID,
                    title: value.title,
                    detail: value.summary ?? "",
                    isComplete: state.isTerminal
                )
                // Preserve the canonical lifecycle, including cancellation
                // and failure, rather than deriving it from the whole message.
                if let index = presentation.traceItems.firstIndex(where: {
                    $0.externalID == value.partID && $0.kind == .thinking
                }) {
                    presentation.traceItems[index].state = state
                }
            case let .tool(value):
                guard !isProtocolPlumbingTool(value.capabilityID) else { continue }
                let output = value.output?.displayString ?? value.outputPreview
                let isRecoverableProtocolCorrection = value.state == .failed
                    && output?.contains("tool_execution_skipped") == true
                presentation.upsertCanonicalTool(
                    partID: value.partID,
                    title: value.title,
                    input: value.input.displayString,
                    state: isRecoverableProtocolCorrection ? .completed : traceState(value.state),
                    metadata: MacAssistantToolMetadata(toolCallID: value.toolCallID,
                        capabilityID: value.capabilityID, origin: value.origin,
                        permission: value.permission, summary: value.summary,
                        outputPreview: value.outputPreview, output: value.output,
                        fullOutput: value.fullOutput, artifacts: value.artifacts,
                        startedAtMs: value.startedAtMs, finishedAtMs: value.finishedAtMs),
                    affectedFiles: Array(Set((value.affectedResources.map(\.uri) + value.artifacts.map(\.uri)).compactMap {
                        filePath(fromResourceURI: $0)
                    })).sorted()
                )
            case let .source(value):
                evidence.append(MacCoreEvidencePart(
                    kind: "source",
                    id: value.partID,
                    messageID: message.messageID,
                    turnID: nil,
                    sourceID: value.sourceID,
                    citationID: nil,
                    title: value.title,
                    uri: value.uri,
                    mimeType: value.mimeType,
                    location: nil,
                    range: nil,
                    evidence: value.evidence
                ))
            case let .citation(value):
                evidence.append(MacCoreEvidencePart(
                    kind: "citation",
                    id: value.partID,
                    messageID: message.messageID,
                    turnID: nil,
                    sourceID: value.sourceID,
                    citationID: value.citationID,
                    title: value.title,
                    uri: value.uri,
                    mimeType: value.mimeType,
                    location: value.location,
                    range: ifPresentRange(value.rangeStart, value.rangeEnd),
                    evidence: nil,
                    originToolCallID: value.originToolCallID
                ))
            case let .task(value):
                presentation.upsertStatus(
                    externalID: value.partID,
                    title: value.displayKey,
                    detail: value.currentStep
                        ?? value.progress.map { "\(Int($0 * 100))%" }
                        ?? value.resultRef
                        ?? "",
                    state: traceState(value.state)
                )
            case let .skill(value):
                presentation.upsertTool(
                    externalID: value.partID,
                    title: value.displayName,
                    detail: value.failure
                        ?? value.resultRef
                        ?? "",
                    state: traceState(value.state),
                    result: value.resultRef
                )
            case let .modelAttempt(value):
                let attempt = AppleModelAttemptPresentation(state: value.state.rawValue,
                    provider: value.provider, model: value.model, successorAttemptID: value.fallbackAttemptID,
                    successorProvider: value.fallbackProvider, successorModel: value.fallbackModel)
                var details = [attempt.modelName, value.failure.message]
                if let nextModel = attempt.successorModelName { details.append(nextModel) }
                let status: MacAssistantTrace.State
                switch attempt.kind {
                case .failed: status = .failed
                case .cancelled: status = .cancelled
                case .pending, .running: status = .running
                case .waiting: status = .waiting
                case .completed: status = .completed
                case .restarted, .routeChanged, .recorded: status = .incomplete
                }
                presentation.upsertStatus(externalID: value.partID, title: attempt.title(),
                    detail: details.filter { !$0.isEmpty }.joined(separator: "\n"), state: status)
            case let .compaction(value):
                presentation.upsertCompactionReference(.init(
                    id: value.partID,
                    summary: value.summary,
                    sourceItemIDs: value.sourceItemIDs,
                    sourceFingerprint: value.sourceFingerprint
                ))
                presentation.upsertStatus(
                    externalID: value.partID,
                    title: String(localized: "Context compacted"),
                    detail: value.summary,
                    state: .completed
                )
            case let .error(value):
                presentation.upsertStatus(
                    externalID: value.partID,
                    title: value.displayKey,
                    detail: value.code,
                    state: value.recoverable ? .waiting : .failed
                )
            case .attachment, .uiSurface, .uiAction:
                continue
            }
        }
        if !hasAssistantText, message.state == .cancelled {
            presentation.append(assembler.append(String(localized: "Stopped")))
        }
        presentation.append(assembler.finish())
        presentation.updateMutableTextNode(nil)
        presentation.replaceSourceReferences(
            MacCoreSourceProjection.project(evidence, messageID: message.messageID)
        )
        if message.state != .running && message.state != .pending {
            // Replaying a stored message is not a new completion event. If an
            // unfinished Part has no recorded finish time, leave it unknown;
            // wall-clock timestamps make identical pages compare unequal and
            // repeatedly invalidate the transcript during history navigation.
            presentation.finish(finishedAt: nil)
        }
        return presentation
    }

    nonisolated private static func traceState(
        _ state: MacCanonicalConversationProjectionV1.ActivityState
    ) -> MacAssistantTrace.State {
        switch state {
        case .pending, .running: .running
        case .waitingForPermission, .waitingForInput: .waiting
        case .completed: .completed
        case .sealed: .incomplete
        case .cancelled: .cancelled
        case .failed: .failed
        }
    }

    private struct HistoricalTool {
        let capabilityID: String
        let state: MacAssistantTrace.State
        let input: String
        let output: String?
        let affectedFiles: [String]
    }

    /// A historical tool is restored only when the Store verified an actual
    /// canonical tool schema and its call identity. Arbitrary legacy metadata
    /// remains available in the history inspector and never becomes a chat row.
    nonisolated private static func historicalTool(
        _ part: MacCanonicalConversationProjectionV1.HistoricalPart
    ) -> HistoricalTool? {
        let projection = part.projection
        guard projection.canonicalKind == "tool",
              projection.binding == "canonical",
              projection.status.state == .supported || projection.status.state == .deferred,
              projection.schema == "agent_part" || projection.schema == "message_part_view",
              let capabilityID = historicalString("capability_id", in: projection),
              !capabilityID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let stateValue = historicalString("state", in: projection),
              let state = MacCanonicalConversationProjectionV1.ActivityState(rawValue: stateValue),
              historicalString(
                projection.schema == "agent_part" ? "call_id" : "tool_call_id",
                in: projection
              ) != nil,
              let input = historicalValue("input", in: projection) else { return nil }
        guard !isProtocolPlumbingTool(capabilityID) else { return nil }
        let outputName = projection.schema == "agent_part" ? "output_preview" : "output"
        let resourcesName = projection.schema == "agent_part" ? "artifacts" : "affected_resources"
        let output = historicalValue(outputName, in: projection)?.displayString
        let traceState = state == .failed && output?.contains("tool_execution_skipped") == true
            ? MacAssistantTrace.State.completed
            : traceState(state)
        return HistoricalTool(
            capabilityID: capabilityID,
            state: traceState,
            input: input.displayString,
            output: output,
            affectedFiles: historicalFilePaths(resourcesName, in: projection)
        )
    }

    nonisolated private static func historicalValue(
        _ name: String,
        in projection: MacCanonicalConversationProjectionV1.HistoricalProjection
    ) -> MacCanonicalJSONValue? {
        guard case let .complete(value)? = projection.fields[name] else { return nil }
        return value
    }

    nonisolated private static func historicalString(
        _ name: String,
        in projection: MacCanonicalConversationProjectionV1.HistoricalProjection
    ) -> String? {
        guard case let .string(value)? = historicalValue(name, in: projection) else { return nil }
        return value
    }

    nonisolated private static func historicalFilePaths(
        _ name: String,
        in projection: MacCanonicalConversationProjectionV1.HistoricalProjection
    ) -> [String] {
        guard case let .array(resources)? = historicalValue(name, in: projection) else { return [] }
        return resources.compactMap { resource in
            guard case let .object(fields) = resource,
                  case let .string(uri)? = fields["uri"] else { return nil }
            return filePath(fromResourceURI: uri)
        }
    }

    nonisolated private static func isProtocolPlumbingTool(_ capabilityID: String) -> Bool {
        let normalized = capabilityID
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        return normalized == "tool_search" || normalized.hasPrefix("tool_output_")
    }

    nonisolated private static func filePath(fromResourceURI uri: String) -> String? {
        guard let url = URL(string: uri), url.isFileURL else { return nil }
        return url.standardizedFileURL.path
    }

    nonisolated private static func ifPresentRange(
        _ start: UInt64?,
        _ end: UInt64?
    ) -> MacCoreCitationRange? {
        guard let start, let end else { return nil }
        return MacCoreCitationRange(start: start, end: end)
    }

    nonisolated private static func milliseconds(_ date: Date) -> Int64 {
        Int64((date.timeIntervalSince1970 * 1_000).rounded())
    }

    nonisolated private static func date(milliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(milliseconds) / 1_000)
    }
}
