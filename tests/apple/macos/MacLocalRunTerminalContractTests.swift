import Foundation
import XCTest
@testable import Nola

/// Script only the A08 service boundary; the surrounding conversation, Run,
/// recovery owner and shared resume are the real WorkspaceModel/Application path.
private final class UnknownRecoveryServiceFixture: @unchecked Sendable {
    private let lock = NSLock()
    let resolutionGate: DispatchSemaphore?
    private var resolutionStarted = false
    init(resolutionGate: DispatchSemaphore? = nil) { self.resolutionGate = resolutionGate }
    var hasStartedResolution: Bool {
        lock.lock(); defer { lock.unlock() }; return resolutionStarted
    }
    private var authorized = false
    private var submitted: [MacUnknownEffectRecoveryDecision] = []
    var decisions: [MacUnknownEffectRecoveryDecision] {
        lock.lock(); defer { lock.unlock() }; return submitted
    }
    func execute(_ data: Data, _ requestID: String) throws -> Data {
        let command = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        if command["type"] as? String == "resolve_unknown_effect_recovery", let resolutionGate {
            lock.lock(); resolutionStarted = true; lock.unlock()
            guard resolutionGate.wait(timeout: .now() + 5) == .success else {
                throw MacApplicationServiceError.connectionFailed("fixture resolution gate timed out")
            }
        }
        lock.lock(); defer { lock.unlock() }
        switch command["type"] as? String {
        case "inspect_unknown_effect_recovery":
            if authorized { return Data("null".utf8) }
            return try JSONEncoder().encode(MacUnknownEffectRecoveryInspection(
                conversationID: try XCTUnwrap(command["conversation_id"] as? String),
                runID: try XCTUnwrap(command["expected_run_id"] as? String),
                assistantMessageID: try XCTUnwrap(command["assistant_message_id"] as? String),
                invocationConfigDigest: "fixture-digest", targets: [.init(effectID: "effect-1",
                    requestID: "request-1", attemptID: "attempt-1", workerGeneration: "worker-1",
                    leaseOwner: "owner", leaseToken: "private-token", expectedAttemptRevision: 1)],
                retryAllowed: true, independentPendingEffects: 0))
        case "resolve_unknown_effect_recovery":
            let input = try JSONSerialization.data(withJSONObject: XCTUnwrap(command["input"]))
            let decision = try JSONDecoder().decode(MacUnknownEffectRecoveryDecision.self, from: input)
            submitted.append(decision)
            authorized = true
            return try JSONSerialization.data(withJSONObject: ["outcome": "retry_authorized",
                "resolution_id": decision.resolutionID, "run_id": decision.runID])
        default: throw MacApplicationServiceError.invalidResponse
        }
    }
}

