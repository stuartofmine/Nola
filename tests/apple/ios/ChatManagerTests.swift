// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatManagerTests logic in the project module.
// - Interfaces:
//   - ChatManagerTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

/// Exercises the real configured OpenAI provider and Core admission. Only the
/// HTTP peer is replaced; profile, capability and output validation remain live.
private final class ChatManagerHTTPFixture: URLProtocol, @unchecked Sendable {
    @MainActor static var responders: [String: @MainActor (Data) async throws -> String] = [:]
    @MainActor static var requests: [String: [Data]] = [:]
    @MainActor static var chunks: [String: [[String: Any]]] = [:]
    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "chat-manager-fixture.invalid"
    }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let request = request
        Task { @MainActor in
            do {
                let id = try XCTUnwrap(request.url?.pathComponents.dropFirst().first)
                var body = request.httpBody ?? Data()
                if body.isEmpty, let stream = request.httpBodyStream {
                    stream.open()
                    defer { stream.close() }
                    var buffer = [UInt8](repeating: 0, count: 4096)
                    while stream.hasBytesAvailable {
                        let count = stream.read(&buffer, maxLength: buffer.count)
                        guard count > 0 else { break }
                        body.append(contentsOf: buffer.prefix(count))
                    }
                }
                Self.requests[id, default: []].append(body)
                let content: [String: Any]
                if var queued = Self.chunks[id], !queued.isEmpty {
                    content = queued.removeFirst()
                    Self.chunks[id] = queued
                } else {
                    let responseText = try await XCTUnwrap(Self.responders[id])(body)
                    content = ["id": "fixture-response", "object": "chat.completion.chunk",
                        "choices": [["index": 0, "delta": ["content": responseText], "finish_reason": "stop"]],
                        "usage": ["prompt_tokens": 321, "completion_tokens": 12, "total_tokens": 333]]
                }
                let encoded = try JSONSerialization.data(withJSONObject: content)
                let data = Data("data: \(String(decoding: encoded, as: UTF8.self))\n\ndata: [DONE]\n\n".utf8)
                let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil,
                    headerFields: ["Content-Type": "text/event-stream"])!
                self.client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                self.client?.urlProtocol(self, didLoad: data)
                self.client?.urlProtocolDidFinishLoading(self)
            } catch { self.client?.urlProtocol(self, didFailWithError: error) }
        }
    }
    override func stopLoading() {}
}

@MainActor
final class ChatManagerTests: XCTestCase {
    private static var retainedManagers: [ChatManager] = []

