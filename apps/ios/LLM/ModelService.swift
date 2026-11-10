// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelService logic in the iOS LLM integration layer.
// - Interfaces:
//   - ModelService
// - APIs:
//   - none
//
//
//  ModelService.swift
//  Nola
//
//  A thin facade that centralizes model provider selection (local vs remote)
//  and exposes a unified LLMClient interface used across the app.
//

import Foundation

public final class ModelService: LLMClient {
    // Current active provider (delegates actual work)
    private let providerLock = NSLock()
    private var installedProvider: LLMClient
    private var installedProviderRevision: UInt64 = 0
    #if DEBUG
    private var testingWebAgentTransport: (baseURL: URL, loader: WebAgentStreamLoader)?
    #endif
    private var provider: LLMClient {
        get {
            providerLock.lock()
            defer { providerLock.unlock() }
            return installedProvider
        }
        set {
            providerLock.lock()
            defer { providerLock.unlock() }
            installedProvider = newValue
            installedProviderRevision &+= 1
        }
    }
    private var activeTasks: [Task<Void, Never>] = []
    private var auxiliaryLedgerOverride: IOSApplicationRuntime?
    private struct AuxiliaryRefreshCursor: Codable, Sendable {
        let source_host_id: String
        let operation_id: String
        var wire: AgentCoreJSONValue { .object(["source_host_id": .string(source_host_id), "operation_id": .string(operation_id)]) }
    }
    private struct AuxiliaryRefreshPage: Decodable {
        struct Reference: Decodable { let source_host_id: String; let operation_id: String; let attempt_id: String }
        let references: [Reference]
        let next_cursor: AuxiliaryRefreshCursor?
    }
    private var auxiliaryRefreshCursors: [UUID: AuxiliaryRefreshCursor] = [:]
    public struct AuxiliarySourceRefreshReport: Sendable {
        public let refreshed: Int
        public let skipped: Int
        public let failed: Int
        public let pending: Int
        public let hasMore: Bool
    }

    public init(provider: LLMClient) {
        self.installedProvider = provider
    }

    /// Freeze the provider for one execution owner. Debug transport bindings
    /// belong to that provider origin and must survive later reconfiguration.
    func makeScopedClient() -> ModelService {
        providerLock.lock()
        defer { providerLock.unlock() }
        let scoped = ModelService(provider: installedProvider)
        #if DEBUG
        scoped.testingWebAgentTransport = testingWebAgentTransport
        #endif
        return scoped
    }

    func makeScopedWebAgentClient(config: WebAgentProvider.Config) -> ModelService {
        let scoped = makeScopedClient()
        #if DEBUG
        if let loader = scoped.testingWebAgentLoader(for: config.baseURL) {
            scoped.provider = WebAgentProvider(config: config, streamLoader: loader)
            return scoped
        }
        #endif
        scoped.provider = WebAgentProvider(config: config)
        return scoped
    }

    func hostAgentSettings(config: WebAgentProvider.Config) async throws -> WebAgentProvider.HostAgentSettings {
        let owner = makeScopedWebAgentClient(config: config)
        guard let host = owner.currentProvider() as? WebAgentProvider else {
            throw IOSApplicationRuntimeError.rejected("Host provider is unavailable")
        }
        return try await host.hostAgentSettings()
    }

    /// A caller may construct LLMSummaryInvocation through the LLMClient
    /// facade. Capture the actual provider instead of wrapping this service's
    /// already-ledgered generateSummary in a second physical Attempt.
    func captureAuxiliaryProvider() -> (provider: LLMClient, revision: UInt64) {
        providerLock.lock()
        defer { providerLock.unlock() }
        return (installedProvider, installedProviderRevision)
    }

