// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

nonisolated private func unexpectedPermissionEnvelopeHostCall(_ name: String) -> NSError {
    XCTFail("Unexpected Host operation: \(name)")
    return NSError(domain: "PermissionEnvelopeTest", code: 1)
}

@MainActor
final class PermissionEnvelopeRemoteHost: MacRemoteAgentRunning {
    var permissionConfigurationRequests: [(MacRemoteSubmissionIdentity, Int64?, MacPermissionMode?)] = []
    var permissionConfigurationResponse: MacRemotePermissionConfiguration?
    var permissionConfigurationHandler: ((MacRemoteSubmissionIdentity, Int64?, MacPermissionMode?) async throws -> MacRemotePermissionConfiguration)?
    func permissionConfiguration(identity: MacRemoteSubmissionIdentity, target: MacRemoteSubmissionTarget,
                                 expectedRevision: Int64?, profile: MacPermissionMode?, minimumPolicyRevision: Int64?) async throws -> MacRemotePermissionConfiguration {
        permissionConfigurationRequests.append((identity, expectedRevision, profile))
        if let permissionConfigurationHandler {
            let response = try await permissionConfigurationHandler(identity, expectedRevision, profile)
            try response.validate(identity: identity)
            return response
        }
        guard let permissionConfigurationResponse else {
            throw unexpectedPermissionEnvelopeHostCall("remote.permissionConfiguration")
        }
        try permissionConfigurationResponse.validate(identity: identity)
        return permissionConfigurationResponse
    }
    var selectionRequests: [(String, String?)] = []
    var selectionFailure = false
    func saveConversationSelection(conversationID: String, title: String, profile: MacRemoteModelProfile?,
        allowCreate: Bool, target: MacRemoteSubmissionTarget) async throws {
        XCTAssertEqual(target.configuration.expectedHostID, capturedHostID)
        selectionRequests.append((conversationID, profile?.profileID))
        if selectionFailure { throw URLError(.networkConnectionLost) }
    }
    var goalProgressTargets: [MacRemoteSubmissionTarget] = []
    func conversationProgress(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacRemoteConversationProgress {
        goalProgressTargets.append(target)
        return .init(source_host_id: capturedHostID, conversation_id: conversationID, goal_projection: .null,
            plan: .init(conversationID: conversationID, todos: .array([]), revision: 7, updatedAtMs: 42))
    }
    var allowsEmptyUIQueue = false
    func queuedUIInput(conversationID: String, target: MacRemoteSubmissionTarget) async throws -> MacUIQueueState {
        guard allowsEmptyUIQueue else { throw unexpectedPermissionEnvelopeHostCall("remote.queuedUIInput") }
        XCTAssertEqual(target.configuration.expectedHostID, capturedHostID)
        XCTAssertEqual(target.clientID, "fixture-client")
        XCTAssertFalse(try XCTUnwrap(target.expectedSourceStoreEpoch).isEmpty)
        return try JSONDecoder().decode(MacUIQueueState.self, from: Data(#"{"head":null,"dispatch":null}"#.utf8))
    }
    var applicationPort: MacApplicationPort?
    var sourceFixtures: [String: MacAssistantJournalTestFixture] = [:]
    var allowsRun = false
    var probesTaskControls = false
    var taskControlTargets: [(String, String)] = []
    var runRequests = 0
    var lastRunContextComponents: [MacCoreContextComponent] = []
    var lastRunExternalSessionID: String?
    var lastRunPrompt: String?
    var lastRunMemoryEnabled: Bool?
    var runContinuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation?
    let fixtureProfile = MacRemoteModelProfile(hostID: "fixture-host", profileID: "fixture-profile",
        revision: 1, displayName: "Fixture", modelName: "fixture-model")
    var sentPayloads: [[String: Any]] = []
    var response: CheckedContinuation<Void, Never>?
    var failsResponse = false
    var resumeRequests: [(String, String)] = []
    var observationRequests: [(String, String?, String?)] = []
    var observation: ((String, String?, String?) throws -> MacRemoteRunObservationBatch)?
    var cancellationRequests: [String] = []
    var failsCancellation = false
    var permissionDecisionRequests = 0
    var resumeContinuation: AsyncThrowingStream<MacLocalAgentEvent, Error>.Continuation?
    nonisolated let capturedHostID: String
    init(capturedHostID: String = "fixture-host") { self.capturedHostID = capturedHostID }
    nonisolated func captureSubmissionTarget() throws -> MacRemoteSubmissionTarget {
        .init(configuration: .init(baseURL: URL(string: "https://fixture.invalid")!,
                                   token: "fixture", expectedHostID: capturedHostID), clientID: "fixture-client")
    }
    func resume(conversationID: String, expectedRunID: String) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        resumeRequests.append((conversationID, expectedRunID))
        return AsyncThrowingStream { resumeContinuation = $0 }
    }
    func providePermissionInstruction(request: MacRemotePermissionRequest, instruction: String) async throws {
        sentPayloads.append(try request.instructionDecision(instruction).payload)
        await withCheckedContinuation { response = $0 }
        if failsResponse { throw NSError(domain: "PermissionEnvelopeTest", code: 2) }
    }
    func isConfigured() async -> Bool { true }
    func testSavedConnection() async throws -> MacRemoteHostConnectionReport {
        throw unexpectedPermissionEnvelopeHostCall("remote.testConnection")
    }
    func requestWebManagementURL(target: String) async throws -> URL {
        throw unexpectedPermissionEnvelopeHostCall("remote.managementURL")
    }
    func fetchExtensions() async throws -> [MacExtensionItem] {
        throw unexpectedPermissionEnvelopeHostCall("remote.extensions")
    }
    func fetchExecutionBackends() async throws -> [MacRemoteExecutionBackend] {
        if allowsRun { return [] }
        throw unexpectedPermissionEnvelopeHostCall("remote.backends")
    }
    func fetchRemoteModelCatalog() async throws -> MacRemoteModelCatalogSnapshot {
        if allowsRun { return .init(hostID: fixtureProfile.hostID, profiles: [fixtureProfile],
            defaultResolution: .resolved(profileID: fixtureProfile.profileID, origin: .automatic), defaultRevision: nil) }
        throw unexpectedPermissionEnvelopeHostCall("remote.models")
    }
    func run(conversationID: String, submissionID: String, userMessageID: String,
             assistantMessageID: String, prompt: String, contextComponents: [MacCoreContextComponent],
             modelID: String, modelProviderID: String?, modelIntent: MacRemoteModelIntent?, memoryEnabled: Bool, executionBackend: String,
             externalSessionID: String?, submission: MacRemoteSubmissionControl) async -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        if allowsRun {
            do {
                // Freeze the fixture caller's canonical read before dispatch
                // and preparation awaits; never rebase a pending submission.
                let metadata = try applicationPort?.loadConversationMetadata(conversationID)
                let expectedRevision = metadata?.configurationRevision
                let expectedParent = try applicationPort?.loadConversation(conversationID)?.messages.last?.id
                try submission.ticket.claimDispatch()
                try await submission.beforeDispatch()
                if let applicationPort {
                    if let intent = try applicationPort.remoteSubmissionIntent(conversationID: conversationID), intent.phase == .prepared {
                        let request: [String: Any] = ["conversationId":conversationID, "expected_host_id":"fixture-host",
                            "client_id":"fixture-client", "submission_id":submissionID, "user_message_id":userMessageID,
                            "assistant_message_id":assistantMessageID, "messages":[["role":"user", "content":prompt]],
                            "expected_configuration_revision": try XCTUnwrap(expectedRevision),
                            "expected_parent_message_id": expectedParent as Any? ?? NSNull()]
                        _ = try applicationPort.bindRemoteSubmissionRequest(identity: intent.identity,
                            request: JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONSerialization.data(withJSONObject: request)))
                    }
                    sourceFixtures[submissionID] = try MacAssistantJournalTestFixture.forReceiver(receiver: applicationPort,
                        conversationID: conversationID, runID: submissionID, assistantMessageID: assistantMessageID)
                }
            } catch {
                if submission.ticket.phase == .dispatching { submission.ticket.rejectBeforeTransport() }
                return AsyncThrowingStream { $0.finish(throwing: error) }
            }
            submission.ticket.markAccepted()
            runRequests += 1
            lastRunContextComponents = contextComponents
            lastRunExternalSessionID = externalSessionID
            lastRunPrompt = prompt
            lastRunMemoryEnabled = memoryEnabled
            return AsyncThrowingStream { runContinuation = $0 }
        }
        return AsyncThrowingStream { $0.finish(throwing: unexpectedPermissionEnvelopeHostCall("remote.run")) }
    }
    func observeRun(conversationID: String, expectedRunID: String?, assistantMessageID: String?,
                    target: MacRemoteSubmissionTarget) async throws -> MacRemoteRunObservationBatch {
        observationRequests.append((conversationID, expectedRunID, assistantMessageID))
        guard let observation else { throw MacRemoteHostError.invalidResponse }
        return try observation(conversationID, expectedRunID, assistantMessageID)
    }
    func resolvePermission(request: MacRemotePermissionRequest, allow: Bool,
                           scope: MacPermissionGrantScope) async throws {
        permissionDecisionRequests += 1
        throw unexpectedPermissionEnvelopeHostCall("remote.resolvePermission")
    }
    func sendTaskGuidance(conversationID: String, parentRunID: String, taskID: String,
                          message: String, target: MacRemoteSubmissionTarget) async throws -> AppleTaskGuidanceReceipt {
        if probesTaskControls {
            XCTAssertEqual(parentRunID, "parent-run")
            XCTAssertEqual(target.configuration.expectedHostID, capturedHostID)
            XCTAssertNotNil(target.expectedSourceStoreEpoch)
            taskControlTargets.append((conversationID, taskID))
            throw NSError(domain: "TaskControlProbe", code: 1)
        }
        throw unexpectedPermissionEnvelopeHostCall("remote.taskGuidance")
    }
    func requestTaskCancellation(conversationID: String, parentRunID: String,
                                 taskID: String, target: MacRemoteSubmissionTarget) async throws -> AppleTaskCancellationReceipt {
        if probesTaskControls {
            XCTAssertEqual(parentRunID, "parent-run")
            XCTAssertEqual(target.configuration.expectedHostID, capturedHostID)
            XCTAssertNotNil(target.expectedSourceStoreEpoch)
            taskControlTargets.append((conversationID, taskID))
            throw NSError(domain: "TaskControlProbe", code: 1)
        }
        throw unexpectedPermissionEnvelopeHostCall("remote.cancelTask")
    }
    func cancelRun(identity: MacRemoteSubmissionIdentity, requestDigest: String) async throws {
        cancellationRequests.append(identity.conversation_id)
        if failsCancellation { throw NSError(domain: "PermissionEnvelopeTest", code: 3) }
    }
    func taskEvents(conversationID: String,
                    afterCursor: Int64) async -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error> {
        if allowsRun { return AsyncThrowingStream { $0.finish() } }
        return AsyncThrowingStream { $0.finish(throwing: unexpectedPermissionEnvelopeHostCall("remote.taskEvents")) }
    }
}

