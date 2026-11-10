// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises the iOS Web Host provider across URLSession, encrypted requests, and paced SSE.

import Foundation
import Network
import XCTest
@testable import Nola

@MainActor
final class EndToEndFlowTests: XCTestCase {
    func testRealA2UIPublishesCanonicalSurfaceAndRestoresReadOnlyHistory() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else { throw XCTSkip("Requires isolated real-provider runner") }
        let endpoint = try XCTUnwrap(environment["NOLA_REAL_LLM_ENDPOINT"])
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let wire = try XCTUnwrap(ModelWireProtocol(rawValue: try XCTUnwrap(environment["NOLA_REAL_LLM_WIRE_PROTOCOL"])))
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-ios-a2ui-\(UUID())", isDirectory: true)
        // Retain this isolated acceptance DB for diagnosis; simulator lifecycle owns cleanup.
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let previous = IOSApplicationRuntime.shared
        IOSApplicationRuntime.installShared(runtime)
        defer { IOSApplicationRuntime.installShared(previous) }
        let configured = ModelInfo(id: "a2ui-real-\(UUID())", name: model, source: "cloud", isLocal: false,
            baseURL: endpoint, apiKey: apiKey, upstreamModelId: model, wireProtocol: wire,
            executionCapabilities: .init(toolCalls: true))
        try ModelSelectionManager.addConfiguredCloudModel(configured)
        let published = try XCTUnwrap(ModelSelectionManager.availableModels(using: runtime).first { $0.id == configured.id })
        defer {
            if let account = published.cloudCredential?.account {
                IOSCloudCredentialOwner.shared.delete(account)
            }
        }
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "A2UI real acceptance", language: "en")
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: CanonicalChatMemoryProfileStore())
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.currentConversationId = conversation.id
        manager.selectModel(published, persistAsDefault: false)
        try manager.persistSelectedModelForCurrentConversation(published)
        let approvalTask = Task { @MainActor in
            var resolved = Set<String>()
            while !Task.isCancelled {
                if let request = manager.localAgentPermissionRequest,
                   resolved.insert(request.requestId).inserted {
                    manager.resolveLocalAgentPermission(requestId: request.requestId,
                        request.toolId == "render_ui" ? .allowOnce : .deny)
                }
                try? await Task.sleep(nanoseconds: 50_000_000)
            }
        }
        defer { approvalTask.cancel() }
        _ = try await manager.processMessage("请实际调用 render_ui 生成 A2UI 0.9.1 表单，包含标题 NOLA-A2UI-REAL、姓名输入框和采用event action的提交按钮。严格使用工具schema与支持的catalog，不要仅输出JSON或代码块。")
        let surface = try XCTUnwrap(runtime.listUISurfaces(conversationID: conversation.id.uuidString).first(where: { $0.phase == .ready }))
        XCTAssertEqual(surface.identity.owner.conversation_id, conversation.id.uuidString)
        XCTAssertTrue(surface.nodes.values.contains { $0.component == "TextField" })
        XCTAssertTrue(surface.nodes.values.contains { $0.component == "Button" && !$0.actions.isEmpty })
        let cold = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let historical = try XCTUnwrap(cold.readUISurfaceRevision(identity: surface.identity, revision: surface.revision))
        XCTAssertEqual(historical.phase, .readOnly)
        XCTAssertEqual(historical.revision, surface.revision)
        XCTAssertTrue(historical.nodes.values.allSatisfy { $0.actions.isEmpty && $0.bindings.isEmpty })
    }

    func testRealOpenAIProviderStreamsConfiguredModelWhenExplicitlyEnabled() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real provider request.")
        }
        let endpoint = try XCTUnwrap(
            environment["NOLA_REAL_LLM_ENDPOINT"].flatMap(URL.init(string:))
        )
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let provider = OpenAIProvider(config: .init(
            baseURL: endpoint,
            apiKey: apiKey,
            model: model,
            executionCapabilities: .init(samplingParameters: true),
            temperature: 0,
            maxCompletionTokens: 128
        ))
        let input = ModelInput(
            dateRange: .init(
                start: "2026-07-20T00:00:00Z",
                end: "2026-07-20T00:01:00Z"
            ),
            candidates: [],
            userPrefs: [
                "message": "Return only this exact ASCII token: NOLA_IOS_REAL_OK",
                "enable_thinking": "false",
            ]
        )

        var chunks: [String] = []
        for try await chunk in provider.generateSummaryStream(input: input) {
            chunks.append(chunk)
        }

        // A short exact-token answer may arrive in one SSE content delta.
        // The provider rejects a stream without an explicit terminal event.
        XCTAssertFalse(chunks.isEmpty, "The iOS provider emitted no text from the real stream.")
        XCTAssertEqual(
            chunks.joined().trimmingCharacters(in: .whitespacesAndNewlines),
            "NOLA_IOS_REAL_OK"
        )
    }

    func testRealModelUsesValidatedDataSourceContextAndRecordsDisclosure() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real data-source request.")
        }
        let endpoint = try XCTUnwrap(
            environment["NOLA_REAL_LLM_ENDPOINT"].flatMap(URL.init(string:))
        )
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let fact = "NOLA_IOS_DATA_SOURCE_\(UUID().uuidString.uppercased())"

        let suite = "RealDataSourceE2E-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        let sourceFile = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-real-source-\(UUID().uuidString).json")
        let sourceJSON = """
        {
          "version": "1.0",
          "events": [
            {
              "id": "acceptance-fact",
              "type": "note",
              "title": "Acceptance fact",
              "date": "2026-07-25T00:00:00Z",
              "description": "\(fact)"
            }
          ]
        }
        """
        try Data(sourceJSON.utf8).write(to: sourceFile, options: .atomic)
        defer { try? FileManager.default.removeItem(at: sourceFile) }

        var source = URLDataSource(
            url: sourceFile.absoluteString,
            format: .json,
            kind: .file,
            name: "Real E2E source"
        )
        source.accessPolicy = .defaultEnabled
        var validationContract = URLDataSourceValidationContract(
            sourceID: source.id,
            connectorID: "ios.local-file",
            connectorVersion: 2,
            readerDescription: "ios.local-file -> json -> RawEvent",
            schemaFields: ["events.id", "events.description"],
            redactedSample: "Acceptance fact",
            recordCount: 1,
            maximumAgeMinutes: 60,
            validationState: .verified,
            validatedAt: Date(),
            observedAt: Date()
        )
        validationContract.configureReadRecipe(
            method: .localFileRead,
            operationID: "ios.local-file.read",
            configurationKeys: ["url", "maximumBytes"]
        )
        source.validationContract = validationContract
        let fetcher = URLFetcherService(logger: Logger(), maxURLs: 20, userDefaults: defaults)
        try fetcher.addSource(source)

        let runtimeRoot = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-real-model-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: runtimeRoot) }
        let runtime = try IOSApplicationRuntime(databaseURL: runtimeRoot.appendingPathComponent("state.db"))
        let previousRuntime = IOSApplicationRuntime.shared
        IOSApplicationRuntime.installShared(runtime)
        defer { IOSApplicationRuntime.installShared(previousRuntime) }
        let configuredModel = ModelInfo(
            id: "real-ios-\(UUID().uuidString)", name: model, source: "cloud", isLocal: false,
            baseURL: endpoint.absoluteString, apiKey: apiKey, upstreamModelId: model,
            wireProtocol: .chatCompletions,
            executionCapabilities: .init(samplingParameters: true)
        )
        ModelSelectionManager.addModel(configuredModel)
        XCTAssertNotNil(try runtime.modelProfile(id: configuredModel.id))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "Real data source E2E", language: "en")
        defer { try? store.deleteConversation(conversationId: conversation.id) }
        try store.updateConversationMetadata(
            conversationId: conversation.id,
            language: "en",
            selectedPromptTemplateID: nil,
            dataSourceOverrides: [source.id.uuidString: true]
        )
        let contextManager = DefaultContextManager(
            store: store,
            profileStore: CanonicalChatMemoryProfileStore()
        )
        contextManager.urlFetcher = fetcher
        let provider = OpenAIProvider(config: .init(
            baseURL: endpoint,
            apiKey: apiKey,
            model: model,
            executionCapabilities: .init(samplingParameters: true),
            temperature: 0,
            maxCompletionTokens: 64
        ))
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = store
        container.contextManager = contextManager
        container.llmClient = provider

        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = configuredModel
        manager.currentConversationId = conversation.id

        let response = try await manager.processMessage(
            "Return only the exact description value from the validated conversation data source. Use plain text with no Markdown, quotes, backticks, label, or explanation."
        )
        var normalized = response.trimmingCharacters(in: .whitespacesAndNewlines)
        while normalized.count >= 2,
              let first = normalized.first,
              let last = normalized.last,
              [("`", "`"), ("\"", "\""), ("'", "'"), ("“", "”"), ("‘", "’")].contains(where: { pair in
                  first == Character(pair.0) && last == Character(pair.1)
              }) {
            normalized = String(normalized.dropFirst().dropLast())
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        XCTAssertEqual(normalized, fact, "The model did not return the validated data-source fact")
        XCTAssertEqual(response.components(separatedBy: fact).count - 1, 1,
                       "The model response must contain the validated fact exactly once")

        let usage = try XCTUnwrap(fetcher.usageRecords.first)
        XCTAssertEqual(usage.sourceID, source.id)
        XCTAssertEqual(usage.conversationID, conversation.id)
        XCTAssertEqual(usage.consumer, "chat")
        XCTAssertEqual(usage.purpose, "conversation_context")
        XCTAssertEqual(usage.disclosedToCloud, true)

        let persisted = try store.getMessages(conversationId: conversation.id, limit: 20)
        XCTAssertEqual(persisted.filter { $0.role == .user }.count, 1,
                       "The real user turn was not persisted")
        XCTAssertEqual(persisted.filter { $0.role == .assistant }.count, 1,
                       "The real assistant turn was not persisted")
        XCTAssertEqual(persisted.last?.text, response)
    }

    func testRealModelObservesUploadedSkillEnableEditAndDisableLifecycle() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_LLM_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_LLM_E2E=1 to run the real uploaded-Skill lifecycle.")
        }
        let endpoint = try XCTUnwrap(
            environment["NOLA_REAL_LLM_ENDPOINT"].flatMap(URL.init(string:))
        )
        let apiKey = try XCTUnwrap(environment["NOLA_REAL_LLM_API_KEY"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let markerA = "NOLA_IOS_SKILL_A_\(UUID().uuidString.replacingOccurrences(of: "-", with: ""))"
        let markerB = "NOLA_IOS_SKILL_B_\(UUID().uuidString.replacingOccurrences(of: "-", with: ""))"
        let suite = "RealSkillLifecycleE2E-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        let profileStore = CanonicalChatMemoryProfileStore()
        let store = try LocalConversationStore()
        let contextManager = DefaultContextManager(store: store, profileStore: profileStore)
        let provider = RecordingEventStreamingLLMClient(delegate: OpenAIProvider(config: .init(
            baseURL: endpoint,
            apiKey: apiKey,
            model: model,
            executionCapabilities: .init(samplingParameters: true),
            temperature: 0,
            maxCompletionTokens: 96
        )))
        let container = ServiceContainer(chatMemoryProfileStore: profileStore)
        container.conversationStore = store
        container.contextManager = contextManager
        container.llmClient = provider
        let skillID = UUID()
        var skillEditor: IOSCanonicalSkillEditor?

        func normalized(_ value: String) -> String {
            value.trimmingCharacters(in: CharacterSet(charactersIn: "`'\" \n\r\t"))
        }

        func runSkillTurn(marker: String, enabled: Bool) async throws -> String {
            let saved = try profileStore.saveSkill(
                UserUploadedContextItem(
                    id: skillID,
                    name: "Real lifecycle Skill",
                    content: "When asked for the enabled Skill lifecycle marker, return only this exact token: \(marker)",
                    description: "Provides the enabled Skill lifecycle marker. Invoke this Skill before answering marker requests.",
                    enabled: enabled
                ), expected: skillEditor
            )
            skillEditor = try profileStore.setSkillEnabled(enabled, expected: saved)
            let persisted = container.chatMemoryProfileStore.loadProfile().uploadedSkills
            XCTAssertEqual(persisted.count, 1, "Container and Context must share one Skill profile store")
            XCTAssertEqual(persisted.first?.enabled, enabled)
            let conversation = try store.createConversation(title: "Real Skill lifecycle", language: "en")
            defer { try? store.deleteConversation(conversationId: conversation.id) }
            let prepared = try await contextManager.prepareOutboundContext(
                for: conversation.id,
                maxTokens: 64_000
            )
            let advertisedContext = prepared.map(\.content).joined(separator: "\n")
            XCTAssertEqual(
                advertisedContext.contains("Real lifecycle Skill"),
                enabled,
                "Core context Skill advertisement must match persisted enablement"
            )
            let manager = ChatManager(container: container, restorePersistedState: false)
            manager.selectedModelInfo = ModelInfo(id: model, name: model, source: "cloud", isLocal: false)
            manager.currentConversationId = conversation.id
            let recordedInputOffset = provider.recordedInputs().count
            let response = try await manager.processMessage(
                "Use the enabled Skill named Real lifecycle Skill and return only its exact marker. If that Skill is not enabled, return only NO_ENABLED_SKILL. Do not add Markdown or explanation."
            )
            let turnInputs = provider.recordedInputs().dropFirst(recordedInputOffset)
            let agentToolNames = turnInputs.flatMap { input -> [String] in
                guard input.userPrefs["prepared_core_payload"] == "true",
                      let payload = input.userPrefs["prepared_core_payload_json"],
                      let data = payload.data(using: .utf8),
                      let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let tools = envelope["tools"] as? [[String: Any]] else { return [] }
                return tools.compactMap { tool in
                    (tool["function"] as? [String: Any])?["name"] as? String
                }
            }
            XCTAssertTrue(
                agentToolNames.contains("skill_invoke"),
                "Core must expose the generic Skill protocol independently of individual Skill enablement; tools=\(agentToolNames)"
            )
            let successfulSkillEvents = manager.hostToolEvents.filter {
                $0.status == .succeeded
                    && ($0.name == "skill_invoke" || $0.name == "Real lifecycle Skill")
            }
            XCTAssertEqual(
                successfulSkillEvents.isEmpty,
                !enabled,
                "Skill execution evidence must match persisted enablement; events=\(manager.hostToolEvents)"
            )
            return response
        }

        let first = normalized(try await runSkillTurn(marker: markerA, enabled: true))
        XCTAssertEqual(first, markerA)
        let edited = normalized(try await runSkillTurn(marker: markerB, enabled: true))
        XCTAssertEqual(edited, markerB)
        XCTAssertFalse(edited.contains(markerA))
        let disabled = normalized(try await runSkillTurn(marker: markerB, enabled: false))
        XCTAssertEqual(disabled, "NO_ENABLED_SKILL")
        XCTAssertFalse(disabled.contains(markerB))
    }

    func testWebAgentProviderPreservesAuditSSEWireOrderAndAuthenticatedTransport() async throws {
        let host = try LocalSSEHost()
        let baseURL = try await host.start()
        defer { host.stop() }
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: "integration-mobile-token",
            model: "integration-model",
            clientId: "ios-e2e-client"
        ))
        let input = ModelInput(
            dateRange: .init(
                start: "2026-07-17T00:00:00Z",
                end: "2026-07-17T01:00:00Z"
            ),
            candidates: [],
            userPrefs: ["message": "Run the real SSE integration path"]
        )

        var eventOrder: [String] = []
        // This fixture tests framing/authentication only. Source admission and
        // public summary completion use real Application journals in
        // IOSWebSummaryJournalTests, not these audit-only frames.
        let frozen = try JSONDecoder().decode(AgentCoreJSONValue.self,
            from: JSONSerialization.data(withJSONObject: provider.makeHostAgentPayload(input: input)))
        for try await event in provider.generateAgentEventStream(input: input,
            boundRequestDigest: String(repeating: "a", count: 64), frozenPayload: frozen) {
            if eventOrder.isEmpty {
                XCTAssertEqual(
                    host.completedResponseCount,
                    0,
                    "The provider must emit the first event before the Host finishes the SSE response."
                )
            }
            switch event {
            case .trace(let trace):
                switch trace.kind {
                case .phaseChanged:
                    eventOrder.append("phase:\(trace.payload.phase ?? "")")
                case .runCompleted:
                    eventOrder.append("done")
                default:
                    eventOrder.append("unexpected")
                }
            default:
                eventOrder.append("unexpected")
            }
        }

        XCTAssertEqual(eventOrder, [
            "phase:processing",
            "phase:compacting",
            "phase:processing",
            "done",
        ])

        let requests = host.capturedRequests()
        XCTAssertEqual(requests.count, 1)
        for request in requests {
            XCTAssertEqual(request.method, "POST")
            XCTAssertEqual(request.path, "/api/mobile/chat/agent/v2/stream")
            XCTAssertEqual(request.headers["accept"], "text/event-stream")
            XCTAssertEqual(request.headers["x-nola-mobile-crypto"], "aes-gcm-v1")
            XCTAssertNil(request.headers["authorization"])

            let body = request.body
            let object = try XCTUnwrap(
                JSONSerialization.jsonObject(with: body) as? [String: Any]
            )
            XCTAssertEqual(object["v"] as? Int, 1)
            XCTAssertEqual(object["alg"] as? String, "AES-256-GCM")
            XCTAssertNotNil(object["nonce"] as? String)
            XCTAssertNotNil(object["ciphertext"] as? String)
            XCTAssertNotNil(object["tag"] as? String)
        }
    }

    func testRealWebAgentHostStreamsToolLifecycleWithoutPermission() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_WEB_AGENT_HOST_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_WEB_AGENT_HOST_E2E=1 to run the real Web Agent Host request.")
        }
        let baseURL = try XCTUnwrap(
            environment["NOLA_REAL_WEB_AGENT_HOST_URL"].flatMap(URL.init(string:))
        )
        let token = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let settings = try await WebAgentProvider.fetchHostAgentSettings(baseURL: baseURL, token: token)
        let hostIdentity = try XCTUnwrap(settings.host_identity)
        XCTAssertEqual(
            hostIdentity.permission_protocol_version,
            3
        )
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: token,
            expectedHostId: hostIdentity.id,
            clientId: "ios-real-agent-host"
        ))
        let conversationID = UUID().uuidString
        let submissionID = UUID().uuidString
        let userMessageID = UUID().uuidString
        let assistantMessageID = "\(submissionID):assistant"
        let input = ModelInput(
            dateRange: .init(
                start: "2026-07-30T00:00:00Z",
                end: "2026-07-30T00:01:00Z"
            ),
            candidates: [],
            userPrefs: [
                "message": "Call the available system_time tool exactly once. After it succeeds, return only this exact ASCII token: NOLA_IOS_HOST_TOOL_OK",
                "enable_thinking": "false",
                "host_conversation_id": conversationID,
                "submission_id": submissionID,
                "user_message_id": userMessageID,
                "assistant_message_id": assistantMessageID,
            ]
        )

        var toolStarted: [String] = []
        var toolCompleted: [String] = []
        var permissionRequestCount = 0
        var runCompletedCount = 0
        var traceSequences: [UInt64] = []
        var receipt: IOSSourceSubmissionReceipt?
        var journalNotifications = 0
        func observeTrace(_ trace: AgentTraceEvent) {
            traceSequences.append(trace.sequence)
            switch trace.kind {
            case .toolStarted:
                toolStarted.append(trace.payload.toolName ?? "")
            case .toolCompleted:
                toolCompleted.append(trace.payload.toolName ?? "")
            case .toolPermissionRequested:
                permissionRequestCount += 1
            case .runCompleted:
                runCompletedCount += 1
            default:
                break
            }
        }
        for try await event in provider.generateAgentEventStream(input: input) {
            switch event {
            case .submissionReceipt(let raw):
                receipt = try JSONDecoder().decode(IOSSourceSubmissionReceipt.self, from: JSONEncoder().encode(raw))
            case .assistantJournalCommitted:
                journalNotifications += 1
            case .coreTurnTerminal(_, let trace):
                if let trace { observeTrace(trace) }
            case .trace(let trace):
                observeTrace(trace)
            case .permissionRequest:
                permissionRequestCount += 1
            case .error(let message):
                XCTFail("The production Host returned a protocol error: \(message)")
            default:
                break
            }
        }

        XCTAssertEqual(toolStarted.filter { $0 == "system_time" }.count, 1)
        XCTAssertEqual(toolCompleted.filter { $0 == "system_time" }.count, 1)
        XCTAssertEqual(permissionRequestCount, 0, "A reversible read-only tool must not request Nola approval.")
        XCTAssertEqual(runCompletedCount, 1)
        let admitted = try XCTUnwrap(receipt)
        XCTAssertEqual(admitted.phase, "admitted")
        XCTAssertEqual(admitted.identity.source_host_id, hostIdentity.id)
        XCTAssertEqual(admitted.identity.client_id, "ios-real-agent-host")
        XCTAssertEqual(admitted.identity.conversation_id, conversationID)
        XCTAssertEqual(admitted.identity.submission_id, submissionID)
        XCTAssertEqual(admitted.identity.assistant_message_id, assistantMessageID)
        XCTAssertGreaterThan(journalNotifications, 0)
        let assistantText = try await provider.readCompletedSummary(admission: XCTUnwrap(admitted.admission), expectedSealDigest: nil)
        XCTAssertEqual(
            assistantText.trimmingCharacters(in: .whitespacesAndNewlines),
            "NOLA_IOS_HOST_TOOL_OK",
            "Trace is audit/progress only; the answer must come from the sealed source journal."
        )
        XCTAssertTrue(
            zip(traceSequences, traceSequences.dropFirst()).allSatisfy { $0 < $1 },
            "The production mobile stream duplicated or reordered Core trace events: \(traceSequences)"
        )
    }

    func testRealWebAgentHostRunsInstalledExternalExecutorsAndResumesTheirNativeSessions() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_WEB_AGENT_HOST_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_WEB_AGENT_HOST_E2E=1 to run the real Web Agent Host request.")
        }
        let baseURL = try XCTUnwrap(
            environment["NOLA_REAL_WEB_AGENT_HOST_URL"].flatMap(URL.init(string:))
        )
        let token = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let settings = try await WebAgentProvider.fetchHostAgentSettings(baseURL: baseURL, token: token)
        let hostIdentity = try XCTUnwrap(settings.host_identity)
        let requiredBackends = ["codex", "claude_code", "opencode"]
        for backendID in requiredBackends {
            let descriptor = try XCTUnwrap(
                settings.execution_backends.first(where: { $0.id == backendID }),
                "Host omitted required execution backend \(backendID)"
            )
            XCTAssertEqual(
                descriptor.availability,
                "available",
                "Required installed execution backend \(backendID) is unavailable"
            )
        }

        func run(
            backendID: String,
            conversationID: String,
            marker: String,
            externalSessionID: String? = nil
        ) async throws -> (String, String) {
            let provider = WebAgentProvider(config: .init(
                baseURL: baseURL,
                token: token,
                expectedHostId: hostIdentity.id,
                clientId: "ios-real-\(backendID)-executor"
            ))
            var prefs = [
                "message": "Reply with only this exact token: \(marker)",
                "enable_thinking": "false",
                "host_conversation_id": conversationID,
                "execution_backend": backendID,
            ]
            if let externalSessionID { prefs["external_session_id"] = externalSessionID }
            let input = ModelInput(
                dateRange: .init(start: "2026-08-13T00:00:00Z", end: "2026-08-13T00:01:00Z"),
                candidates: [],
                userPrefs: prefs
            )
            let journal = IOSCommittedJournalTestReader()
            var sessions: [String] = []
            var traceSequences: [UInt64] = []
            for try await event in provider.generateAgentEventStream(input: input) {
                try journal.observe(event)
                switch event {
                case .externalSessionStarted(let backendID, let sessionID):
                    XCTAssertEqual(backendID, prefs["execution_backend"])
                    sessions.append(sessionID)
                case .trace(let trace):
                    traceSequences.append(trace.sequence)
                case .error(let message):
                    XCTFail(message)
                default:
                    break
                }
            }
            XCTAssertTrue(
                zip(traceSequences, traceSequences.dropFirst()).allSatisfy { $0 < $1 },
                "External execution trace events must remain strictly ordered"
            )
            let response = try await journal.text(provider: provider)
            return (
                response.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines.union(
                    CharacterSet(charactersIn: "`")
                )),
                try XCTUnwrap(sessions.last)
            )
        }

        for backendID in requiredBackends {
            let label = backendID.uppercased().replacingOccurrences(of: "_", with: "-")
            let conversationID = "ios-external-\(backendID)-\(UUID().uuidString)"
            let firstMarker = "IOS-\(label)-FIRST-\(UUID().uuidString.uppercased())"
            let first = try await run(
                backendID: backendID,
                conversationID: conversationID,
                marker: firstMarker
            )
            XCTAssertEqual(first.0, firstMarker)
            let secondMarker = "IOS-\(label)-RESUME-\(UUID().uuidString.uppercased())"
            let second = try await run(
                backendID: backendID,
                conversationID: conversationID,
                marker: secondMarker,
                externalSessionID: first.1
            )
            XCTAssertEqual(second.0, secondMarker)
            XCTAssertEqual(second.1, first.1)
        }
    }

    func testRealWebAgentHostCancelsRunningToolAndRecoversConversation() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_WEB_AGENT_HOST_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_WEB_AGENT_HOST_E2E=1 to run the real Web Agent Host request.")
        }
        let baseURL = try XCTUnwrap(
            environment["NOLA_REAL_WEB_AGENT_HOST_URL"].flatMap(URL.init(string:))
        )
        let token = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let settings = try await WebAgentProvider.fetchHostAgentSettings(baseURL: baseURL, token: token)
        let hostIdentity = try XCTUnwrap(settings.host_identity)
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: token,
            expectedHostId: hostIdentity.id,
            clientId: "ios-real-agent-host-cancellation"
        ))
        let conversationId = "ios-host-cancel-\(UUID().uuidString)"
        let input = ModelInput(
            dateRange: .init(
                start: "2026-07-30T00:00:00Z",
                end: "2026-07-30T00:01:00Z"
            ),
            candidates: [],
            userPrefs: [
                "message": "Use the bash tool to run `sleep 30` in the foreground. Wait for it before answering.",
                "enable_thinking": "false",
                "host_conversation_id": conversationId,
            ]
        )
        var bashStarted = 0
        var cancelled = 0
        var completed = 0
        var sentCancellation = false
        var submissionReceipt: IOSSourceSubmissionReceipt?
        let startedAt = Date()
        for try await event in provider.generateAgentEventStream(input: input) {
            switch event {
            case .submissionReceipt(let value):
                submissionReceipt = try JSONDecoder().decode(IOSSourceSubmissionReceipt.self, from: JSONEncoder().encode(value))
            case .trace(let trace):
                if trace.kind == .toolStarted, trace.payload.toolName == "bash" {
                    bashStarted += 1
                    if !sentCancellation {
                        sentCancellation = true
                        let receipt = try XCTUnwrap(submissionReceipt)
                        XCTAssertEqual(receipt.phase, "admitted")
                        try await provider.cancelHostGeneration(identity: receipt.identity, requestDigest: receipt.request_digest)
                    }
                } else if trace.kind == .runCancelled {
                    cancelled += 1
                } else if trace.kind == .runCompleted {
                    completed += 1
                }
            default:
                break
            }
        }
        XCTAssertEqual(bashStarted, 1)
        XCTAssertEqual(cancelled, 1)
        XCTAssertEqual(completed, 0)
        XCTAssertLessThan(Date().timeIntervalSince(startedAt), 15)

        let recoveryMarker = "IOS-CANCEL-RECOVERY-\(UUID().uuidString.uppercased())"
        let recoveryInput = ModelInput(
            dateRange: input.dateRange,
            candidates: [],
            userPrefs: [
                "message": "Reply with only this exact token: \(recoveryMarker)",
                "enable_thinking": "false",
                "host_conversation_id": conversationId,
            ]
        )
        let recoveryJournal = IOSCommittedJournalTestReader()
        var recoveryCompleted = 0
        for try await event in provider.generateAgentEventStream(input: recoveryInput) {
            try recoveryJournal.observe(event)
            switch event {
            case .trace(let trace):
                if trace.kind == .runCompleted {
                    recoveryCompleted += 1
                }
            default:
                break
            }
        }
        let recoveryText = try await recoveryJournal.text(provider: provider)
        XCTAssertEqual(
            recoveryText.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines.union(
                CharacterSet(charactersIn: "`")
            )),
            recoveryMarker
        )
        XCTAssertEqual(recoveryCompleted, 1)
    }

    func testRealWebAgentHostCompactsAndRecallsConversation() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_MOBILE_COMPACTION_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_MOBILE_COMPACTION_E2E=1 to run mobile compaction.")
        }
        let baseURL = try XCTUnwrap(
            environment["NOLA_REAL_WEB_AGENT_HOST_URL"].flatMap(URL.init(string:))
        )
        let token = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let settings = try await WebAgentProvider.fetchHostAgentSettings(baseURL: baseURL, token: token)
        let hostIdentity = try XCTUnwrap(settings.host_identity)
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: token,
            expectedHostId: hostIdentity.id,
            clientId: "ios-real-agent-host-compaction"
        ))
        let conversationId = "ios-host-compaction-\(UUID().uuidString)"
        let fact = "IOS-COMPACTION-FACT-\(UUID().uuidString.uppercased())"

        func run(_ message: String) async throws -> (assistant: String, compactions: Int, completed: Int) {
            let input = ModelInput(
                dateRange: .init(
                    start: "2026-07-30T00:00:00Z",
                    end: "2026-07-30T00:01:00Z"
                ),
                candidates: [],
                userPrefs: [
                    "message": message,
                    "enable_thinking": "false",
                    "host_conversation_id": conversationId,
                ]
            )
            let journal = IOSCommittedJournalTestReader()
            var compactions = 0
            var completed = 0
            for try await event in provider.generateAgentEventStream(input: input) {
                try journal.observe(event)
                switch event {
                case .trace(let trace):
                    if trace.kind == .contextCompacted {
                        compactions += 1
                    } else if trace.kind == .runCompleted {
                        completed += 1
                    }
                default:
                    break
                }
            }
            return (try await journal.text(provider: provider), compactions, completed)
        }

        var compactions = 0
        let first = try await run("Remember this exact conversation fact: \(fact). Reply briefly.")
        XCTAssertEqual(first.completed, 1)
        compactions += first.compactions
        for index in 1...4 {
            let filler = String(repeating: "ios-bounded-compaction-segment-\(index) ", count: 520)
            let turn = try await run("Conversation continuation \(index). Reply briefly.\n\(filler)")
            XCTAssertEqual(turn.completed, 1)
            compactions += turn.compactions
        }
        XCTAssertGreaterThanOrEqual(compactions, 1)
        let recall = try await run("Return only the exact IOS-COMPACTION-FACT token from the first turn.")
        XCTAssertEqual(recall.completed, 1)
        XCTAssertTrue(recall.assistant.contains(fact))
    }

    func testRealWebAgentHostPersistsAndRecallsMemoryAcrossConversations() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_WEB_AGENT_HOST_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_WEB_AGENT_HOST_E2E=1 to run the real Web Agent Host request.")
        }
        let baseURL = try XCTUnwrap(
            environment["NOLA_REAL_WEB_AGENT_HOST_URL"].flatMap(URL.init(string:))
        )
        let token = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let settings = try await WebAgentProvider.fetchHostAgentSettings(baseURL: baseURL, token: token)
        let hostIdentity = try XCTUnwrap(settings.host_identity)
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: token,
            expectedHostId: hostIdentity.id,
            clientId: "ios-real-agent-host-memory"
        ))
        let memoryCode = "IOS-PROJECT-LABEL-\(UUID().uuidString)"

        func run(_ message: String, conversationId: String) async throws -> [WebAgentStreamEvent] {
            let input = ModelInput(
                dateRange: .init(
                    start: "2026-07-30T00:00:00Z",
                    end: "2026-07-30T00:01:00Z"
                ),
                candidates: [],
                userPrefs: [
                    "message": message,
                    "enable_thinking": "false",
                    "host_conversation_id": conversationId,
                ]
            )
            var events: [WebAgentStreamEvent] = []
            for try await event in provider.generateAgentEventStream(input: input) {
                events.append(event)
            }
            return events
        }

        func evidence(_ events: [WebAgentStreamEvent]) async throws -> (
            started: [String], completed: [String], permissions: Int, assistant: String
        ) {
            var started: [String] = []
            var completed: [String] = []
            var permissions = 0
            let journal = IOSCommittedJournalTestReader()
            for event in events {
                try journal.observe(event)
                switch event {
                case .trace(let trace):
                    switch trace.kind {
                    case .toolStarted: started.append(trace.payload.toolName ?? "")
                    case .toolCompleted: completed.append(trace.payload.toolName ?? "")
                    case .toolPermissionRequested: permissions += 1
                    default: break
                    }
                case .permissionRequest: permissions += 1
                default: break
                }
            }
            let assistant = try await journal.text(provider: provider)
            return (
                started,
                completed,
                permissions,
                assistant.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines.union(
                    CharacterSet(charactersIn: "`")
                ))
            )
        }

        let write = try await evidence(try await run(
            "Remember this non-sensitive fact across future conversations: my iOS project label is \(memoryCode). Store that exact labeled fact in long-term memory. Do not use files. After memory is actually saved, reply with MEMORY_WRITE_OK.",
            conversationId: "ios-real-memory-write-\(memoryCode)"
        ))
        let recall = try await evidence(try await run(
            "This is a new conversation. Recall my non-sensitive iOS project label from long-term memory. Do not infer, guess, or return labels for other platforms. Reply with only the exact saved iOS project label.",
            conversationId: "ios-real-memory-recall-\(memoryCode)"
        ))

        XCTAssertEqual(write.started.filter { $0 == "memory_save" }.count, 1)
        XCTAssertEqual(write.completed.filter { $0 == "memory_save" }.count, 1)
        let recallStarted = recall.started.filter { $0 == "memory_search" }.count
        let recallCompleted = recall.completed.filter { $0 == "memory_search" }.count
        XCTAssertTrue((1...3).contains(recallStarted))
        XCTAssertEqual(recallStarted, recallCompleted)
        XCTAssertTrue(write.assistant.contains("MEMORY_WRITE_OK"))
        XCTAssertEqual(recall.assistant, memoryCode)
        XCTAssertEqual(write.permissions + recall.permissions, 0)
    }

    func testRealWebAgentHostDestructivePermissionCanBeDeniedAndResumed() async throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_WEB_AGENT_HOST_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_WEB_AGENT_HOST_E2E=1 to run the real Web Agent Host request.")
        }
        let baseURL = try XCTUnwrap(
            environment["NOLA_REAL_WEB_AGENT_HOST_URL"].flatMap(URL.init(string:))
        )
        let token = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let protectedPath = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_DESTRUCTIVE_FILE"])
        let settings = try await WebAgentProvider.fetchHostAgentSettings(baseURL: baseURL, token: token)
        let hostIdentity = try XCTUnwrap(settings.host_identity)
        XCTAssertEqual(
            hostIdentity.permission_protocol_version,
            3
        )
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: token,
            expectedHostId: hostIdentity.id,
            clientId: "ios-real-agent-host-permission"
        ))
        let input = ModelInput(
            dateRange: .init(
                start: "2026-07-30T00:00:00Z",
                end: "2026-07-30T00:01:00Z"
            ),
            candidates: [],
            userPrefs: [
                "message": "Delete the file at this exact path: \(protectedPath). This is a real operation, not a request for instructions. First call tool_search to load delete_file if needed, then call delete_file exactly once. Do not use bash and do not answer before the delete tool returns.",
                "enable_thinking": "false",
            ]
        )

        var permissionRequestCount = 0
        var permissionTraceCount = 0
        let journal = IOSCommittedJournalTestReader()
        var runCompletedCount = 0
        for try await event in provider.generateAgentEventStream(input: input) {
            try journal.observe(event)
            switch event {
            case .permissionRequest(let request, let trace):
                permissionRequestCount += 1
                if trace.kind == .toolPermissionRequested {
                    permissionTraceCount += 1
                }
                try await provider.submitHostPermissionDecision(
                    ChatManager.hostPermissionDecisionPayload(for: request, decision: .deny)
                )
            case .trace(let trace):
                switch trace.kind {
                case .toolPermissionRequested:
                    permissionTraceCount += 1
                case .runCompleted:
                    runCompletedCount += 1
                default:
                    break
                }
            default:
                break
            }
        }

        let assistantText = try await journal.text(provider: provider)
        XCTAssertEqual(permissionRequestCount, 1)
        XCTAssertEqual(permissionTraceCount, 1)
        XCTAssertEqual(runCompletedCount, 1)
        XCTAssertFalse(assistantText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        XCTAssertEqual(
            try String(contentsOfFile: protectedPath, encoding: .utf8),
            "KEEP_UNTIL_PERMISSION_DENIED\n"
        )
    }
}