    /// Explicit source refresh, including late usage on an already terminal
    /// operation. This never admits or dispatches a new physical request.
    public func refreshAuxiliarySource(scope: LLMAuxiliaryInvocationScope,
        sourceHostID: String, operationID: String) async throws -> Bool {
        let (provider, ledger) = captureAuxiliaryRefreshContext()
        guard let remote = provider as? LLMRemoteAuxiliaryInvocationProviding,
              remote.remoteAuxiliarySourceHostID == sourceHostID, let ledger else {
            throw NolaError.invalidInput(reason: "Auxiliary refresh requires the original paired source Host")
        }
        let owner = scope.wire
        let receipt = try ledger.executeAgentCommand(.object([
            "type": .string("auxiliary_invocation"), "command": .object([
                "type": .string("inspect_source"), "local_owner": owner,
                "source_host_id": .string(sourceHostID), "operation_id": .string(operationID)
            ])
        ]))
        let observation: AgentCoreJSONValue = .object([
            "source_host_id": .string(sourceHostID), "operation_id": .string(operationID),
            "attempt_id": .string("\(operationID):physical"), "receipt": receipt
        ])
        return try await remote.inspectRemoteAuxiliary(observation) { observation in
            _ = try ledger.executeAuxiliaryInvocation([
                "type": .string("observe_source"), "local_owner": owner, "observation": observation
            ])
        }
    }

    private func captureAuxiliaryRefreshContext() -> (LLMClient, IOSApplicationRuntime?) {
        providerLock.lock()
        defer { providerLock.unlock() }
        return (installedProvider, auxiliaryLedgerOverride ?? IOSApplicationRuntime.shared)
    }

    /// Manual context reload only. Read at most sixteen original references;
    /// paginate across reloads, never inspect full history on token updates.
    public func refreshConversationAuxiliarySources(conversationID: UUID) async throws -> AuxiliarySourceRefreshReport {
        let (_, ledger) = captureAuxiliaryRefreshContext()
        guard let ledger else { throw NolaError.invalidInput(reason: "Canonical auxiliary source observations unavailable") }
        let after = auxiliaryRefreshCursor(for: conversationID)
        let raw = try ledger.executeAgentCommand(.object([
            "type": .string("list_conversation_auxiliary_sources"), "conversation_id": .string(conversationID.uuidString),
            "after": after?.wire ?? .null, "limit": .number(16)
        ]))
        let page = try JSONDecoder().decode(AuxiliaryRefreshPage.self, from: JSONEncoder().encode(raw))
        guard page.references.count <= 16 else { throw NolaError.invalidInput(reason: "Auxiliary source reference page exceeds bound") }
        var refreshed = 0, skipped = 0, failed = 0, pending = 0
        for reference in page.references {
            try Task.checkCancellation()
            let (provider, _) = captureAuxiliaryRefreshContext()
            guard let remote = provider as? LLMRemoteAuxiliaryInvocationProviding,
                  remote.remoteAuxiliarySourceHostID == reference.source_host_id else {
                skipped += 1 // Never guess another Host, endpoint or credential.
                continue
            }
            do {
                guard reference.attempt_id == "\(reference.operation_id):physical" else {
                    throw NolaError.invalidInput(reason: "Auxiliary source reference identity mismatch")
                }
                let terminal = try await refreshAuxiliarySource(scope: .conversation(conversationID),
                    sourceHostID: reference.source_host_id, operationID: reference.operation_id)
                refreshed += 1
                if !terminal { pending += 1 }
            } catch is CancellationError { throw CancellationError() }
            catch { failed += 1 }
        }
        setAuxiliaryRefreshCursor(page.next_cursor, for: conversationID)
        return AuxiliarySourceRefreshReport(refreshed: refreshed, skipped: skipped, failed: failed,
            pending: pending, hasMore: page.next_cursor != nil)
    }

    private func auxiliaryRefreshCursor(for conversationID: UUID) -> AuxiliaryRefreshCursor? {
        providerLock.lock(); defer { providerLock.unlock() }
        return auxiliaryRefreshCursors[conversationID]
    }
    private func setAuxiliaryRefreshCursor(_ cursor: AuxiliaryRefreshCursor?, for conversationID: UUID) {
        providerLock.lock(); defer { providerLock.unlock() }
        auxiliaryRefreshCursors[conversationID] = cursor
    }