@MainActor
final class MacRemotePermissionConsumptionTests: XCTestCase {
    private func coldPermissionFixture(root: URL) async throws ->
        (MacWorkspaceModel, PermissionEnvelopeRemoteHost, MacApplicationPort, MacAssistantJournalTestFixture) {
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("receiver.db"))
        let source = try MacAssistantJournalTestFixture(conversationID: "policy-owner-\(UUID())", runID: "policy-run",
            assistantMessageID: "policy-assistant")
        try source.ingestAdmissions(into: port)
        let receipt = try source.submissionReceipt()
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.observation = { conversation, run, assistant in
            XCTAssertEqual(conversation, source.conversationID)
            XCTAssertEqual(run, source.runID)
            XCTAssertEqual(assistant, source.assistantMessageID)
            var observation = MacRemoteRunObservation(protocol_version: 1, conversation_id: conversation,
                client_id: "fixture-client", run_id: source.runID, assistant_message_id: source.assistantMessageID,
                status: "running", worker: .init(state: .running, generation: "7"), recovery: .observe,
                admission: receipt.admission, terminal_seal: nil, permissions: [], can_submit: false)
            observation.submission_receipt = receipt
            return .init(observation: observation, events: [])
        }
        let sourcePort = try MacApplicationPort(databaseURL: source.databaseURL)
        let revision = try XCTUnwrap(sourcePort.loadConversationMetadata(source.conversationID)?.configurationRevision)
        host.permissionConfigurationResponse = .init(conversation_id: source.conversationID,
            configuration_revision: revision, permission_profile: .cautious, active_run_id: source.runID,
            active_policy: .init(runID: source.runID, configurationRevision: revision, profile: .cautious), owner: .init(kind: .local, run_id: source.runID, assistant_message_id: source.assistantMessageID, identity: nil), state: .active)
        let model = MacWorkspaceModel(initialConversationID: source.conversationID,
            canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        await model.loadSelectedConversationIfNeeded()
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.presentationDidAttach()
        _ = try await waitForValue { model.remoteRunRecovery }
        _ = try await waitForValue { model.selectedPermissionActiveMode == .cautious ? true : nil }
        XCTAssertFalse(model.isRunning, "Cold discovery must not start an execution transport")
        return (model, host, port, source)
    }