/// Only the transport is scripted. Run creation, effect ownership, model
/// completion and terminal queries execute the real Application transactions.
private actor TerminalContractLocalRuntime: MacLocalAgentRuntime {
    enum StreamEnd: Sendable { case eof, cancelled, failed }
    enum FixtureError: Error { case transport, preparation, invalidContract }
    struct RequestSnapshot: Equatable, Sendable {
        let conversationID: String
        let runID: String
        let assistantID: String
        let prompt: String
        let reasoningTier: String?
        let modelID: String?
        let contextWindowTokens: Int?
    }
    private struct Run {
        let request: MacLocalAgentRunRequest
        let client: MacApplicationServiceClient
        let digest: String
        var modelRequest: [String: Any]
        var continuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation
    }
    private var runs: [String: Run] = [:]
    private var order: [String] = []
    private var preparationFailure = false
    private var pausesBeforeAdmission = false
    private var pendingAdmission: (MacRunSubmissionTicket, AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation)?
    private var failure: String?
    private var cancellationCount = 0
    private var cancellationDeliveryFailure = false
    private var cancellationRequests: [MacRunCancellationRequest] = []
    private var ownedTransports: Set<String> = []
    private var interruptedRuns: [String] = []
    private var resumeFailure = false
    private var resumedRequests: [RequestSnapshot] = []
    private var beforeRevalidate: (@MainActor @Sendable () -> Void)?
    private var pauseBeforeReady = false
    private var readyGateEntered = false
    private var readyGate: CheckedContinuation<Void, Never>?
    private var completedResumeAttempts = 0

    func setBeforeRevalidate(_ hook: @escaping @MainActor @Sendable () -> Void) { beforeRevalidate = hook }
    func pauseNextReady() { pauseBeforeReady = true }
    func isWaitingForReady() -> Bool { readyGateEntered }
    func releaseReady() { readyGate?.resume(); readyGate = nil }
    func finishedResumeAttempts() -> Int { completedResumeAttempts }

    func failPreparation() { preparationFailure = true }
    func pauseBeforeAdmission() { pausesBeforeAdmission = true }
    func hasPendingAdmission() -> Bool { pendingAdmission != nil }
    func finishPendingAdmission(rejected: Bool) {
        guard let (ticket, continuation) = pendingAdmission else { return }
        pendingAdmission = nil
        if rejected {
            _ = ticket.rejectBeforeAdmission()
            continuation.finish(throwing: MacLocalAgentRuntimeError.submissionRejected(
                code: "request_rejected_before_admission", message: "Host invocation unavailable: incompatible checkpoint"))
        } else {
            continuation.finish(throwing: FixtureError.transport)
        }
    }
    func runIDs() -> [String] { order }
    func capturedRequest(_ id: String) -> MacLocalAgentRunRequest? { runs[id]?.request }
    func setupFailure() -> String? { failure }
    func cancelCount() -> Int { cancellationCount }
    func failNextCancellationDelivery() { cancellationDeliveryFailure = true }
    func deliveredCancellationRequests() -> [MacRunCancellationRequest] { cancellationRequests }
    func interruptedRunIDs() -> [String] { interruptedRuns }
    func failNextResume() { resumeFailure = true }
    func resumeSnapshots() -> [RequestSnapshot] { resumedRequests }
    func requestSnapshot(_ id: String) -> RequestSnapshot? {
        guard let request = runs[id]?.request else { return nil }
        return RequestSnapshot(conversationID: request.conversationID, runID: request.submissionID,
            assistantID: request.assistantMessageID, prompt: request.prompt, reasoningTier: request.reasoningTier,
            modelID: request.modelID, contextWindowTokens: request.contextWindowTokens)
    }
    func canResume(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool {
        guard let run = runs[expectedRunID] else { return false }
        return run.request.conversationID == conversationID && run.request.assistantMessageID == assistantMessageID
    }
    func resume(conversationID: String, expectedRunID: String, assistantMessageID: String,
                revalidate: @escaping @MainActor @Sendable () async throws -> MacLocalAgentResumeAuthorization,
                transportReady: (@MainActor @Sendable () async -> Void)?) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                defer { completedResumeAttempts += 1 }
                do {
                    guard canResume(conversationID: conversationID, expectedRunID: expectedRunID,
                                    assistantMessageID: assistantMessageID) else { throw FixtureError.invalidContract }
                    await beforeRevalidate?()
                    beforeRevalidate = nil
                    _ = try await revalidate()
                    guard var run = runs[expectedRunID], let snapshot = requestSnapshot(expectedRunID) else {
                        throw FixtureError.invalidContract
                    }
                    resumedRequests.append(snapshot)
                    if resumeFailure { resumeFailure = false; throw FixtureError.transport }
                    // Reattach the fake transport to the SAME real, already
                    // committed Core Run. Never execute begin_assistant_turn
                    // or submit its prompt a second time on this path.
                    run.continuation = continuation
                    runs[expectedRunID] = run
                    ownedTransports.insert(expectedRunID)
                    if pauseBeforeReady {
                        pauseBeforeReady = false
                        await withCheckedContinuation { readyGateEntered = true; readyGate = $0 }
                    }
                    await transportReady?()
                } catch { continuation.finish(throwing: error) }
            }
        }
    }

    func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        AsyncThrowingStream { continuation in
            do {
                if preparationFailure {
                    request.submissionTicket?.rejectBeforeTransport()
                    throw FixtureError.preparation
                }
                try request.submissionTicket?.claimDispatch()
                if pausesBeforeAdmission, let ticket = request.submissionTicket {
                    pendingAdmission = (ticket, continuation)
                    return
                }
                guard let database = request.databaseURLOverride else { throw FixtureError.invalidContract }
                let client = try MacApplicationServiceClient(databaseURL: database)
                let now = Int64(Date().timeIntervalSince1970 * 1_000)
                let capability: [String: Any] = [
                    "id": "model.stream", "title": "Fixture model", "route": "model", "availability": "available",
                    "exposure": "internal", "input_schema": ["type": "object"],
                    "output_policy": ["preview_char_limit": 4000, "persist_full_output": true, "allow_artifacts": true],
                    "supports_streaming": true, "parallelism": "serial", "cancellation": "cooperative",
                    "replay_safety": "recoverable"
                ]
                let accepted = try Self.command(client, ["type": "begin_assistant_turn", "input": [
                    "conversation_id": request.conversationID, "conversation_title": "Local terminal fixture",
                    "expected_configuration_revision": request.expectedConfigurationRevision as Any? ?? NSNull(),
                    "expected_parent_message_id": request.expectedParentMessageID as Any? ?? NSNull(),
                    "user_message_id": request.userMessageID, "user_part_id": "\(request.userMessageID):text",
                    "user_text": request.prompt, "assistant_message_id": request.assistantMessageID,
                    "core_config": ["session_id": request.conversationID, "model": "fixture-model",
                        "turn_identity": ["model_capability_id": "mac-journal-fixture",
                            "model_capability_revision": 1,
                            "model_capability": MacAssistantJournalTestFixture.modelCapabilityProfile(
                                toolCapabilityID: nil, contextWindow: 128_000, maxOutput: 4096)],
                        "host_catalog": ["host_id": "fixture-local-host", "revision": 1, "availability_epoch": 1,
                            "capabilities": ["model.stream": capability]]],
                    "submission": ["type": "user_input", "items": [["type": "message", "id": request.userMessageID,
                        "role": "user", "content": request.prompt]], "submission_id": request.submissionID,
                        "submitted_at_epoch_ms": now, "delivery": "current_turn"], "created_at_ms": now
                ]])
                guard let digest = accepted["invocation_config_digest"] as? String else { throw FixtureError.invalidContract }
                let claimed = try Self.claim(client, runID: request.submissionID)
                guard var model = claimed.first(where: { ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream" }) else {
                    throw FixtureError.invalidContract
                }
                _ = try Self.accept(client, request: request, digest: digest, host: model, sequence: 1,
                    payload: ["type": "execution_started"])
                _ = try Self.accept(client, request: request, digest: digest, host: model, sequence: 2,
                    payload: ["type": "model_attempt_started", "provider": "fixture", "model": "fixture-model", "reason": "initial"])
                let loaded = try Self.command(client, ["type": "load_committed_host_requests", "run_id": request.submissionID])
                guard let refreshed = (loaded["requests"] as? [[String: Any]])?.first(where: {
                    $0["effect_id"] as? String == model["effect_id"] as? String
                }) else { throw FixtureError.invalidContract }
                let delivery = model["delivery"]
                model = refreshed
                model["delivery"] = delivery
                runs[request.submissionID] = Run(request: request, client: client, digest: digest,
                    modelRequest: model, continuation: continuation)
                order.append(request.submissionID)
                ownedTransports.insert(request.submissionID)
                request.submissionTicket?.markAccepted()
                Task { await request.submissionAccepted?() }
            } catch {
                failure = String(reflecting: error)
                continuation.finish(throwing: error)
            }
        }
    }

    func end(_ id: String, as mode: StreamEnd) throws {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        ownedTransports.remove(id)
        switch mode {
        case .eof: run.continuation.finish()
        case .cancelled: run.continuation.finish(throwing: CancellationError())
        case .failed: run.continuation.finish(throwing: FixtureError.transport)
        }
    }

    func commitCompletion(_ id: String, answer: String = "Committed local answer") throws {
        guard let run = runs[id], let payload = run.modelRequest["payload"] as? [String: Any],
              let partID = payload["text_part_id"] as? String else { throw FixtureError.invalidContract }
        _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: run.modelRequest, sequence: 3,
            payload: ["type": "model_completed", "items": [["type": "message", "id": partID,
                "role": "assistant", "content": answer]], "tool_calls": [], "usage": [:]])
        for hook in try Self.claim(run.client, runID: id) {
            guard (hook["payload"] as? [String: Any])?["type"] as? String == "run_lifecycle_hook" else {
                throw FixtureError.invalidContract
            }
            _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: hook, sequence: 1,
                payload: ["type": "execution_started"])
            _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: hook, sequence: 2,
                payload: ["type": "completed", "output": [:], "artifacts": []])
        }
    }

    func emitOldCompletion(into id: String, oldRunID: String) throws {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        run.continuation.yield(MacLocalAgentEvent(kind: .finished, text: "", title: nil, detail: nil,
            isError: false).bindingCanonicalRun(oldRunID))
    }

    func cancellationFacts(_ id: String) throws -> (status: String, revision: Int, pendingCancels: Int) {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        let record = try Self.command(run.client, ["type": "load_checkpoint", "conversation_id": run.request.conversationID])
        guard let json = record["checkpoint_json"] as? String,
              let data = json.data(using: .utf8),
              let checkpoint = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let state = checkpoint["state"] as? [String: Any],
              let execution = state["execution"] as? [String: Any],
              let turn = execution["active_turn"] as? [String: Any],
              let status = turn["status"] as? String,
              let revision = record["revision"] as? Int else { throw FixtureError.invalidContract }
        let requests = try Self.command(run.client, ["type": "load_committed_host_requests", "run_id": id])
        let pending = (requests["requests"] as? [[String: Any]] ?? []).filter {
            ($0["payload"] as? [String: Any])?["type"] as? String == "cancel"
        }.count
        return (status, revision, pending)
    }

    /// Explicit isolated Host evidence for fixture cleanup; this is not a
    /// claim that production cancel-only transport has been implemented.
    func commitCancellation(_ id: String) throws {
        guard let run = runs[id] else { throw FixtureError.invalidContract }
        for _ in 0..<4 {
            let requests = try Self.claim(run.client, runID: id)
            if requests.isEmpty { return }
            for request in requests {
                guard let payload = request["payload"] as? [String: Any],
                      let kind = payload["type"] as? String else { throw FixtureError.invalidContract }
                _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: request,
                    sequence: 1, payload: ["type": "execution_started"])
                let result: [String: Any]
                switch kind {
                case "cancel":
                    guard let target = payload["target_effect_id"] as? String else { throw FixtureError.invalidContract }
                    result = ["type": "cancellation_confirmed", "target_effect_id": target]
                case "run_lifecycle_hook": result = ["type": "completed", "output": [:], "artifacts": []]
                default: throw FixtureError.invalidContract
                }
                _ = try Self.accept(run.client, request: run.request, digest: run.digest, host: request,
                    sequence: 2, payload: result)
            }
        }
        throw FixtureError.invalidContract
    }

    private static func command(_ client: MacApplicationServiceClient, _ object: [String: Any]) throws -> [String: Any] {
        let data = try client.execute(commandData: JSONSerialization.data(withJSONObject: object))
        guard let value = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw FixtureError.invalidContract }
        return value
    }
    private static func claim(_ client: MacApplicationServiceClient, runID: String) throws -> [[String: Any]] {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let value = try command(client, ["type": "claim_committed_host_requests", "input": ["run_id": runID,
            "worker_generation": "test-worker", "lease_owner": "test-owner", "claimed_at_ms": now,
            "lease_expires_at_ms": now + 60_000, "limit": 32]])
        guard let requests = value["requests"] as? [[String: Any]] else { throw FixtureError.invalidContract }
        return requests
    }
    private static func accept(_ client: MacApplicationServiceClient, request: MacLocalAgentRunRequest,
                               digest: String, host: [String: Any], sequence: Int,
                               payload: [String: Any]) throws -> [String: Any] {
        let keys = ["request_id", "attempt_id", "delegated_invocation_id", "effect_id", "host_id", "lease_id",
                    "session_id", "turn_id", "catalog_revision", "availability_epoch", "delivery"]
        var response = host.filter { keys.contains($0.key) }
        response["sequence"] = sequence
        response["payload"] = payload
        return try command(client, ["type": "reduce_and_accept_host_response", "input": [
            "conversation_id": request.conversationID, "run_id": request.submissionID,
            "invocation_config_digest": digest, "response": response,
            "received_at_ms": Int64(Date().timeIntervalSince1970 * 1_000)]])
    }

    func cancel(conversationID: String) { cancellationCount += 1 }
    func interruptOwnedTransport(conversationID: String, expectedRunID: String, assistantMessageID: String) -> Bool {
        guard ownedTransports.contains(expectedRunID), let run = runs[expectedRunID],
              run.request.conversationID == conversationID,
              run.request.assistantMessageID == assistantMessageID else { return false }
        ownedTransports.remove(expectedRunID)
        interruptedRuns.append(expectedRunID)
        run.continuation.finish()
        return true
    }
    func deliverCancellation(_ request: MacRunCancellationRequest, databaseURL: URL) async throws {
        guard let run = runs[request.expectedRunID],
              run.request.conversationID == request.conversationID,
              run.request.assistantMessageID == request.assistantMessageID,
              run.request.databaseURLOverride == databaseURL,
              try cancellationFacts(request.expectedRunID).status == "cancelling" else {
            throw FixtureError.invalidContract
        }
        cancellationRequests.append(request)
        if cancellationDeliveryFailure {
            cancellationDeliveryFailure = false
            throw FixtureError.transport
        }
        cancellationCount += 1
        // Delivery acceptance is deliberately not terminal evidence. Tests
        // separately commit the real Core cancellation response below.
    }
    func resolvePermission(conversationID: String, request: MacRemotePermissionRequest, allow: Bool,
                           scope: MacPermissionGrantScope) throws {}
    func listSessions() async throws -> [MacLocalAgentSessionSummary] { [] }
    func deleteSession(id: String) async throws {}
}

