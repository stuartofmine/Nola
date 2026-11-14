// SPDX-License-Identifier: Apache-2.0
// Actual native provider/Task pump with isolated Core and deterministic HTTP transport.
// This is not real-model acceptance evidence.
import Foundation
import XCTest
@testable import Nola

@MainActor final class IOSExpertNativeTaskIntegrationTests: XCTestCase {
    func testMainAgentDispatchUsesCommonTaskPumpOwnModelAndSealedReopenDoesNotRerun() async throws {
        try await run(cancel: false)
    }
    func testMainAgentCancellationWaitsForActualTargetTransportStop() async throws {
        try await run(cancel: true)
    }

    func testCommittedPeerAdmissionLostAcknowledgementRecoversOriginalIntent() async throws {
        try await run(cancel: false, lostAcknowledgement: "admit_peer")
    }
    func testPreparedPeerTaskLostAcknowledgementRecoversFrozenConfiguration() async throws {
        try await run(cancel: false, lostAcknowledgement: "prepare_peer_task")
    }

    func testBusyStreamingExpertCancellationJoinsTransportAndSealsCanonicalResult() async throws {
        try await run(cancel: true, busyStreaming: true)
    }

    func testOwnerTransportFailureStopsIndependentBusyProducerWithoutInventingTerminalReceipt() async throws {
        try await run(cancel: true, busyStreaming: true, ownerTransportFailure: true)
    }

    func testTwoMainRunsInSameDatabaseMayReuseProviderCallIDs() async throws {
        try await run(cancel: false, repeatsMainRun: true)
    }

