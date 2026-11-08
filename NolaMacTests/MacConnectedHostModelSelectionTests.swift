// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacConnectedHostModelSelectionTests: XCTestCase {
    func testConflictRefreshesOnlyOriginalOwnerWithoutReplayingEdit() async throws {
        let (root, model) = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: root) }
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.setFixtureRemoteModels([profile(id: "first"), profile(id: "second")])
        let owner = try XCTUnwrap(model.selectedConversationID)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let original = try XCTUnwrap(port.loadConversationMetadata(owner))
        let winner = try XCTUnwrap(model.configuredModelOptions.first)
        let loser = try XCTUnwrap(model.configuredModelOptions.last)
        let patch = String(decoding: try JSONSerialization.data(withJSONObject: ["modelSelectionID": winner.id]), as: UTF8.self)
        _ = try port.patchConversationConfiguration(.init(conversationID: owner,
            expectedRevision: XCTUnwrap(original.configurationRevision), patchJSON: patch, updatedAtMs: 1)).committedSnapshot()
        model.selectModel(loser)
        model.newConversation()
        let other = try XCTUnwrap(model.selectedConversationID)
        model.draft = "other conversation draft"
        let otherIdentity = model.selectedConversation?.modelSelectionID
        do {
            try await model.waitForModelConfigurationCommit(conversationID: owner)
            XCTFail("The stale edit must fail before recovery")
        } catch {}
        try await model.waitForModelConfigurationCommit(conversationID: owner)
        XCTAssertEqual(model.selectedConversationID, other)
        XCTAssertEqual(model.draft, "other conversation draft")
        XCTAssertEqual(model.selectedConversation?.modelSelectionID, otherIdentity)
        model.selectedConversationID = owner
        XCTAssertEqual(model.selectedConversation?.modelSelectionID, winner.id)
        XCTAssertEqual(model.selectedConversation?.configurationRevision, try XCTUnwrap(original.configurationRevision) + 1)
    }
    func testHostAndProfileIdentityIsCanonicalAndSeparateFromLocalNames() {
        let option = MacChatModelOption(source: .connectedHost, modelID: "shared-name",
            displayName: "Shared", configurationID: "profile:/中文", hostID: "host:/中文")
        let parsed = MacChatModelOption.parse(option.id)
        XCTAssertEqual(parsed?.source, .connectedHost)
        XCTAssertEqual(parsed?.configurationID, "profile:/中文")
        XCTAssertEqual(parsed?.hostID, "host:/中文")
        XCTAssertEqual(parsed?.id, option.id)
        XCTAssertFalse(option.id.contains("="))
        XCTAssertFalse(option.id.contains("/"))
        XCTAssertFalse(option.id.contains("+"))
        XCTAssertNotEqual(option.id, MacChatModelOption(source: .configured,
            modelID: "shared-name", displayName: "Shared", configurationID: "profile:/中文").id)
        XCTAssertNotEqual(option.id, MacChatModelOption(source: .connectedHost,
            modelID: "shared-name", displayName: "Shared", configurationID: "profile:/中文", hostID: "other-host").id)
        XCTAssertNil(MacChatModelOption.parse("connectedHost::"))
        XCTAssertNil(MacChatModelOption.parse("connectedHost:aG9zdA==:cHJvZmlsZQ=="))
        XCTAssertNil(MacChatModelOption.parse("connectedHost:aG9zdA:cHJvZmlsZQ:extra"))
    }

    func testConnectedHostSelectionDoesNotAliasSameNamedProfilesOrAnotherHost() async throws {
        let (root, model) = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: root) }
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([profile(id: "first"), profile(id: "second")])
        XCTAssertEqual(model.configuredModelOptions.count, 2)
        XCTAssertTrue(model.downloadedModelOptions.isEmpty)
        let second = try XCTUnwrap(model.configuredModelOptions.last)
        model.selectModel(second)
        XCTAssertTrue(model.selectedModelConfigurationPending)
        model.draft = "retained while saving"
        XCTAssertFalse(model.selectedComposerState.canSend)
        model.send()
        XCTAssertFalse(model.isRunning)
        XCTAssertEqual(model.draft, "retained while saving")
        XCTAssertThrowsError(try model.remoteModelIntentForSubmission(tier: .balanced))
        try await model.waitForModelConfigurationCommit()
        XCTAssertEqual(model.selectedRemoteModelProfile?.profileID, "second")
        XCTAssertNil(model.selectedRemoteConfiguration, "Host profiles must never resolve against the local model center.")

        model.setFixtureRemoteModels([profile(id: "first")])
        XCTAssertNil(model.selectedModelOption, "Removed explicit profile must not silently fall back by name.")
        XCTAssertThrowsError(try model.remoteModelIntentForSubmission(tier: .balanced))
        model.setFixtureRemoteModels([profile(id: "second", host: "other-host")])
        XCTAssertNil(model.selectedModelOption, "Same profile ID on another Host is a different selection.")
        model.updateRemoteHostConfigurationState(false)
        XCTAssertTrue(model.configuredModelOptions.isEmpty)
    }

    func testSubmissionFreezesHostRevisionTierAndOnlyExplicitConversationOverrides() async throws {
        let (root, model) = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: root) }
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.setFixtureRemoteModels([profile(id: "profile", revision: 7)])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        let inherited = try XCTUnwrap(model.remoteModelIntentForSubmission(tier: .balanced))
        XCTAssertTrue((try inherited.payload()["overrides"] as? [String: Any])?.isEmpty == true)
        var overrides = MacConversationModelOverride()
        overrides.temperature = 0
        overrides.seed = .value(0)
        model.conversationModelOverride = overrides
        try await model.waitForModelConfigurationCommit()
        let frozen = try XCTUnwrap(model.remoteModelIntentForSubmission(tier: .concise))

        overrides.temperature = 1.8
        model.conversationModelOverride = overrides
        try await model.waitForModelConfigurationCommit()
        model.setFixtureRemoteModels([profile(id: "profile", revision: 8)])
        model.style = .detailed
        try await model.waitForModelConfigurationCommit()
        let payload = try frozen.payload()
        XCTAssertEqual(payload["profile_id"] as? String, "profile")
        XCTAssertEqual(payload["expected_revision"] as? UInt64, 7)
        XCTAssertEqual(payload["tier"] as? String, "concise")
        let fields = try XCTUnwrap(payload["overrides"] as? [String: Any])
        XCTAssertEqual(fields["temperature"] as? Double, 0)
        XCTAssertEqual((fields["generation_options"] as? [String: Any])?["seed"] as? Int, 0)
        XCTAssertNil(fields["top_p"])
        XCTAssertNil(fields["max_output_tokens"])
        XCTAssertNil(fields["model_protocol"])
        XCTAssertNil(fields["reasoning"])
        XCTAssertEqual(try model.remoteModelIntentForSubmission(tier: model.style)?.expectedRevision, 8)
        model.executionHost = .local
        try await model.waitForModelConfigurationCommit()
        XCTAssertNil(try model.remoteModelIntentForSubmission(tier: .balanced))
    }

    private func profile(id: String, host: String = "host", revision: UInt64 = 1) -> MacRemoteModelProfile {
        MacRemoteModelProfile(hostID: host, profileID: id, revision: revision,
            displayName: "Same model", modelName: "same-model")
    }

    private func makeWorkspace() throws -> (URL, MacWorkspaceModel) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-host-model-selection-\(UUID().uuidString)", isDirectory: true)
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.sqlite"))
        )
        let conversation = MacConversation(title: "Remote test", preview: "", updatedAt: .now, isPinned: false)
        try repository.restoreBackupSnapshot([MacStoredConversation(conversation: conversation, messages: [])])
        let model = MacWorkspaceModel(initialConversationID: conversation.id, canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        // This fixture exercises configuration persistence/admission without a
        // presented window. Otherwise the selection lifecycle's deferred task
        // starts live Host observation as soon as an awaited commit yields.
        // Keep the fail-fast Host double: an actual send must still fail here.
        model.presentationDidDetach()
        return (root, model)
    }
}