/// Blocks only the bookmark store's detached resolver, never MainActor.
nonisolated private final class LocalTerminalResolutionGate: @unchecked Sendable {
    private let lock = NSLock()
    private let signal = DispatchSemaphore(value: 0)
    private var armed = false
    private var entered = false
    func arm() { lock.lock(); armed = true; entered = false; lock.unlock() }
    var isEntered: Bool { lock.lock(); defer { lock.unlock() }; return entered }
    func resolveBoundary() {
        lock.lock()
        let wait = armed
        if wait { armed = false; entered = true }
        lock.unlock()
        if wait { signal.wait() }
    }
    func release() { signal.signal() }
}

@MainActor
private final class LocalTerminalBookmarkIO: MacWorkspaceBookmarkIO {
    let root: URL
    nonisolated let gate: LocalTerminalResolutionGate
    init(_ root: URL, gate: LocalTerminalResolutionGate) { self.root = root; self.gate = gate }
    func chooseFolder(suggested: URL?) -> URL? { root }
    func bookmark(_ url: URL) throws -> Data { Data(url.absoluteString.utf8) }
    nonisolated func resolve(_ data: Data) throws -> (url: URL, stale: Bool) {
        gate.resolveBoundary()
        guard let text = String(data: data, encoding: .utf8), let url = URL(string: text) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return (url, false)
    }
    func start(_ url: URL) -> Bool { true }
    func stop(_ url: URL) {}
}