    private func run(cancel: Bool, lostAcknowledgement: String? = nil, busyStreaming: Bool = false,
                     ownerTransportFailure: Bool = false, repeatsMainRun: Bool = false) async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-native-peer-pump-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state.db")
        let fault = NativePeerAcknowledgementFault(kind: lostAcknowledgement, closeOwnerPump: ownerTransportFailure)
        defer { fault.cancelObserver() }
        var runtime = try IOSApplicationRuntime(databaseURL: database, expertHostAcknowledgementObserver: fault.observe,
            childPumpObserver: fault.observePump)
        NativePeerHTTPFixture.reset(runtime: runtime, cancel: cancel, busy: busyStreaming)
        URLProtocol.registerClass(NativePeerHTTPFixture.self)
        defer { URLProtocol.unregisterClass(NativePeerHTTPFixture.self); NativePeerHTTPFixture.clear() }
        let anonymous = try JSONSerialization.jsonObject(with: JSONEncoder().encode(IOSCloudCredentialBinding.anonymous))
        let source: [String: Any] = ["baseURL": "https://ios-native-peer-pump.invalid/v1", "wireProtocol": "responses", "cloudCredential": anonymous, "capabilities": ["chat"],
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true, "samplingParameters": true],
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        for model in ["source", "target"] {
            _ = try runtime.upsertModelProfileCAS(.init(id: model + "-profile", displayName: model, provider: "openai-compatible", modelName: model + "-model",
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: #"{"maxContextLength":131072,"maxOutputTokens":1024}"#, selected: model == "source", enabled: true,
                revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil), expectedRevision: 0)
        }
        let service = IOSAgentRoomService(runtime: runtime)
        var directory = try service.directory()
        directory = try service.createAgent(name: "target", description: "Native Task fixture", defaults: .object([
            "modelSelectionID": .string("configured:target-profile"), "expertReasoningEffort": .string("high")]), directory: directory)
        let targetAgent = try XCTUnwrap(directory.agents.first { $0.name == "target" })
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let parent = try store.createConversation(title: "Main shared Task", language: nil)
        let joined = try service.joinMention(agentID: targetAgent.id, expectedAgentRevision: targetAgent.revision, conversationID: parent.id.uuidString)
        let target = try XCTUnwrap(joined.room.room.participants.first { $0.agentID == targetAgent.id })
        // A user-selected existing snapshot remains pinned after directory edits.
        // The target must still execute its old own model/high effort, never the new source-model/low defaults.
        _ = try service.updateAgent(targetAgent, name: "New directory target", description: "New definition", availability: "active",
            directory: service.directory(), defaults: .object(["modelSelectionID": .string("configured:source-profile"), "expertReasoningEffort": .string("low")]))
        NativePeerHTTPFixture.targetParticipantID = target.id
        NativePeerHTTPFixture.parentConversationID = parent.id.uuidString
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = store
        let manager = ChatManager(container: container, restorePersistedState: false)
        let profile = try XCTUnwrap(runtime.modelProfile(id: "source-profile"))
        let model = try XCTUnwrap(ModelSelectionManager.modelInfo(from: profile))
        var parameters = ModelParameters.webAlignedCloudDefaults
        // Keep dispatch/cancellation tests independent of full tool-catalog input
        // budgeting; canonical blocked-budget presentation has separate coverage.
        parameters.maxContextLength = 131072; parameters.maxOutputTokens = 1024; parameters.reasoningPreference = "low"
        try store.selectConversationModel(conversationID: parent.id, model: model, initialParameters: parameters)
        manager.currentConversationId = parent.id
        manager.selectModel(model, persistAsDefault: false)
        let originalText = "Perform the bounded expert collaboration"
        do {
            _ = try await manager.processMessage(originalText, selectedExperts: [
                .init(agentID: targetAgent.id, agentRevision: target.agentRevision,
                    participantID: target.id, contextConversationID: target.contextConversationID)])
            XCTAssertNil(lostAcknowledgement, "The injected lost ACK must leave the original source operation unresolved")
        } catch IOSApplicationRuntimeError.outcomePending {
            XCTAssertNotNil(lostAcknowledgement)
            XCTAssertTrue(fault.dropped)
            let journal = ApplePendingCommandJournal(databaseURL: database, namespace: .expertHost,
                validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
            let transaction = try journal.beginTransaction()
            let retained = try transaction.load(); transaction.close()
            XCTAssertEqual(retained.count, 1, "Commit alone must not discard the original uncertain intent")
            let original = try XCTUnwrap(retained.values.first)
            let operation = try AppleExpertHostIntentScope.operation(in: original)
            XCTAssertEqual(operation["kind"] as? String, lostAcknowledgement)
            let active = try XCTUnwrap(runtime.loadConversationProjectionV1(id: parent.id.uuidString).activeRun)
            runtime = try IOSApplicationRuntime(databaseURL: database, expertHostAcknowledgementObserver: fault.observe)
            NativePeerHTTPFixture.runtime = runtime
            let request = try XCTUnwrap(fault.sourceRequest)
            let recoveredRuntime = runtime
            let result = try await IOSLocalTaskHost.prepareExpert(request, contextID: parent.id.uuidString,
                runID: active.runID, store: runtime, expertTargetConfiguration: { material in
                    if lostAcknowledgement == "prepare_peer_task" {
                        XCTFail("Prepared configuration must be loaded from the canonical Task, never captured again")
                    }
                    return try await IOSExpertTaskTargetFactory.configuration(material: material,
                        runtime: recoveredRuntime, skills: [], hostExecutor: IOSAgentHostExecutor())
                })
            XCTAssertEqual(result.objectFields?["response"]?.objectFields?["type"], .string("task_promoted"))
            XCTAssertEqual(try taskRows(runtime).count, 1)
            XCTAssertEqual(try taskRows(runtime).first?["child_session_id"], .string(target.contextConversationID))
            XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 0, "Preparing/recovering cannot execute before source ACK")
            let settled = try journal.beginTransaction()
            let remaining = try settled.load(); settled.close()
            XCTAssertTrue(remaining.isEmpty, "Only the original validated receipt may retire the uncertainty")
            XCTAssertEqual(fault.replayedRequestIDs.count, 2)
            XCTAssertEqual(fault.replayedRequestIDs.first, original["request_id"] as? String)
            XCTAssertEqual(fault.replayedRequestIDs.first, fault.replayedRequestIDs.last,
                "The original durable request ID, not a new mutation, resolves the acknowledgement")
            XCTAssertEqual(fault.replayedOperations.first, fault.replayedOperations.last,
                "Replay must preserve the original authority and capture operation")
            if lostAcknowledgement == "prepare_peer_task" {
                XCTAssertEqual(fault.preparedConfigurations.count, 2)
                XCTAssertEqual(fault.preparedConfigurations.first, fault.preparedConfigurations.last,
                    "Retry must retain the first frozen target configuration")
            }
            XCTAssertEqual(try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString)).messages.filter { $0.role == "user" }.count, 1)
            return
        }
        let mainHistory = try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString))
        XCTAssertEqual(mainHistory.messages.filter { $0.role == "user" }.count, 1, "Selection cannot bypass Main or create a duplicate user turn")
        XCTAssertTrue(NativePeerHTTPFixture.failures.isEmpty, NativePeerHTTPFixture.failures.joined(separator: "\n"))
        let tasks = try taskRows(runtime)
        XCTAssertEqual(tasks.count, 1, "The peer must use one common Task rather than a parallel Room runner")
        let task = try XCTUnwrap(tasks.first)
        let taskID = try XCTUnwrap(task["id"]?.stringScalar)
        XCTAssertEqual(task["child_session_id"], .string(target.contextConversationID))
        XCTAssertEqual(task["owner_session_id"], .string(parent.id.uuidString))
        if ownerTransportFailure {
            XCTAssertTrue(fault.ownerPumpClosed)
            XCTAssertTrue(NativePeerHTTPFixture.targetStopped, "Owner failure must join the actual independent provider")
            XCTAssertGreaterThanOrEqual(NativePeerHTTPFixture.targetBusyFrames, 3)
            XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 1)
            let receipt = try runtime.executeAgentCommand(.object(["type": .string("runtime_task"),
                "command": .object(["type": .string("terminal_receipt"), "task_id": .string(taskID)])]))
            XCTAssertEqual(receipt, .null, "Transport failure and physical stop cannot invent a canonical terminal receipt")
            return
        }
        XCTAssertEqual(task["status"], .string("sealed"))
        XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 1, "The target's own provider executes exactly once")
        let sent = try XCTUnwrap(NativePeerHTTPFixture.targetRequests.first)
        XCTAssertEqual(sent["model"] as? String, "target-model")
        XCTAssertEqual((sent["reasoning"] as? [String: Any])?["effort"] as? String, "high")
        XCTAssertEqual(sent["max_output_tokens"] as? Int, 1024)
        XCTAssertEqual(NativePeerHTTPFixture.sourceEfforts, Set(["low"]))
        if cancel {
            let childTerminal = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
                "conversation_id": .string(target.contextConversationID), "run_id": .string(taskID)]))
            let childResult = childTerminal.objectFields?["result"]?.objectFields?["value"]?.objectFields
            XCTAssertEqual(childResult?["kind"], .string("cancelled"))
            XCTAssertEqual(childResult?["unknown_effect_refs"], .array([]))
            XCTAssertTrue(NativePeerHTTPFixture.targetStopped, "A persisted cancel request alone is not transport stop evidence")
            if busyStreaming { XCTAssertGreaterThanOrEqual(NativePeerHTTPFixture.targetBusyFrames, 3) }
        }
        else {
            let history = try service.childHistory(conversationID: parent.id.uuidString, childID: target.contextConversationID)
            let authors = try IOSExpertPeerInputAuthor.map(history.expertExchanges, ownerID: target.contextConversationID,
                parentID: parent.id.uuidString, roomID: joined.roomID)
            XCTAssertEqual(authors[taskID + ":user"]?.participant, .mainAgent(contextConversationID: parent.id.uuidString))
            XCTAssertTrue(IOSConversationProjectionPresenter.messages(from: history.window.projection).contains { $0.content.contains("target-own-high") })
        }
        let before = NativePeerHTTPFixture.targetRequests.count
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let reopenedContainer = ServiceContainer(applicationRuntime: reopened)
        _ = reopenedContainer.chatManager
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertEqual(try taskRows(reopened).first?["status"], task["status"])
        XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, before, "A sealed Task must not infer again on cold reopen")
        XCTAssertEqual(try IOSWorkspaceReadFacts.conversationDirectory(runtime: reopened).map(\.id), [parent.id])
        if repeatsMainRun {
            let firstRunID = try XCTUnwrap(mainHistory.messages.first(where: { $0.role == "assistant" })?.runID)
            let firstTerminal = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
                "conversation_id": .string(parent.id.uuidString), "run_id": .string(firstRunID)]))
            XCTAssertEqual(firstTerminal.objectFields?["result"]?.objectFields?["value"]?.objectFields?["kind"], .string("answered"))
            NativePeerHTTPFixture.sourceCount = 0
            let secondUserID = UUID()
            _ = try await manager.processMessage("Continue the same expert context", userMessageID: secondUserID, selectedExperts: [
                .init(agentID: targetAgent.id, agentRevision: target.agentRevision,
                    participantID: target.id, contextConversationID: target.contextConversationID)])
            XCTAssertTrue(NativePeerHTTPFixture.failures.isEmpty, NativePeerHTTPFixture.failures.joined(separator: "\n"))
            let secondRows = try taskRows(runtime)
            let secondIdentity = IOSCanonicalAgentTurnIdentity(userMessageID: secondUserID)
            let secondTerminal = try runtime.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
                "conversation_id": .string(parent.id.uuidString), "run_id": .string(secondIdentity.submissionID)]))
            let diagnostic = XCTAttachment(string: String(decoding: try JSONEncoder().encode(AgentCoreJSONValue.object([
                "source_requests": .number(Double(NativePeerHTTPFixture.sourceCount)), "terminal": secondTerminal,
                "projection": try runtime.executeAgentCommand(.object(["type": .string("load_conversation_projection_v1"),
                    "conversation_id": .string(parent.id.uuidString)]))
            ])), as: UTF8.self))
            diagnostic.name = "same-main-second-run-canonical-diagnostic"
            diagnostic.lifetime = .keepAlways; add(diagnostic)
            XCTAssertEqual(secondTerminal.objectFields?["result"]?.objectFields?["value"]?.objectFields?["kind"], .string("answered"),
                "Sealed expert Tasks cannot hide a failed Main wait or final response")
            let projection = try runtime.loadConversationProjectionV1(id: parent.id.uuidString)
            let toolParts = projection.messages.filter { $0.role == .assistant }.flatMap { message in
                message.parts.compactMap { part -> IOSConversationProjectionV1.ToolPart? in
                    if case .tool(let tool) = part { return tool }; return nil
                }
            }
            for originalCallID in ["peer-request-call", "peer-wait-call"] {
                let repeats = toolParts.filter { $0.toolCallID == originalCallID }
                XCTAssertEqual(repeats.count, 2, "The provider's original call ID must be preserved in each Run")
                XCTAssertEqual(Set(repeats.map(\.partID)).count, 2, "Each acknowledged model attempt owns a distinct Part")
            }
            XCTAssertEqual(secondRows.count, 2)
            XCTAssertTrue(secondRows.allSatisfy { $0["status"] == .string("sealed") })
            XCTAssertEqual(NativePeerHTTPFixture.targetRequests.count, 2)
            XCTAssertEqual(try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString)).messages.filter { $0.role == "user" }.count, 2)
            let secondBody = try XCTUnwrap(NativePeerHTTPFixture.targetRequests.last)
            XCTAssertEqual(secondBody["model"] as? String, "target-model")
            XCTAssertEqual((secondBody["reasoning"] as? [String: Any])?["effort"] as? String, "high")
            XCTAssertTrue(String(decoding: try JSONSerialization.data(withJSONObject: secondBody), as: UTF8.self).contains("target-own-high"),
                "The next Task must preserve its expert's existing canonical history")
        }
    }
    private func taskRows(_ runtime: IOSApplicationRuntime) throws -> [[String: AgentCoreJSONValue]] {
        let value = try runtime.executeAgentCommand(.object(["type": .string("runtime_task"), "command": .object(["type": .string("list_all")])]))
        guard case .array(let rows) = value else { throw IOSApplicationRuntimeError.invalidResponse }
        return try rows.map { try XCTUnwrap($0.objectFields) }
    }
}