    func testColdAndInterruptedPolicyOwnerSavesAtSourceAndWaitsForExactACKAcrossSelection() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let (model, host, port, source) = try await coldPermissionFixture(root: root)
        defer { model.presentationDidDetach(); model.selectedConversationID = nil }
        let sourcePort = try MacApplicationPort(databaseURL: source.databaseURL)
        let baseline = try XCTUnwrap(sourcePort.loadConversationMetadata(source.conversationID)?.configurationRevision)
        let pending = MacRemotePermissionRequest(id: "policy-pending", allowConversation: true, protocolVersion: 1, nonce: "policy-live-nonce",
            hostID: "fixture-host", clientID: "fixture-client", sessionID: source.conversationID,
            toolName: "web_search", arguments: "", callID: "policy-call", reason: "Fixture permission",
            message: "Confirm fixture", expiresAt: Int64(Date().timeIntervalSince1970) + 300)
        let observe = try XCTUnwrap(host.observation)
        host.observation = { conversation, run, assistant in
            let batch = try observe(conversation, run, assistant)
            return .init(observation: batch.observation, events: [.init(kind: .status, text: "", title: nil,
                detail: nil, isError: false, permissionRequest: pending)])
        }
        let recovery = try XCTUnwrap(model.remoteRunRecovery)
        model.noteRemoteTransportInterruption(conversationID: source.conversationID,
            assistantMessageID: recovery.assistantMessageID, generationID: recovery.generationID)
        _ = try await waitForValue { model.selectedPendingRemotePermission }
        XCTAssertEqual(model.selectedPendingRemotePermission?.id, pending.id)
        model.draft = "Retain original draft"
        var releasePatch: CheckedContinuation<Void, Never>?
        var acknowledge = false
        var savedRevision = baseline
        host.permissionConfigurationHandler = { identity, revision, profile in
            XCTAssertEqual(identity, source.submissionIdentity)
            if let profile {
                XCTAssertEqual(profile, .allowAll)
                XCTAssertEqual(revision, baseline)
                await withCheckedContinuation { releasePatch = $0 }
                _ = try sourcePort.setConversationPermission(.init(conversationID: source.conversationID,
                    expectedRevision: baseline, expectedRunID: source.runID, expectedSourceHostID: nil,
                    profile: "allow_all", updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))).committedSnapshot()
                savedRevision = baseline + 1
            }
            return .init(conversation_id: source.conversationID, configuration_revision: savedRevision,
                permission_profile: savedRevision == baseline ? .cautious : .allowAll, active_run_id: source.runID,
                active_policy: .init(runID: source.runID, configurationRevision: acknowledge ? savedRevision : baseline,
                    profile: acknowledge ? .allowAll : .cautious), owner: .init(kind: .local, run_id: source.runID, assistant_message_id: source.assistantMessageID, identity: nil), state: acknowledge || savedRevision == baseline ? .active : .pending)
        }
        model.permissionMode = .allowAll
        _ = try await waitForValue { releasePatch }
        XCTAssertEqual(model.selectedPermissionConfigurationState, .saving)
        model.newConversation()
        let other = try XCTUnwrap(model.selectedConversationID)
        model.draft = "Other draft"
        releasePatch?.resume()
        releasePatch = nil
        try await model.waitForModelConfigurationCommit(conversationID: source.conversationID)
        XCTAssertEqual(model.selectedConversationID, other)
        XCTAssertEqual(model.draft, "Other draft")
        XCTAssertEqual(model.permissionMode, .cautious)
        model.selectedConversationID = source.conversationID
        XCTAssertEqual(model.draft, "Retain original draft")
        _ = try await waitForValue { model.selectedPermissionActiveMode == .cautious ? true : nil }
        XCTAssertEqual(model.selectedPermissionConfigurationState, .awaitingApplication)
        XCTAssertEqual(model.selectedPermissionActiveMode, .cautious)
        XCTAssertEqual(model.selectedPendingRemotePermission?.id, pending.id, "Saving cannot locally retire the original pending request")
        XCTAssertTrue(model.reloadComposerPermissionConfiguration(conversationID: source.conversationID))
        try await model.waitForModelConfigurationCommit()
        XCTAssertEqual(model.selectedPermissionConfigurationState, .awaitingApplication)
        XCTAssertEqual(model.draft, "Retain original draft")
        acknowledge = true
        _ = try await waitForValue { model.selectedPermissionConfigurationState == .active ? true : nil }
        XCTAssertEqual(model.selectedPermissionActiveMode, .allowAll)
        XCTAssertEqual(host.permissionConfigurationRequests.filter { $0.2 != nil }.count, 1)
        XCTAssertEqual(host.runRequests, 0)
        XCTAssertEqual(host.permissionDecisionRequests, 0, "Policy ACK must not synthesize an approval response")
        XCTAssertEqual(try port.activeRunIdentity(conversationID: source.conversationID)?.runID, source.runID)
    }

    func testInterruptedPolicyCASFailureKeepsDraftAndDoesNotCommitMirror() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let (model, host, port, source) = try await coldPermissionFixture(root: root)
        defer { model.presentationDidDetach(); model.selectedConversationID = nil }
        let before = try port.loadConversationMetadata(source.conversationID)
        model.draft = "Not submitted"
        let baseline = try XCTUnwrap(host.permissionConfigurationResponse)
        host.permissionConfigurationHandler = { _, _, profile in
            if profile != nil { throw MacRemoteHostError.http(409, "configuration_conflict") }
            return baseline
        }
        model.permissionMode = .allowAll
        do { try await model.waitForModelConfigurationCommit(); XCTFail("Source conflict must not become a mirror-only save") }
        catch {}
        XCTAssertEqual(model.selectedPermissionConfigurationState, .failed)
        XCTAssertTrue(model.selectedPermissionHasUncommittedChanges)
        XCTAssertEqual(model.selectedPermissionActiveMode, .cautious)
        XCTAssertEqual(model.draft, "Not submitted")
        XCTAssertEqual(try port.loadConversationMetadata(source.conversationID)?.configurationRevision, before?.configurationRevision)
        XCTAssertEqual(try port.loadConversationMetadata(source.conversationID)?.settingsJSON, before?.settingsJSON)
        XCTAssertEqual(host.permissionConfigurationRequests.filter { $0.2 != nil }.count, 1)
    }

    func testPermissionDisplayGettersDoNotReadAuthorityDuringRepeatedRender() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let (model, host, _, _) = try await coldPermissionFixture(root: root)
        defer { model.presentationDidDetach(); model.selectedConversationID = nil }
        #if DEBUG
        let reads = model.permissionOwnerAuthorityReadCount
        #endif
        let requests = host.permissionConfigurationRequests.count
        for _ in 0..<100 {
            XCTAssertEqual(model.selectedPermissionConfigurationState, .active)
            XCTAssertEqual(model.selectedPermissionActiveMode, .cautious)
        }
        #if DEBUG
        XCTAssertEqual(model.permissionOwnerAuthorityReadCount, reads, "Rendering must not invoke the owner authority reader")
        #endif
        XCTAssertEqual(host.permissionConfigurationRequests.count, requests)
    }

    func testFormalTerminalPolicyEditIsInactiveAndDoesNotPatchOldSourceRun() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let (model, host, port, source) = try await coldPermissionFixture(root: root)
        defer { model.presentationDidDetach(); model.selectedConversationID = nil }
        model.remoteObservationForegroundChanged(false)
        _ = try source.commitTerminal(into: port)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: source.conversationID, runID: source.runID))
        let before = host.permissionConfigurationRequests.count
        model.permissionMode = .allowAll
        try await model.waitForModelConfigurationCommit()
        XCTAssertEqual(model.selectedPermissionConfigurationState, .saved)
        XCTAssertNil(model.selectedPermissionActiveMode)
        XCTAssertEqual(host.permissionConfigurationRequests.count, before)
    }

    func testSourceCommitAndMirrorConflictRetainActualSourcePolicyWithoutReplay() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let (model, host, port, source) = try await coldPermissionFixture(root: root)
        defer { model.presentationDidDetach(); model.selectedConversationID = nil }
        let sourcePort = try MacApplicationPort(databaseURL: source.databaseURL)
        let sourceRevision = try XCTUnwrap(sourcePort.loadConversationMetadata(source.conversationID)?.configurationRevision)
        let mirrorRevision = try XCTUnwrap(port.loadConversationMetadata(source.conversationID)?.configurationRevision)
        _ = try port.patchConversationConfiguration(.init(conversationID: source.conversationID,
            expectedRevision: mirrorRevision, patchJSON: #"{"memoryEnabledOverride":true}"#,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))).committedSnapshot()
        var saved = false
        host.permissionConfigurationHandler = { _, revision, profile in
            if profile != nil {
                XCTAssertFalse(saved, "A failed mirror must not replay the committed source CAS")
                XCTAssertEqual(revision, sourceRevision)
                _ = try sourcePort.setConversationPermission(.init(conversationID: source.conversationID,
                    expectedRevision: sourceRevision, expectedRunID: source.runID, expectedSourceHostID: nil,
                    profile: "allow_all", updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))).committedSnapshot()
                saved = true
            }
            return .init(conversation_id: source.conversationID, configuration_revision: sourceRevision + (saved ? 1 : 0),
                permission_profile: saved ? .allowAll : .cautious, active_run_id: source.runID,
                active_policy: .init(runID: source.runID, configurationRevision: sourceRevision + (saved ? 1 : 0),
                    profile: saved ? .allowAll : .cautious), owner: .init(kind: .local, run_id: source.runID, assistant_message_id: source.assistantMessageID, identity: nil), state: .active)
        }
        model.draft = "Retained after mirror conflict"
        model.permissionMode = .allowAll
        do { try await model.waitForModelConfigurationCommit(); XCTFail("A stale mirror CAS must fail") }
        catch {}
        XCTAssertTrue(saved)
        XCTAssertEqual(model.selectedPermissionConfigurationState, .failed)
        XCTAssertEqual(model.selectedPermissionActiveMode, .allowAll)
        XCTAssertTrue(model.reloadComposerPermissionConfiguration(conversationID: source.conversationID))
        do { try await model.waitForModelConfigurationCommit(); XCTFail("The uncommitted permission draft must remain blocked") }
        catch {}
        XCTAssertEqual(model.selectedPermissionConfigurationState, .failed, "Read reconciliation cannot claim the divergent mirror is saved")
        XCTAssertEqual(model.selectedPermissionActiveMode, .allowAll)
        XCTAssertEqual(model.draft, "Retained after mirror conflict")
        XCTAssertEqual(host.permissionConfigurationRequests.filter { $0.2 != nil }.count, 1)
    }

    func testUnconfirmedSourceOwnerRejectsPolicyEditWithoutMirrorWrite() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let (original, _, port, source) = try await coldPermissionFixture(root: root)
        original.presentationDidDetach()
        original.selectedConversationID = nil
        let host = PermissionEnvelopeRemoteHost(capturedHostID: "different-host")
        let model = MacWorkspaceModel(initialConversationID: source.conversationID,
            canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        let before = try port.loadConversationMetadata(source.conversationID)
        model.draft = "Keep this draft"
        model.permissionMode = .allowAll
        XCTAssertEqual(model.selectedPermissionConfigurationState, .failed)
        XCTAssertTrue(model.selectedPermissionHasUncommittedChanges)
        XCTAssertEqual(model.draft, "Keep this draft")
        XCTAssertTrue(host.permissionConfigurationRequests.isEmpty)
        XCTAssertEqual(try port.loadConversationMetadata(source.conversationID)?.configurationRevision, before?.configurationRevision)
        XCTAssertEqual(try port.loadConversationMetadata(source.conversationID)?.settingsJSON, before?.settingsJSON)
        XCTAssertFalse(model.reloadComposerPermissionConfiguration(conversationID: source.conversationID))
    }

    func testSourcePermissionReceiptRejectsAnotherRunAndFuturePolicyRevision() throws {
        let identity = MacRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        let decoder = JSONDecoder()
        let pending = try decoder.decode(MacRemotePermissionConfiguration.self, from: Data(
            #"{"conversation_id":"conversation","configuration_revision":7,"permission_profile":"allow_all","owner":{"kind":"local","run_id":"run","assistant_message_id":"assistant"},"state":"pending","active_run_id":"run","active_policy":{"run_id":"run","configuration_revision":6,"profile":"cautious"}}"#.utf8))
        XCTAssertNoThrow(try pending.validate(identity: identity))
        XCTAssertEqual(pending.permission_profile, .allowAll)
        XCTAssertEqual(pending.active_policy?.profile, .cautious, "Saving a profile is not an ACK")
        let anotherRun = try decoder.decode(MacRemotePermissionConfiguration.self, from: Data(
            #"{"conversation_id":"conversation","configuration_revision":7,"permission_profile":"allow_all","owner":{"kind":"local","run_id":"run","assistant_message_id":"assistant"},"state":"pending","active_run_id":"other","active_policy":null}"#.utf8))
        XCTAssertThrowsError(try anotherRun.validate(identity: identity))
        let futurePolicy = try decoder.decode(MacRemotePermissionConfiguration.self, from: Data(
            #"{"conversation_id":"conversation","configuration_revision":7,"permission_profile":"allow_all","owner":{"kind":"local","run_id":"run","assistant_message_id":"assistant"},"state":"pending","active_run_id":"run","active_policy":{"run_id":"run","configuration_revision":8,"profile":"allow_all"}}"#.utf8))
        XCTAssertThrowsError(try futurePolicy.validate(identity: identity))
    }
    private var journalFixtures: [String: MacAssistantJournalTestFixture] = [:]
    func testBackgroundPermissionReadMatchesCanonicalReadAndHonorsCancellation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let source = try MacAssistantJournalTestFixture(conversationID: "permission-read", runID: "permission-run",
            assistantMessageID: "permission-assistant")
        try source.ingestAdmissions(into: port)
        let repository = MacCanonicalConversationRepository(store: port)
        let expected = try repository.permissionProjection(conversationID: source.conversationID)
        let actual = try await repository.permissionProjectionAsync(conversationID: source.conversationID)
        XCTAssertEqual(actual, expected)

        let cancelled = Task { @MainActor in
            try await repository.permissionProjectionAsync(conversationID: source.conversationID)
        }
        cancelled.cancel()
        do {
            _ = try await cancelled.value
            XCTFail("Cancelled permission hydration must not publish a projection")
        } catch is CancellationError {
            // Cancellation abandons presentation, never changes authorization.
        }
        XCTAssertEqual(try repository.permissionProjection(conversationID: source.conversationID), expected)
    }

    func testColdAdmittedRunWithoutJournalObservesPermissionThenSealedBodyWithoutExecution() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let source = try MacAssistantJournalTestFixture(conversationID: "cold-observation", runID: "cold-run",
            assistantMessageID: "cold-assistant")
        try source.ingestAdmissions(into: port)
        let admission = try JSONDecoder().decode(MacRunMessageAdmissionCursor.self, from: source.source.execute(
            commandData: JSONSerialization.data(withJSONObject: ["type":"local_run_message_admission_head",
                "conversation_id":source.conversationID, "run_id":source.runID])))
        let request = MacRemotePermissionRequest(id: "cold-request", allowConversation: true, protocolVersion: 1, nonce: "original-live-nonce",
            hostID: "fixture-host", clientID: "fixture-client", sessionID: source.conversationID,
            toolName: "web_search", arguments: "", callID: "call", reason: "Review original operation",
            message: "Confirm", expiresAt: Int64(Date().timeIntervalSince1970) + 300)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        var finish = false
        var terminalDelivered = false
        host.observation = { conversation, run, assistant in
            XCTAssertEqual(conversation, source.conversationID)
            if terminalDelivered && run == nil { return .init(observation: nil, events: []) }
            XCTAssertEqual(run, source.runID)
            XCTAssertEqual(assistant, source.assistantMessageID)
            var seal: MacAssistantRunTerminalSeal?
            var events: [MacLocalAgentEvent] = []
            if finish {
                _ = try source.finish("Recovered final body")
                seal = try source.commitTerminal(into: port)
                terminalDelivered = true
                events = try source.heads().map { cursor in
                    MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil, isError: false,
                        assistantJournalCursor: cursor).bindingCanonicalRun(source.runID)
                }
            } else {
                events = [.init(kind: .status, text: "", title: nil, detail: nil, isError: false,
                    permissionRequest: request)]
            }
            let observation = MacRemoteRunObservation(protocol_version: 1, conversation_id: conversation,
                client_id: "fixture-client", run_id: source.runID, assistant_message_id: source.assistantMessageID,
                status: seal == nil ? "running" : "completed",
                worker: .init(state: seal == nil ? .running : .absent, generation: seal == nil ? "7" : nil),
                recovery: seal == nil ? .observe : .terminal, admission: admission,
                terminal_seal: seal, permissions: [], can_submit: seal != nil)
            return .init(observation: observation, events: events)
        }
        let model = MacWorkspaceModel(initialConversationID: source.conversationID,
            canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach(); model.selectedConversationID = nil }
        model.selectedConversationID = source.conversationID
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.presentationDidAttach()
        model.recheckRemoteRun()
        let pending = try await waitForValue { model.selectedPendingRemotePermission }
        XCTAssertEqual(pending.nonce, request.nonce)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .waitingApproval)
        model.newConversation()
        XCTAssertNil(model.selectedPendingRemotePermission)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .ready)
        XCTAssertEqual(model.pendingRemotePermission, pending, "Switching drafts must not discard another conversation's authorization request")
        model.selectedConversationID = source.conversationID
        XCTAssertEqual(model.selectedPendingRemotePermission, pending)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .waitingApproval)
        XCTAssertFalse(model.canRecoverRemoteRun, "A live worker only permits observation, not resume")
        model.remoteObservationForegroundChanged(false)
        let reads = host.observationRequests.count
        for _ in 0..<3 { await Task.yield() }
        XCTAssertEqual(host.observationRequests.count, reads)
        finish = true
        model.remoteObservationForegroundChanged(true)
        _ = try await waitForValue { terminalDelivered && model.remoteRunRecovery == nil ? true : nil }
        XCTAssertNil(model.selectedPendingRemotePermission)
        XCTAssertNil(try port.activeRunIdentity(conversationID: source.conversationID))
        XCTAssertEqual(host.runRequests, 0)
        XCTAssertTrue(host.resumeRequests.isEmpty)
        let archive = try XCTUnwrap(MacCanonicalConversationRepository(store: port).load(conversationID: source.conversationID))
        let reference = try XCTUnwrap(archive.messages.last(where: { $0.role == .assistant })?.journalReference)
        let body = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
        XCTAssertEqual(body, "Recovered final body")
    }
    func testExplicitRemoteSelectionWaitsForSourceACKAndRetainsChoiceOnFailure() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("receiver.db"))
        let host = PermissionEnvelopeRemoteHost()
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        defer { model.selectedConversationID = nil }
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        let option = try XCTUnwrap(model.configuredModelOptions.first)
        let original = model.selectedConversation?.modelSelectionID
        host.selectionFailure = true
        model.selectModel(option)
        do { try await model.waitForModelConfigurationCommit(); XCTFail("A missing source ACK cannot report selection saved") } catch {}
        XCTAssertEqual(model.selectedConversation?.modelSelectionID, original)
        XCTAssertEqual(host.selectionRequests.first?.0, conversation)
        XCTAssertEqual(host.selectionRequests.first?.1, host.fixtureProfile.profileID)
        host.selectionFailure = false
        model.selectModel(option)
        try await model.waitForModelConfigurationCommit()
        XCTAssertEqual(model.selectedConversation?.modelSelectionID, option.id)
        XCTAssertEqual(host.selectionRequests.count, 2)
    }

    func testLocalTaskCancellationUsesCoreOwnershipWithoutRunningHostProcess() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("source.db"))
        // Reach the current Application command directly: neither a process nor
        // today's selected Run may substitute for an immutable task owner.
        XCTAssertThrowsError(try port.requestOwnedTaskCancellation(conversationID: "conversation",
            parentRunID: " parent ", taskID: "task")) { error in
                XCTAssertTrue(error.localizedDescription.contains("exact bounded ownership"), error.localizedDescription)
            }
        XCTAssertThrowsError(try port.requestOwnedTaskCancellation(conversationID: "conversation",
            parentRunID: "parent", taskID: "missing-task")) { error in
                XCTAssertTrue(error.localizedDescription.contains("target is unavailable"), error.localizedDescription)
            }
        let cold = try MacApplicationPort(databaseURL: port.databaseURL)
        XCTAssertThrowsError(try cold.requestOwnedTaskCancellation(conversationID: "conversation",
            parentRunID: "successor-run", taskID: "missing-task"))
    }

    func testColdGoalReadsCommittedSourceAfterSelectionChanges() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let receiver = try MacApplicationPort(databaseURL: root.appendingPathComponent("receiver.db"))
        let source = try MacAssistantJournalTestFixture(conversationID: "goal-source-\(UUID())",
            runID: "completed-parent", assistantMessageID: "completed-assistant")
        try source.commitTerminal(into: receiver)
        XCTAssertNil(try receiver.remoteSubmissionIntent(conversationID: source.conversationID))
        let cold = try MacApplicationPort(databaseURL: receiver.databaseURL)
        let host = PermissionEnvelopeRemoteHost()
        let model = MacWorkspaceModel(initialConversationID: source.conversationID,
            canonicalRepository: MacCanonicalConversationRepository(store: cold),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        defer { model.selectedConversationID = nil }
        model.executionHost = .local
        XCTAssertEqual(try model.goalTransportHostID(conversationID: source.conversationID), "fixture-host")
        _ = try await model.inspectorGoalSnapshot(conversationID: source.conversationID)
        XCTAssertEqual(host.goalProgressTargets.count, 1)
        XCTAssertFalse(try XCTUnwrap(host.goalProgressTargets.first?.expectedSourceStoreEpoch).isEmpty)
        let sourcePlan = try await model.inspectorConversationPlan(owner: source.conversationID)
        XCTAssertEqual(sourcePlan?.revision, 7, "The plan must come from the same source, not the empty mirror")

        let foreign = PermissionEnvelopeRemoteHost(capturedHostID: "another-host")
        let reopened = MacWorkspaceModel(initialConversationID: source.conversationID,
            canonicalRepository: MacCanonicalConversationRepository(store: cold),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: foreign)
        defer { reopened.selectedConversationID = nil }
        reopened.executionHost = .remote
        XCTAssertThrowsError(try reopened.goalTransportHostID(conversationID: source.conversationID))
        do {
            _ = try await reopened.inspectorGoalSnapshot(conversationID: source.conversationID)
            XCTFail("A different Host cannot observe the original Goal")
        } catch {}
        XCTAssertTrue(foreign.goalProgressTargets.isEmpty)
    }

    func testTaskControlsCaptureLocationAndRejectStaleConversationCards() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.probesTaskControls = true
        let source = try MacAssistantJournalTestFixture(conversationID: "task-controls-\(UUID())",
            runID: "parent-run", assistantMessageID: "parent-assistant")
        try source.commitTerminal(into: port)
        let model = MacWorkspaceModel(initialConversationID: source.conversationID,
            canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.selectedConversationID = nil }
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        model.sendTaskGuidance(conversationID: conversation, parentRunID: "parent-run", taskID: "guidance-target", message: "Continue")
        model.requestTaskCancellation(conversationID: conversation, parentRunID: "parent-run", taskID: "cancel-target")
        // Neither MainActor Task has run yet. A subsequent UI change must not
        // retarget the already submitted controls to the local Host.
        model.executionHost = .local
        _ = try await waitForValue { host.taskControlTargets.count == 2 ? true : nil }
        XCTAssertTrue(host.taskControlTargets.allSatisfy { $0.0 == conversation })
        XCTAssertEqual(Set(host.taskControlTargets.map { $0.1 }), Set(["guidance-target", "cancel-target"]))
        model.executionHost = .remote
        model.sendTaskGuidance(conversationID: "obsolete-conversation", parentRunID: "parent-run", taskID: "foreign", message: "No")
        model.requestTaskCancellation(conversationID: "obsolete-conversation", parentRunID: "parent-run", taskID: "foreign")
        for _ in 0..<3 { await Task.yield() }
        XCTAssertEqual(host.taskControlTargets.count, 2)
    }

    func testNewSubmissionRetiresPreviousPermissionRevisionFloor() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.selectedConversationID = nil }
        let conversation = try XCTUnwrap(model.selectedConversationID)
        let payload: [String: Any] = ["type": "core_permission_projection_updated", "session_id": conversation,
            "permission_projection_revision": 100, "permission_projection": ["revision": 100, "pending": []]]
        let data = try JSONSerialization.data(withJSONObject: payload)
        let previous = try XCTUnwrap(MacAgentHostRuntime.decode(line: String(decoding: data, as: UTF8.self),
            conversationID: conversation, coreSessionID: conversation))
        XCTAssertFalse(previous.isError)
        await consume(previous, model: model, conversation: conversation)
        XCTAssertEqual(model.permissionProjectionsByConversation[conversation]?.revision, 100)
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        model.draft = "Start a new permission revision scope"
        model.send()
        let continuation = try await waitForValue { host.runContinuation }
        XCTAssertNil(model.permissionProjectionsByConversation[conversation],
            "a new Run must not inherit the predecessor's revision floor or pending requests")
        continuation.finish(throwing: MacRemoteHostError.transportInterrupted)
        _ = try await waitForValue { !model.isRunning ? true : nil }
    }

    func testInitialRemoteEOFDoesNotCompleteAnswerOrPermitDuplicateSubmission() async throws {
        for failsTransport in [false, true] {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
            let host = PermissionEnvelopeRemoteHost()
            host.applicationPort = port
            host.allowsRun = true
            let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
                localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
            defer { model.selectedConversationID = nil }
            let conversation = try XCTUnwrap(model.selectedConversationID)
            model.executionHost = .remote
            model.updateRemoteHostConfigurationState(true)
            model.setFixtureRemoteModels([host.fixtureProfile])
            model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
            let terminalRevision = model.selectedTerminalTranscriptRevision
            model.draft = "Original input"
            model.send()
            let continuation = try await waitForValue { host.runContinuation }
            if failsTransport { continuation.finish(throwing: MacRemoteHostError.transportInterrupted) }
            else { continuation.finish() }
            _ = try await waitForValue { !model.isRunning ? true : nil }
            XCTAssertEqual(model.selectedTerminalTranscriptRevision, terminalRevision)
            XCTAssertNotNil(model.remoteRunRecovery)
            XCTAssertEqual(model.remoteRunRecovery?.conversationID, conversation)
            XCTAssertNotNil(model.remoteRecoveryError)
            let assistant = try XCTUnwrap(model.selectedMessages.last(where: { $0.role == .assistant }))
            XCTAssertNotEqual(assistant.presentation?.isComplete, true)
            XCTAssertFalse(assistant.text.contains("Connected host error:"), "Transport failure is not model-authored content")
            let messageCount = model.selectedMessages.count
            model.draft = "Do not duplicate the unresolved Run"
            model.send()
            try await Task.sleep(for: .milliseconds(50))
            XCTAssertEqual(host.runRequests, 1)
            XCTAssertEqual(model.selectedMessages.count, messageCount)
        }
    }
    func testAcceptedCancellationAfterEOFOnlyObservesAndPreservesAuthorityUntilCommit() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.selectedConversationID = nil }
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        model.draft = "Run requiring confirmation"
        model.send()
        let continuation = try await waitForValue { host.runContinuation }
        let initialRunID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        continuation.yield(try event("pending-ask", sequence: 4, conversation: conversation,
            nonce: "retained-authority", canonicalRunID: initialRunID))
        _ = try await waitForValue { model.pendingRemotePermission }
        model.stop()
        _ = try await waitForValue { host.cancellationRequests.count == 1 ? true : nil }
        continuation.finish()
        let recovery = try await waitForValue { !model.isRunning ? model.remoteRunRecovery : nil }
        let runID = try XCTUnwrap(recovery.expectedRunID)
        XCTAssertTrue(model.remoteRecoveryIsCancellationPending)
        XCTAssertTrue(model.canRecoverRemoteRun)
        let revision = model.selectedTerminalTranscriptRevision
        let messageIDs = model.selectedMessages.map(\.id)
        // Another Run's committed journal cannot substitute for the exact
        // pending Run. An unavailable observation must never authorize resume.
        _ = try committedJournalEvent(receiver: port, conversation: conversation,
            assistantCanonicalID: "\(runID):assistant", runID: "stale-run", submissionID: "stale-run")
        host.observation = { _, _, _ in throw MacRemoteHostError.invalidResponse }
        await model.resumeRemoteRun()
        XCTAssertTrue(host.resumeRequests.isEmpty, "Checking accepted cancellation must never POST resume")
        XCTAssertEqual(host.cancellationRequests.count, 1)
        XCTAssertTrue(model.remoteRecoveryIsCancellationPending)
        XCTAssertEqual(model.remoteRunRecovery?.generationID, recovery.generationID)
        XCTAssertEqual(model.pendingRemotePermission?.nonce, "retained-authority")
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, revision)
        XCTAssertEqual(model.selectedMessages.map(\.id), messageIDs)

        _ = try committedJournalEvent(receiver: port, conversation: conversation,
            assistantCanonicalID: "\(runID):assistant", runID: runID, submissionID: runID)
        await model.resumeRemoteRun()
        XCTAssertTrue(host.resumeRequests.isEmpty)
        XCTAssertEqual(host.cancellationRequests.count, 1)
        XCTAssertNil(model.remoteRunRecovery)
        XCTAssertNil(model.pendingRemotePermission)
        XCTAssertFalse(model.remoteRecoveryIsCancellationPending)
        XCTAssertNil(try port.activeRunIdentity(conversationID: conversation))
    }

    func testAcceptedCancellationWithoutTraceAllowsReadOnlyRecheck() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.selectedConversationID = nil }
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        model.draft = "Cancel before the first trace"
        model.send()
        let continuation = try await waitForValue { host.runContinuation }
        model.stop()
        _ = try await waitForValue { host.cancellationRequests.count == 1 ? true : nil }
        continuation.finish()
        _ = try await waitForValue { !model.isRunning ? model.remoteRunRecovery : nil }
        XCTAssertTrue(model.remoteRecoveryIsCancellationPending)
        XCTAssertNil(model.remoteRunRecovery?.observedTraceID)
        XCTAssertTrue(model.canRecoverRemoteRun)
        await model.resumeRemoteRun()
        XCTAssertTrue(host.resumeRequests.isEmpty)
        XCTAssertEqual(host.cancellationRequests.count, 1)
        XCTAssertNotNil(model.remoteRunRecovery)
    }

    func testCommittedTerminalSubscriptionAfterEOFUnblocksNextSubmission() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        host.allowsEmptyUIQueue = true
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.selectedConversationID = nil }
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        model.draft = "Original unresolved Run"
        model.send()
        let continuation = try await waitForValue { host.runContinuation }
        continuation.finish()
        let recovery = try await waitForValue { !model.isRunning ? model.remoteRunRecovery : nil }
        let runID = try XCTUnwrap(recovery.expectedRunID)
        let revision = model.selectedTerminalTranscriptRevision
        let terminal = try committedJournalEvent(receiver: port, conversation: conversation,
            assistantCanonicalID: "\(runID):assistant", runID: runID, submissionID: runID)
        // Commit through the repository, not the stream consumer: the running
        // projection subscription must discover the exact durable Run receipt.
        XCTAssertNotNil(terminal.assistantRunTerminalSealDigest)
        _ = try await waitForValue {
            model.remoteRunRecovery == nil && model.selectedTerminalTranscriptRevision > revision ? true : nil
        }
        XCTAssertGreaterThan(model.selectedTerminalTranscriptRevision, revision)
        XCTAssertFalse(model.isRunning)
        XCTAssertNil(model.remoteRecoveryError)
        // Terminal finalization replaces the active observer with an idle one.
        // The old confirmed Run must not make that replacement exit on its
        // first snapshot and silently stop observing later canonical changes.
        for title in ["First idle canonical update", "Second idle canonical update"] {
            try repository.rename(conversationID: conversation, title: title)
            _ = try await waitForValue {
                model.conversations.first(where: { $0.id == conversation })?.title == title ? true : nil
            }
        }
        model.draft = "A subsequent independent request"
        model.send()
        _ = try await waitForValue { host.runRequests == 2 ? true : nil }
        let nextRun = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        XCTAssertNotEqual(nextRun, runID)
        // A delayed completion of the old sealed owner must neither retire
        // the new owner nor reopen its submission gate.
        var oldResponse = ""
        let oldAssistant = try XCTUnwrap(MacCanonicalMessageIdentity.stableID("\(runID):assistant"))
        var oldAssembler = MacSemanticStreamAssembler(identitySeed: oldAssistant.uuidString)
        let oldFinish = await model.finishAssistantStream(response: &oldResponse,
            assembler: &oldAssembler, conversationID: conversation, messageID: oldAssistant)
        XCTAssertEqual(oldFinish, .stale)
        XCTAssertTrue(model.isRunning)
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, nextRun)
        model.draft = "Must not replace the new active owner"
        model.send()
        XCTAssertEqual(host.runRequests, 2)
        let nextSource = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
            runID: nextRun, assistantMessageID: "\(nextRun):assistant")
        let nextSeal = try nextSource.finish("The subsequent request completed.")
        try nextSource.commitTerminal(into: port)
        host.runContinuation?.yield(nextSource.terminalEvent(seal: nextSeal))
        host.runContinuation?.finish()
        _ = try await waitForValue { !model.isRunning ? true : nil }
        // The new Run is now finalized, but a fresh remote observation has
        // unknown transport state. Neither an old message's stale finish nor
        // an already-completed owner's duplicate finish may retire this gate.
        model.recheckRemoteRun()
        _ = try await waitForValue { model.remoteObservationNeedsAttention ? true : nil }
        let staleAfterFinalization = await model.finishAssistantStream(response: &oldResponse,
            assembler: &oldAssembler, conversationID: conversation, messageID: oldAssistant)
        XCTAssertEqual(staleAfterFinalization, .stale)
        XCTAssertTrue(model.remoteObservationNeedsAttention)
        let nextAssistant = try XCTUnwrap(MacCanonicalMessageIdentity.stableID("\(nextRun):assistant"))
        var nextResponse = ""
        var nextAssembler = MacSemanticStreamAssembler(identitySeed: nextAssistant.uuidString)
        let duplicateFinish = await model.finishAssistantStream(response: &nextResponse,
            assembler: &nextAssembler, conversationID: conversation, messageID: nextAssistant)
        XCTAssertEqual(duplicateFinish, .alreadyCompleted)
        XCTAssertTrue(model.remoteObservationNeedsAttention)
        XCTAssertNil(try port.activeRunIdentity(conversationID: conversation))
        model.draft = "Observation must settle before another submission"
        model.send()
        XCTAssertEqual(host.runRequests, 2)
    }

    func testStreamAndSubscriptionTerminalRaceKeepsRunBusyUntilPresentationFinishes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        host.allowsEmptyUIQueue = true
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.selectedConversationID = nil }
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        model.draft = "Complete through the stream and subscription together"
        model.send()
        let continuation = try await waitForValue { host.runContinuation }
        let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        let revision = model.selectedTerminalTranscriptRevision
        let text = (1...24).map { "Paragraph \($0): terminal rendering remains owned until all canonical text is visible." }
            .joined(separator: "\n\n")
        let terminal = try committedJournalEvent(receiver: port, conversation: conversation,
            assistantCanonicalID: "\(runID):assistant", runID: runID, submissionID: runID, assistantText: text)
        // Wake both production callers in the same actor turn. Rendering yields
        // between semantic segments so the losing caller can race the owner.
        XCTAssertNotNil(terminal.assistantRunTerminalSealDigest)
        continuation.yield(terminal)
        continuation.finish()
        let deadline = Date().addingTimeInterval(8)
        while model.selectedTerminalTranscriptRevision == revision && Date() < deadline {
            XCTAssertTrue(model.isRunning, "A losing finish caller cannot publish idle while the owner is rendering")
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, revision + 1, "Only one caller completes presentation")
        _ = try await waitForValue { !model.isRunning ? true : nil }
        let assistant = try XCTUnwrap(model.selectedMessages.last(where: { $0.role == .assistant }))
        let reference = try XCTUnwrap(assistant.journalReference)
        let completedText = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
        XCTAssertEqual(completedText, text)
        XCTAssertNil(model.remoteRunRecovery)
        try repository.rename(conversationID: conversation, title: "Still observing after competing completion")
        _ = try await waitForValue {
            model.conversations.first(where: { $0.id == conversation })?.title == "Still observing after competing completion" ? true : nil
        }
    }

    private func waitForValue<T>(_ value: () -> T?, file: StaticString = #filePath,
                                 line: UInt = #line) async throws -> T {
        let deadline = Date().addingTimeInterval(2)
        while value() == nil && Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        return try XCTUnwrap(value(), "Expected asynchronous transport callback within two seconds", file: file, line: line)
    }

    private func event(_ requestID: String, sequence: UInt64, conversation: String,
                       nonce: String? = nil, traceRequestID: String? = nil,
                       hostID: String = "host", clientID: String = "client", turnID: String = "turn", canonicalRunID: String? = nil,
                       coreSession: String? = nil) throws -> MacLocalAgentEvent {
        var object: [String: Any] = [
            "type": "permission_request", "allow_conversation": true, "id": requestID,
            "protocol_version": MacRemotePermissionRequest.supportedProtocolVersion,
            "nonce": nonce ?? "nonce-\(requestID)", "host_id": hostID, "client_id": clientID,
            "session_id": conversation, "tool_name": "write_file",
            "args": ["path": "/original"],
            "permission_instructions": requestID == "p2" ? [["permission_request_id": "p1", "instruction": "Preview first."]] : [],
            "trace": ["schema_version": 1, "trace_id": "trace", "run_id": "run",
                "conversation_id": conversation, "message_id": "message", "turn_id": turnID,
                "event_id": "event-\(sequence)", "sequence": sequence,
                "emitted_at": "2026-09-08T00:00:00Z", "kind": "tool.permission.requested",
                "status": "waiting", "visibility": "summary", "payload": [
                    "permission_request_id": traceRequestID ?? requestID,
                    "call_id": "original-call", "tool_name": "write_file"
                ]]
        ]
        object["canonical_run_id"] = canonicalRunID
        let data = try JSONSerialization.data(withJSONObject: object)
        return try XCTUnwrap(MacAgentHostRuntime.decode(line: String(decoding: data, as: UTF8.self),
                                                      conversationID: conversation, coreSessionID: coreSession ?? conversation,
                                                      controlSessionID: conversation))
    }

    private func lifecycleEvent(_ kind: String, sequence: UInt64, conversation: String,
                                requestID: String = "p2", callID: String = "original-call",
                                runID: String = "run", turnID: String = "turn") throws -> MacLocalAgentEvent {
        let object: [String: Any] = [
            "type": "status", "status": "permission_resolved",
            "trace": ["schema_version": 1, "trace_id": "trace", "run_id": runID,
                "conversation_id": conversation, "message_id": "message", "turn_id": turnID,
                "event_id": "event-\(sequence)", "sequence": sequence,
                "emitted_at": "2026-09-08T00:00:00Z", "kind": kind,
                "status": "succeeded", "visibility": "summary", "payload": [
                    "permission_request_id": requestID, "call_id": callID, "tool_name": "write_file"
                ]]
        ]
        let data = try JSONSerialization.data(withJSONObject: object)
        return try XCTUnwrap(MacAgentHostRuntime.decode(
            line: String(decoding: data, as: UTF8.self), conversationID: conversation))
    }

    private func consume(_ event: MacLocalAgentEvent, model: MacWorkspaceModel,
                         conversation: String, generation: UInt64 = 0, local: Bool = false) async {
        var response = ""
        var assembler = MacSemanticStreamAssembler(identitySeed: "permission-test")
        var succeeded = true
        await model.consumeTracedAgentEvent(event, prompt: "Original prompt", conversationID: conversation,
            modelID: "model", assistantMessageID: UUID(), generationID: generation,
            permissionUsesLocalRuntime: local, permissionAttachmentURLs: [],
            response: &response, assembler: &assembler, succeeded: &succeeded)
        XCTAssertEqual(response, "")
    }

    func testLocalPermissionJoinsEitherArrivalOrderAndCannotResurrectRetiredRequest() async throws {
        for (projectionFirst, distinctCoreSession) in [(true, false), (false, false), (true, true), (false, true)] {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
            let runtime = PersistenceTestLocalRuntime()
            let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
                localAgentRuntime: runtime, remoteHost: PermissionEnvelopeRemoteHost())
            let conversation = try XCTUnwrap(model.selectedConversationID)
            let coreSession = distinctCoreSession ? "actual-core-session-\(UUID())" : conversation
            let challenge = try event("p1", sequence: 1, conversation: conversation, coreSession: coreSession)
            func projection(_ revision: Int, pending: Bool, callID: String = "original-call") throws -> MacLocalAgentEvent {
                let data = try JSONSerialization.data(withJSONObject: [
                    "type": "core_permission_projection_updated", "session_id": coreSession,
                    "permission_projection_revision": revision,
                    "permission_projection": ["revision": revision, "pending": pending ? [[
                        "request_id": "p1", "allow_conversation": true, "turn_id": "turn", "call_id": callID,
                        "capability_id": "write_file", "resource": "/original",
                        "permission_domain": "files_write", "rationale": "test"
                    ]] : []]
                ] as [String: Any])
                return try XCTUnwrap(MacAgentHostRuntime.decode(
                    line: String(decoding: data, as: UTF8.self), conversationID: conversation,
                    coreSessionID: coreSession, controlSessionID: conversation))
            }
            let pending = try projection(1, pending: true)
            if projectionFirst {
                await consume(pending, model: model, conversation: conversation, local: true)
                XCTAssertNil(model.pendingRemotePermission)
            }
            await runtime.setPermissionRequest(challenge.permissionRequest)
            await consume(challenge, model: model, conversation: conversation, local: true)
            if !projectionFirst {
                XCTAssertNil(model.pendingRemotePermission, "Challenge alone is not Core authority")
                await consume(pending, model: model, conversation: conversation, local: true)
            }
            XCTAssertEqual(model.pendingRemotePermission?.id, "p1")
            let acceptedProjection = model.permissionProjectionsByConversation[conversation]
            await consume(try projection(1, pending: false), model: model, conversation: conversation, local: true)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p1",
                "the same permission revision cannot clear an accepted request with different content")
            XCTAssertEqual(model.permissionProjectionsByConversation[conversation], acceptedProjection)
            await consume(challenge, model: model, conversation: conversation, local: true)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p1")
            let foreignCoreChallenge = try event("foreign-core-request", sequence: 2,
                conversation: conversation, coreSession: "foreign-core-session")
            await consume(foreignCoreChallenge, model: model, conversation: conversation, local: true)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p1",
                "A same-generation event cannot replace the immutable Core witness with another Thread")
            await consume(try projection(2, pending: false), model: model, conversation: conversation, local: true)
            XCTAssertNil(model.pendingRemotePermission)
            await consume(try projection(2, pending: true), model: model, conversation: conversation, local: true)
            XCTAssertNil(model.pendingRemotePermission, "same-revision conflict cannot resurrect a retired request")
            await consume(challenge, model: model, conversation: conversation, local: true)
            await consume(pending, model: model, conversation: conversation, local: true)
            XCTAssertNil(model.pendingRemotePermission, "Late challenges and stale snapshots cannot resurrect a retired request")
            await consume(try projection(3, pending: true, callID: "different-call"),
                          model: model, conversation: conversation, local: true)
            XCTAssertNil(model.pendingRemotePermission, "Call identity must match, not merely tool name")
            await consume(pending, model: model, conversation: conversation, generation: 99, local: true)
            await consume(challenge, model: model, conversation: conversation, generation: 99, local: true)
            XCTAssertNil(model.pendingRemotePermission, "Stale generation cannot open a card")
        }
    }

    func testActualConsumerPresentsRemoteSuccessorAndOldHTTPCompletionCannotClearIt() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        let p1 = try event("p1", sequence: 1, conversation: conversation)
        await consume(p1, model: model, conversation: conversation)
        XCTAssertEqual(model.pendingRemotePermission?.id, "p1")
        XCTAssertEqual(model.pendingRemotePermission?.callID, "original-call")
        let sending = Task { try await model.providePermissionInstruction("Preview first.", requestID: "p1") }
        _ = try await waitForValue { host.response }
        await consume(try event("p2", sequence: 2, conversation: conversation), model: model, conversation: conversation)
        XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
        XCTAssertFalse(model.isResolvingPermission)
        XCTAssertEqual(model.pendingRemotePermission?.permissionInstructions.first?.permissionRequestID, "p1")
        // Core snapshots and live Host challenges travel independently. Even a
        // snapshot without this nonce-backed request must not clear its card.
        let projectionData = try JSONSerialization.data(withJSONObject: [
            "type": "core_permission_projection_updated", "session_id": conversation,
            "permission_projection_revision": 1,
            "permission_projection": ["revision": 1, "pending": []]
        ] as [String: Any])
        let projectionEvent = try XCTUnwrap(MacAgentHostRuntime.decode(
            line: String(decoding: projectionData, as: UTF8.self), conversationID: conversation,
            coreSessionID: conversation, controlSessionID: conversation))
        await consume(projectionEvent, model: model, conversation: conversation)
        XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
        host.response?.resume()
        try await sending.value
        XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
        await consume(p1, model: model, conversation: conversation)
        XCTAssertEqual(model.pendingRemotePermission?.id, "p2", "Old replay must not replace successor")
        do {
            try await model.providePermissionInstruction("late p1 input", requestID: "p1")
            XCTFail("Old callback must not submit guidance for p2")
        } catch {}
        XCTAssertEqual(host.sentPayloads.count, 1)
        XCTAssertEqual(host.sentPayloads[0]["request_id"] as? String, "p1")
        let resolution = try XCTUnwrap(host.sentPayloads[0]["resolution"] as? [String: Any])
        XCTAssertEqual(resolution["type"] as? String, "intervention")
        XCTAssertNil(resolution["decision"])
    }

    func testFailedStopDoesNotUnlockAnInFlightPermissionResponse() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        host.failsCancellation = true
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.selectedConversationID = nil }
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        model.draft = "Keep the original permission transport owned"
        model.send()
        let continuation = try await waitForValue { host.runContinuation }
        let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        continuation.yield(try event("in-flight", sequence: 1, conversation: conversation, canonicalRunID: runID))
        let pending = try await waitForValue { model.pendingRemotePermission }
        let sending = Task {
            try await model.providePermissionInstruction("Preview first.", requestID: pending.id, expectedRequest: pending)
        }
        let response = try await waitForValue { host.response }
        XCTAssertTrue(model.isResolvingPermission)
        model.stop()
        _ = try await waitForValue {
            host.cancellationRequests.count == 1 && !model.remoteRecoveryIsCancellationPending ? true : nil
        }
        XCTAssertEqual(model.pendingRemotePermission, pending)
        XCTAssertTrue(model.isResolvingPermission, "Stop does not own the pending HTTP request's lock")
        model.resolvePermission(allow: true, requestID: pending.id, expectedRequest: pending)
        await Task.yield()
        XCTAssertEqual(host.sentPayloads.count, 1)
        XCTAssertEqual(host.permissionDecisionRequests, 0)
        XCTAssertTrue(model.isResolvingPermission)
        response.resume()
        try await sending.value
        XCTAssertFalse(model.isResolvingPermission)
        XCTAssertNil(model.pendingRemotePermission)
        continuation.finish()
        _ = try await waitForValue { !model.isRunning ? true : nil }
    }

    func testReplayedIdenticalAskDoesNotUnlockAnInFlightResponse() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        await consume(try event("p1", sequence: 1, conversation: conversation),
            model: model, conversation: conversation)
        let pending = try XCTUnwrap(model.pendingRemotePermission)
        let sending = Task {
            try await model.providePermissionInstruction(
                "Keep original input", requestID: pending.id, expectedRequest: pending
            )
        }
        _ = try await waitForValue { host.response }
        XCTAssertTrue(model.isResolvingPermission)

        await consume(try event("p1", sequence: 2, conversation: conversation),
            model: model, conversation: conversation)
        XCTAssertEqual(model.pendingRemotePermission, pending)
        XCTAssertTrue(model.isResolvingPermission)
        XCTAssertEqual(host.sentPayloads.count, 1)

        host.response?.resume()
        try await sending.value
    }

    func testActualConsumerRejectsWrongSessionGenerationNonceAndRequestBinding() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        let conversation = try XCTUnwrap(model.selectedConversationID)
        await consume(try event("p1", sequence: 1, conversation: "foreign"), model: model, conversation: conversation)
        await consume(try event("p1", sequence: 2, conversation: conversation), model: model, conversation: conversation, generation: 99)
        await consume(try event("p1", sequence: 3, conversation: conversation, nonce: ""), model: model, conversation: conversation)
        await consume(try event("p1", sequence: 4, conversation: conversation, traceRequestID: "other"), model: model, conversation: conversation)
        XCTAssertNil(model.pendingRemotePermission)
        // Local raw envelopes continue to be admitted only through Core's pending projection.
        await consume(try event("local", sequence: 5, conversation: conversation), model: model, conversation: conversation, local: true)
        XCTAssertNil(model.pendingRemotePermission)
    }
    func testActualConsumerOnlyResolvesTheMatchingCanonicalRequest() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        await consume(try event("p1", sequence: 1, conversation: conversation), model: model, conversation: conversation)
        let p2 = try event("p2", sequence: 2, conversation: conversation)
        await consume(p2, model: model, conversation: conversation)
        for rejected in [
            try lifecycleEvent("tool.permission.resolved", sequence: 3, conversation: conversation, requestID: "p1"),
            try lifecycleEvent("tool.permission.resolved", sequence: 4, conversation: conversation, callID: "other-call"),
            try lifecycleEvent("tool.permission.resolved", sequence: 5, conversation: conversation, runID: "other-run"),
            try lifecycleEvent("tool.permission.resolved", sequence: 6, conversation: conversation, turnID: "other-turn")
        ] {
            await consume(rejected, model: model, conversation: conversation)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
        }
        let resolved = try lifecycleEvent("tool.permission.resolved", sequence: 7, conversation: conversation)
        await consume(resolved, model: model, conversation: conversation, generation: 99)
        XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
        await consume(resolved, model: model, conversation: "foreign")
        XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
        await consume(resolved, model: model, conversation: conversation)
        XCTAssertNil(model.pendingRemotePermission)
        await consume(p2, model: model, conversation: conversation)
        XCTAssertNil(model.pendingRemotePermission, "A replay cannot resurrect a resolved challenge")
        XCTAssertTrue(host.sentPayloads.isEmpty)
    }

    func testTraceTerminalClaimsCannotRetirePermissionWithoutCommittedRunProjection() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        for (index, kind) in ["run.completed", "run.cancelled", "run.failed"].enumerated() {
            let sequence = UInt64(index * 10 + 1)
            let request = try event("p2", sequence: sequence, conversation: conversation)
            await consume(request, model: model, conversation: conversation)
            let transportFailure = MacLocalAgentEvent(kind: .failed, text: "Connection lost",
                title: nil, detail: nil, isError: true)
            await consume(transportFailure, model: model, conversation: conversation)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p2", "Transport loss is not a canonical terminal")
            await consume(try lifecycleEvent(kind, sequence: sequence + 1, conversation: conversation,
                runID: "other-run"), model: model, conversation: conversation)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
            await consume(try lifecycleEvent(kind, sequence: sequence + 2, conversation: conversation,
                turnID: "other-turn"), model: model, conversation: conversation)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
            await consume(try lifecycleEvent(kind, sequence: sequence + 3, conversation: conversation),
                model: model, conversation: conversation)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p2",
                "Even a matching trace is not a committed canonical Run terminal")
            await consume(request, model: model, conversation: conversation)
            XCTAssertEqual(model.pendingRemotePermission?.id, "p2")
        }
        XCTAssertTrue(host.sentPayloads.isEmpty)
    }

    func testSameAskReissuedNonceSurvivesOldHTTPSuccessAndFailure() async throws {
        for failsResponse in [false, true] {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
            let host = PermissionEnvelopeRemoteHost()
            host.applicationPort = port
            host.failsResponse = failsResponse
            let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
                localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
            let conversation = try XCTUnwrap(model.selectedConversationID)
            await consume(try event("same-ask", sequence: 1, conversation: conversation, nonce: "old-nonce"),
                model: model, conversation: conversation)
            let old = try XCTUnwrap(model.pendingRemotePermission)
            let sending = Task {
                try await model.providePermissionInstruction("Keep original input", requestID: old.id, expectedRequest: old)
            }
            _ = try await waitForValue { host.response }
            await consume(try event("same-ask", sequence: 2, conversation: conversation, nonce: "new-nonce"),
                model: model, conversation: conversation)
            let renewed = try XCTUnwrap(model.pendingRemotePermission)
            XCTAssertEqual(renewed.nonce, "new-nonce")
            host.response?.resume()
            do {
                try await sending.value
                XCTAssertFalse(failsResponse)
            } catch {
                XCTAssertTrue(failsResponse)
            }
            XCTAssertEqual(model.pendingRemotePermission, renewed)
            XCTAssertFalse(model.isResolvingPermission)
            do {
                try await model.providePermissionInstruction("stale editor", requestID: old.id, expectedRequest: old)
                XCTFail("Old nonce callback must not submit the renewed challenge")
            } catch {}
            model.resolvePermission(allow: false, requestID: old.id, expectedRequest: old)
            XCTAssertEqual(model.pendingRemotePermission, renewed)
            XCTAssertEqual(host.sentPayloads.count, 1)
            XCTAssertEqual(host.sentPayloads[0]["nonce"] as? String, "old-nonce")
        }
    }

    func testSameAskReissueRejectsDifferentHostOrClientAndOlderSequence() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        let conversation = try XCTUnwrap(model.selectedConversationID)
        await consume(try event("same-ask", sequence: 2, conversation: conversation, nonce: "current"),
            model: model, conversation: conversation)
        let current = try XCTUnwrap(model.pendingRemotePermission)
        for rejected in [
            try event("same-ask", sequence: 3, conversation: conversation, nonce: "foreign", hostID: "other-host"),
            try event("same-ask", sequence: 4, conversation: conversation, nonce: "foreign", clientID: "other-client"),
            try event("same-ask", sequence: 1, conversation: conversation, nonce: "old")
        ] {
            await consume(rejected, model: model, conversation: conversation)
            XCTAssertEqual(model.pendingRemotePermission, current)
        }
    }

    func testResumePayloadContainsOnlyCanonicalIdentityAndPairedAuthority() throws {
        let payload = try MacRemoteResumeRequest(conversationID: "conversation", expectedRunID: "canonical-run",
            expectedHostID: "paired-host", clientID: "paired-client").payload()
        XCTAssertEqual(Set(payload.keys), ["conversation_id", "expected_run_id", "expected_host_id", "client_id"])
        XCTAssertEqual(payload["expected_run_id"] as? String, "canonical-run")
        XCTAssertThrowsError(try MacRemoteResumeRequest(conversationID: "conversation", expectedRunID: "",
            expectedHostID: "paired-host", clientID: "paired-client").payload())
    }

    func testUserResumeReadsCanonicalRunAndConsumesRenewedChallengeWithoutNewMessage() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsEmptyUIQueue = true
        let conversation = UUID().uuidString
        let canonicalRunID = "canonical-run-distinct-from-trace"
        let assistantCanonicalID = "\(canonicalRunID):assistant"
        let assistantID = try XCTUnwrap(MacCanonicalMessageIdentity.stableID(assistantCanonicalID))
        _ = try port.prepareFixtureRemoteSubmission(MacPrepareRemoteSubmissionInput(
            conversationID: conversation, conversationTitle: "Pending remote run",
            submissionID: canonicalRunID, sourceHostID: "fixture-host", clientID: "fixture-client",
            userMessageID: UUID().uuidString, userPartID: UUID().uuidString,
            userText: "Original input", userContentJSON: nil, assistantMessageID: assistantCanonicalID,
            assistantPartID: "\(assistantCanonicalID):text", contextCheckpointID: nil,
            createdAtMs: Int64(Date().timeIntervalSince1970 * 1_000)))
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
            runID: canonicalRunID, assistantMessageID: assistantCanonicalID)
        journalFixtures[canonicalRunID] = source
        let admission = try XCTUnwrap(try source.submissionReceipt().admission)
        let sourcePort = try MacApplicationPort(databaseURL: source.databaseURL)
        host.permissionConfigurationResponse = .init(conversation_id: conversation,
            configuration_revision: try XCTUnwrap(sourcePort.loadConversationMetadata(conversation)?.configurationRevision),
            permission_profile: nil, active_run_id: canonicalRunID, active_policy: nil, owner: .init(kind: .local, run_id: canonicalRunID, assistant_message_id: source.assistantMessageID, identity: nil), state: .pending)
        // The interrupted transport cannot simultaneously report an
        // authoritative empty permission list. Permit the explicit resume
        // preflight, then make its background observation unavailable until
        // the next user retry; retain the challenge actually received live.
        var observationAvailable = true
        host.observation = { requestedConversation, requestedRun, requestedAssistant in
            guard observationAvailable else { throw MacRemoteHostError.http(503, "observation_unavailable") }
            XCTAssertEqual(requestedConversation, conversation)
            XCTAssertEqual(requestedRun, canonicalRunID)
            XCTAssertEqual(requestedAssistant, assistantCanonicalID)
            var observation = MacRemoteRunObservation(protocol_version: 1, conversation_id: conversation,
                client_id: "fixture-client", run_id: canonicalRunID, assistant_message_id: assistantCanonicalID,
                status: "running", worker: .init(state: .absent, generation: nil), recovery: .resume_required,
                admission: admission, terminal_seal: nil, permissions: [], can_submit: false)
            observation.submission_receipt = try source.submissionReceipt()
            return .init(observation: observation, events: [])
        }
        let model = MacWorkspaceModel(initialConversationID: conversation, canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        model.selectedConversationID = conversation
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.presentationDidAttach()
        defer { model.presentationDidDetach() }
        model.recheckRemoteRun()
        _ = try await waitForValue { model.canRecoverRemoteRun ? true : nil }
        let before = try port.loadConversationWindow(conversation)
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, canonicalRunID)
        await consume(try event("same-ask", sequence: 1, conversation: conversation, nonce: "expired", turnID: canonicalRunID,
            coreSession: source.coreSessionID),
            model: model, conversation: conversation)
        model.noteRemoteTransportInterruption(conversationID: conversation, assistantMessageID: assistantID, generationID: 0)
        XCTAssertEqual(model.remoteRunRecovery?.expectedRunID, "canonical-run-distinct-from-trace")
        let resuming = Task { await model.resumeRemoteRun() }
        _ = try await waitForValue { host.resumeContinuation }
        observationAvailable = false
        await model.resumeRemoteRun()
        XCTAssertEqual(host.resumeRequests.count, 1, "A live resume must not be started twice")
        XCTAssertEqual(host.resumeRequests[0].0, conversation)
        XCTAssertEqual(host.resumeRequests[0].1, "canonical-run-distinct-from-trace")
        host.resumeContinuation?.yield(try event("same-ask", sequence: 2, conversation: conversation, nonce: "renewed",
            turnID: "successor-core-turn", canonicalRunID: canonicalRunID, coreSession: source.coreSessionID))
        host.resumeContinuation?.finish(throwing: MacRemoteHostError.http(409, "already_running"))
        await resuming.value
        XCTAssertEqual(model.pendingRemotePermission?.nonce, "renewed")
        XCTAssertNotNil(model.remoteRunRecovery)
        XCTAssertNotNil(model.remoteRecoveryError)
        XCTAssertFalse(model.isResumingRemoteRun)
        let after = try port.loadConversationWindow(conversation)
        XCTAssertEqual(before.projection.messages.map(\.messageID), after.projection.messages.map(\.messageID))
        XCTAssertEqual(before.projection.revision, after.projection.revision)
        XCTAssertEqual(try repository.activeCanonicalRunID(conversationID: conversation), "canonical-run-distinct-from-trace")

        // Old trace and typed terminal events from the same conversation must
        // not complete this resumed canonical Run or clear its recovery UI.
        host.resumeContinuation = nil
        observationAvailable = true
        let staleTerminalAttempt = Task { await model.resumeRemoteRun() }
        _ = try await waitForValue { host.resumeContinuation }
        observationAvailable = false
        host.resumeContinuation?.yield(try lifecycleEvent("run.completed", sequence: 9,
            conversation: conversation, turnID: "old-canonical-run"))
        let staleTyped = try XCTUnwrap(MacCoreTurnTerminalEnvelope.validated(
            sessionID: source.coreSessionID, turnID: "old-canonical-run", cursor: 10, revision: 10,
            status: .completed, reason: nil))
        host.resumeContinuation?.yield(MacLocalAgentEvent(kind: .finished, text: "", title: nil,
            detail: nil, isError: false, coreTurnTerminal: staleTyped))
        host.resumeContinuation?.finish()
        await staleTerminalAttempt.value
        XCTAssertNotNil(model.remoteRunRecovery)
        XCTAssertEqual(model.pendingRemotePermission?.nonce, "renewed")

        host.resumeContinuation = nil
        observationAvailable = true
        let matchingTerminalAttempt = Task { await model.resumeRemoteRun() }
        _ = try await waitForValue { host.resumeContinuation }
        observationAvailable = false
        // Same Run, successor Core Turn completion is still not Run completion.
        let matching = try XCTUnwrap(MacCoreTurnTerminalEnvelope.validated(
            sessionID: source.coreSessionID, turnID: "successor-core-turn", cursor: 11, revision: 11,
            status: .completed, reason: nil))
        host.resumeContinuation?.yield(MacLocalAgentEvent(kind: .finished, text: "", title: nil,
            detail: nil, isError: false, coreTurnTerminal: matching).bindingCanonicalRun(canonicalRunID))
        host.resumeContinuation?.finish()
        await matchingTerminalAttempt.value
        XCTAssertNotNil(model.remoteRunRecovery)
        XCTAssertNotNil(model.pendingRemotePermission)

        func projection(runID: String?, submissionID: String) throws -> MacLocalAgentEvent {
            try committedJournalEvent(receiver: port, conversation: conversation, assistantCanonicalID: assistantCanonicalID,
                runID: runID, submissionID: submissionID)
        }
        host.resumeContinuation = nil
        observationAvailable = true
        let rejected = Task { await model.resumeRemoteRun() }
        _ = try await waitForValue { host.resumeContinuation }
        observationAvailable = false
        host.resumeContinuation?.yield(try projection(runID: nil, submissionID: canonicalRunID))
        host.resumeContinuation?.yield(try projection(runID: "old-run", submissionID: canonicalRunID))
        host.resumeContinuation?.yield(try projection(runID: canonicalRunID, submissionID: "old-run"))
        host.resumeContinuation?.finish()
        await rejected.value
        XCTAssertNotNil(model.remoteRunRecovery)
        XCTAssertNotNil(model.pendingRemotePermission)

        host.resumeContinuation = nil
        observationAvailable = true
        let failedCommit = Task { await model.resumeRemoteRun() }
        _ = try await waitForValue { host.resumeContinuation }
        observationAvailable = false
        let unreceivedSource = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
            runID: canonicalRunID, assistantMessageID: assistantCanonicalID)
        let unreceivedSeal = try unreceivedSource.finish("A real source seal has not been admitted locally")
        host.resumeContinuation?.yield(MacLocalAgentEvent(kind: .status, text: "", title: nil,
            detail: nil, isError: false, assistantRunTerminalSealDigest: unreceivedSeal.seal_digest).bindingCanonicalRun(canonicalRunID))
        host.resumeContinuation?.finish()
        await failedCommit.value
        XCTAssertNotNil(model.remoteRunRecovery)
        XCTAssertNotNil(model.pendingRemotePermission)
        XCTAssertEqual(try repository.activeCanonicalRunID(conversationID: conversation), canonicalRunID)

        host.resumeContinuation = nil
        observationAvailable = true
        let committed = Task { await model.resumeRemoteRun() }
        _ = try await waitForValue { host.resumeContinuation }
        observationAvailable = false
        let finalEvent = try projection(runID: canonicalRunID, submissionID: canonicalRunID)
        XCTAssertEqual(finalEvent.addingArtifactURLs([]).canonicalRunID, canonicalRunID)
        host.resumeContinuation?.yield(finalEvent)
        host.resumeContinuation?.finish(throwing: MacRemoteHostError.http(502, "closed_after_commit"))
        await committed.value
        let sourceOwner = try port.conversationSourceOwner(conversationID: conversation)
        guard case .remote(let sourceHost, let epoch) = sourceOwner else {
            return XCTFail("Committed remote terminal must retain its durable source owner")
        }
        XCTAssertEqual(sourceHost, "fixture-host")
        XCTAssertFalse(epoch.isEmpty)
        let cold = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        XCTAssertEqual(try cold.conversationSourceOwner(conversationID: conversation), sourceOwner)
        XCTAssertNil(model.remoteRecoveryError)
        XCTAssertFalse(model.isResumingRemoteRun)
        XCTAssertNil(try repository.activeCanonicalRunID(conversationID: conversation))
        XCTAssertNil(model.remoteRunRecovery)
        XCTAssertNil(model.pendingRemotePermission)
    }

    private func committedJournalEvent(receiver: MacApplicationPort, conversation: String, assistantCanonicalID: String,
                                     runID: String?, submissionID: String,
                                     assistantText: String? = nil) throws -> MacLocalAgentEvent {
        let current = try receiver.activeRunIdentity(conversationID: conversation)
        let matchesCurrent = runID == submissionID && current?.runID == submissionID
        let source: MacAssistantJournalTestFixture
        if let retained = journalFixtures[submissionID], runID == submissionID {
            source = retained
        } else if matchesCurrent {
            source = try MacAssistantJournalTestFixture.forReceiver(receiver: receiver, conversationID: conversation,
                runID: submissionID, assistantMessageID: assistantCanonicalID)
            journalFixtures[submissionID] = source
        } else {
            source = try .init(conversationID: conversation, runID: submissionID,
                assistantMessageID: assistantCanonicalID)
        }
        let seal = try source.finish(assistantText ?? "The projection is the durable UI source.")
        if matchesCurrent || journalFixtures[submissionID] === source {
            _ = try source.commitTerminal(into: receiver)
        }
        return source.terminalEvent(seal: seal).bindingCanonicalRun(runID)
    }

    func testUnknownCanonicalRunDisablesResumeInsteadOfGuessingTraceIdentity() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        let conversation = try XCTUnwrap(model.selectedConversationID)
        await consume(try event("ask", sequence: 1, conversation: conversation), model: model, conversation: conversation)
        model.noteRemoteTransportInterruption(conversationID: conversation, assistantMessageID: UUID(), generationID: 0)
        XCTAssertNil(model.remoteRunRecovery?.expectedRunID)
        await model.resumeRemoteRun()
        XCTAssertTrue(host.resumeRequests.isEmpty)
        XCTAssertEqual(model.pendingRemotePermission?.id, "ask")
    }

    func testUnknownRemoteOwnerCanParkWithoutBlockingOrCancellingAnotherConversation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        let original = try XCTUnwrap(model.selectedConversationID)
        let assistant = UUID()
        model.noteRemoteTransportInterruption(conversationID: original, assistantMessageID: assistant, generationID: 0)
        let previousGeneration = try XCTUnwrap(model.remoteRunRecovery?.generationID)
        model.newConversation()
        XCTAssertNotEqual(model.selectedConversationID, original)
        XCTAssertNil(model.remoteRunRecovery)
        model.stop()
        XCTAssertTrue(host.cancellationRequests.isEmpty)
        model.selectedConversationID = original
        let restored = try XCTUnwrap(model.remoteRunRecovery)
        XCTAssertEqual(restored.conversationID, original)
        XCTAssertEqual(restored.assistantMessageID, assistant)
        XCTAssertNil(restored.expectedRunID)
        XCTAssertGreaterThan(restored.generationID, previousGeneration)
        XCTAssertFalse(model.canRecoverRemoteRun)
    }

}
