// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacRemoteModelIntentTests: XCTestCase {
    func testSharedSourceFrontierValidatesOwnerAndDisplayedParent() throws {
        let value = AppleSubmissionFrontier(protocol_version: 1, source_host_id: "source",
            client_id: "client", conversation_id: "conversation", configuration_revision: 19,
            parent_message_id: "source-message")
        try value.validate(host: "source", client: "client", conversation: "conversation", observedParent: "source-message")
        XCTAssertThrowsError(try value.validate(host: "foreign", client: "client", conversation: "conversation", observedParent: "source-message"))
        XCTAssertThrowsError(try value.validate(host: "source", client: "other", conversation: "conversation", observedParent: "source-message"))
        XCTAssertThrowsError(try value.validate(host: "source", client: "client", conversation: "conversation", observedParent: "stale-message"))
        let fresh = AppleSubmissionFrontier(protocol_version: 1, source_host_id: "source",
            client_id: "client", conversation_id: "conversation", configuration_revision: 3, parent_message_id: nil)
        try fresh.validate(host: "source", client: "client", conversation: "conversation", observedParent: nil)
    }

    func testInheritedSubmissionChecksFreshHostDefaultBeforeEmittingPayload() throws {
        var fresh = settings()
        fresh["default_chat_model_resolution"] = ["kind": "resolved", "profile_id": "profile-a", "origin": "default"]
        fresh["default_chat_model_revision"] = 5
        let profile = try XCTUnwrap(MacRemoteModelCatalog.decode(settings: fresh, expectedHostID: "host-a").first)
        let intent = try MacRemoteModelIntent(profile: profile, tier: "balanced",
            expectedDefault: .init(revision: 5, profileID: profile.profileID))
        var payload: [String: Any] = [:]
        try MacRemoteModelPayload.apply(to: &payload, intent: intent, settings: fresh,
            expectedHostID: "host-a", executionBackend: "nola", modelID: "", modelProviderID: nil)
        XCTAssertEqual((payload["model_intent"] as? [String: Any])?["profile_id"] as? String, profile.profileID)
        let unavailableResolutions: [[String: Any]] = [
            ["kind": "unavailable", "profile_id": "profile-a", "origin": "default", "reason": "disabled"],
            ["kind": "invalid_configuration"], ["kind": "no_automatic_candidate"]
        ]
        for resolution in unavailableResolutions {
            var changed = fresh
            changed["default_chat_model_resolution"] = resolution
            var rejected: [String: Any] = [:]
            XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &rejected, intent: intent, settings: changed,
                expectedHostID: "host-a", executionBackend: "nola", modelID: "", modelProviderID: nil))
            XCTAssertTrue(rejected.isEmpty)
        }
        fresh["default_chat_model_revision"] = 6
        var rejected: [String: Any] = [:]
        XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &rejected, intent: intent, settings: fresh,
            expectedHostID: "host-a", executionBackend: "nola", modelID: "", modelProviderID: nil))
        XCTAssertTrue(rejected.isEmpty)
        // Explicit sessions do not consume a malformed or changed Host preference.
        fresh["default_chat_model_resolution"] = ["kind": "invalid_configuration"]
        try MacRemoteModelPayload.apply(to: &rejected, intent: MacRemoteModelIntent(profile: profile, tier: "balanced"),
            settings: fresh, expectedHostID: "host-a", executionBackend: "nola", modelID: "", modelProviderID: nil)
    }

    func testHostDefaultProjectionUsesExactIdentityAndPreservesUnavailableState() throws {
        var value = settings()
        value["default_chat_model_resolution"] = ["kind": "resolved", "profile_id": "profile-a", "origin": "default"]
        value["default_chat_model_revision"] = 3
        let snapshot = try MacRemoteModelCatalog.decodeSnapshot(settings: value, expectedHostID: "host-a")
        XCTAssertEqual(snapshot.defaultResolution, .resolved(profileID: "profile-a", origin: .default))
        XCTAssertEqual(snapshot.defaultRevision, 3)
        value["default_chat_model_resolution"] = ["kind": "unavailable", "profile_id": "removed:same-name", "origin": "default", "reason": "missing"]
        XCTAssertEqual(try MacRemoteModelCatalog.decodeSnapshot(settings: value, expectedHostID: "host-a").defaultResolution,
                       .unavailable(profileID: "removed:same-name", origin: .default, reason: .missing))
        value["default_chat_model_resolution"] = ["kind": "resolved", "profile_id": "missing:same-name", "origin": "default"]
        XCTAssertThrowsError(try MacRemoteModelCatalog.decodeSnapshot(settings: value, expectedHostID: "host-a"))
        value.removeValue(forKey: "default_chat_model_resolution")
        XCTAssertThrowsError(try MacRemoteModelCatalog.decodeSnapshot(settings: value, expectedHostID: "host-a"))
    }

    func testResumeContainsIdentityOnlyAndCannotOverrideFrozenMemory() throws {
        let payload = try MacRemoteResumeRequest(conversationID: "conversation", expectedRunID: "run",
            expectedHostID: "host", clientID: "client").payload()
        XCTAssertEqual(Set(payload.keys), ["conversation_id", "expected_run_id", "expected_host_id", "client_id"])
        XCTAssertNil(payload["memory_enabled"])
    }

    private func settings(revision: UInt64 = 7, hostID: String = "host-a") -> [String: Any] {
        ["host_identity": ["id": hostID], "model_profiles": [
            ["profile_id": "profile-a", "revision": revision, "display_name": "First route", "model_name": "same-model"],
            ["profile_id": "profile-b", "revision": revision, "display_name": "Second route", "model_name": "same-model"]
        ]]
    }

    func testDirectoryPreservesDistinctProfilesWithSameWireModelName() throws {
        let profiles = try MacRemoteModelCatalog.decode(settings: settings(), expectedHostID: "host-a")
        XCTAssertEqual(profiles.count, 2)
        XCTAssertEqual(profiles.map(\.profileID), ["profile-a", "profile-b"])
        XCTAssertEqual(profiles.map(\.hostID), ["host-a", "host-a"])
        XCTAssertEqual(profiles.map(\.revision), [7, 7])
    }

    func testCatalogFailsClosedForWrongHostMissingDirectoryAndDuplicateIdentity() throws {
        XCTAssertThrowsError(try MacRemoteModelCatalog.decode(settings: settings(), expectedHostID: "host-b"))
        XCTAssertThrowsError(try MacRemoteModelCatalog.decode(settings: ["host_identity": ["id": "host-a"]], expectedHostID: nil))
        var duplicate = settings()
        let record = try XCTUnwrap((duplicate["model_profiles"] as? [[String: Any]])?.first)
        duplicate["model_profiles"] = [record, record]
        XCTAssertThrowsError(try MacRemoteModelCatalog.decode(settings: duplicate, expectedHostID: nil))
        for revision in [0, -1, 1.5] {
            var invalid = record
            invalid["revision"] = revision
            duplicate["model_profiles"] = [invalid]
            XCTAssertThrowsError(try MacRemoteModelCatalog.decode(settings: duplicate, expectedHostID: nil))
        }
    }

    func testIntentCarriesSparseGenerationOverridesWithoutRouteOrCapabilityAuthority() throws {
        let profile = try XCTUnwrap(MacRemoteModelCatalog.decode(settings: settings(), expectedHostID: nil).first)
        var overrides = MacConversationModelOverride()
        overrides.wireProtocol = .anthropic
        XCTAssertThrowsError(try MacRemoteModelIntent(profile: profile, tier: "balanced", overrides: overrides))
        overrides.wireProtocol = nil
        overrides.temperature = 0
        overrides.topP = 0
        overrides.seed = .value(0)
        overrides.maxOutputTokens = 4096
        overrides.stopSequences = "one\n\ntwo"
        let intent = try MacRemoteModelIntent(profile: profile, tier: "balanced", overrides: overrides)
        let payload = try intent.payload()
        XCTAssertEqual(Set(payload.keys), ["profile_id", "expected_revision", "tier", "overrides"])
        XCTAssertEqual(payload["profile_id"] as? String, "profile-a")
        XCTAssertEqual(payload["expected_revision"] as? UInt64, 7)
        let fields = try XCTUnwrap(payload["overrides"] as? [String: Any])
        XCTAssertEqual(Set(fields.keys), ["temperature", "top_p", "max_output_tokens", "sampling_override", "generation_options"])
        XCTAssertEqual(fields["temperature"] as? Double, 0)
        XCTAssertEqual(fields["top_p"] as? Double, 0)
        let generation = try XCTUnwrap(fields["generation_options"] as? [String: Any])
        XCTAssertEqual(generation["seed"] as? Int, 0)
        XCTAssertEqual(generation["stop"] as? [String], ["one", "two"])
        overrides.seed = .value(Int64.max)
        let exact = try MacRemoteModelIntent(profile: profile, tier: "balanced", overrides: overrides).payload()
        let exactFields = try XCTUnwrap(exact["overrides"] as? [String: Any])
        XCTAssertEqual((exactFields["generation_options"] as? [String: Any])?["seed"] as? Int, Int.max)
    }

    func testNativeRemotePayloadUsesCatalogIdentityNotLocalSameNameOrOldRevision() throws {
        let profile = try XCTUnwrap(MacRemoteModelCatalog.decode(settings: settings(), expectedHostID: nil).first)
        let intent = try MacRemoteModelIntent(profile: profile, tier: "concise")
        var payload: [String: Any] = [:]
        try MacRemoteModelPayload.apply(to: &payload, intent: intent, settings: settings(), expectedHostID: "host-a",
            executionBackend: "nola", modelID: "unrelated-local-profile", modelProviderID: "local-provider")
        XCTAssertEqual(payload["expected_host_id"] as? String, "host-a")
        XCTAssertEqual((payload["model_intent"] as? [String: Any])?["profile_id"] as? String, "profile-a")
        XCTAssertNil(payload["model"])
        XCTAssertNil(payload["execution_model"])
        for changedSettings in [settings(revision: 8), settings(hostID: "host-b")] {
            var rejected: [String: Any] = [:]
            XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &rejected, intent: intent,
                settings: changedSettings, expectedHostID: nil, executionBackend: "nola",
                modelID: "same-model", modelProviderID: nil))
            XCTAssertTrue(rejected.isEmpty)
        }
        XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &payload, intent: nil,
            settings: settings(), expectedHostID: "host-a", executionBackend: "nola",
            modelID: "same-model", modelProviderID: nil))
    }

    func testExternalBackendNeverReceivesNolaModelIntent() throws {
        var payload: [String: Any] = [:]
        try MacRemoteModelPayload.apply(to: &payload, intent: nil, settings: [:], expectedHostID: nil,
            executionBackend: "external", modelID: "external-model", modelProviderID: "external-provider")
        XCTAssertEqual(payload["execution_model"] as? String, "external-model")
        XCTAssertNil(payload["model_intent"])
        let profile = try XCTUnwrap(MacRemoteModelCatalog.decode(settings: settings(), expectedHostID: nil).first)
        XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &payload,
            intent: MacRemoteModelIntent(profile: profile, tier: "detailed"), settings: settings(), expectedHostID: nil,
            executionBackend: "external", modelID: "external-model", modelProviderID: nil))
    }
}