private nonisolated final class NativePeerHTTPFixture: URLProtocol, @unchecked Sendable {
    @MainActor static var runtime: IOSApplicationRuntime?
    @MainActor static var targetParticipantID = ""
    @MainActor static var parentConversationID = ""
    @MainActor static var sourceCount = 0
    @MainActor static var sourceEfforts = Set<String>()
    @MainActor static var targetRequests: [[String: Any]] = []
    @MainActor static var targetStopped = false
    @MainActor static var cancel = false
    @MainActor static var busy = false
    @MainActor static var targetBusyFrames = 0
    @MainActor static var failures: [String] = []
    private let operationLock = NSLock()
    private var operation: Task<Void, Never>?
    private var isTarget = false
    @MainActor static func reset(runtime: IOSApplicationRuntime, cancel: Bool, busy: Bool) {
        self.runtime = runtime; self.cancel = cancel; self.busy = busy; targetBusyFrames = 0
        targetParticipantID = ""; sourceCount = 0
        sourceEfforts = []; targetRequests = []; targetStopped = false; failures = []
    }
    @MainActor static func clear() { runtime = nil }
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "ios-native-peer-pump.invalid" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {
        operationLock.withLock { operation?.cancel() }
        Task { @MainActor in if self.isTarget { Self.targetStopped = true } }
    }
    override func startLoading() {
        let request = request
        let work = Task { @MainActor in
            var headersSent = false
            do {
                var bytes = request.httpBody ?? Data()
                if bytes.isEmpty, let stream = request.httpBodyStream {
                    stream.open(); defer { stream.close() }
                    var chunk = [UInt8](repeating: 0, count: 4096)
                    while stream.hasBytesAvailable {
                        let count = stream.read(&chunk, maxLength: chunk.count)
                        guard count > 0 else { break }; bytes.append(contentsOf: chunk.prefix(count))
                    }
                }
                let body = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
                let model = try XCTUnwrap(body["model"] as? String)
                let events: [[String: Any]]
                if model == "target-model" {
                    self.isTarget = true; Self.targetRequests.append(body)
                    let names = (body["tools"] as? [[String: Any]] ?? []).compactMap { ($0["name"] as? String) ?? (($0["function"] as? [String: Any])?["name"] as? String) }
                    guard !names.contains("delegate_expert"), !names.contains("expert_exchange"),
                          !names.contains("spawn_agent"), !names.contains("request_user_input") else {
                        throw IOSApplicationRuntimeError.rejected("Expert inference advertised dispatch or direct human interaction")
                    }
                    if Self.cancel, Self.busy {
                        let url = try XCTUnwrap(request.url)
                        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil,
                            headerFields: ["Content-Type": "text/event-stream"])!, cacheStoragePolicy: .notAllowed)
                        headersSent = true
                        for index in 0..<1800 {
                            let event: [String: Any] = ["type": "response.output_text.delta", "delta": "busy-\(index) "]
                            let frame = "data: " + String(decoding: try JSONSerialization.data(withJSONObject: event), as: UTF8.self) + "\n\n"
                            client?.urlProtocol(self, didLoad: Data(frame.utf8))
                            Self.targetBusyFrames += 1
                            try await Task.sleep(for: .milliseconds(25))
                        }
                    }
                    if Self.cancel, !Self.busy { try await Task.sleep(for: .seconds(45)) }
                    events = Self.text("target-own-high")
                } else {
                    guard model == "source-model" else { throw IOSApplicationRuntimeError.invalidResponse }
                    Self.sourceCount += 1
                    if let effort = (body["reasoning"] as? [String: Any])?["effort"] as? String { Self.sourceEfforts.insert(effort) }
                    if Self.sourceCount == 1 {
                        events = try Self.tool(call: "peer-request-call", args: ["action": "request", "target_participant_id": Self.targetParticipantID, "text": "Check this implementation with your own model"])
                    } else if Self.sourceCount == 2 || (Self.cancel && Self.sourceCount == 3) {
                        if Self.cancel && Self.sourceCount == 2 {
                            for _ in 0..<500 where Self.targetRequests.isEmpty || (Self.busy && Self.targetBusyFrames < 3) { try await Task.sleep(for: .milliseconds(10)) }
                            guard !Self.targetRequests.isEmpty, !Self.busy || Self.targetBusyFrames >= 3 else { throw IOSApplicationRuntimeError.rejected("Target transport did not start before cancellation") }
                        }
                        let runtime = try XCTUnwrap(Self.runtime)
                        let value = try runtime.executeAgentCommand(.object(["type": .string("runtime_task"), "command": .object(["type": .string("list_all")])]))
                        let currentRun = try XCTUnwrap(runtime.loadConversationProjectionV1(id: Self.parentConversationID).activeRun?.runID)
                        guard case .array(let tasks) = value else { throw IOSApplicationRuntimeError.invalidResponse }
                        let currentTask = try tasks.first { task in
                            guard let metadata = task.objectFields?["metadata_json"]?.stringScalar else { return false }
                            return try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(metadata.utf8))
                                .objectFields?["expert_target"]?.objectFields?["source_run_id"] == .string(currentRun)
                        }
                        guard let id = currentTask?.objectFields?["id"]?.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }
                        let action = Self.cancel && Self.sourceCount == 2 ? "cancel" : "wait"
                        var arguments: [String: Any] = ["action": action, "exchange_id": id]
                        if action == "wait" { arguments["timeout_ms"] = 30000 }
                        events = try Self.tool(call: "peer-" + action + "-call", args: arguments)
                    } else { events = Self.text("source-finished") }
                }
                try Task.checkCancellation()
                let payload = try events.reduce(into: Data()) { data, event in
                    data.append(Data("data: ".utf8)); data.append(try JSONSerialization.data(withJSONObject: event)); data.append(Data("\n\n".utf8))
                }
                let url = try XCTUnwrap(request.url)
                if !headersSent {
                    client?.urlProtocol(self, didReceive: HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "text/event-stream"])!, cacheStoragePolicy: .notAllowed)
                }
                client?.urlProtocol(self, didLoad: payload); client?.urlProtocolDidFinishLoading(self)
            } catch is CancellationError { client?.urlProtocol(self, didFailWithError: URLError(.cancelled)) }
            catch { Self.failures.append(error.localizedDescription); client?.urlProtocol(self, didFailWithError: error) }
        }
        operationLock.withLock { operation = work }
    }
    @MainActor static func text(_ value: String) -> [[String: Any]] {
        [["type": "response.output_text.delta", "delta": value], ["type": "response.completed", "response": ["id": "response-\(UUID())", "status": "completed", "output": []]]]
    }
    @MainActor static func tool(call: String, args: [String: Any]) throws -> [[String: Any]] {
        [["type": "response.completed", "response": ["id": "response-\(UUID())", "status": "completed", "output": [["type": "function_call", "call_id": call, "name": "delegate_expert", "arguments": String(decoding: try JSONSerialization.data(withJSONObject: args), as: UTF8.self)]]]]]
    }
}