    private func configuredHTTPFixture(container: ServiceContainer, id: String) throws -> ModelInfo {
        URLProtocol.registerClass(ChatManagerHTTPFixture.self)
        let client = container.llmClient
        ChatManagerHTTPFixture.requests[id] = []
        ChatManagerHTTPFixture.responders[id] = { body in
            try await client.chat(message: String(decoding: body, as: UTF8.self),
                systemPrompt: nil, enableThinking: true, timeout: nil)
        }
        let model = ModelInfo(id: id, name: id, source: "api", isLocal: false,
            baseURL: "https://chat-manager-fixture.invalid/\(id)/v1",
            wireProtocol: .chatCompletions, executionCapabilities: .init(toolCalls: true),
            cloudCredential: .anonymous)
        var source = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(model)) as? [String: Any])
        source["capabilities"] = ["chat"]
        source["reasoning"] = ["control": "automatic"]
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        try container.applicationRuntime.upsertModelProfile(.init(id: id, displayName: id,
            provider: model.source, modelName: model.effectiveUpstreamModelId, runtime: "remote",
            sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: String(decoding: try JSONEncoder().encode(ModelParameters.webAlignedCloudDefaults), as: UTF8.self),
            selected: false, enabled: true, revision: 0, createdAtMs: now, updatedAtMs: now, deletedAtMs: nil))
        // Validate current Core admission before a test waits on a provider
        // continuation, so fixture setup failures retain their real cause.
        _ = try ModelSelectionManager.resolvedCloudProfileForRun(model, using: container.applicationRuntime)
        return model
    }

    private func makeApplicationRuntime() throws -> (IOSApplicationRuntime, URL) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-chat-manager-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return (
            try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite")),
            root
        )
    }

    /// Existing conversations own their model and parameter baseline; a page's
    /// selectedModelInfo is not an implicit configuration source for them.
    private func configureConversation(_ id: UUID, model: ModelInfo, store: LocalConversationStore) throws {
        try store.selectConversationModel(conversationID: id, model: model,
            initialParameters: .webAlignedCloudDefaults)
    }

    private func submitThroughUseCase(_ manager: ChatManager, owner: UUID, text: String) throws -> Task<String, Error> {
        var drafts = ConversationComposerDraftStore()
        drafts.select(owner)
        drafts.current.text = text
        let prepared = try manager.submissionUseCase.prepare(title: text, initialParameters: nil, drafts: &drafts)
        let input = SendConversationTurnUseCase.Input(prompt: text, displayText: text, metadata: nil,
            attachments: [], responseStyle: .casual, retrievalEnabled: false, preferredHostID: "",
            hybridToolsEnabled: false, hybridCapabilities: [])
        return Task {
            try await withCheckedThrowingContinuation { continuation in
                manager.submissionUseCase.start(prepared, input: input, observers: []) { result in
                    if case .failure(let error) = result {
                        let diagnostic = XCTAttachment(string: "Boundary: \(String(reflecting: result))\n\(error.localizedDescription)")
                        diagnostic.name = "submission-failure-boundary"
                        diagnostic.lifetime = .keepAlways
                        self.add(diagnostic)
                    }
                    continuation.resume(with: result)
                }
            }
        }
    }

    func testSubmissionRejectsLaggingDraftSelectionBeforeCreatingOrClearingAnything() throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let a = try store.createConversation(title: "A", language: nil)
        let b = try store.createConversation(title: "B", language: nil)
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = store
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.currentConversationId = a.id
        var drafts = ConversationComposerDraftStore()
        drafts.select(b.id)
        drafts.current.text = "belongs to B"
        XCTAssertThrowsError(try manager.submissionUseCase.prepare(title: "A", initialParameters: nil, drafts: &drafts))
        XCTAssertEqual(drafts.selected, .conversation(b.id))
        XCTAssertEqual(drafts.current.text, "belongs to B")
        XCTAssertEqual(manager.currentConversationId, a.id)
        XCTAssertTrue(try store.getMessages(conversationId: a.id, limit: 0).isEmpty)
        XCTAssertTrue(try store.getMessages(conversationId: b.id, limit: 0).isEmpty)
    }

    private static func removeStreamingFixtureRoot(_ root: URL) async throws {
        // Final persistence callbacks can outlive the response Task briefly.
        // Cleanup remains strict: exhausting this bounded drain fails the test.
        for attempt in 0..<5 {
            do { try FileManager.default.removeItem(at: root); return }
            catch {
                if attempt == 4 { throw error }
                try await Task.sleep(nanoseconds: 50_000_000)
            }
        }
    }

    /// Real SQLite Application, real provider parser and a deterministic HTTP peer.
    /// The second Application has no retained Core or delivery state from the first.
    func testColdLocalPermissionRecoveryPreservesFrozenProviderJournalAndSeal() async throws {
        let (store, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let container = ServiceContainer(applicationRuntime: store)
        let id = "cold-permission-\(UUID().uuidString)"
        let model = try configuredHTTPFixture(container: container, id: id)
        defer {
            ChatManagerHTTPFixture.chunks.removeValue(forKey: id)
            ChatManagerHTTPFixture.responders.removeValue(forKey: id)
            ChatManagerHTTPFixture.requests.removeValue(forKey: id)
        }
        ChatManagerHTTPFixture.chunks[id] = [
            ["id": "first", "object": "chat.completion.chunk", "choices": [["index": 0,
                "delta": ["tool_calls": [["index": 0, "id": "permission-call", "type": "function",
                    "function": ["name": "fixture_inspect", "arguments": "{}"]]]], "finish_reason": "tool_calls"]]],
            ["id": "second", "object": "chat.completion.chunk", "choices": [["index": 0,
                "delta": ["content": "Recovered answer from the frozen provider."], "finish_reason": "stop"]]],
        ]
        let prepared = try IOSPreparedRunModel.prepare(model: model, parameters: .webAlignedCloudDefaults,
            resolveCloud: { try ModelSelectionManager.resolvedCloudProfileForRun($0, using: store) },
            admitOutput: { try IOSPreparedHostOutputAdmission.admit(route: $0, outputTokens: $1, using: store) })
        let host = try prepared.prepareHostInvocation()
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let conversationID = UUID().uuidString
        let coreID = "independent-core:\(UUID())"
        let descriptor = IOSCanonicalAgentRunDescriptor(conversationID: conversationID, conversationTitle: "Cold recovery",
            identity: identity, userText: "Inspect the fixture", userContentJSON: nil, contextCheckpointID: nil)
        func wire(_ value: Any) throws -> AgentCoreJSONValue {
            try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: value))
        }
        func object(_ value: AgentCoreJSONValue) throws -> [String: AgentCoreJSONValue] {
            guard case .object(let fields) = value else { throw IOSApplicationRuntimeError.invalidResponse }
            return fields
        }
        func request(_ transition: AgentCoreJSONValue, _ type: String) throws -> AgentCoreJSONValue {
            guard case .array(let requests)? = try object(transition)["host_requests"],
                  let value = requests.first(where: {
                      guard case .object(let envelope) = $0, case .object(let payload)? = envelope["payload"] else { return false }
                      return payload["type"] == .string(type)
                  }) else {
                throw IOSApplicationRuntimeError.rejected("Missing real \(type) delivery")
            }
            return value
        }
        let tool = try wire(["type": "function", "function": ["name": "fixture_inspect",
            "description": "Inspect fixture", "parameters": ["type": "object", "properties": [:]]]])
        func capability(_ name: String, route: String) throws -> AgentCoreJSONValue {
            try wire(["id": name, "title": name, "route": route, "availability": "available",
                "exposure": route == "model" ? "internal" : "visible", "input_schema": ["type": "object"],
                "input_normalization": ["origin": ["kind": "builtin"],
                    "effect": ["schema_version": 1, "fixed": "investigate", "variants": [:]]],
                "output_policy": ["preview_char_limit": 4000, "persist_full_output": true, "allow_artifacts": true],
                "supports_streaming": route == "model", "parallelism": "serial", "cancellation": "cooperative",
                "replay_safety": "recoverable"])
        }
        let client = TextAgentModelClient(client: prepared.client)
        var turnIdentity = try object(DeviceAgentCoordinator.promptTurnIdentity(stablePromptDigest: nil,
            promptFragmentIds: [], stablePromptFragmentDigests: [:], modelCapability: client.coreModelCapabilityProfile) ?? .object([:]))
        turnIdentity["host_invocation"] = try host.binding(executionPolicy: .object([:]))
        let coreConfig: AgentCoreJSONValue = .object([
            "session_id": .string(coreID), "model": .string(prepared.modelName), "turn_identity": .object(turnIdentity),
            "context_window": try wire(["context_window_tokens": prepared.parameters.maxContextLength,
                "reserved_output_tokens": prepared.parameters.maxOutputTokens,
                "prompt_token_limit": prepared.parameters.maxContextLength - prepared.parameters.maxOutputTokens]),
            "tools": .array([tool]),
            "permission_rules": try wire(["rules": [["capability_pattern": "fixture_inspect", "resource_pattern": "*",
                "decision": "ask", "source": "product_default"]]]),
            "host_catalog": .object(["host_id": .string("ios-fixture"), "revision": .number(1), "availability_epoch": .number(1),
                "capabilities": .object(["model.stream": try capability("model.stream", route: "model"),
                    "fixture_inspect": try capability("fixture_inspect", route: "tool")])]),
        ])
        var responseTime = Int64(Date().timeIntervalSince1970 * 1000)
        var sequence: [String: Int] = [:]
        func respond(_ application: IOSCanonicalAgentApplication, _ request: AgentCoreJSONValue,
                     _ payload: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            let key = String(describing: try object(request)["request_id"])
            sequence[key, default: 0] += 1
            responseTime += 1
            return try application.acceptHostResponse(DeviceAgentCoordinator.hostResponseEnvelope(request: request,
                payload: payload, sequence: sequence[key]!), receivedAtMs: responseTime)
        }
        func executeModel(_ application: IOSCanonicalAgentApplication, _ request: AgentCoreJSONValue,
                          _ prepared: IOSPreparedRunModel) async throws -> AgentCoreJSONValue {
            let envelope = try object(request)
            let payload = try object(XCTUnwrap(envelope["payload"]))
            guard case .string(let turn)? = envelope["turn_id"], case .string(let part)? = payload["text_part_id"] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            _ = try respond(application, request, .object(["type": .string("execution_started")]))
            _ = try respond(application, request, try wire(["type": "model_attempt_started", "provider": "fixture",
                "model": prepared.modelName, "reason": "initial"]))
            let messages = try XCTUnwrap(payload["messages"])
            guard case .array(let items) = messages, case .array(let tools)? = payload["tools"] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            var history: [AgentCoreMessage] = []
            for item in items {
                let fields = try object(item)
                func string(_ key: String) -> String? { if case .string(let text)? = fields[key] { return text }; return nil }
                switch string("type") {
                case "message": history.append(.init(role: string("role") ?? "user", content: string("content") ?? ""))
                case "tool_call":
                    let call = AgentCoreToolCall(id: try XCTUnwrap(string("call_id")), name: try XCTUnwrap(string("name")),
                        arguments: fields["arguments"] ?? .object([:]))
                    if let last = history.indices.last, history[last].role == "assistant" {
                        history[last].toolCalls.append(call)
                    } else { history.append(.init(role: "assistant", content: "", toolCalls: [call])) }
                case "tool_result": history.append(.init(role: "tool",
                    content: String(decoding: try JSONEncoder().encode(fields["output"] ?? .null), as: UTF8.self),
                    name: string("name"), toolCallId: string("call_id")))
                default: break
                }
            }
            let modelRequest = AgentModelRequest(sessionId: coreID, turnId: turn, iteration: 1,
                messages: history, tools: tools,
                reservedOutputTokens: prepared.parameters.maxOutputTokens,
                invocationPlan: .init(maxOutputTokens: prepared.parameters.maxOutputTokens,
                    nativeToolCalls: true, streamText: true, streamToolCalls: true))
            var completion: AgentModelCompletion?
            for try await event in TextAgentModelClient(client: prepared.client).generateAgentTurn(request: modelRequest) {
                if case .completed(let value) = event { completion = value }
            }
            let result = try XCTUnwrap(completion)
            let calls = result.toolCalls.map { AgentCoreJSONValue.object([
                "call_id": .string($0.id), "name": .string($0.name), "arguments": $0.arguments]) }
            return try respond(application, request, .object(["type": .string("model_completed"),
                "items": .array([try wire(["type": "message", "id": part, "role": "assistant", "content": result.content])]),
                "tool_calls": .array(calls), "usage": try wire(["input_tokens": 32, "output_tokens": 16, "total_tokens": 48])]))
        }
        let first = IOSCanonicalAgentApplication(store: store, descriptor: descriptor)
        var transition = try first.begin(coreConfig: coreConfig, submission: wire([
            "type": "user_input", "submission_id": identity.submissionID, "delivery": "current_turn",
            "items": [["type": "message", "id": identity.userMessageID, "role": "user", "content": descriptor.userText]]]),
            createdAtMs: responseTime)
        XCTAssertNil(try first.committedTerminalSeal(coreSessionID: coreID))
        transition = try await executeModel(first, request(transition, "model_stream"), prepared)
        let oldPermission = try request(transition, "request_permission")
        let frozen = try first.loadExactRunInvocation()
        let reopened = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let recovered = IOSCanonicalAgentApplication(store: reopened, descriptor: descriptor)
        let recoveredModel = try IOSPreparedRunModel.recover(frozen, using: reopened)
        let recoveryTime = responseTime + 300_000
        transition = try await recovered.recoverPendingPermissionWait(prepareRecovery: { witness in
            try await MainActor.run {
                let authorized = try IOSPreparedRunModel.recover(witness, using: reopened)
                XCTAssertEqual(try authorized.prepareHostInvocation().route, host.route)
            }
        }, nowMilliseconds: { recoveryTime }, sleep: { _ in XCTFail("Expired delivery must be immediately recoverable") })
        // Recovery and subsequent acknowledgements share the same clock;
        // returning to wall time would predate the successor delivery lease.
        responseTime = recoveryTime
        XCTAssertNotNil(try object(transition)["task_projection"])
        let permission = try request(transition, "request_permission")
        XCTAssertNotEqual(try object(oldPermission)["delivery"], try object(permission)["delivery"])
        sequence.removeAll()
        _ = try respond(recovered, permission, .object(["type": .string("execution_started")]))
        let permissionID = try XCTUnwrap(object(object(permission)["payload"]!)["permission_request_id"])
        transition = try respond(recovered, permission, .object(["type": .string("permission_resolved"), "request_id": permissionID,
            "resolution": try wire(["type": "authorization", "decision": "deny", "grant": NSNull()])]))
        transition = try await executeModel(recovered, request(transition, "model_stream"), recoveredModel)
        let seal = try XCTUnwrap(recovered.committedTerminalSeal(coreSessionID: coreID))
        // A readable reply does not turn the denied requested inspection into
        // success. Preserve Core's permission blocker across cold recovery.
        XCTAssertEqual(seal.protocol_version, 1)
        XCTAssertEqual(seal.result.value.kind, .blocked)
        let blocked = try object(seal.result.value.payload)
        XCTAssertEqual(try object(XCTUnwrap(blocked["reason"]))["code"], .string("permission.denied"))
        guard case .array(let blockers)? = blocked["blocking_fact_refs"] else {
            return XCTFail("Permission denial must retain its canonical blocker evidence")
        }
        XCTAssertFalse(blockers.isEmpty)
        XCTAssertEqual(seal.result.run_id, identity.submissionID)
        XCTAssertThrowsError(try recovered.committedTerminalSeal(coreSessionID: "foreign-core"))
        let heads = try reopened.executeAgentCommand(.object(["type": .string("local_run_assistant_journal_heads"),
            "conversation_id": .string(conversationID), "run_id": .string(identity.submissionID), "limit": .number(64)]))
        let cursors = try JSONDecoder().decode([IOSAssistantJournalCursor].self, from: JSONEncoder().encode(heads))
        let cursor = try XCTUnwrap(cursors.first { $0.identity.assistant_message_id == identity.assistantMessageID })
        let content = try await IOSAssistantJournalReader(store: reopened).read(at: cursor)
        XCTAssertTrue(content.text.contains("Recovered answer from the frozen provider."))
        XCTAssertEqual(ChatManagerHTTPFixture.requests[id]?.count, 2)
    }

    func testHostPermissionDecisionPayloadSupportsAllowAndDeny() {
        let request = HostAgentPermissionRequest(
            id: "call-1",
            allowConversation: true,
            toolName: "computer_click",
            match: "click:100,200"
        )

        let allow = ChatManager.hostPermissionDecisionPayload(for: request, allow: true)
        let deny = ChatManager.hostPermissionDecisionPayload(for: request, allow: false)
        let conversation = ChatManager.hostPermissionDecisionPayload(for: request, decision: .allowConversation)

        let allowResolution = allow["resolution"] as? [String: Any]
        let denyResolution = deny["resolution"] as? [String: Any]
        let conversationResolution = conversation["resolution"] as? [String: Any]
        XCTAssertEqual(allowResolution?["type"] as? String, "authorization")
        XCTAssertEqual(allowResolution?["decision"] as? String, "allow_once")
        XCTAssertEqual(denyResolution?["decision"] as? String, "deny")
        XCTAssertEqual(deny["id"] as? String, "call-1")
        XCTAssertEqual(deny["tool_name"] as? String, "computer_click")
        XCTAssertEqual(deny["match"] as? String, "click:100,200")
        XCTAssertEqual(conversationResolution?["decision"] as? String, "allow_conversation")
        XCTAssertNil(conversation["grant_scope"])
    }

    func testHostPermissionResumeLimitUsesConfiguredMaximum() {
        XCTAssertTrue(ChatManager.canResumeHostPermission(permissionResumeCount: 0, maxPermissionResumes: 1))
        XCTAssertFalse(ChatManager.canResumeHostPermission(permissionResumeCount: 1, maxPermissionResumes: 1))
        XCTAssertTrue(ChatManager.canResumeHostPermission(permissionResumeCount: 2, maxPermissionResumes: 30))
        XCTAssertFalse(ChatManager.canResumeHostPermission(permissionResumeCount: 30, maxPermissionResumes: 30))
        XCTAssertTrue(ChatManager.canResumeHostPermission(permissionResumeCount: 0, maxPermissionResumes: 0))
        XCTAssertTrue(ChatManager.canResumeHostPermission(permissionResumeCount: 10_000, maxPermissionResumes: 0))
    }

    func testWebBackendModelIsAvailableAndRecognized() {
        let webModel = ModelSelectionManager.webBackendModel(
            baseURL: "http://127.0.0.1:5001",
            token: "mobile-token"
        )

        XCTAssertTrue(webModel.isWebBackend)
        XCTAssertEqual(webModel.id, "web-backend:local-host")
        XCTAssertEqual(webModel.effectiveUpstreamModelId, "web-host")
        XCTAssertTrue(ModelSelectionManager.getAvailableModels().contains { $0.isWebBackend })
        XCTAssertNotNil(ModelSelectionManager.firstWebBackendModel())
    }

    func testHostCapabilityDiscoveryRequiresExplicitPairing() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://host-capability-test.invalid/\(UUID().uuidString)"))
        let model = ModelInfo(
            id: "web-backend:capability-test",
            name: "Capability Test Host",
            source: ModelSelectionManager.webBackendSource,
            isLocal: false,
            baseURL: baseURL.absoluteString
        )
        XCTAssertFalse(ModelSelectionManager.hasPairedWebHost(for: model))
        XCTAssertTrue(ModelSelectionManager.installEphemeralWebHostCredential(
            token: "test-token",
            identity: "host-1234567890123456",
            for: baseURL
        ))
        XCTAssertTrue(ModelSelectionManager.hasPairedWebHost(for: model))

        let ordinaryModel = ModelInfo(
            id: "ordinary-cloud-model",
            name: "Ordinary Cloud Model",
            source: "OpenAI-compatible",
            isLocal: false,
            baseURL: baseURL.absoluteString
        )
        XCTAssertFalse(ModelSelectionManager.hasPairedWebHost(for: ordinaryModel))
    }

    func testSelectingWebBackendModelConfiguresWebAgentProvider() {
        let container = ServiceContainer()
        container.baseURLString = "http://127.0.0.1:5001"
        container.apiKey = "mobile-token"
        container.model = ""
        container.useLocalModel = false
        let manager = ChatManager(container: container, restorePersistedState: false)
        Self.retainedManagers.append(manager)

        manager.selectModel(ModelSelectionManager.webBackendModel())

        let service = container.llmClient as? ModelService
        XCTAssertNotNil(service?.currentProvider() as? WebAgentProvider)
        XCTAssertEqual(container.model, "web-host")
        XCTAssertEqual(container.baseURLString, "http://127.0.0.1:5001")
        XCTAssertEqual(container.apiKey, "mobile-token")
        XCTAssertNil(manager.modelFallbackNotice)
    }

    func testNewHostConversationCanSubmitBeforeRecoveryButRestoredEmptyConversationCannot() async throws {
        let (runtime, _) = try makeApplicationRuntime()
        let container = ServiceContainer(applicationRuntime: runtime)
        let model = ModelSelectionManager.webBackendModel(
            baseURL: "https://new-host-conversation.invalid", token: "fixture-token")
        let manager = ChatManager(container: container, restorePersistedState: false)
        Self.retainedManagers.append(manager)
        manager.selectModel(model, persistAsDefault: false)
        let conversationID = try XCTUnwrap(manager.startConversation(title: "Fresh Host turn"))
        let execution = try XCTUnwrap(manager.executionForConversation(conversationID))
        XCTAssertTrue(execution.canBeginNewMessage,
            "Creation provenance proves no prior remote submission; first click must be admitted")
        await Task.yield()
        XCTAssertTrue(execution.canBeginNewMessage,
            "Deferred selection recovery must not turn a new empty conversation into streaming")
        manager.currentConversationId = nil
        manager.currentConversationId = conversationID
        XCTAssertTrue(execution.canBeginNewMessage,
            "Selecting the same not-yet-submitted session preserves its creation provenance")

        let restored = ChatManager(container: container, restorePersistedState: false)
        Self.retainedManagers.append(restored)
        restored.selectModel(model, persistAsDefault: false)
        restored.currentConversationId = conversationID
        let restoredExecution = try XCTUnwrap(restored.executionForConversation(conversationID))
        XCTAssertFalse(restoredExecution.canBeginNewMessage,
            "An empty transcript after restart does not prove that no remote effect exists")
    }

    func testColdLocalSelectionInitializesPermissionOwnerWithoutAnyStreamingSnapshot() async throws {
        let (runtime, _) = try makeApplicationRuntime()
        let container = ServiceContainer(applicationRuntime: runtime)
        let store = try LocalConversationStore(applicationRuntime: runtime)
        container.conversationStore = store
        let a = try store.createConversation(title: "Cold local A", language: nil)
        let b = try store.createConversation(title: "Cold local B", language: nil)
        let manager = ChatManager(container: container, restorePersistedState: false)
        Self.retainedManagers.append(manager)
        manager.currentConversationId = a.id
        let executionA = try XCTUnwrap(manager.executionForConversation(a.id))
        for _ in 0..<50 where executionA.permissionConfigurationOwner == .unknown { await Task.yield() }
        XCTAssertEqual(executionA.permissionConfigurationOwner, .inactive)
        XCTAssertEqual(manager.permissionConfigurationStatus(conversationID: a.id), "saved")
        manager.currentConversationId = b.id
        let executionB = try XCTUnwrap(manager.executionForConversation(b.id))
        for _ in 0..<50 where executionB.permissionConfigurationOwner == .unknown { await Task.yield() }
        XCTAssertEqual(executionB.permissionConfigurationOwner, .inactive)
        manager.currentConversationId = a.id
        XCTAssertEqual(manager.permissionConfigurationStatus(conversationID: a.id), "saved")
        XCTAssertEqual(manager.processingState, .idle)
    }

    func testCoreProviderEnvelopePreservesCanonicalHistoryWithoutPromptReplacement() throws {
        let json = #"""
        {
          "schema_version":1,
          "action":"ready",
          "messages":[
            {"id":"m1","role":"user","content":"打开设置"},
            {"id":"m2","role":"assistant","content":"已经打开。"},
            {"id":"m3","role":"user","content":"继续"}
          ],
          "tools":[{"type":"function","function":{"name":"web.search","parameters":{}}}],
          "payload_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "changed":false,
          "reason":"within_budget",
          "metrics":{
            "message_tokens":30,
            "tool_schema_tokens":10,
            "total_prompt_tokens":40,
            "prompt_token_limit":12000,
            "reserved_output_tokens":4096,
            "usage_percent":0.33,
            "before_tokens":40,
            "after_tokens":40,
            "trigger_tokens":10800,
            "target_tokens":8400,
            "archive_message_count":0,
            "transaction_count":0
          },
          "component_metrics":[]
        }
        """#
        let response = try JSONDecoder().decode(
            AgentCoreContextPrepareResponse.self,
            from: Data(json.utf8)
        )
        let prepared = try CorePreparedOutboundPayload(response: response)
        let envelopeData = try XCTUnwrap(
            prepared.providerEnvelopeJSON(
                systemPrompt: "composed policy",
                userQuery: "继续"
            ).data(using: .utf8)
        )
        let envelope = try XCTUnwrap(
            JSONSerialization.jsonObject(with: envelopeData) as? [String: Any]
        )
        let messages = try XCTUnwrap(envelope["messages"] as? [[String: Any]])
        let tools = try XCTUnwrap(envelope["tools"] as? [[String: Any]])

        XCTAssertEqual(envelope["schema_version"] as? Int, 1)
        XCTAssertEqual(envelope["system_prompt"] as? String, "composed policy")
        XCTAssertEqual(messages.compactMap { $0["role"] as? String }, ["user", "assistant", "user"])
        XCTAssertEqual(messages.compactMap { $0["content"] as? String }, ["打开设置", "已经打开。", "继续"])
        XCTAssertEqual(tools.count, 1)
        XCTAssertEqual(envelope["payload_digest"] as? String, String(repeating: "a", count: 64))
    }

    private final class InMemoryConversationStore: ConversationStore {
        var conversations: [UUID: Conversation] = [:]
        var messages: [UUID: [ConversationMessage]] = [:]

        func appendMessage(_ message: ConversationMessage) throws {
            guard conversations[message.conversationId] != nil else {
                throw NSError(domain: "InMemoryConversationStore", code: 1, userInfo: nil)
            }
            messages[message.conversationId, default: []].append(message)
        }

        func getMessages(conversationId: UUID, limit: Int) throws -> [ConversationMessage] {
            let rows = messages[conversationId, default: []].sorted { $0.createdAt < $1.createdAt }
            if limit > 0 && rows.count > limit {
                return Array(rows.suffix(limit))
            }
            return rows
        }

        func getConversation(conversationId: UUID) throws -> Conversation? {
            conversations[conversationId]
        }

        func createConversation(title: String?, language: String?) throws -> Conversation {
            let conversation = Conversation(title: title, language: language)
            conversations[conversation.id] = conversation
            return conversation
        }

        func deleteConversation(conversationId: UUID) throws {
            conversations.removeValue(forKey: conversationId)
            messages.removeValue(forKey: conversationId)
        }

        func exportConversation(conversationId: UUID) throws -> Data {
            try JSONEncoder().encode(messages[conversationId, default: []])
        }

        func getAllConversations() throws -> [Conversation] {
            Array(conversations.values)
        }

        func updateConversationTitle(conversationId: UUID, title: String) throws {
            guard var conversation = conversations[conversationId] else { return }
            conversation.title = title
            conversations[conversationId] = conversation
        }

        func setPromptResponseStyle(conversationId: UUID, style: PromptResponseStyle?) throws {
            guard var conversation = conversations[conversationId] else {
                throw NSError(domain: "InMemoryConversationStore", code: 1, userInfo: nil)
            }
            conversation.promptResponseStyle = style
            conversations[conversationId] = conversation
        }

        func updateConversationMetadata(conversationId: UUID, language: String?, selectedPromptTemplateID: UUID?, dataSourceOverrides: [String: Bool]?) throws {
            guard var conversation = conversations[conversationId] else { return }
            conversation.language = language
            conversation.selectedPromptTemplateID = selectedPromptTemplateID
            if let dataSourceOverrides {
                conversation.dataSourceOverrides = dataSourceOverrides
            }
            conversations[conversationId] = conversation
        }

        func upsertConversation(_ conversation: Conversation) throws {
            conversations[conversation.id] = conversation
        }

        func upsertMessages(conversationId: UUID, messages: [ConversationMessage]) throws {
            var existing = self.messages[conversationId, default: []]
            for message in messages where !existing.contains(where: { $0.id == message.id }) {
                existing.append(message)
            }
            self.messages[conversationId] = existing.sorted { $0.createdAt < $1.createdAt }
        }

        func importMessages(
            conversationId: UUID,
            messages: [ConversationMessage],
            source: ConversationMessageImportSource
        ) throws {
            _ = source
            for message in messages {
                try appendMessage(message)
            }
        }

    }

    @MainActor
    private final class AutoMemoryLLMClient: LLMClient, AgentModelClient, @unchecked Sendable {
        var generatedInputs: [ModelInput] = []
        var streamedInputs: [ModelInput] = []
        var emitsMemoryToolRequest = true
        var streamedResponseOverride: String?
        func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
            ChatManagerTests.noActionParseAgentEventStream()
        }

        func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
            generatedInputs.append(input)
            return ModelOutput(items: [], explain: "summary")
        }

        func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
            streamedInputs.append(input)
            let hiddenTool = """
            <nola_memory_tool>{"long_term_memory":["用户长期维护 Nola iOS 项目。"],"user_preferences":["用户希望默认中文输出，并先给结论。"]}</nola_memory_tool>
            """
            let defaultResponse = emitsMemoryToolRequest
                ? "记住了，我后续会按这个偏好回答。" + hiddenTool
                : "好的，我会继续按当前上下文回答。"
            let response = streamedResponseOverride ?? defaultResponse
            return AsyncThrowingStream { continuation in
                continuation.yield(response)
                continuation.finish()
            }
        }

        func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
            let hiddenTool = """
            <nola_memory_tool>{"long_term_memory":["用户长期维护 Nola iOS 项目。"],"user_preferences":["用户希望默认中文输出，并先给结论。"]}</nola_memory_tool>
            """
            let defaultResponse = emitsMemoryToolRequest
                ? "记住了，我后续会按这个偏好回答。" + hiddenTool
                : "好的，我会继续按当前上下文回答。"
            return streamedResponseOverride ?? defaultResponse
        }
    }

    @MainActor
    private final class ControlledStreamLLMClient: LLMClient, AgentModelClient, @unchecked Sendable {
        var responseContinuation: CheckedContinuation<String, Error>?

        func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
            ChatManagerTests.noActionParseAgentEventStream()
        }

        func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
            ModelOutput.empty
        }

        func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
            AsyncThrowingStream { continuation in
                continuation.finish()
            }
        }

        func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
            try await withCheckedThrowingContinuation { continuation in
                responseContinuation = continuation
            }
        }
    }

    func testDraftCallbacksRemainBoundToSelectedConversationAndAssistant() {
        let conversationA = UUID(), conversationB = UUID()
        let assistantA = UUID(), assistantB = UUID()
        XCTAssertTrue(ChatDraftPresentationOwnership.accepts(
            assistantID: assistantA, activeAssistantID: assistantA,
            ownerConversationID: conversationA, selectedConversationID: conversationA))
        XCTAssertFalse(ChatDraftPresentationOwnership.accepts(
            assistantID: assistantA, activeAssistantID: assistantA,
            ownerConversationID: conversationA, selectedConversationID: conversationB))
        XCTAssertFalse(ChatDraftPresentationOwnership.accepts(
            assistantID: assistantA, activeAssistantID: assistantB,
            ownerConversationID: conversationA, selectedConversationID: conversationA))
        XCTAssertFalse(ChatDraftPresentationOwnership.accepts(
            assistantID: assistantA, activeAssistantID: assistantA,
            ownerConversationID: nil, selectedConversationID: nil))
    }

    func testErrorStateDoesNotBlockInputOrShowInlineStatus() {
        let state = ChatProcessingState.error("无法连接服务器")
        XCTAssertFalse(state.blocksInput)
        XCTAssertFalse(state.showsInlineStatus)
    }

    func testColdStartPrefersCanonicalLocalPermissionWaitOverNewerIdleAndWebRuns() throws {
        let now = Date()
        let localWaiting = Conversation(
            id: UUID(uuidString: "11111111-2222-3333-4444-555555555555")!,
            title: "Local wait",
            createdAt: now.addingTimeInterval(-30),
            lastActiveAt: now.addingTimeInterval(-20),
            selectedModelInfo: ModelInfo(
                id: "local-provider",
                name: "Local",
                source: "local",
                isLocal: true
            )
        )
        let newerIdle = Conversation(
            id: UUID(uuidString: "22222222-3333-4444-5555-666666666666")!,
            title: "Idle",
            createdAt: now.addingTimeInterval(-20),
            lastActiveAt: now.addingTimeInterval(-10)
        )
        let newestWebWait = Conversation(
            id: UUID(uuidString: "33333333-4444-5555-6666-777777777777")!,
            title: "Web wait",
            createdAt: now.addingTimeInterval(-10),
            lastActiveAt: now,
            selectedModelInfo: ModelSelectionManager.webBackendModel()
        )
        let localRunID = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        let webRunID = "bbbbbbbb-cccc-dddd-eeee-ffffffffffff"
        let runs: [UUID: IOSConversationProjectionV1.RunActivity] = [
            localWaiting.id: .init(
                runID: localRunID,
                state: .waitingForPermission,
                activePartID: "permission-local",
                displayKey: nil,
                progress: nil
            ),
            newestWebWait.id: .init(
                runID: webRunID,
                state: .waitingForPermission,
                activePartID: "permission-web",
                displayKey: nil,
                progress: nil
            ),
        ]

        let selected = ChatManager.coldStartConversation(
            from: [newerIdle, localWaiting, newestWebWait],
            fallbackModel: nil,
            activeRun: { runs[$0] }
        )

        XCTAssertEqual(selected?.id, localWaiting.id)
        XCTAssertEqual(runs[selected?.id ?? newerIdle.id]?.runID, localRunID)
    }

    func testFirstUserMessageNamesDefaultConversation() throws {
        let store = InMemoryConversationStore()
        let container = ServiceContainer()
        container.conversationStore = store
        let manager = ChatManager(container: container, restorePersistedState: false)
        Self.retainedManagers.append(manager)
        let conversation = try store.createConversation(title: L10n.chatConversationDefaultTitle, language: nil)
        let firstQuery = "请帮我检查 iOS 对话标题是否使用首个 query"

        try manager.appendMessageToConversation(
            conversationId: conversation.id,
            message: ConversationMessage(conversationId: conversation.id, role: .user, text: firstQuery)
        )
        try manager.appendMessageToConversation(
            conversationId: conversation.id,
            message: ConversationMessage(conversationId: conversation.id, role: .user, text: "第二个 query 不能覆盖标题")
        )

        XCTAssertEqual(store.conversations[conversation.id]?.title, firstQuery)
    }

    func testFirstUserMessagePreservesExplicitConversationTitle() throws {
        let store = InMemoryConversationStore()
        let container = ServiceContainer()
        container.conversationStore = store
        let manager = ChatManager(container: container, restorePersistedState: false)
        Self.retainedManagers.append(manager)
        let conversation = try store.createConversation(title: "Pinned title", language: nil)

        try manager.appendMessageToConversation(
            conversationId: conversation.id,
            message: ConversationMessage(conversationId: conversation.id, role: .user, text: "首个 query 不应覆盖显式标题")
        )

        XCTAssertEqual(store.conversations[conversation.id]?.title, "Pinned title")
    }

    func testFirstNonEmptyUserMessageNamesDefaultConversation() throws {
        let store = InMemoryConversationStore()
        let container = ServiceContainer()
        container.conversationStore = store
        let manager = ChatManager(container: container, restorePersistedState: false)
        Self.retainedManagers.append(manager)
        let conversation = try store.createConversation(title: L10n.chatConversationDefaultTitle, language: nil)

        try manager.appendMessageToConversation(
            conversationId: conversation.id,
            message: ConversationMessage(conversationId: conversation.id, role: .user, text: " \n\t ")
        )
        XCTAssertEqual(store.conversations[conversation.id]?.title, L10n.chatConversationDefaultTitle)

        try manager.appendMessageToConversation(
            conversationId: conversation.id,
            message: ConversationMessage(conversationId: conversation.id, role: .user, text: "  第一行\n第二行  ")
        )

        XCTAssertEqual(store.conversations[conversation.id]?.title, "第一行 第二行")
    }

    func testLocalAgentModePromptStatesIOSNoCodeBoundary() {
        let prompt = ChatManager.localAgentModePrompt(for: "帮我分析复杂问题")

        XCTAssertTrue(prompt.contains("Nola Agent Mode"))
        XCTAssertTrue(prompt.contains("iOS-only Agent mode"))
        XCTAssertTrue(prompt.contains("must not claim it executed code"))
        XCTAssertTrue(prompt.contains("帮我分析复杂问题"))
    }

    func testContextManagerIncludesGlobalMemoryAndPreferences() async throws {
        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = InMemoryConversationStore()
        let conversation = try store.createConversation(title: "Memory", language: "zh-Hans")
        try store.appendMessage(ConversationMessage(conversationId: conversation.id, role: .user, text: "请记住我是长期维护这个仓库的人。"))
        try store.appendMessage(ConversationMessage(conversationId: conversation.id, role: .assistant, text: "好的，我会结合上下文继续协助你。"))

        let profileStore = CanonicalChatMemoryProfileStore(applicationRuntime: applicationRuntime)
        try profileStore.saveMemoryText(
            longTermMemory: "长期项目：Nola 本地优先聊天。",
            userPreferences: "默认使用中文，回答简洁。",
            expected: profileStore.loadProfile()
        )
        try profileStore.saveMCP(
            UserMCPServer(name: "Docs MCP", endpoint: "https://mcp.example.com/docs", description: "文档上下文"),
            expected: nil
        )
        try profileStore.saveSkill(
            UserUploadedContextItem(name: "Review Skill", content: "Always check correctness first."), expected: nil
        )
        let documents = try JSONEncoder().encode([
            UserUploadedContextItem(name: "Project Brief", content: "Nola is local-first.")
        ])
        _ = try applicationRuntime.putProductSetting(key: "assistant.uploaded_documents",
            valueJSON: String(decoding: documents, as: UTF8.self), updatedAtMs: 1)

        let manager = DefaultContextManager(store: store, profileStore: profileStore)
        let segments = try await manager.buildContext(for: conversation.id, maxTokens: 32_768)
        let canonicalContext = segments.map(\.content).joined(separator: "\n")
        XCTAssertTrue(canonicalContext.contains("默认使用中文"), canonicalContext)
        XCTAssertTrue(canonicalContext.contains("长期项目"), canonicalContext)
        XCTAssertFalse(canonicalContext.contains("https://mcp.example.com/docs"))
        XCTAssertTrue(canonicalContext.contains("Review Skill"), canonicalContext)
        XCTAssertTrue(canonicalContext.contains("Nola is local-first"), canonicalContext)

        // This verifies inclusion/accounting, not eviction under a 4k prompt
        // budget after reserving output. Leave room for the current Core policy.
        let usage = try await manager.estimateContextUsage(for: conversation.id, maxTokens: 32_768)
        XCTAssertEqual(usage.longTermMemory, "长期项目：Nola 本地优先聊天。")
        XCTAssertEqual(usage.userPreferences, "默认使用中文，回答简洁。")
        XCTAssertGreaterThan(usage.longTermMemoryTokens, 0)
        // Stored/learned preferences are reference memory, not an elevated
        // user-instruction component. Prove inclusion using Core's accepted
        // component identities rather than the storage namespace label.
        XCTAssertEqual(usage.userPreferencesTokens, 0)
        let prepared = try await manager.prepareOutboundPayload(for: conversation.id,
            maxTokens: 32_768, additionalComponents: [], tools: [])
        let profileIDs = Set(profileStore.loadProfile().activeMemoryRecords.map { "profile:memory:\($0.id)" })
        XCTAssertEqual(profileIDs.count, 2)
        let usedMemoryIDs = Set(prepared.componentMetrics.filter { $0.kind == .memory }.flatMap(\.usedIds))
        XCTAssertTrue(profileIDs.isSubset(of: usedMemoryIDs), "Both durable namespaces must survive Core budgeting")
        XCTAssertEqual(Set(prepared.contextComponents.filter { profileIDs.contains($0.id) }.map(\.kind)), [.memory])
        XCTAssertEqual(usage.mcpServersTokens, 0)
        XCTAssertGreaterThan(usage.uploadedSkillsTokens, 0)
        XCTAssertGreaterThan(usage.uploadedDocumentsTokens, 0)
        XCTAssertTrue(canonicalContext.contains("available_skill"), canonicalContext)
        XCTAssertTrue(canonicalContext.contains("skill_invoke"), canonicalContext)
        XCTAssertTrue(canonicalContext.contains("do not claim the Skill was used unless that call succeeds"), canonicalContext)
    }

    func testOrdinaryIOSChatDoesNotExecuteUnsupportedInlineMemoryMarkup() async throws {
        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let container = ServiceContainer(applicationRuntime: applicationRuntime)
        let store = try XCTUnwrap(container.conversationStore as? LocalConversationStore)

        let llm = AutoMemoryLLMClient()
        container.llmClient = llm
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = try configuredHTTPFixture(container: container, id: "ios-local-contract-test")
        Self.retainedManagers.append(manager)

        let response = try await manager.processMessage("以后默认用中文先给结论。我长期维护 Nola iOS 项目。")
        XCTAssertTrue(response.contains("记住了"))
        XCTAssertTrue(response.contains("nola_memory_tool"))

        let saved = container.chatMemoryProfileStore.loadProfile()
        XCTAssertTrue(saved.longTermMemory.isEmpty)
        XCTAssertTrue(saved.userPreferences.isEmpty)
        let requests = ChatManagerHTTPFixture.requests["ios-local-contract-test"] ?? []
        XCTAssertFalse(requests.isEmpty)
        XCTAssertFalse(requests.contains { String(decoding: $0, as: UTF8.self).contains("Nola Hidden Memory Tool") })
        XCTAssertFalse(llm.generatedInputs.contains { $0.userPrefs["purpose"] == "extract_chat_memory_profile" })
        let assistantHistory = try store.getMessages(conversationId: manager.currentConversationId!, limit: 0)
            .filter { $0.role == .assistant }
            .map(\.text)
            .joined(separator: "\n")
        XCTAssertTrue(assistantHistory.contains("nola_memory_tool"))
    }

    func testOrdinaryIOSChatDoesNotPersistMemoryWithoutLLMToolRequest() async throws {
        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try LocalConversationStore(applicationRuntime: applicationRuntime)
        let container = ServiceContainer(applicationRuntime: applicationRuntime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)

        let llm = AutoMemoryLLMClient()
        llm.emitsMemoryToolRequest = false
        container.llmClient = llm
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = try configuredHTTPFixture(container: container, id: "ios-local-contract-test")
        Self.retainedManagers.append(manager)

        _ = try await manager.processMessage("以后默认用中文先给结论。我长期维护 Nola iOS 项目。")
        try await Task.sleep(nanoseconds: 200_000_000)

        let saved = container.chatMemoryProfileStore.loadProfile()
        XCTAssertTrue(saved.longTermMemory.isEmpty)
        XCTAssertTrue(saved.userPreferences.isEmpty)
        XCTAssertFalse(llm.generatedInputs.contains { $0.userPrefs["purpose"] == "extract_chat_memory_profile" })
    }

    func testConversationStopAndPermissionResponseDoNotTouchHiddenExecution() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = store
        let a = try store.createConversation(title: "A", language: nil)
        let b = try store.createConversation(title: "B", language: nil)
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.currentConversationId = a.id
        let executionA = try XCTUnwrap(manager.executionForConversation(a.id))
        manager.currentConversationId = b.id
        let executionB = try XCTUnwrap(manager.executionForConversation(b.id))
        let requestA = HostAgentPermissionRequest(id: "same-local-id", allowConversation: true, toolName: "test", match: "A", nonce: "nonce-a")
        let requestB = HostAgentPermissionRequest(id: "same-local-id", allowConversation: true, toolName: "test", match: "B", nonce: "nonce-b")
        var sentA = false, sentB = false
        let waitA = executionA.startHostPermissionInteraction(requestA, conversationId: a.id,
            isCurrent: { true }, send: { _ in sentA = true })
        let waitB = executionB.startHostPermissionInteraction(requestB, conversationId: b.id,
            isCurrent: { true }, send: { _ in sentB = true })
        defer { waitA.cancel(); waitB.cancel(); executionA.cancelHostPermissionInteraction(); executionB.cancelHostPermissionInteraction() }
        for _ in 0..<1_000 where executionA.pendingHostPermissionRequest == nil || executionB.pendingHostPermissionRequest == nil {
            await Task.yield()
        }
        // This is the same handler factory used by the rendered permission card.
        // A delayed tap on B after selection changes to A must not resolve A's
        // identically named request.
        let capturedCard = executionB.permissionDecisionHandler(localRequest: nil, hostRequest: requestB)
        manager.currentConversationId = a.id
        capturedCard(requestB.id, .deny)
        for _ in 0..<1_000 where !sentB { await Task.yield() }
        XCTAssertTrue(sentB)
        XCTAssertFalse(sentA)
        XCTAssertEqual(executionA.pendingHostPermissionRequest?.nonce, "nonce-a")
        executionA.processingState = .processingWithCloud
        executionB.processingState = .processingWithCloud
        manager.currentConversationId = b.id
        manager.cancelCurrentGeneration()
        XCTAssertTrue(executionA.processingState.blocksInput)
        XCTAssertNotNil(executionA.pendingHostPermissionRequest)
        manager.currentConversationId = a.id
        XCTAssertTrue(manager.processingState.blocksInput)
        XCTAssertEqual(manager.pendingHostPermissionRequest?.nonce, "nonce-a")
    }

    func testConcurrentConversationsKeepBothHTTPExecutionsAndSelectedProjection() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        addTeardownBlock { try await Self.removeStreamingFixtureRoot(root) }
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let a = try store.createConversation(title: "A", language: nil)
        let b = try store.createConversation(title: "B", language: nil)
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)
        let clientA = ControlledStreamLLMClient(), clientB = ControlledStreamLLMClient()
        let idA = "concurrent-a-\(UUID().uuidString)", idB = "concurrent-b-\(UUID().uuidString)"
        container.llmClient = clientA
        let modelA = try configuredHTTPFixture(container: container, id: idA)
        container.llmClient = clientB
        let modelB = try configuredHTTPFixture(container: container, id: idB)
        try configureConversation(a.id, model: modelA, store: store)
        try configureConversation(b.id, model: modelB, store: store)
        defer {
            for id in [idA, idB] {
                ChatManagerHTTPFixture.responders.removeValue(forKey: id)
                ChatManagerHTTPFixture.requests.removeValue(forKey: id)
            }
        }
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = modelA
        manager.currentConversationId = a.id
        let executionA = try XCTUnwrap(manager.executionForConversation(a.id))
        let taskA = try submitThroughUseCase(manager, owner: a.id, text: "Answer for A")
        for _ in 0..<200 where clientA.responseContinuation == nil {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertNotNil(clientA.responseContinuation)
        manager.selectedModelInfo = modelB
        manager.currentConversationId = b.id
        let executionB = try XCTUnwrap(manager.executionForConversation(b.id))
        XCTAssertFalse(executionA === executionB)
        XCTAssertFalse(manager.processingState.blocksInput)
        let taskB = try submitThroughUseCase(manager, owner: b.id, text: "Answer for B")
        for _ in 0..<200 where clientB.responseContinuation == nil {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertNotNil(clientB.responseContinuation)
        XCTAssertTrue(executionA.processingState.blocksInput)
        XCTAssertTrue(executionB.processingState.blocksInput)
        manager.currentConversationId = a.id
        clientB.responseContinuation?.resume(returning: "B completed independently")
        clientB.responseContinuation = nil
        let answerB = try await taskB.value
        XCTAssertEqual(answerB, "B completed independently")
        XCTAssertTrue(manager.processingState.blocksInput)
        XCTAssertNotEqual(manager.currentResponse, answerB)
        clientA.responseContinuation?.resume(returning: "A completed independently")
        clientA.responseContinuation = nil
        let answerA = try await taskA.value
        XCTAssertEqual(manager.currentResponse, answerA)
        manager.currentConversationId = b.id
        XCTAssertEqual(manager.currentResponse, answerB)
        XCTAssertTrue(try store.getMessages(conversationId: a.id, limit: 0).contains { $0.text == answerA })
        XCTAssertFalse(try store.getMessages(conversationId: a.id, limit: 0).contains { $0.text == answerB })
        XCTAssertTrue(try store.getMessages(conversationId: b.id, limit: 0).contains { $0.text == answerB })
    }

    func testStreamingReplyPersistsToOriginalConversationAfterSelectionChanges() async throws {
        let (applicationRuntime, root) = try makeApplicationRuntime()
        // Cleanup runs after the test's execution result is recorded. A
        // secondary filesystem error must neither replace nor hide the actual
        // generation error, and cleanup failures remain test failures.
        addTeardownBlock { try await Self.removeStreamingFixtureRoot(root) }
        let store = try LocalConversationStore(applicationRuntime: applicationRuntime)
        let first = try store.createConversation(title: "First", language: nil)
        let second = try store.createConversation(title: "Second", language: nil)

        let container = ServiceContainer(applicationRuntime: applicationRuntime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)
        let client = ControlledStreamLLMClient()
        container.llmClient = client

        let manager = ChatManager(container: container, restorePersistedState: false)
        let model = try configuredHTTPFixture(container: container, id: "controlled-stream-test")
        try configureConversation(first.id, model: model, store: store)
        try configureConversation(second.id, model: model, store: store)
        manager.selectedModelInfo = model
        manager.currentConversationId = first.id
        Self.retainedManagers.append(manager)

        let retainedManagerID = ObjectIdentifier(manager)
        addTeardownBlock {
            await MainActor.run {
                Self.retainedManagers.removeAll { ObjectIdentifier($0) == retainedManagerID }
                ChatManagerHTTPFixture.responders.removeValue(forKey: "controlled-stream-test")
                ChatManagerHTTPFixture.requests.removeValue(forKey: "controlled-stream-test")
            }
        }

        let userMessageID = UUID()
        let assistantMessageID = IOSCanonicalAgentTurnIdentity(
            userMessageID: userMessageID
        ).visibleAssistantMessageID
        let generation = Task {
            try await manager.processMessage(
                "请检查宿主机上的网页状态",
                userMessageID: userMessageID,
                assistantMessageID: assistantMessageID
            )
        }
        for _ in 0..<40 {
            let hasUserMessage = (try? store.getMessages(conversationId: first.id, limit: 0))?
                .contains { $0.role == .user } == true
            if client.responseContinuation != nil && hasUserMessage {
                break
            }
            try await Task.sleep(nanoseconds: 25_000_000)
        }

        XCTAssertNotNil(client.responseContinuation)
        XCTAssertTrue(try store.getMessages(conversationId: first.id, limit: 0).contains { $0.role == .user })

        manager.currentConversationId = second.id
        client.responseContinuation?.resume(returning: "宿主机网页状态检查完成。")
        client.responseContinuation = nil

        let response = try await generation.value
        XCTAssertEqual(response, "宿主机网页状态检查完成。")

        let firstMessages = try store.getMessages(conversationId: first.id, limit: 0)
        let secondMessages = try store.getMessages(conversationId: second.id, limit: 0)
        let firstAssistantMessages = firstMessages
            .filter { $0.role == .assistant }
            .map(\.text)
        let secondAssistantMessages = secondMessages
            .filter { $0.role == .assistant }
            .map(\.text)

        XCTAssertEqual(firstAssistantMessages, ["宿主机网页状态检查完成。"])
        XCTAssertTrue(secondAssistantMessages.isEmpty)
        XCTAssertEqual(firstMessages.first(where: { $0.role == .user })?.id, userMessageID)
        XCTAssertEqual(firstMessages.first(where: { $0.role == .assistant })?.id, assistantMessageID)
    }

    func testOrdinaryIOSChatDoesNotTreatUnsupportedNameMarkupAsMemory() async throws {
        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try LocalConversationStore(applicationRuntime: applicationRuntime)
        let container = ServiceContainer(applicationRuntime: applicationRuntime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)

        let llm = AutoMemoryLLMClient()
        llm.streamedResponseOverride = """
        记住了，我会这样称呼你。<nola_memory_tool>{"long_term_memory":["用户姓名：小林（用户明确要求记住）"],"user_preferences":[]}</nola_memory_tool>
        """
        container.llmClient = llm
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = try configuredHTTPFixture(container: container, id: "ios-local-contract-test")
        Self.retainedManagers.append(manager)

        let response = try await manager.processMessage("请记住我的名字叫小林，以后都这样叫我。")
        XCTAssertTrue(response.contains("记住了"))
        XCTAssertTrue(response.contains("nola_memory_tool"))

        let saved = container.chatMemoryProfileStore.loadProfile()
        XCTAssertTrue(saved.longTermMemory.isEmpty)
        XCTAssertTrue(saved.userPreferences.isEmpty)
    }

    func testOrdinaryIOSChatDoesNotPersistWhenMemoryToolPayloadIsEmpty() async throws {
        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try LocalConversationStore(applicationRuntime: applicationRuntime)
        let container = ServiceContainer(applicationRuntime: applicationRuntime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)

        let llm = AutoMemoryLLMClient()
        llm.streamedResponseOverride = """
        我理解了。<nola_memory_tool>{"long_term_memory":[],"user_preferences":[]}</nola_memory_tool>
        """
        container.llmClient = llm
        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = try configuredHTTPFixture(container: container, id: "ios-local-contract-test")
        Self.retainedManagers.append(manager)

        _ = try await manager.processMessage("以后你知道就好。")
        try await Task.sleep(nanoseconds: 200_000_000)

        let saved = container.chatMemoryProfileStore.loadProfile()
        XCTAssertTrue(saved.longTermMemory.isEmpty)
        XCTAssertTrue(saved.userPreferences.isEmpty)
    }

    func testAmbiguousMemoryQuestionContinuesThroughConfiguredModelPath() async throws {
        @MainActor
        final class ParserNoActionClient: LLMClient, AgentModelClient, @unchecked Sendable {
            func generateAgentTurn(request: AgentModelRequest) -> AsyncThrowingStream<AgentModelEvent, Error> {
                ChatManagerTests.noActionParseAgentEventStream()
            }
            func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
                ModelOutput.empty
            }

            func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
                AsyncThrowingStream { continuation in
                    continuation.yield("这是常规聊天回复。")
                    continuation.finish()
                }
            }

            func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
                "这是常规聊天回复。"
            }
        }

        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try LocalConversationStore(applicationRuntime: applicationRuntime)
        let container = ServiceContainer(applicationRuntime: applicationRuntime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)
        container.llmClient = ParserNoActionClient()

        let manager = ChatManager(container: container, restorePersistedState: false)
        manager.selectedModelInfo = try configuredHTTPFixture(container: container, id: "non-action-parse-test")
        Self.retainedManagers.append(manager)

        let response = try await manager.processMessage("你还记得我喜欢什么吗")

        XCTAssertEqual(response, "这是常规聊天回复。")
    }

    private static func noActionParseArguments() -> AgentCoreJSONValue {
        .object([
            "intent": .string("none"),
            "type": .null,
            "title": .null,
            "description": .null,
            "targetDate": .null,
            "isRecurring": .null,
            "recurrenceRule": .null,
            "relatedPerson": .null,
            "amount": .null,
            "currency": .null,
            "dueDate": .null,
            "scheduledAction": .null,
            "confidence": .number(0.3),
            "reason": .string("No actionable task detected."),
        ])
    }

    private static func noActionParseAgentEventStream() -> AsyncThrowingStream<AgentModelEvent, Error> {
        AsyncThrowingStream { continuation in
            continuation.yield(.completed(.init(
                content: "",
                toolCalls: [.init(
                    id: "task-parse",
                    name: "submit_task_parse",
                    arguments: noActionParseArguments()
                )],
                finishReason: "tool_calls"
            )))
            continuation.finish()
        }
    }

    func testTokenCountRemainsCorePromptUsageAfterTruncationNotice() async throws {
        // When a thinking model produces long reasoning (>200 chars) but a very short reply
        // (<20 chars), ChatManager appends a UI-only truncation notice to the displayed text
        // (finalClean / displayText). The token count estimate must be derived from the raw
        // LLM reply (persistText), NOT from the notice-inflated display text.
        final class ThinkingShortReplyClient: LLMClient {
            // Reasoning >200 chars triggers the truncation-notice injection path
            let thinking = String(repeating: "思", count: 210)
            // Reply <20 chars so trimmedFinal.count < 20 is satisfied
            let reply = "好的"

            func generateSummary(input: ModelInput, timeout: TimeInterval?) async throws -> ModelOutput {
                ModelOutput(items: [], explain: "")
            }

            func generateSummaryStream(input: ModelInput) -> AsyncThrowingStream<String, Error> {
                let chunk = "<think>\(thinking)</think>\(reply)"
                return AsyncThrowingStream { continuation in
                    continuation.yield(chunk)
                    continuation.finish()
                }
            }

            func chat(message: String, systemPrompt: String?, enableThinking: Bool, timeout: TimeInterval?) async throws -> String {
                reply
            }
        }

        let client = ThinkingShortReplyClient()
        let (applicationRuntime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try LocalConversationStore(applicationRuntime: applicationRuntime)
        let container = ServiceContainer(applicationRuntime: applicationRuntime)
        container.conversationStore = store
        container.contextManager = DefaultContextManager(store: store, profileStore: container.chatMemoryProfileStore)
        container.llmClient = client
        let manager = ChatManager(container: container, restorePersistedState: false)
        // Use a unique model name so ModelParametersStore returns defaults (showThinking = true)
        manager.selectedModelInfo = try configuredHTTPFixture(container: container, id: "thinking-short-reply-test")
        Self.retainedManagers.append(manager)

        _ = try await manager.processMessage("测试")

        // The visible truncation notice and streamed output must not overwrite the
        // authoritative prompt count returned by the shared context engine.
        let tokenCountFromNoticeOnly = Telemetry.estimateTokens(from: String(repeating: "x", count: 90))
        XCTAssertGreaterThan(manager.currentTokenCount, 0)
        XCTAssertNotEqual(manager.currentTokenCount, tokenCountFromNoticeOnly)
    }

    func testPreparedDataSourceUsageIsRecordedOnlyAtExplicitRequestBoundary() async throws {
        let suite = "PreparedDataSourceUsageTests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        let source = URLDataSource(
            url: "file:///tmp/nola-context.json",
            format: .json,
            name: "Local context"
        )
        let fetcher = URLFetcherService(logger: Logger(), maxURLs: 20, userDefaults: defaults)
        try fetcher.addSource(source)
        let store = InMemoryConversationStore()
        let contextManager = DefaultContextManager(
            store: store,
            profileStore: CanonicalChatMemoryProfileStore()
        )
        contextManager.urlFetcher = fetcher
        let conversationID = UUID()
        let response = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [AgentCoreContextPrepareMessage(id: "user", role: "user", content: "question")],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 10,
                toolSchemaTokens: 0,
                totalPromptTokens: 10,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 10,
                beforeTokens: 10,
                afterTokens: 10
            ),
            componentMetrics: [
                AgentCoreContextComponentMetric(
                    kind: .platformData,
                    usedTokens: 4,
                    droppedTokens: 0,
                    truncatedTokens: 0,
                    usedIds: ["data_source:\(source.id.uuidString)"],
                    droppedIds: [],
                    truncatedIds: []
                )
            ]
        )
        let payload = try CorePreparedOutboundPayload(response: response)

        XCTAssertTrue(fetcher.usageRecords.isEmpty, "Preparing a payload must not count as disclosure")
        await contextManager.recordPreparedDataSourceUsage(
            payload: payload,
            conversationId: conversationID,
            disclosedToCloud: true
        )
        fetcher.waitForPersistence()

        let restored = URLFetcherService(logger: Logger(), maxURLs: 20, userDefaults: defaults)
        let record = try XCTUnwrap(restored.usageRecords.first)
        XCTAssertEqual(record.sourceID, source.id)
        XCTAssertEqual(record.sourceTitle, source.name)
        XCTAssertEqual(record.conversationID, conversationID)
        XCTAssertEqual(record.consumer, "chat")
        XCTAssertEqual(record.purpose, "conversation_context")
        XCTAssertEqual(record.disclosedToCloud, true)
    }
}

extension ChatManagerTests {
    func testCustomPermissionInstructionUsesTypedInterventionForRemoteAndDeviceCore() throws {
        let request = HostAgentPermissionRequest(id: "original-ask", allowConversation: true, toolName: "write_file", argsPreview: "original bytes", nonce: "nonce", hostId: "host", sessionId: "session")
        let response = try XCTUnwrap(HostAgentPermissionDecision.validatedInstruction("  Keep the destination unchanged  ", requestID: request.id))
        XCTAssertFalse(response.isAllowed, "Guidance must never authorize execution")
        let remote = ChatManager.hostPermissionDecisionPayload(for: request, decision: response)
        let resolution = try XCTUnwrap(remote["resolution"] as? [String: Any])
        XCTAssertEqual(resolution["type"] as? String, "intervention")
        XCTAssertNil(resolution["decision"])
        XCTAssertNil(remote["grant"])
        XCTAssertEqual(remote["request_id"] as? String, "original-ask")
        XCTAssertEqual(remote["nonce"] as? String, "nonce")
        let intervention = try XCTUnwrap(resolution["intervention"] as? [String: Any])
        XCTAssertEqual(intervention["type"] as? String, "provide_instruction")
        XCTAssertEqual(intervention["instruction"] as? String, "Keep the destination unchanged")
        let local = try DeviceAgentCoordinator.permissionInstructionPayload(requestId: request.id, instruction: "Keep the destination unchanged")
        let localObject = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(local)) as? [String: Any])
        XCTAssertEqual(localObject["type"] as? String, "permission_resolved")
        XCTAssertEqual(localObject["request_id"] as? String, request.id)
        XCTAssertEqual(try JSONSerialization.data(withJSONObject: localObject["resolution"]!, options: .sortedKeys), try JSONSerialization.data(withJSONObject: resolution, options: .sortedKeys))
        XCTAssertNil(localObject["grant"])
        XCTAssertNil(localObject["input"])
        XCTAssertEqual(request.argsPreview, "original bytes")
    }

    func testInstructionBudgetsKeepExplicitApprovalAvailableForSuccessorAsk() throws {
        XCTAssertNil(HostAgentPermissionDecision.validatedInstruction(" \n "))
        XCTAssertNil(HostAgentPermissionDecision.validatedInstruction(String(repeating: "字", count: 1366)))
        let history = (0..<16).map { AgentPermissionInstruction(permissionRequestId: "ask-\($0)", instruction: "guidance") }
        XCTAssertNil(HostAgentPermissionDecision.validatedInstruction("next", requestID: "successor", existing: history))
        let full = (0..<4).map { _ in AgentPermissionInstruction(permissionRequestId: String(repeating: "r", count: 97), instruction: String(repeating: "x", count: 4000)) }
        XCTAssertNil(HostAgentPermissionDecision.validatedInstruction("next", requestID: "successor", existing: full))
        let successor = HostAgentPermissionRequest(id: "successor", allowConversation: true, toolName: "write_file", permissionInstructions: history)
        for (decision, value) in [(HostAgentPermissionDecision.allowOnce, "allow_once"), (.deny, "deny")] {
            let payload = ChatManager.hostPermissionDecisionPayload(for: successor, decision: decision)
            XCTAssertEqual(payload["request_id"] as? String, "successor")
            XCTAssertEqual((payload["resolution"] as? [String: Any])?["decision"] as? String, value)
        }
        XCTAssertThrowsError(try DeviceAgentCoordinator.permissionInstructionPayload(requestId: "successor", instruction: String(repeating: "x", count: 4097)))
    }
}


extension ChatManagerTests {
    func testRemotePermissionRetryKeepsOriginalRequestAndRejectsStaleCard() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let manager = ChatManager(container: ServiceContainer(applicationRuntime: runtime), restorePersistedState: false)
        Self.retainedManagers.append(manager)
        manager.processingState = .streaming(progress: 0.5)
        let first = HostAgentPermissionRequest(id: "permission-1", allowConversation: true, toolName: "test_tool", match: "original", nonce: "original-nonce")
        var submissions: [[String: Any]] = []
        let interaction = Task { @MainActor in
            try await manager.submitHostPermissionInteraction(first, conversationId: nil, isCurrent: { true }) { payload in
                submissions.append(payload)
                if submissions.count == 1 { throw URLError(.networkConnectionLost) }
            }
        }
        for _ in 0..<1_000 where manager.pendingHostPermissionRequest?.id != first.id { await Task.yield() }
        XCTAssertEqual(manager.pendingHostPermissionRequest?.id, first.id)
        manager.resolveHostPermissionRequest(requestId: "stale-card", .allowOnce)
        XCTAssertTrue(submissions.isEmpty)
        manager.resolveHostPermissionRequest(requestId: first.id, .provideInstruction("keep original arguments"))
        for _ in 0..<1_000 where submissions.isEmpty { await Task.yield() }
        XCTAssertEqual(submissions.count, 1)
        XCTAssertNotNil(manager.hostPermissionSubmissionError)
        XCTAssertTrue(manager.processingState.blocksInput)
        XCTAssertEqual(manager.pendingHostPermissionRequest?.id, first.id)
        manager.resolveHostPermissionRequest(requestId: first.id, .provideInstruction("keep original arguments"))
        let result = try await interaction.value
        XCTAssertEqual(result, .provideInstruction("keep original arguments"))
        XCTAssertEqual(submissions.count, 2)
        XCTAssertEqual(submissions[0]["id"] as? String, first.id)
        XCTAssertEqual(submissions[1]["nonce"] as? String, "original-nonce")
        XCTAssertEqual(NSDictionary(dictionary: submissions[0]), NSDictionary(dictionary: submissions[1]))
        XCTAssertNil(manager.pendingHostPermissionRequest)
        XCTAssertNil(manager.hostPermissionSubmissionError)

        let second = HostAgentPermissionRequest(id: "permission-2", allowConversation: true, toolName: "test_tool", match: "original")
        let successor = Task { @MainActor in
            try await manager.submitHostPermissionInteraction(second, conversationId: nil, isCurrent: { true }) { payload in
                submissions.append(payload)
            }
        }
        for _ in 0..<1_000 where manager.pendingHostPermissionRequest?.id != second.id { await Task.yield() }
        manager.resolveHostPermissionRequest(requestId: first.id, .provideInstruction("old draft"))
        manager.resolveHostPermissionRequest(requestId: first.id, .allowConversation)
        manager.resolveHostPermissionRequest(requestId: first.id, .deny)
        await Task.yield()
        XCTAssertEqual(submissions.count, 2)
        XCTAssertEqual(manager.pendingHostPermissionRequest?.id, second.id)
        manager.resolveHostPermissionRequest(requestId: second.id, .deny)
        let successorDecision = try await successor.value
        XCTAssertEqual(successorDecision, .deny)
        XCTAssertEqual(submissions.count, 3)
        XCTAssertEqual(submissions[2]["request_id"] as? String, second.id)
    }
}


extension ChatManagerTests {
    func testPermissionStreamShowsSuccessorBeforeLostHTTPReplyAndIgnoresLateReply() async throws {
        let (runtime, root) = try makeApplicationRuntime()
        defer { try? FileManager.default.removeItem(at: root) }
        let manager = ChatManager(container: ServiceContainer(applicationRuntime: runtime), restorePersistedState: false)
        Self.retainedManagers.append(manager)
        manager.processingState = .streaming(progress: 0.5)
        let first = HostAgentPermissionRequest(id: "accepted-p1", allowConversation: true, toolName: "tool", match: "original")
        let second = HostAgentPermissionRequest(id: "successor-p2", allowConversation: true, toolName: "tool", match: "original")
        let pair = AsyncStream<HostAgentPermissionRequest>.makeStream()
        var lateHTTPReply: CheckedContinuation<Void, Error>?
        var sentIDs: [String] = []
        var resolvedIDs: [String] = []
        var interactions: [Task<Void, Never>] = []
        let consumer = Task { @MainActor in
            defer { manager.cancelHostPermissionInteraction() }
            for await request in pair.stream {
                interactions.append(manager.startHostPermissionInteraction(
                    request, conversationId: nil, isCurrent: { true },
                    send: { payload in
                        sentIDs.append(payload["request_id"] as! String)
                        if request.id == first.id {
                            // The broker has accepted p1 and published p2 while the p1
                            // HTTP acknowledgement is still lost/in flight.
                            pair.continuation.yield(second)
                            try await withCheckedThrowingContinuation { lateHTTPReply = $0 }
                        }
                    },
                    onSubmitted: { _ in resolvedIDs.append(request.id) }
                ))
            }
        }
        pair.continuation.yield(first)
        for _ in 0..<1_000 where manager.pendingHostPermissionRequest?.id != first.id { await Task.yield() }
        // Starting publishes immediately; let its task install the decision continuation.
        await Task.yield()
        manager.resolveHostPermissionRequest(requestId: first.id, .provideInstruction("retain call"))
        for _ in 0..<1_000 where manager.pendingHostPermissionRequest?.id != second.id { await Task.yield() }
        XCTAssertEqual(manager.pendingHostPermissionRequest?.id, second.id)
        XCTAssertEqual(sentIDs, [first.id])
        XCTAssertNotNil(lateHTTPReply)
        lateHTTPReply?.resume(throwing: URLError(.networkConnectionLost))
        lateHTTPReply = nil
        if let old = interactions.first { await old.value }
        XCTAssertEqual(manager.pendingHostPermissionRequest?.id, second.id)
        XCTAssertNil(manager.hostPermissionSubmissionError)
        XCTAssertTrue(resolvedIDs.isEmpty)
        manager.resolveHostPermissionRequest(requestId: first.id, .deny)
        XCTAssertEqual(manager.pendingHostPermissionRequest?.id, second.id)
        manager.resolveHostPermissionRequest(requestId: second.id, .allowOnce)
        if let current = interactions.last { await current.value }
        XCTAssertEqual(sentIDs, [first.id, second.id])
        XCTAssertEqual(resolvedIDs, [second.id])
        XCTAssertNil(manager.pendingHostPermissionRequest)

        // End-of-stream cancels an unanswered ask without posting an invented deny.
        pair.continuation.yield(HostAgentPermissionRequest(id: "terminal-p3", allowConversation: true, toolName: "tool", match: "original"))
        for _ in 0..<1_000 where manager.pendingHostPermissionRequest?.id != "terminal-p3" { await Task.yield() }
        pair.continuation.finish()
        await consumer.value
        for interaction in interactions { await interaction.value }
        XCTAssertNil(manager.pendingHostPermissionRequest)
        XCTAssertEqual(sentIDs, [first.id, second.id])
        XCTAssertEqual(resolvedIDs, [second.id])
    }
}