    /// Capture the provider and revision together. Background work must never
    /// consult the currently selected chat provider between attempts.
    public func summaryInvocation(ownerID: String, operationID: UUID = UUID(),
                                  scope: LLMAuxiliaryInvocationScope? = nil, scenario: LLMAuxiliaryInvocationScenario = .eventSummary) -> LLMSummaryInvocation {
        providerLock.lock()
        defer { providerLock.unlock() }
        let invocation = LLMSummaryInvocation(ownerID: ownerID, operationID: operationID,
            configurationRevision: installedProviderRevision, provider: installedProvider, scope: scope, scenario: scenario)
        return auxiliaryLedgerOverride.map { invocation.usingLedger($0) } ?? invocation
    }

    #if DEBUG
    func installTestingProvider(_ provider: LLMClient) {
        self.provider = provider
    }

    func installTestingAuxiliaryLedger(_ runtime: IOSApplicationRuntime) {
        providerLock.lock()
        auxiliaryLedgerOverride = runtime
        providerLock.unlock()
    }

    /// Bind the fixture at the transport boundary, not to one transient model
    /// selection. Conversation restoration legitimately rebuilds providers.
    func installTestingWebAgentTransport(baseURL: URL, loader: @escaping WebAgentStreamLoader) {
        providerLock.lock()
        defer { providerLock.unlock() }
        testingWebAgentTransport = (baseURL, loader)
    }

    private func testingWebAgentLoader(for baseURL: URL) -> WebAgentStreamLoader? {
        providerLock.lock()
        defer { providerLock.unlock() }
        guard testingWebAgentTransport?.baseURL == baseURL else { return nil }
        return testingWebAgentTransport?.loader
    }
    #endif