private nonisolated final class NativePeerAcknowledgementFault: @unchecked Sendable {
    private let lock = NSLock()
    let kind: String?
    private let closeOwnerPump: Bool
    private var didCloseOwnerPump = false
    private var pumpObserverTask: Task<Void, Never>?
    private var didDrop = false
    private var configurations: [Data] = []
    private var originalRequest: AgentCoreJSONValue?
    private var requestIDs: [String] = []
    private var operations: [Data] = []
    init(kind: String?, closeOwnerPump: Bool = false) { self.kind = kind; self.closeOwnerPump = closeOwnerPump }
    var ownerPumpClosed: Bool { lock.withLock { didCloseOwnerPump } }
    func cancelObserver() { lock.withLock { pumpObserverTask?.cancel() } }
    func observePump(_ pump: AppleChildPump) {
        guard closeOwnerPump else { return }
        let observer = Task { @MainActor in
            for _ in 0..<1000 {
                if Task.isCancelled { return }
                if !NativePeerHTTPFixture.targetRequests.isEmpty, NativePeerHTTPFixture.targetBusyFrames >= 3 { break }
                do { try await Task.sleep(for: .milliseconds(10)) } catch { return }
            }
            if Task.isCancelled { return }
            self.lock.withLock { self.didCloseOwnerPump = true }
            pump.close()
        }
        lock.withLock { pumpObserverTask = observer }
    }
    var dropped: Bool { lock.withLock { didDrop } }
    var sourceRequest: AgentCoreJSONValue? { lock.withLock { originalRequest } }
    var replayedRequestIDs: [String] { lock.withLock { requestIDs } }
    var replayedOperations: [Data] { lock.withLock { operations } }
    var preparedConfigurations: [Data] { lock.withLock { configurations } }
    func observe(_ operation: Data, _ reply: Data) throws {
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: operation) as? [String: Any])
        let current = fields["kind"] as? String
        let response = try XCTUnwrap(JSONSerialization.jsonObject(with: reply) as? [String: Any])
        let shouldDrop = try lock.withLock {
            if current == kind, let id = response["request_id"] as? String {
                requestIDs.append(id); operations.append(operation)
            }
            if current == "derive_peer_authority", let request = fields["request"] {
                originalRequest = try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: request))
            }
            if current == "prepare_peer_task", let config = fields["target_configuration"] {
                configurations.append(try JSONSerialization.data(withJSONObject: config, options: [.sortedKeys]))
            }
            guard current == kind, !didDrop else { return false }
            didDrop = true
            return true
        }
        if shouldDrop { throw IOSApplicationRuntimeError.invalidResponse }
    }
}