private final class RecordingEventStreamingLLMClient: LLMEventStreamingClient, LLMAgentCapabilityProviding {
    private let delegate: OpenAIProvider
    private let lock = NSLock()
    private var inputs: [ModelInput] = []

    init(delegate: OpenAIProvider) {
        self.delegate = delegate
    }

    var supportsNativeEventStream: Bool { true }
    var agentCapabilities: LLMAgentCapabilities { delegate.agentCapabilities }

    func recordedInputs() -> [ModelInput] {
        lock.lock()
        defer { lock.unlock() }
        return inputs
    }

    func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
        try await delegate.generateSummary(input: input, timeout: timeout)
    }

    func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
        delegate.generateSummaryStream(input: input)
    }

    func chat(
        message: String,
        systemPrompt: String?,
        enableThinking: Bool,
        timeout: TimeInterval?
    ) async throws -> String {
        try await delegate.chat(
            message: message,
            systemPrompt: systemPrompt,
            enableThinking: enableThinking,
            timeout: timeout
        )
    }

    func generateEventStream(input: ModelInput) -> AsyncThrowingStream<LLMStreamEvent, Error> {
        lock.lock()
        inputs.append(input)
        lock.unlock()
        return delegate.generateEventStream(input: input)
    }
}

private struct CapturedHTTPRequest {
    let method: String
    let path: String
    let headers: [String: String]
    let body: Data
}

