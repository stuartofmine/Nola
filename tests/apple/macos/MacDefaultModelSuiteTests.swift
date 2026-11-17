// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS default chat model store, settings summary, workspace inheritance, and submission boundary suite.

import AppKit
import Foundation
import SQLite3
import SwiftUI
import XCTest
@testable import Nola

private actor DefaultBoundaryCaptureRuntime: MacLocalAgentRuntime {
    private(set) var requests: [MacLocalAgentRunRequest] = []
    func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
        requests.append(request)
        return AsyncThrowingStream { $0.finish(throwing: MacApplicationPortError.rejected("Captured fixture route")) }
    }
    func cancel(conversationID: String) {}
    func resolvePermission(conversationID: String, request: MacRemotePermissionRequest,
                           allow: Bool, scope: MacPermissionGrantScope) throws { throw MacApplicationPortError.invalidResponse }
    func listSessions() async throws -> [MacLocalAgentSessionSummary] { [] }
    func deleteSession(id: String) async throws {}
}

final class MacDefaultModelSuiteTests: XCTestCase {
    func testCanonicalDefaultStoreCASResolutionUnavailableFactsAndCorruptRepair() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("default-chat-suite-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("store.db")
        let application = try MacApplicationPort(databaseURL: database)
        let store = MacDefaultChatModelStore(application: application)

        // Phase 1: Unavailable facts & malformed configuration fail through Rust bridge before any default is saved
        let facts: [(MacChatModelAvailability, MacChatModelUnavailableReason)] = [
            (.disabled, .disabled), (.notInstalled, .notInstalled), (.unsupportedRuntime, .unsupportedRuntime)
        ]
        for (availability, reason) in facts {
            let candidates = [MacChatModelCandidate(profileID: "local:model", availability: availability), MacChatModelCandidate(profileID: "api:other")]
            XCTAssertEqual(try store.resolve(intent: .explicit("local:model"), candidates: candidates), .unavailable(profileID: "local:model", origin: .conversation, reason: reason))
            XCTAssertThrowsError(try store.set(.profile("local:model"), expectedRevision: nil, candidates: candidates))
        }
        XCTAssertEqual(try application.resolveChatModel(intent: .inherit, defaultValueJSON: "{}", candidates: []), .invalidConfiguration)
        XCTAssertEqual(try application.resolveChatModel(intent: .explicit("api:valid"), defaultValueJSON: "{}", candidates: [.init(profileID: "api:valid")]), .resolved(profileID: "api:valid", origin: .conversation))
        XCTAssertThrowsError(try application.putProductSettingChecked(key: MacDefaultChatModelStore.key, valueJSON: "{}", expectedRevision: nil, updatedAtMs: 1))
        XCTAssertNil(try store.snapshot().revision)
        XCTAssertEqual(try store.resolve(intent: .explicit("api:restricted"), candidates: [.init(profileID: "api:restricted", allowedForChat: false)]), .unavailable(profileID: "api:restricted", origin: .conversation, reason: .notAllowedForChat))

        // Phase 2: Canonical default CAS, Rust resolution, and persistence reopen round-trip
        let candidates = [MacChatModelCandidate(profileID: "api:a:same"), MacChatModelCandidate(profileID: "local:b:same")]
        XCTAssertEqual(try store.snapshot().selection, .automatic)
        XCTAssertEqual(try store.resolve(candidates: candidates), .resolved(profileID: "api:a:same", origin: .automatic))
        let saved = try store.set(.profile("local:b:same"), expectedRevision: nil, candidates: candidates)
        XCTAssertEqual(saved.revision, 1)
        XCTAssertEqual(try store.resolve(candidates: candidates), .resolved(profileID: "local:b:same", origin: .default))
        XCTAssertEqual(try store.resolve(intent: .explicit("api:a:same"), candidates: candidates), .resolved(profileID: "api:a:same", origin: .conversation))
        XCTAssertThrowsError(try store.set(.profile("api:a:same"), expectedRevision: nil, candidates: candidates))
        XCTAssertEqual(try store.snapshot(), saved)
        XCTAssertEqual(try store.resolve(candidates: [candidates[0]]), .unavailable(profileID: "local:b:same", origin: .default, reason: .missing))
        XCTAssertEqual(try store.resolve(intent: .explicit("absent:same"), candidates: candidates), .unavailable(profileID: "absent:same", origin: .conversation, reason: .missing))
        let reopened = MacDefaultChatModelStore(application: try MacApplicationPort(databaseURL: database))
        XCTAssertEqual(try reopened.snapshot(), saved)
        let restored = try store.set(.automatic, expectedRevision: saved.revision, candidates: [])
        XCTAssertEqual(restored.selection, .automatic)
        XCTAssertEqual(try store.resolve(candidates: []), .noAutomaticCandidate)

        // Phase 3: Corrupt SQLite preference retains CAS revision for repair
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &connection), SQLITE_OK)
        defer { sqlite3_close(connection) }
        XCTAssertEqual(sqlite3_exec(connection, "UPDATE product_settings SET value_json='{}' WHERE key='chat.default_model'", nil, nil, nil), SQLITE_OK)
        XCTAssertThrowsError(try store.snapshot())
        XCTAssertEqual(try store.resolve(candidates: []), .invalidConfiguration)
        let repair = try store.rawSnapshot()
        XCTAssertEqual(repair.revision, restored.revision)
        XCTAssertEqual(repair.valueJSON, "{}")
        XCTAssertEqual(try store.set(.automatic, expectedRevision: repair.revision, candidates: []).selection, .automatic)
    }

    @MainActor
    func testSettingsDefaultSummaryCoversLocalProfileAndHostedProfileNavigation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("settings-default-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }

        let localApplication = try MacApplicationPort(databaseURL: root.appendingPathComponent("local-default.db"))
        let localProfile = try XCTUnwrap(MacLocalModelProfile.available.first)
        let localStore = MacDefaultChatModelStore(application: localApplication)
        try localStore.set(.profile(localProfile.id), expectedRevision: nil, candidates: [.init(profileID: localProfile.id)])
        let localPreference = MacDefaultChatModelPresentation(store: localStore)
        XCTAssertNil(localPreference.error)
        XCTAssertEqual(localPreference.snapshot?.selection, .profile(localProfile.id))
        switch localPreference.resolution {
        case let .resolved(id, origin):
            XCTAssertEqual(id, localProfile.id)
            XCTAssertEqual(origin, .default)
        case let .unavailable(id, origin, reason):
            XCTAssertEqual(id, localProfile.id)
            XCTAssertEqual(origin, .default)
            XCTAssertEqual(reason, .notInstalled)
        default:
            XCTFail("A local model must resolve or show its installation state, not an API configuration error.")
        }

        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("settings.db"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: application)
        try configuration.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Fixture", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "first", modelName: "first-model", displayName: "First"),
                     MacRemoteModelConfiguration(id: "second", modelName: "second-model", displayName: "Second")])])
        let store = MacDefaultChatModelStore(application: application)
        let preference = MacDefaultChatModelPresentation(store: store)
        var opened: [String?] = []
        let hosting = NSHostingView(rootView: VStack {
            MacSettingsDefaultChatModelSummary(preference: preference) { opened.append($0) }
        }.environment(\.accessibilityEnabled, true))
        hosting.frame = NSRect(x: 0, y: 0, width: 600, height: 300)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting)

        let saved = try store.set(.profile("second"), expectedRevision: nil, candidates: preference.catalog.candidates)
        try await settle(hosting)
        XCTAssertEqual(preference.snapshot?.selection, .profile("second"))
        XCTAssertEqual(preference.resolution, .resolved(profileID: "second", origin: .default))
        XCTAssertTrue(preference.title.contains("Second"))
        let button = try XCTUnwrap(elements(hosting).first {
            $0.accessibilityIdentifier?() == "settings.default-chat-model.manage"
        })
        XCTAssertEqual(button.accessibilityPerformPress?(), true)
        XCTAssertEqual(opened.count, 1)
        XCTAssertEqual(opened.first!, "second")

        try configuration.replaceEndpoints([])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        try await settle(hosting)
        XCTAssertEqual(preference.snapshot?.selection, .profile("second"))
        XCTAssertEqual(preference.resolution, .unavailable(profileID: "second", origin: .default, reason: .missing))
        XCTAssertEqual(try store.snapshot().revision, saved.revision, "Rendering a missing default must not rewrite the user's choice.")

        _ = try store.set(.automatic, expectedRevision: saved.revision, candidates: [])
        try await settle(hosting)
        XCTAssertEqual(preference.snapshot?.selection, .automatic)
    }

    @MainActor
    func testWorkspaceCredentialIsolationCatalogEditsAndMissingExplicitModelBlocking() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("workspace-default-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }

        // Phase 1: Unrelated connection credentials are not read when loading catalog facts or chosen route
        let credPort = try MacApplicationPort(databaseURL: directory.appendingPathComponent("creds.db"))
        try MacAppleChatConfigurationStore(applicationPort: credPort).replaceEndpoints(["good", "bad"].map { id in
            MacRemoteEndpointConfiguration(id: id, name: id, baseURL: "https://\(id).invalid/v1", apiKey: "isolated-fixture-key",
                models: [MacRemoteModelConfiguration(id: id, modelName: "same-name", executionCapabilities: .init())])
        })
        let reader = MacAppleChatConfigurationStore(applicationPort: credPort, credentialReader: { account in
            if account.hasPrefix("endpoint:bad:") { throw MacApplicationPortError.rejected("Unavailable unrelated credential") }
            return "isolated-fixture-key"
        })
        XCTAssertThrowsError(try reader.endpointSnapshot())
        XCTAssertEqual(try reader.endpointSnapshot(includeCredentials: false).endpoints.count, 2)
        XCTAssertEqual(try reader.loadThrowing(model: "good")?.profileID, "good")
        XCTAssertEqual(try reader.routePlanThrowing(preferredModel: "good")?.primary.profileID, "good")
        XCTAssertThrowsError(try reader.loadThrowing(model: "bad"))

        // Phase 2: Default identity, explicit conversation override, catalog edits, and missing explicit model blocking
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("models.db"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        var endpoints = ["a", "b"].map { id in
            MacRemoteEndpointConfiguration(id: id, name: id, baseURL: "https://\(id).invalid/v1",
                models: [MacRemoteModelConfiguration(id: "profile:\(id)", modelName: "same-name",
                    executionCapabilities: .init())])
        }
        try store.replaceEndpoints(endpoints)
        let preference = MacDefaultChatModelStore(application: port)
        let facts = try MacChatModelCatalog.load(application: port).candidates
        let saved = try preference.set(.profile("profile:b"), expectedRevision: nil, candidates: facts)
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        XCTAssertTrue(model.inheritsDefaultModel)
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:b")
        XCTAssertEqual(try store.loadThrowing()?.profileID, "profile:b")
        let a = try XCTUnwrap(model.availableModelOptions.first { $0.configurationID == "profile:a" })
        model.selectModel(a)
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:a")
        XCTAssertEqual(model.selectedRemoteConfiguration?.endpointName, "a")
        XCTAssertEqual(model.modelSelectionResolution, .resolved(profileID: "profile:a", origin: .conversation))
        _ = try preference.set(.automatic, expectedRevision: saved.revision, candidates: facts)
        model.refreshModelConfiguration()
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:a")

        endpoints[0].name = "Renamed"
        endpoints[1].models[0].priority = 100
        try store.replaceEndpoints(endpoints)
        model.refreshModelConfiguration()
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:a")
        XCTAssertEqual(model.selectedRemoteConfiguration?.endpointName, "Renamed")
        XCTAssertEqual(try preference.snapshot().selection, .automatic)
        model.restoreDefaultModelInheritance()
        XCTAssertTrue(model.inheritsDefaultModel)
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:b")

        model.selectModel(MacChatModelOption(source: .configured, modelID: "same-name",
            displayName: "Removed", configurationID: "removed-profile"))
        XCTAssertNil(model.selectedModelOption)
        XCTAssertEqual(model.modelSelectionResolution,
            .unavailable(profileID: "removed-profile", origin: .conversation, reason: .missing))
        XCTAssertThrowsError(try store.routePlanThrowing(preferredModel: "removed-profile"))
        XCTAssertThrowsError(try store.loadThrowing(model: "same-name"), "Provider names are not canonical profile identity")
        model.draft = "Do not send to another provider"
        let count = model.selectedMessages.count
        model.send()
        XCTAssertFalse(model.isRunning)
        XCTAssertEqual(model.selectedMessages.count, count)
        XCTAssertEqual(model.draft, "Do not send to another provider")
        model.restoreDefaultModelInheritance()
        XCTAssertEqual(model.selectedModelOption?.configurationID, "profile:b")
    }

    @MainActor
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

    @MainActor
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
        try await service.saveConversationSelection(conversationID: "conversation", title: "Fixture",
            profile: nil, allowCreate: true, target: service.captureSubmissionTarget())
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

    @MainActor
    private func settle(_ view: NSView) async throws {
        for _ in 0..<8 {
            try await Task.sleep(for: .milliseconds(20))
            view.layoutSubtreeIfNeeded()
        }
    }

    @MainActor
    private func elements(_ view: NSView) -> [AnyObject] {
        var queue: [AnyObject] = [view]
        var result: [AnyObject] = []
        var visited: Set<ObjectIdentifier> = []
        var index = 0
        while index < queue.count, result.count < 1000 {
            let element = queue[index]
            index += 1
            guard visited.insert(ObjectIdentifier(element)).inserted else { continue }
            result.append(element)
            let children: [Any] = element.accessibilityChildren?() ?? []
            queue.append(contentsOf: children.map { $0 as AnyObject })
            if let nativeView = element as? NSView { queue.append(contentsOf: nativeView.subviews) }
        }
        return result
    }

    @MainActor
    private func get<T>(_ url: URL) async throws -> T {
        let (data, response) = try await URLSession.shared.data(from: url)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? T)
    }
}