@MainActor
final class MacLocalRunTerminalContractTests: XCTestCase {
    @MainActor
    private final class Fixture {
        let root: URL
        let suite: String
        let defaults: UserDefaults
        let bookmarks: MacSecurityScopedBookmarkStore
        let port: MacApplicationPort
        let repository: MacCanonicalConversationRepository
        let activity: MacActivityStore
        let runtime = TerminalContractLocalRuntime()
        let resolutionGate = LocalTerminalResolutionGate()
        let model: MacWorkspaceModel
        let conversation: String
        init(unknownTransport: MacUnknownEffectRecoveryControl.Transport? = nil) async throws {
            root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            suite = "Nola.LocalTerminal.\(UUID().uuidString)"
            defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
            bookmarks = MacSecurityScopedBookmarkStore(defaults: defaults, io: LocalTerminalBookmarkIO(root, gate: resolutionGate))
            port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
            repository = MacCanonicalConversationRepository(store: port)
            activity = MacActivityStore(baseDirectory: root.appendingPathComponent("activity"))
            let recoveryURL = root.appendingPathComponent("recovery.db")
            model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: runtime,
                remoteHost: PersistenceTestRemoteHost(), modelApplication: port, bookmarkStore: bookmarks,
                unknownEffectRecoveryControl: unknownTransport.map {
                    MacUnknownEffectRecoveryControl(databaseURL: recoveryURL,
                        clientID: "fixture", transport: $0)
                },
                activityStore: activity, completionNotifier: { _, _, _ in },
                contextPreparation: { $0.contextComponents }, readinessIssue: { _ in nil })
            conversation = try XCTUnwrap(model.selectedConversationID)
            let selectedWorkspace = await bookmarks.chooseWorkspace(conversationID: conversation)
            model.workspaceURL = try XCTUnwrap(selectedWorkspace)
            model.executionHost = .local
            model.selectExecutionBackend(.fallback(id: "codex"), available: true)
            model.presentationDidAttach()
        }
        func cleanUp() {
            model.presentationDidDetach()
            bookmarks.close()
            defaults.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: root)
        }
        func executeSQLite(_ sql: String) throws {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
            process.arguments = [root.appendingPathComponent("store.db").path, sql]
            let errors = Pipe()
            process.standardError = errors
            try process.run()
            let detail = errors.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            XCTAssertEqual(process.terminationStatus, 0, String(decoding: detail, as: UTF8.self))
        }
    }

    private func waitUntil(_ predicate: () async throws -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while try await !predicate(), Date() < deadline { try await Task.sleep(for: .milliseconds(10)) }
        let satisfied = try await predicate()
        XCTAssertTrue(satisfied, "Expected real local lifecycle boundary", file: file, line: line)
    }

    private func start(_ fixture: Fixture, prompt: String = "Explain the result") async throws -> String {
        let prior = await fixture.runtime.runIDs().count
        fixture.model.draft = prompt
        try await fixture.model.waitForModelConfigurationCommit()
        fixture.model.send()
        try await waitUntil {
            let count = await fixture.runtime.runIDs().count
            let failure = await fixture.runtime.setupFailure()
            return count > prior || failure != nil
        }
        let failure = await fixture.runtime.setupFailure()
        XCTAssertNil(failure)
        let ids = await fixture.runtime.runIDs()
        return try XCTUnwrap(ids.dropFirst(prior).first,
            "This submission must admit its own Run; a predecessor cannot stand in for a rejected Begin")
    }

    func testFirstDraftSubmissionCreatesOwnerBeforeFreezingItsBeginWitness() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        // Keep this a genuinely unpersisted draft. Precreating it in Fixture
        // would hide a history/frontier read performed before Ensure.
        XCTAssertNil(try fixture.port.loadConversation(fixture.conversation))

        let run = try await start(fixture)
        let captured = await fixture.runtime.capturedRequest(run)
        let request = try XCTUnwrap(captured)
        let metadata = try XCTUnwrap(fixture.port.loadConversationMetadata(fixture.conversation))
        XCTAssertEqual(request.conversationID, fixture.conversation)
        XCTAssertNotNil(request.expectedConfigurationRevision)
        XCTAssertEqual(request.expectedConfigurationRevision, metadata.configurationRevision,
            "Begin must carry the new owner's creation witness, not an absent or invented revision")
        XCTAssertNil(request.expectedParentMessageID, "The new owner's frozen transcript is empty")
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
    }

    func testUnavailableSessionAndDefaultModelsBlockBothLocalAndAPISubmissions() async throws {
        for local in [false, true] {
            for inherited in [false, true] {
                let fixture = try await Fixture()
                defer { fixture.cleanUp() }
                let unavailableID = local ? MacLocalModelProfile.qwen35_08B.id : "disabled-api"
                let configurations = MacAppleChatConfigurationStore(applicationPort: fixture.port)
                try configurations.replaceEndpoints([MacRemoteEndpointConfiguration(id: "api", name: "API",
                    baseURL: "https://never-contact.invalid/v1", models: [
                        MacRemoteModelConfiguration(id: "fallback", modelName: "fallback", executionCapabilities: .init()),
                        MacRemoteModelConfiguration(id: "disabled-api", modelName: "disabled", isEnabled: false,
                            executionCapabilities: .init())
                    ])])
                if local {
                    let parameters = String(decoding: try JSONEncoder().encode(ModelParameters()), as: UTF8.self)
                    try fixture.port.upsertModelProfile(.init(id: unavailableID, displayName: "Disabled local",
                        provider: "local", modelName: unavailableID, runtime: "mlx", sourceJSON: "{}",
                        parametersJSON: parameters, selected: false, enabled: false, revision: 0,
                        createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
                }
                fixture.model.selectExecutionBackend(.fallback(id: "nola"), available: true)
                if inherited {
                    // A previously selected/synced default can reference a now
                    // disabled model. Persist that valid canonical reference;
                    // the send gate must not substitute the available fallback.
                    let value = try JSONSerialization.data(withJSONObject: ["version": 1,
                        "selection": ["kind": "profile", "profile_id": unavailableID]])
                    _ = try fixture.port.putProductSettingChecked(key: MacDefaultChatModelStore.key,
                        valueJSON: String(decoding: value, as: UTF8.self), expectedRevision: nil, updatedAtMs: 1)
                } else {
                    fixture.model.selectModel(MacChatModelOption(source: local ? .local : .configured,
                        modelID: local ? unavailableID : "disabled", displayName: "Disabled",
                        configurationID: local ? nil : unavailableID))
                }
                fixture.model.refreshModelConfiguration()
                XCTAssertEqual(fixture.model.modelSelectionResolution, .unavailable(profileID: unavailableID,
                    origin: inherited ? .default : .conversation, reason: .disabled))
                let prior = fixture.model.selectedMessages.count
                fixture.model.draft = "Must not use the available fallback"
                fixture.model.send()
                try await Task.sleep(for: .milliseconds(30))
                let dispatched = await fixture.runtime.runIDs()
                XCTAssertTrue(dispatched.isEmpty, "Unavailable local=\(local), inherited=\(inherited) dispatched a run")
                XCTAssertFalse(fixture.model.isRunning)
                XCTAssertEqual(fixture.model.selectedMessages.count, prior)
                XCTAssertEqual(fixture.model.draft, "Must not use the available fallback")
            }
        }
    }

    func testChangingDefaultKeepsActiveInvocationBytesAndNextInheritedRunUsesNewProfile() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let configurations = MacAppleChatConfigurationStore(applicationPort: fixture.port)
        try configurations.replaceEndpoints(["a", "b"].map { id in
            MacRemoteEndpointConfiguration(id: id, name: id, baseURL: "https://\(id).invalid/v1",
                models: [MacRemoteModelConfiguration(id: "profile:\(id)", modelName: "wire-\(id)",
                    executionCapabilities: .init())])
        })
        let preference = MacDefaultChatModelStore(application: fixture.port)
        let candidates = try MacChatModelCatalog.load(application: fixture.port).candidates
        let initial = try preference.set(.profile("profile:a"), expectedRevision: nil, candidates: candidates)
        fixture.model.selectExecutionBackend(.fallback(id: "nola"), available: true)
        fixture.model.refreshModelConfiguration()
        XCTAssertTrue(fixture.model.inheritsDefaultModel)
        let first = try await start(fixture)
        let firstRequest = await fixture.runtime.capturedRequest(first)
        let captured = try XCTUnwrap(firstRequest)
        XCTAssertEqual(captured.modelProfileID, "profile:a")
        let serializer = MacAgentHostRuntime(credentialStore: configurations)
        let before = try await serializer.initialRequestData(for: captured)

        _ = try preference.set(.profile("profile:b"), expectedRevision: initial.revision, candidates: candidates)
        fixture.model.refreshModelConfiguration()
        XCTAssertTrue(fixture.model.isRunning)
        XCTAssertTrue(fixture.model.inheritsDefaultModel, "Admission must not turn inheritance into an explicit selection")
        let unchanged = await fixture.runtime.capturedRequest(first)
        let after = try await serializer.initialRequestData(for: XCTUnwrap(unchanged))
        XCTAssertEqual(after, before, "Real Host invocation serialization must not reread a changed global default")

        try await fixture.runtime.commitCompletion(first)
        try await fixture.runtime.end(first, as: .eof)
        try await waitUntil { !fixture.model.isRunning && fixture.model.localRunRecovery == nil }
        let next = try await start(fixture, prompt: "Use the current inherited default")
        let nextRequest = await fixture.runtime.capturedRequest(next)
        let nextCaptured = try XCTUnwrap(nextRequest)
        XCTAssertEqual(nextCaptured.modelProfileID, "profile:b")
        XCTAssertEqual(nextCaptured.modelID, "wire-b")
        let nextData = try await serializer.initialRequestData(for: nextCaptured)
        let nextPayload = try XCTUnwrap(JSONSerialization.jsonObject(with: nextData) as? [String: Any])
        XCTAssertEqual((nextPayload["mac_launch"] as? [String: Any])?["model_profile_id"] as? String, "profile:b")
        try await fixture.runtime.commitCompletion(next)
        try await fixture.runtime.end(next, as: .eof)
        try await waitUntil { !fixture.model.isRunning && fixture.model.localRunRecovery == nil }
    }

    func testPostLaunchEOFAndErrorsRequireCanonicalTerminalAndLateCommitRecovers() async throws {
        for mode: TerminalContractLocalRuntime.StreamEnd in [.eof, .cancelled, .failed] {
            let fixture = try await Fixture()
            defer { fixture.cleanUp() }
            let run = try await start(fixture)
            try await fixture.runtime.end(run, as: mode)
            try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
            XCTAssertEqual(fixture.model.selectedConversationDisplayStatus, .uncertain,
                "Transport EOF without a committed terminal must not be presented as Ready")
            XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
            XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
            XCTAssertTrue(fixture.activity.entries.isEmpty, "transport loss must not invent a completed or cancelled activity")
            fixture.model.draft = "Must not start a replacement yet"
            fixture.model.send()
            let ids = await fixture.runtime.runIDs()
            XCTAssertEqual(ids, [run])
            XCTAssertNotNil(fixture.model.localRunRecovery)
            await fixture.model.checkLocalRunRecovery()
            guard let inspection = fixture.model.localRecoveryInspection,
                  case .active = inspection.observation else {
                return XCTFail("Status check must read the exact persisted Run through Application")
            }
            XCTAssertNotNil(fixture.model.localRunRecovery, "a read-only check cannot invent terminal evidence")
            XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
            let checkedIDs = await fixture.runtime.runIDs()
            XCTAssertEqual(checkedIDs, [run], "recovery check must not launch another transport")
            let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
            try await fixture.runtime.commitCompletion(run)
            // Clearing recovery means canonical terminal was confirmed. The
            // sole owner must still drain presentation and record its outcome.
            try await waitUntil {
                fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                    && fixture.model.selectedTerminalTranscriptRevision > terminalRevision
                    && fixture.activity.entries.map(\.status) == [.answered]
            }
            XCTAssertEqual(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run)?.result.value.kind, .answered)
            XCTAssertTrue(fixture.model.selectedMessages.contains { $0.text == "Committed local answer" })
            XCTAssertEqual(fixture.activity.entries.map(\.status), [.answered])
        }
    }

    func testDisconnectedRecoveryDoesNotBlockAnotherConversationAndRestoresExactOwner() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let first = try await start(fixture)
        try await fixture.runtime.end(first, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let original = try XCTUnwrap(fixture.model.localRunRecovery)
        fixture.model.newConversation()
        fixture.model.draft = "Independent task"
        XCTAssertTrue(fixture.model.selectedComposerState.canSend)
        XCTAssertNil(fixture.model.selectedComposerState.submissionBlockReason)
        XCTAssertNil(fixture.model.localRunRecovery)
        // Exercise the real configuration-write/admission ordering. Do not
        // refresh the revision or wait for an unrelated store read here:
        // dispatch must consume this edit's own commit receipt before Begin.
        fixture.model.selectExecutionBackend(.fallback(id: "codex"), available: true)
        XCTAssertTrue(fixture.model.selectedModelConfigurationPending)
        XCTAssertFalse(fixture.model.selectedComposerState.canSend)
        fixture.model.send()
        XCTAssertEqual(fixture.model.draft, "Independent task",
            "A configuration write in flight must not consume the unsent draft")
        XCTAssertFalse(fixture.model.isRunning)
        let blockedIDs = await fixture.runtime.runIDs()
        XCTAssertEqual(blockedIDs, [first], "Pending configuration must not dispatch a new Begin")
        try await fixture.model.waitForModelConfigurationCommit()
        XCTAssertFalse(fixture.model.selectedModelConfigurationPending)
        XCTAssertTrue(fixture.model.selectedComposerState.canSend)
        let second = try await start(fixture, prompt: "Independent task")
        XCTAssertNotEqual(first, second)
        try await fixture.runtime.commitCompletion(second)
        try await fixture.runtime.end(second, as: .eof)
        try await waitUntil { !fixture.model.isRunning }
        fixture.model.selectedConversationID = fixture.conversation
        await fixture.model.loadSelectedConversationIfNeeded()
        let restored = try XCTUnwrap(fixture.model.localRunRecovery)
        XCTAssertEqual(restored.expectedRunID, first)
        XCTAssertEqual(restored.canonicalAssistantMessageID, original.canonicalAssistantMessageID)
        XCTAssertGreaterThan(restored.generationID, original.generationID)
        XCTAssertEqual(fixture.model.selectedConversationDisplayStatus, .uncertain)
        fixture.model.draft = "Do not replace the unresolved run"
        XCTAssertFalse(fixture.model.selectedComposerState.canSend)
        fixture.model.send()
        let ids = await fixture.runtime.runIDs()
        XCTAssertEqual(ids, [first, second])
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: first))
    }

    func testMissingActivityAdvisoriesStillAllowsRecoveryReads() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        try fixture.executeSQLite("""
            UPDATE core_checkpoint_bodies SET checkpoint_json=json_remove(checkpoint_json,
                '$.state.execution.active_turn.engineering.recent_activity',
                '$.state.execution.active_turn.engineering.mutation_streak',
                '$.state.execution.active_turn.engineering.similar_mutation_streak',
                '$.state.execution.active_turn.engineering.last_mutation_sample');
            """)
        // This fixture changes the checkpoint body directly, so it is not a
        // journal-presentation frontier fixture. Exercise the exact recovery
        // reader, which must decode Core without fabricating a terminal.
        await fixture.model.checkLocalRunRecovery()
        XCTAssertNil(fixture.model.localRecoveryError)
        XCTAssertNotNil(fixture.model.localRecoveryInspection)
        XCTAssertEqual(fixture.model.localRunRecovery?.expectedRunID, run)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
    }

    func testPendingCancellationSurvivesConversationSwitchWithoutBlockingNewDraft() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        fixture.model.stop()
        try await waitUntil { !fixture.model.isDeliveringLocalCancellation }
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        fixture.model.newConversation()
        fixture.model.draft = "Another conversation"
        XCTAssertTrue(fixture.model.selectedComposerState.canSend)
        fixture.model.selectedConversationID = fixture.conversation
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertFalse(fixture.model.canResumeLocalRun)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
    }

    func testCommittedCompletionWinsLateCancellationError() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.commitCompletion(run)
        try await fixture.runtime.end(run, as: .cancelled)
        try await waitUntil {
            !fixture.model.isRunning && fixture.model.localRunRecovery == nil
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        XCTAssertEqual(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run)?.result.value.kind, .answered)
        XCTAssertTrue(fixture.model.selectedMessages.contains { $0.text == "Committed local answer" })
        XCTAssertEqual(fixture.activity.entries.map(\.status), [.answered])
        let cancellationCount = await fixture.runtime.cancelCount()
        XCTAssertEqual(cancellationCount, 0)
    }

    func testOldRunEventCannotCompleteNextRun() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let old = try await start(fixture)
        try await fixture.runtime.commitCompletion(old)
        try await fixture.runtime.end(old, as: .eof)
        try await waitUntil {
            !fixture.model.isRunning && fixture.model.localRunRecovery == nil
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        let next = try await start(fixture, prompt: "Start the next request")
        XCTAssertNotEqual(old, next)
        try await fixture.runtime.emitOldCompletion(into: next, oldRunID: old)
        try await fixture.runtime.end(next, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil }
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, next)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: next))
        try await fixture.runtime.commitCompletion(next, answer: "Next committed answer")
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.count == 2
        }
    }

    func testDefinitePreparationFailureRestoresUnsentInputWithoutCanonicalRun() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        await fixture.runtime.failPreparation()
        fixture.model.draft = "  Keep the original draft  "
        fixture.model.send()
        try await waitUntil { !fixture.model.isRunning }
        XCTAssertEqual(fixture.model.draft, "  Keep the original draft  ")
        XCTAssertNil(fixture.model.localRunRecovery)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertNil(try fixture.port.activeRunIdentity(conversationID: fixture.conversation))
        let cancellationCount = await fixture.runtime.cancelCount()
        XCTAssertEqual(cancellationCount, 0)
    }

    func testTypedHostRejectionRollsBackOptimisticMessagesWithoutOverwritingNewDraft() async throws {
        for newDraft in ["", "A later unsent draft"] {
            let fixture = try await Fixture()
            defer { fixture.cleanUp() }
            await fixture.runtime.pauseBeforeAdmission()
            let before = fixture.model.selectedMessages.map(\.id)
            fixture.model.draft = "  Original rejected request  "
            fixture.model.send()
            try await waitUntil { await fixture.runtime.hasPendingAdmission() }
            XCTAssertTrue(fixture.model.selectedMessages.contains { $0.role == .user && $0.text.contains("Original rejected request") })
            if !newDraft.isEmpty { fixture.model.draft = newDraft }
            await fixture.runtime.finishPendingAdmission(rejected: true)
            try await waitUntil { !fixture.model.isRunning }
            XCTAssertEqual(fixture.model.selectedMessages.map(\.id), before)
            XCTAssertEqual(fixture.model.draft, newDraft.isEmpty ? "  Original rejected request  " : newDraft)
            XCTAssertNil(fixture.model.localRunRecovery)
            XCTAssertEqual(fixture.model.localSubmissionFailure?.conversationID, fixture.conversation)
            XCTAssertEqual(fixture.model.localSubmissionFailure?.code, "request_rejected_before_admission")
            XCTAssertNil(try fixture.port.activeRunIdentity(conversationID: fixture.conversation))
        }
    }

    func testUnknownTransportBeforeAckRetainsBubbleAndBlocksDuplicateSubmission() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        await fixture.runtime.pauseBeforeAdmission()
        fixture.model.draft = "May already have been admitted"
        fixture.model.send()
        try await waitUntil { await fixture.runtime.hasPendingAdmission() }
        let optimistic = fixture.model.selectedMessages.map(\.id)
        await fixture.runtime.finishPendingAdmission(rejected: false)
        try await waitUntil { fixture.model.localRunRecovery != nil }
        XCTAssertNil(fixture.model.localSubmissionFailure)
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), optimistic)
        XCTAssertEqual(fixture.model.draft, "")
        fixture.model.draft = "Do not duplicate"
        fixture.model.send()
        XCTAssertEqual(fixture.model.draft, "Do not duplicate")
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), optimistic)
    }

    func testUnknownRiskDecisionUsesFrozenOwnerAndResumesOnlyTheExistingRun() async throws {
        let service = UnknownRecoveryServiceFixture()
        let fixture = try await Fixture(unknownTransport: service.execute)
        defer { fixture.cleanUp() }
        let run = try await start(fixture, prompt: "Preserve this run")
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let presentation = try XCTUnwrap(fixture.model.currentUnknownEffectRecovery)
        XCTAssertFalse(fixture.model.canResumeLocalRun)
        XCTAssertTrue(service.decisions.isEmpty, "inspection must never authorize retry")
        let decision = try presentation.decision(targetIndex: 0, reason: "I checked the external state")
        let delivery = Task { await fixture.model.resolveUnknownEffects(decision, presentation: presentation) }
        try await waitUntil { await fixture.runtime.resumeSnapshots().count == 1 }
        XCTAssertEqual(service.decisions, [decision])
        let snapshots = await fixture.runtime.resumeSnapshots()
        XCTAssertEqual(snapshots.map(\.runID), [run])
        let runs = await fixture.runtime.runIDs()
        XCTAssertEqual(runs, [run])
        try await fixture.runtime.commitCompletion(run)
        try await fixture.runtime.end(run, as: .eof)
        await delivery.value
    }

    func testStaleUnknownDecisionAfterConversationSwitchNeverReachesService() async throws {
        let service = UnknownRecoveryServiceFixture()
        let fixture = try await Fixture(unknownTransport: service.execute)
        defer { fixture.cleanUp() }
        let run = try await start(fixture, prompt: "First owner")
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let presentation = try XCTUnwrap(fixture.model.currentUnknownEffectRecovery)
        let decision = try presentation.decision(targetIndex: 0, reason: "Explicit confirmation")
        fixture.model.newConversation()
        XCTAssertNotEqual(fixture.model.selectedConversationID, presentation.owner.conversationID)
        await fixture.model.resolveUnknownEffects(decision, presentation: presentation)
        XCTAssertTrue(service.decisions.isEmpty)
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumed.isEmpty)
    }

    func testInFlightRecoveryDoesNotMakeAnotherConversationBusy() async throws {
        let gate = DispatchSemaphore(value: 0)
        let service = UnknownRecoveryServiceFixture(resolutionGate: gate)
        let fixture = try await Fixture(unknownTransport: service.execute)
        defer { gate.signal(); fixture.cleanUp() }
        let run = try await start(fixture, prompt: "Recovery belongs to this conversation")
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let presentation = try XCTUnwrap(fixture.model.currentUnknownEffectRecovery)
        let decision = try presentation.decision(targetIndex: 0, reason: "Explicit risk confirmation")
        let delivery = Task { await fixture.model.resolveUnknownEffects(decision, presentation: presentation) }
        try await waitUntil { service.hasStartedResolution }
        XCTAssertTrue(fixture.model.isResolvingUnknownEffects)
        fixture.model.newConversation()
        XCTAssertFalse(fixture.model.isResolvingUnknownEffects,
                       "A durable decision for another Run must not make this composer busy")
        XCTAssertFalse(fixture.model.isCheckingLocalRunRecovery)
        gate.signal()
        await delivery.value
        XCTAssertFalse(fixture.model.isResolvingUnknownEffects)
        XCTAssertNil(fixture.model.currentUnknownEffectRecovery)
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumed.isEmpty, "Detached reply must not resume whichever conversation is selected now")
    }

    func testExplicitResumeRetainsRunMessagesAndOriginalRequestWhenComposerDefaultsChange() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture, prompt: "Original request only once")
        let captured = await fixture.runtime.requestSnapshot(run)
        let original = try XCTUnwrap(captured)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let before = try fixture.port.loadConversationWindow(fixture.conversation).projection.messages.map(\.messageID)
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        fixture.model.style = .detailed
        fixture.model.draft = "A different unsent follow-up"
        let resume = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { await fixture.runtime.resumeSnapshots().count == 1 }
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertEqual(resumed, [original], "resume has no authority to replace the original prompt or model settings")
        let runIDs = await fixture.runtime.runIDs()
        XCTAssertEqual(runIDs, [run])
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        XCTAssertEqual(try fixture.port.loadConversationWindow(fixture.conversation).projection.messages.map(\.messageID), before)
        XCTAssertEqual(fixture.model.draft, "A different unsent follow-up")
        try await fixture.runtime.commitCompletion(run)
        try await fixture.runtime.end(run, as: .eof)
        await resume.value
        try await waitUntil {
            !fixture.model.isRunning && fixture.model.localRunRecovery == nil
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        let messages = try fixture.port.loadConversationWindow(fixture.conversation).projection.messages
        XCTAssertEqual(messages.map(\.messageID), before)
        XCTAssertEqual(messages.filter { $0.role == .user }.count, 1)
    }

    func testResumeTransportFailureKeepsSameRunPendingWithoutCompletionActivity() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .failed)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.runtime.failNextResume()
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        await fixture.model.resumeLocalRun()
        try await waitUntil { !fixture.model.isResumingLocalRun && fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertEqual(resumed.map(\.runID), [run])
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        try await fixture.runtime.commitCompletion(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.answered]
        }
    }

    func testRevokedFrozenWorkspacePreventsResumeTransport() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        fixture.bookmarks.removeWorkspace(conversationID: fixture.conversation)
        await fixture.model.resumeLocalRun()
        let resumed = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumed.isEmpty, "revoked original authority cannot be replaced by a sandbox or later default")
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertFalse(fixture.model.isRunning)
        XCTAssertNotNil(fixture.model.localRecoveryError)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertEqual(try fixture.port.activeRunIdentity(conversationID: fixture.conversation)?.runID, run)
        try await fixture.runtime.commitCompletion(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.answered]
        }
    }

    func testCanonicalTerminalDuringAwaitedRevalidationPreventsOldRunDispatch() async throws {
        let fixture = try await Fixture()
        defer { fixture.resolutionGate.release(); fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        let bookmarks = fixture.bookmarks
        let gate = fixture.resolutionGate
        await fixture.runtime.setBeforeRevalidate {
            // Initial admission already passed. Force the second, launch-bound
            // authorization check through real off-actor bookmark resolution.
            bookmarks.close()
            gate.arm()
        }
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { gate.isEntered }
        let beforeDispatch = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(beforeDispatch.isEmpty)
        try await fixture.runtime.commitCompletion(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.answered]
        }
        gate.release()
        // The resumed validation must unwind; no stale transport may replace
        // the already-finalized canonical Run after this await returns.
        try await waitUntil { await fixture.runtime.finishedResumeAttempts() == 1 }
        await resumeTask.value
        let afterDispatch = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(afterDispatch.isEmpty)
        XCTAssertEqual(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run)?.result.value.kind, .answered)
        XCTAssertNil(fixture.model.localRunRecovery)
    }

    func testStopBeforeResumeTransportReadyRetainsIntentWithoutSyntheticTerminal() async throws {
        let fixture = try await Fixture()
        defer { Task { await fixture.runtime.releaseReady() }; fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        await fixture.runtime.pauseNextReady()
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { await fixture.runtime.isWaitingForReady() }
        let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
        fixture.model.stop()
        let beforeReady = await fixture.runtime.cancelCount()
        XCTAssertEqual(beforeReady, 0, "No cancel may be delivered to an unready transport")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        let durable = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(durable.status, "cancelling")
        XCTAssertEqual(durable.pendingCancels, 1)
        await fixture.runtime.releaseReady()
        try await waitUntil { await fixture.runtime.cancelCount() == 1 }
        let delivered = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertEqual(delivered, [intent], "Ready resumes deliver the persisted exact cancellation request")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        try await fixture.runtime.end(run, as: .cancelled)
        await resumeTask.value
        try await waitUntil { !fixture.model.isRunning && fixture.model.localRunRecovery != nil }
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }

    func testCancellationReceiptFailureInterruptsOnlyOwnedResumeAndRetainsRetryableIntent() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { await fixture.runtime.resumeSnapshots().count == 1 }
        let messages = fixture.model.selectedMessages.map(\.id)
        let revision = fixture.model.selectedTerminalTranscriptRevision
        let wrongIdentityInterrupted = await fixture.runtime.interruptOwnedTransport(
            conversationID: fixture.conversation, expectedRunID: run, assistantMessageID: "wrong-assistant")
        XCTAssertFalse(wrongIdentityInterrupted)
        // Fail only the durable cancellation command receipt, not reads,
        // projection subscriptions or unrelated Application transactions.
        try fixture.executeSQLite("""
            CREATE TRIGGER reject_test_cancellation BEFORE INSERT ON command_receipts
            WHEN NEW.command_type = 'request_run_cancellation'
            BEGIN SELECT RAISE(ABORT, 'isolated cancellation receipt fault'); END;
            """)
        fixture.model.stop()
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        try await waitUntil { await fixture.runtime.interruptedRunIDs() == [run] }
        await resumeTask.value
        try await waitUntil { !fixture.model.isRunning && fixture.model.canRetryLocalRunCancellation }
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRecoveryError)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, revision)
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        let beforeRetry = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertTrue(beforeRetry.isEmpty, "Storage rejection cannot dispatch a durable cancellation")
        let facts = try await fixture.runtime.cancellationFacts(run)
        XCTAssertNotEqual(facts.status, "cancelling", "Receipt rejection rolls back the cancellation transaction")
        XCTAssertEqual(facts.pendingCancels, 0)
        try fixture.executeSQLite("DROP TRIGGER reject_test_cancellation;")
        await fixture.model.retryLocalRunCancellation()
        let delivered = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertEqual(delivered, [intent])
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), messages)
        let runs = await fixture.runtime.runIDs()
        XCTAssertEqual(runs, [run])
        let persisted = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(persisted.status, "cancelling")
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }

    func testFailedCancellationDeliveryRetriesSameIntentWithoutWorkspaceAuthorizationOrNewTurn() async throws {
        let fixture = try await Fixture()
        defer { fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        let originalMessages = fixture.model.selectedMessages.map(\.id)
        let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
        await fixture.runtime.failNextCancellationDelivery()
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canCancelLocalRecovery)
        fixture.model.stop()
        try await waitUntil {
            await fixture.runtime.deliveredCancellationRequests().count == 1
                && !fixture.model.isDeliveringLocalCancellation
        }
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        XCTAssertTrue(fixture.model.canRetryLocalRunCancellation)
        XCTAssertFalse(fixture.model.canCancelLocalRecovery, "Pending cancellation must not offer a new intent")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRecoveryError)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        fixture.bookmarks.removeWorkspace(conversationID: fixture.conversation)
        await fixture.model.retryLocalRunCancellation()
        let attempts = await fixture.runtime.deliveredCancellationRequests()
        XCTAssertEqual(attempts, [intent, intent], "Retry preserves cancellation identity and requested timestamp")
        let successfulDeliveries = await fixture.runtime.cancelCount()
        XCTAssertEqual(successfulDeliveries, 1)
        let runs = await fixture.runtime.runIDs()
        XCTAssertEqual(runs, [run])
        let resumes = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(resumes.isEmpty, "Cancellation-only delivery must not resume model execution")
        XCTAssertEqual(fixture.model.selectedMessages.map(\.id), originalMessages)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }

    func testStopDuringSecondBookmarkValidationPersistsIntentWithoutLaunchingResume() async throws {
        let fixture = try await Fixture()
        defer { fixture.resolutionGate.release(); fixture.cleanUp() }
        let run = try await start(fixture)
        try await fixture.runtime.end(run, as: .eof)
        try await waitUntil { fixture.model.localRunRecovery != nil && !fixture.model.isRunning }
        await fixture.model.checkLocalRunRecovery()
        XCTAssertTrue(fixture.model.canResumeLocalRun)
        let gate = fixture.resolutionGate
        let bookmarks = fixture.bookmarks
        await fixture.runtime.setBeforeRevalidate { bookmarks.close(); gate.arm() }
        let resumeTask = Task { await fixture.model.resumeLocalRun() }
        try await waitUntil { gate.isEntered }
        let terminalRevision = fixture.model.selectedTerminalTranscriptRevision
        fixture.model.stop()
        let intent = try XCTUnwrap(fixture.model.localCancellationRequest)
        XCTAssertEqual(intent.expectedRunID, run)
        let durable = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(durable.status, "cancelling")
        XCTAssertEqual(durable.pendingCancels, 1)
        XCTAssertNil(try fixture.repository.canonicalRunTerminal(conversationID: fixture.conversation, runID: run))
        XCTAssertTrue(fixture.activity.entries.isEmpty)
        // Revoking access after cancellation must not retract its committed
        // intent or make directory reauthorization necessary for read-only checks.
        fixture.bookmarks.removeWorkspace(conversationID: fixture.conversation)
        gate.release()
        await resumeTask.value
        let dispatched = await fixture.runtime.resumeSnapshots()
        XCTAssertTrue(dispatched.isEmpty)
        let cancellationDeliveries = await fixture.runtime.cancelCount()
        XCTAssertEqual(cancellationDeliveries, 0, "no Host transport was launched")
        XCTAssertTrue(fixture.model.remoteRecoveryIsCancellationPending)
        XCTAssertNotNil(fixture.model.localRunRecovery)
        XCTAssertEqual(fixture.model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        fixture.model.stop()
        await fixture.model.checkLocalRunRecovery()
        let repeated = try await fixture.runtime.cancellationFacts(run)
        XCTAssertEqual(repeated.revision, durable.revision)
        XCTAssertEqual(fixture.model.localCancellationRequest, intent)
        XCTAssertFalse(fixture.model.canResumeLocalRun)
        try await fixture.runtime.commitCancellation(run)
        try await waitUntil {
            fixture.model.localRunRecovery == nil && !fixture.model.isRunning
                && fixture.activity.entries.map(\.status) == [.cancelled]
        }
    }
}