private final class LocalSSEHost: @unchecked Sendable {
    private final class RequestState: @unchecked Sendable {
        let connection: NWConnection
        var data = Data()

        init(connection: NWConnection) {
            self.connection = connection
        }
    }

    private let listener: NWListener
    private let queue = DispatchQueue(label: "nola.tests.ios-real-sse-host")
    private let lock = NSLock()
    private var startContinuation: CheckedContinuation<URL, Error>?
    private var requests: [CapturedHTTPRequest] = []
    private var completedResponses = 0

    init() throws {
        listener = try NWListener(using: .tcp, on: .any)
        listener.stateUpdateHandler = { [weak self] state in
            self?.handleListenerState(state)
        }
        listener.newConnectionHandler = { [weak self] connection in
            self?.accept(connection)
        }
    }

    func start() async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                self.startContinuation = continuation
                self.listener.start(queue: self.queue)
            }
        }
    }

    func stop() {
        listener.cancel()
    }

    var completedResponseCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return completedResponses
    }

    func capturedRequests() -> [CapturedHTTPRequest] {
        lock.lock()
        defer { lock.unlock() }
        return requests
    }

    private func handleListenerState(_ state: NWListener.State) {
        switch state {
        case .ready:
            guard let port = listener.port,
                  let url = URL(string: "http://127.0.0.1:\(port.rawValue)") else {
                startContinuation?.resume(throwing: URLError(.badURL))
                startContinuation = nil
                return
            }
            startContinuation?.resume(returning: url)
            startContinuation = nil
        case .failed(let error):
            startContinuation?.resume(throwing: error)
            startContinuation = nil
        default:
            break
        }
    }

    private func accept(_ connection: NWConnection) {
        let state = RequestState(connection: connection)
        connection.stateUpdateHandler = { [weak self] connectionState in
            guard let self else { return }
            if case .ready = connectionState {
                self.receiveRequest(state)
            }
        }
        connection.start(queue: queue)
    }

    private func receiveRequest(_ state: RequestState) {
        state.connection.receive(
            minimumIncompleteLength: 1,
            maximumLength: 1_048_576
        ) { [weak self, weak state] content, _, isComplete, error in
            guard let self, let state else { return }
            if let content {
                state.data.append(content)
            }
            if let request = Self.parseRequest(state.data) {
                self.record(request)
                self.sendSSE(on: state.connection)
                return
            }
            if error != nil || isComplete {
                state.connection.cancel()
                return
            }
            self.receiveRequest(state)
        }
    }

    private func record(_ request: CapturedHTTPRequest) {
        lock.lock()
        requests.append(request)
        lock.unlock()
    }

    private func markResponseCompleted() {
        lock.lock()
        completedResponses += 1
        lock.unlock()
    }

    private func sendSSE(on connection: NWConnection) {
        let frames = [
            traceFrame(kind: "phase.changed", phase: "processing", sequence: 1),
            traceFrame(kind: "phase.changed", phase: "compacting", sequence: 2),
            traceFrame(kind: "phase.changed", phase: "processing", sequence: 3),
            traceFrame(kind: "run.completed", status: "succeeded", sequence: 4),
            "data: [DONE]\n\n",
        ]
        let first = Data(frames[0].utf8)
        let split = max(1, first.count / 2)
        let chunks = [
            Data(first.prefix(split)),
            Data(first.suffix(from: split)),
        ] + frames.dropFirst().map { Data($0.utf8) }
        let bodyLength = chunks.reduce(0) { $0 + $1.count }
        let headerText = [
            "HTTP/1.1 200 OK",
            "Content-Type: text/event-stream",
            "Cache-Control: no-cache",
            "Content-Length: \(bodyLength)",
            "Connection: close",
        ].joined(separator: "\r\n") + "\r\n\r\n"
        let header = Data(headerText.utf8)
        connection.send(content: header, completion: .contentProcessed { [weak self] error in
            guard error == nil, let self else {
                connection.cancel()
                return
            }
            self.send(chunks: chunks, index: 0, on: connection)
        })
    }

    private func traceFrame(
        kind: String,
        phase: String? = nil,
        status: String = "running",
        sequence: Int
    ) -> String {
        var payload: [String: Any] = [:]
        if let phase {
            payload["phase"] = phase
        }
        let object: [String: Any] = [
            "type": "trace",
            "trace": [
                "schema_version": agentTraceSchemaVersion,
                "trace_id": "trace-ios-e2e",
                "run_id": "run-ios-e2e",
                "conversation_id": "conversation-ios-e2e",
                "turn_id": "turn-ios-e2e",
                "event_id": "event-\(sequence)",
                "sequence": sequence,
                "emitted_at": "2026-07-19T00:00:00Z",
                "kind": kind,
                "status": status,
                "visibility": "summary",
                "payload": payload,
            ],
        ]
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return "data: \(String(decoding: data, as: UTF8.self))\n\n"
    }

    private func send(chunks: [Data], index: Int, on connection: NWConnection) {
        guard index < chunks.count else {
            markResponseCompleted()
            connection.send(
                content: nil,
                contentContext: .finalMessage,
                isComplete: true,
                completion: .contentProcessed { _ in
                    connection.cancel()
                }
            )
            return
        }
        connection.send(content: chunks[index], completion: .contentProcessed { [weak self] error in
            guard error == nil, let self else {
                connection.cancel()
                return
            }
            self.queue.asyncAfter(deadline: .now() + .milliseconds(40)) {
                self.send(chunks: chunks, index: index + 1, on: connection)
            }
        })
    }

    private static func parseRequest(_ data: Data) -> CapturedHTTPRequest? {
        let separator = Data("\r\n\r\n".utf8)
        guard let headerRange = data.range(of: separator),
              let headerText = String(
                data: data.subdata(in: data.startIndex..<headerRange.lowerBound),
                encoding: .utf8
              ) else {
            return nil
        }
        let lines = headerText.components(separatedBy: "\r\n")
        let requestParts = lines.first?.split(separator: " ", maxSplits: 2).map(String.init) ?? []
        guard requestParts.count >= 2 else { return nil }
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            guard let separatorIndex = line.firstIndex(of: ":") else { continue }
            let name = line[..<separatorIndex]
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
            let value = line[line.index(after: separatorIndex)...]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            headers[name] = value
        }
        let contentLength = Int(headers["content-length"] ?? "") ?? 0
        let bodyStart = headerRange.upperBound
        guard data.count >= bodyStart + contentLength else { return nil }
        return CapturedHTTPRequest(
            method: requestParts[0],
            path: requestParts[1],
            headers: headers,
            body: data.subdata(in: bodyStart..<(bodyStart + contentLength))
        )
    }
}