    // MARK: - LLMClient
    public func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        try Task.checkCancellation()
        return try await summaryInvocation(ownerID: "local-account").generate(input: input, timeout: timeout)
    }

    // MARK: - Configuration helpers
    public func useLocalProvider(
        model: LMModel? = nil,
        maxTokens: Int = 512,
        temperature: Float = Float(ModelParameters.localDefaults.temperature),
        topP: Float = Float(ModelParameters.localDefaults.topP),
        topK: Int? = nil,
        enableThinking: Bool = true
    ) {
        // If running on Simulator, avoid initializing MLX/local provider which can crash.
        let isSimulator = ProcessInfo.processInfo.environment["SIMULATOR_DEVICE_NAME"] != nil
        if isSimulator {
            Logger.shared.log(.warn, "ModelService", NSLocalizedString("autogen.ModelService.e09bb0e6", comment: "AUTOGEN: Simulator detected — using placeholder local provider instead of MLX"))
            self.provider = PlaceholderLocalProvider.shared
            return
        }

        let local = LocalProvider(model: model, maxTokens: maxTokens, temperature: temperature, topP: topP, topK: topK, enableThinking: enableThinking)
        self.provider = local
    }

    public func usePlaceholderProvider() {
        self.provider = PlaceholderLocalProvider.shared
    }

    public func useOpenAIProvider(config: OpenAIProvider.Config) throws {
        try OpenAIProvider.validateGenerationConfiguration(config)
        let openai = OpenAIProvider(config: config)
        self.provider = openai
        // Installing a prepared provider must not publish credentials. Cloud
        // secrets/profile references belong to IOSCloudCredentialOwner.
    }

    public func useWebAgentProvider(config: WebAgentProvider.Config) {
        #if DEBUG
        if let loader = testingWebAgentLoader(for: config.baseURL) {
            self.provider = WebAgentProvider(config: config, streamLoader: loader)
            return
        }
        #endif
        self.provider = WebAgentProvider(config: config)
        KeychainService.save(service: "nola.llm", account: "webBackendBaseURL", value: Data(config.baseURL.absoluteString.utf8))
        KeychainService.save(service: "nola.llm", account: "webBackendToken", value: Data(config.token.utf8))
        if !config.model.isEmpty {
            KeychainService.save(service: "nola.llm", account: "webBackendModel", value: Data(config.model.utf8))
        }
    }

    // Expose the underlying provider for diagnostics (read-only)
    public func currentProvider() -> LLMClient {
        return provider
    }

    public func generateWebAgentEventStream(input: ModelInput) -> AsyncThrowingStream<WebAgentStreamEvent, Error>? {
        guard let provider = provider as? WebAgentEventStreaming else { return nil }
        return provider.generateAgentEventStream(input: input)
    }

    public func submitWebAgentHostPermissionDecision(_ payload: [String: Any]) async throws {
        guard let provider = provider as? WebAgentProvider else {
            throw NolaError.llm(.invalidResponse(reason: "The active provider is not a Web Agent Host."))
        }
        try await provider.submitHostPermissionDecision(payload)
    }

    public func cancelWebAgentHostTask(
        conversationId: String,
        parentRunId: String,
        taskId: String
    ) async throws -> AppleTaskCancellationReceipt {
        guard let provider = provider as? WebAgentProvider else {
            throw NolaError.llm(.invalidResponse(reason: "The active provider is not a Web Agent Host."))
        }
        return try await provider.cancelHostTask(
            conversationId: conversationId,
            parentRunId: parentRunId,
            taskId: taskId
        )
    }

    public func submitWebAgentHostTaskGuidance(
        conversationId: String,
        parentRunId: String,
        taskId: String,
        message: String
    ) async throws -> AppleTaskGuidanceReceipt {
        guard let provider = provider as? WebAgentProvider else {
            throw NolaError.llm(.invalidResponse(reason: "The active provider is not a Web Agent Host."))
        }
        return try await provider.submitHostTaskGuidance(
            conversationId: conversationId,
            parentRunId: parentRunId,
            taskId: taskId,
            message: message
        )
    }

    public func webAgentHostTaskEvents(
        conversationId: String,
        afterCursor: UInt64
    ) -> AsyncThrowingStream<WebAgentTaskProjectionEvent, Error>? {
        guard let provider = provider as? WebAgentProvider else { return nil }
        return provider.taskProjectionEvents(
            conversationId: conversationId,
            afterCursor: afterCursor
        )
    }

    public func submitWebAgentDeviceToolResult(_ payload: [String: Any]) async throws {
        guard let provider = provider as? WebAgentProvider else {
            throw NolaError.llm(.invalidResponse(reason: "The active provider is not a Web Agent Host."))
        }
        try await provider.submitDeviceToolResult(payload)
    }

    // MARK: - Streaming support
    public func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        let providerName = String(describing: type(of: provider))
        let start = Date()
        let stream = provider.generateSummaryStream(input: input)

        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    for try await chunk in stream {
                        continuation.yield(chunk)
                    }
                    let duration = Date().timeIntervalSince(start)
                    let est = Telemetry.estimateTokens(from: "")
                    Telemetry.recordInference(provider: providerName, duration: duration, estimatedTokens: est, success: true, error: nil)
                    continuation.finish()
                } catch {
                    let duration = Date().timeIntervalSince(start)
                    Telemetry.recordInference(provider: providerName, duration: duration, estimatedTokens: 0, success: false, error: error)
                    continuation.finish(throwing: error)
                }
            }
            activeTasks.append(task)
        }
    }

    public func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
        // Delegate to underlying provider so provider-specific handling (systemPrompt/enableThinking)
        return try await provider.chat(message: message, systemPrompt: systemPrompt, enableThinking: enableThinking, timeout: timeout)
    }

    // Cancel any active streaming/inference tasks
    public func cancelAllRequests() {
        for t in activeTasks {
            t.cancel()
        }
        activeTasks.removeAll()
    }

    // Helper to serialize ModelOutput to JSON string for token estimation
    private func JSONString(of output: ModelOutput) -> String {
        if let d = try? JSONEncoder().encode(output), let s = String(data: d, encoding: .utf8) {
            return s
        }
        return ""
    }
}

extension ModelService: LLMEventStreamingClient {
    public var supportsNativeEventStream: Bool {
        (provider as? LLMEventStreamingClient)?.supportsNativeEventStream == true
    }

    public func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        guard let eventProvider = provider as? LLMEventStreamingClient,
              eventProvider.supportsNativeEventStream else {
            return AsyncThrowingStream { continuation in
                continuation.finish(throwing: NolaError.llm(.invalidResponse(
                    reason: "The active provider does not support typed Agent streaming."
                )))
            }
        }
        return eventProvider.generateEventStream(input: input)
    }
}
