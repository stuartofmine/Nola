// SPDX-License-Identifier: Apache-2.0
import Foundation
import SQLite3
import XCTest
@testable import Nola

private actor DefaultBoundaryCaptureRuntime: MacLocalAgentRuntime {
    private(set) var requests: [MacLocalAgentRunRequest] = []
    func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        requests.append(request)
        // The test stops at the executable boundary, before launching a provider.
        return AsyncThrowingStream { $0.finish(throwing: MacApplicationPortError.rejected("Captured fixture route")) }
    }
    func cancel(conversationID: String) {}
    func resolvePermission(conversationID: String, request: MacRemotePermissionRequest,
                           allow: Bool, scope: MacPermissionGrantScope) throws { throw MacApplicationPortError.invalidResponse }
    func listSessions() async throws -> [MacLocalAgentSessionSummary] { [] }
    func deleteSession(id: String) async throws {}
}

@MainActor
final class MacDefaultModelSubmissionBoundaryTests: XCTestCase {
    func testPersistedMalformedGlobalBlocksInheritanceButExplicitWorkspaceSubmissionCapturesExactRoute() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("default-boundary-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        try MacAppleChatConfigurationStore(applicationPort: port).replaceEndpoints([
            .init(id: "connection", name: "Fixture", baseURL: "https://fixture.invalid/v1",
                models: [.init(id: "profile:explicit", modelName: "same-name", executionCapabilities: .init())])
        ])
        _ = try MacDefaultChatModelStore(application: port).set(.automatic, expectedRevision: nil, candidates: [])
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(port.databaseURL.path, &connection), SQLITE_OK)
        XCTAssertEqual(sqlite3_exec(connection, "UPDATE product_settings SET value_json='{}' WHERE key='chat.default_model'", nil, nil, nil), SQLITE_OK)
        sqlite3_close(connection)
        let runtime = DefaultBoundaryCaptureRuntime()
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: runtime, remoteHost: PersistenceTestRemoteHost(), modelApplication: port,
            completionNotifier: { _, _, _ in }, contextPreparation: { $0.contextComponents }, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        XCTAssertTrue(model.inheritsDefaultModel)
        XCTAssertEqual(model.modelSelectionResolution, .invalidConfiguration)
        model.draft = "fixture"
        let before = model.selectedMessages.count
        model.send()
        XCTAssertFalse(model.isRunning)
        XCTAssertEqual(model.selectedMessages.count, before)
        XCTAssertEqual(model.draft, "fixture")
        let blockedRequests = await runtime.requests
        XCTAssertTrue(blockedRequests.isEmpty)

        model.selectModel(try XCTUnwrap(model.availableModelOptions.first { $0.configurationID == "profile:explicit" }))
        try await model.waitForModelConfigurationCommit()
        XCTAssertEqual(model.modelSelectionResolution, .resolved(profileID: "profile:explicit", origin: .conversation))
        model.send()
        let deadline = Date().addingTimeInterval(5)
        while await runtime.requests.isEmpty, Date() < deadline { try await Task.sleep(for: .milliseconds(10)) }
        let requests = await runtime.requests
        XCTAssertEqual(requests.count, 1)
        let request = try XCTUnwrap(requests.first)
        XCTAssertEqual(request.modelProfileID, "profile:explicit")
        XCTAssertEqual(request.modelID, "same-name")
        XCTAssertEqual(request.endpoint, "https://fixture.invalid/v1")
        XCTAssertEqual(request.prompt, "fixture")
        let owner = try XCTUnwrap(model.selectedConversationID)
        let metadata = try XCTUnwrap(port.loadConversationMetadata(owner))
        XCTAssertEqual(request.conversationID, owner)
        XCTAssertEqual(request.expectedConfigurationRevision, metadata.configurationRevision)
        XCTAssertNotNil(request.expectedConfigurationRevision, "A first draft dispatch must carry its Create receipt")
        XCTAssertNil(request.expectedParentMessageID)
    }

    func testFreshHostDefaultRevisionRejectsBeforeHTTPAndExplicitBadGlobalReachesExactHostProfile() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("host-default-boundary-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let server = try CancellationJournalHTTPServer(status: 202, delayed: false, mode: "cancel")
        let baseURL = try server.start()
        defer { server.terminate() }
        var settings: [String: Any] = [
            "host_identity": ["id": "fixture-host"], "network_policy": "allow", "enable_local_dev_tools": false,
            "model_profiles": ["first", "second"].map { ["profile_id": $0, "revision": 7, "display_name": $0, "model_name": "same-name"] as [String: Any] },
            "default_chat_model_resolution": ["kind": "resolved", "profile_id": "second", "origin": "default"],
            "default_chat_model_revision": 5
        ]
        server.replaceModelSettings(settings)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try repository.restoreBackupSnapshot([.init(conversation: .init(id: "conversation", title: "Fixture",
            preview: "", updatedAt: .now, isPinned: false), messages: [])])
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        let model = MacWorkspaceModel(initialConversationID: "conversation", canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: service, modelApplication: port)
        defer { model.presentationDidDetach() }
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.updateRemoteHostConfigurationState(true)
        model.updateRemoteModelCatalog(try await service.fetchRemoteModelCatalog())
        let inherited = try XCTUnwrap(model.remoteModelIntentForSubmission(tier: .balanced))
        XCTAssertEqual(inherited.profileID, "second")
        XCTAssertEqual(inherited.expectedDefault, .init(revision: 5, profileID: "second"))
        settings["default_chat_model_revision"] = 6
        server.replaceModelSettings(settings)
        var commits = 0
        let blockedTicket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: "blocked", generationID: 1))
        let blocked = await service.run(conversationID: "conversation", submissionID: "blocked", userMessageID: "blocked:user",
            assistantMessageID: "blocked:assistant", prompt: "fixture", modelID: "same-name", modelProviderID: nil,
            modelIntent: inherited, memoryEnabled: false, executionBackend: "nola", externalSessionID: nil,
            submission: .init(target: try service.captureSubmissionTarget(), ticket: blockedTicket, beforeDispatch: { commits += 1 }))
        do {
            for try await _ in blocked {}
            XCTFail("Changed Host default must reject the inherited submission")
        } catch MacRemoteHostError.http(let status, _) { XCTAssertEqual(status, 409) }
        XCTAssertEqual(blockedTicket.phase, .rejectedBeforeDispatch)
        XCTAssertEqual(commits, 0)
        let blockedCounts: [String: Int] = try await get(baseURL.appendingPathComponent("counts"))
        XCTAssertEqual(blockedCounts["streams"], 0)
        XCTAssertNil(try port.remoteSubmissionIntent(conversationID: "conversation"))

        settings["default_chat_model_resolution"] = ["kind": "invalid_configuration"]
        server.replaceModelSettings(settings)
        model.updateRemoteModelCatalog(try await service.fetchRemoteModelCatalog())
        XCTAssertEqual(model.modelSelectionResolution, .invalidConfiguration)
        model.selectModel(try XCTUnwrap(model.availableModelOptions.first { $0.configurationID == "first" }))
        try await model.waitForModelConfigurationCommit()
        let explicit = try XCTUnwrap(model.remoteModelIntentForSubmission(tier: .balanced))
        XCTAssertNil(explicit.expectedDefault)
        let target = try service.captureSubmissionTarget()
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: "explicit", generationID: 2))
        let sent = await service.run(conversationID: "conversation", submissionID: "explicit", userMessageID: "explicit:user",
            assistantMessageID: "explicit:assistant", prompt: "fixture", modelID: "same-name", modelProviderID: nil,
            modelIntent: explicit, memoryEnabled: false, executionBackend: "nola", externalSessionID: nil,
            submission: .init(target: target, ticket: ticket, beforeDispatch: {
                _ = try port.prepareRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Fixture",
                    submissionID: "explicit", sourceHostID: "fixture-host", clientID: target.clientID,
                    userMessageID: "explicit:user", userPartID: "explicit:user:text", userText: "fixture", userContentJSON: nil,
                    assistantMessageID: "explicit:assistant", assistantPartID: "explicit:assistant:text", contextCheckpointID: nil, createdAtMs: 1))
            }))
        do { for try await _ in sent {} } catch MacRemoteHostError.transportInterrupted {}
        XCTAssertEqual(ticket.phase, .accepted)
        let requests: [[String: Any]] = try await get(baseURL.appendingPathComponent("decoded-stream-requests"))
        XCTAssertEqual(requests.count, 1)
        let request = try XCTUnwrap(requests.first)
        XCTAssertEqual(request["expected_host_id"] as? String, "fixture-host")
        let intent = try XCTUnwrap(request["model_intent"] as? [String: Any])
        XCTAssertEqual(intent["profile_id"] as? String, "first")
        XCTAssertEqual(intent["expected_revision"] as? Int, 7)
    }

    private func get<T>(_ url: URL) async throws -> T {
        let (data, response) = try await URLSession.shared.data(from: url)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? T)
    }
}
