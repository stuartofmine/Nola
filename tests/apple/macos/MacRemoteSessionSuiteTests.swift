// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS remote session, host configuration, model intent, run observation, projection events, submission intents, cancellation receipts, and permission consumption suite.

import CryptoKit
import Foundation
import Network
import XCTest
@testable import Nola

final class MacRemoteHostConfigurationAndIntentSuiteTests: XCTestCase {
    func testHostDraftIsolationAtomicPersistenceAndColdReopenLifecycle() throws {
        var configurationWritten = false
        XCTAssertThrowsError(try MacHostConfigurationPersistence.commit(token: "new", previousToken: "old", writeToken: { _ in
            throw NSError(domain: "fixture-keychain", code: 1)
        }, writeConfiguration: { configurationWritten = true }))
        XCTAssertFalse(configurationWritten)

        var tokens: [String] = []
        XCTAssertThrowsError(try MacHostConfigurationPersistence.commit(token: "new", previousToken: "old", writeToken: { tokens.append($0) }, writeConfiguration: {
            throw NSError(domain: "fixture-database", code: 1)
        }))
        XCTAssertEqual(tokens, ["new", "old"])

        let bound = try XCTUnwrap(URL(string: "http://0.0.0.0:5001"))
        XCTAssertEqual(MacHostAdvertisedAddress.origin(bound, addresses: ["127.0.0.1", "192.168.1.8"], localHostName: "studio")?.absoluteString, "http://192.168.1.8:5001")
        XCTAssertEqual(MacHostAdvertisedAddress.origin(bound, addresses: [], localHostName: "studio")?.host, "studio.local")
        XCTAssertNil(MacHostAdvertisedAddress.origin(bound, addresses: [], localHostName: nil))

        let defaults = UserDefaults.standard
        let obsoleteKeys = [
            "nola.mac.connectedHost.url",
            "nola.mac.connectedHost.id",
            "nola.mac.connectedHost.clientID",
        ]
        let previous = Dictionary(uniqueKeysWithValues: obsoleteKeys.map { ($0, defaults.object(forKey: $0)) })
        defer {
            for key in obsoleteKeys {
                if let value = previous[key] ?? nil { defaults.set(value, forKey: key) }
                else { defaults.removeObject(forKey: key) }
            }
        }
        defaults.set("https://obsolete.invalid", forKey: obsoleteKeys[0])
        defaults.set("obsolete-host", forKey: obsoleteKeys[1])
        defaults.set("obsolete-client", forKey: obsoleteKeys[2])

        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-host-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let database = workspace.appendingPathComponent("nola.db")
        let firstClientID: String
        do {
            let canonical = try MacApplicationPort(databaseURL: database)
            let configuration = MacRemoteHostConfigurationStore(applicationPort: canonical)
            XCTAssertEqual(configuration.displayValues().url, "")
            try configuration.save(url: "https://old.example:5001/v1", token: "old-token", hostID: "old-host")
            let original = try XCTUnwrap(URL(string: "https://old.example:5001/v1"))
            let savedDraft = try XCTUnwrap(configuration.configurationForDraft(url: original, enteredToken: ""))
            XCTAssertEqual(savedDraft.token, "old-token")
            XCTAssertEqual(savedDraft.expectedHostID, "old-host")
            XCTAssertEqual(try configuration.configurationForDraft(url: original, enteredToken: "new-token")?.expectedHostID, "old-host")
            for address in ["https://new.example:5001/v1", "http://old.example:5001/v1",
                            "https://old.example:5002/v1", "https://old.example:5001/other"] {
                let destination = try XCTUnwrap(URL(string: address))
                XCTAssertNil(try configuration.configurationForDraft(url: destination, enteredToken: ""), address)
                let replacement = try XCTUnwrap(configuration.configurationForDraft(url: destination, enteredToken: " new-token "))
                XCTAssertEqual(replacement.token, "new-token")
                XCTAssertNil(replacement.expectedHostID)
                XCTAssertEqual(try configuration.configurationForDraft(url: destination, enteredToken: "new-token",
                    importedIdentity: "pasted-host")?.expectedHostID, "pasted-host")
            }
            let alias = try XCTUnwrap(URL(string: original.absoluteString + "/"))
            XCTAssertThrowsError(try configuration.configurationForDraft(url: alias, enteredToken: "new-token",
                                                                         importedIdentity: "different-host")) { error in
                XCTAssertEqual(error.localizedDescription, String(localized: "The Nola Server identity changed. Remove it and pair again before continuing.", table: "ServerCenter"))
            }
            XCTAssertEqual(try configuration.configurationForDraft(url: alias, enteredToken: "")?.expectedHostID, "old-host")
            XCTAssertEqual(configuration.load()?.baseURL, original)
            XCTAssertEqual(configuration.load()?.token, "old-token")

            try configuration.save(url: "https://host.example/v1", token: "test-token", hostID: "host-1")
            firstClientID = try configuration.clientID()
        }

        let reopened = try MacApplicationPort(databaseURL: database)
        let configuration = MacRemoteHostConfigurationStore(applicationPort: reopened)
        XCTAssertEqual(configuration.displayValues().url, "https://host.example/v1")
        XCTAssertEqual(configuration.displayValues().hostID, "host-1")
        XCTAssertEqual(try configuration.clientID(), firstClientID)
        try configuration.save(url: "", token: "", hostID: nil)

        let finalStore = try MacApplicationPort(databaseURL: database)
        let finalConfiguration = MacRemoteHostConfigurationStore(applicationPort: finalStore)
        XCTAssertFalse(finalConfiguration.isConfigured)
        XCTAssertEqual(finalConfiguration.displayValues().url, "")
        XCTAssertNil(try finalStore.productSetting(key: MacProductSettingKeys.connectedHostConfiguration))
    }

    private func settings(revision: UInt64 = 7, hostID: String = "host-a") -> [String: Any] {
        ["host_identity": ["id": hostID], "model_profiles": [
            ["profile_id": "profile-a", "revision": revision, "display_name": "First route", "model_name": "same-model"],
            ["profile_id": "profile-b", "revision": revision, "display_name": "Second route", "model_name": "same-model"]
        ]]
    }

    func testFrontierResumeAndHostDefaultCatalogValidation() throws {
        let frontier = AppleSubmissionFrontier(protocol_version: 1, source_host_id: "source",
            client_id: "client", conversation_id: "conversation", configuration_revision: 19,
            parent_message_id: "source-message")
        try frontier.validate(host: "source", client: "client", conversation: "conversation", observedParent: "source-message")
        XCTAssertThrowsError(try frontier.validate(host: "foreign", client: "client", conversation: "conversation", observedParent: "source-message"))
        XCTAssertThrowsError(try frontier.validate(host: "source", client: "other", conversation: "conversation", observedParent: "source-message"))
        XCTAssertThrowsError(try frontier.validate(host: "source", client: "client", conversation: "conversation", observedParent: "stale-message"))
        let freshFrontier = AppleSubmissionFrontier(protocol_version: 1, source_host_id: "source",
            client_id: "client", conversation_id: "conversation", configuration_revision: 3, parent_message_id: nil)
        try freshFrontier.validate(host: "source", client: "client", conversation: "conversation", observedParent: nil)

        let resumePayload = try MacRemoteResumeRequest(conversationID: "conversation", expectedRunID: "run",
            expectedHostID: "host", clientID: "client").payload()
        XCTAssertEqual(Set(resumePayload.keys), ["conversation_id", "expected_run_id", "expected_host_id", "client_id"])
        XCTAssertNil(resumePayload["memory_enabled"])

        let profiles = try MacRemoteModelCatalog.decode(settings: settings(), expectedHostID: "host-a")
        XCTAssertEqual(profiles.count, 2)
        XCTAssertEqual(profiles.map(\.profileID), ["profile-a", "profile-b"])
        XCTAssertEqual(profiles.map(\.hostID), ["host-a", "host-a"])
        XCTAssertEqual(profiles.map(\.revision), [7, 7])

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
        for resolution in [
            ["kind": "unavailable", "profile_id": "profile-a", "origin": "default", "reason": "disabled"],
            ["kind": "invalid_configuration"], ["kind": "no_automatic_candidate"]
        ] as [[String: Any]] {
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
        fresh["default_chat_model_resolution"] = ["kind": "invalid_configuration"]
        try MacRemoteModelPayload.apply(to: &rejected, intent: MacRemoteModelIntent(profile: profile, tier: "balanced"),
            settings: fresh, expectedHostID: "host-a", executionBackend: "nola", modelID: "", modelProviderID: nil)

        var snapshotValue = settings()
        snapshotValue["default_chat_model_resolution"] = ["kind": "resolved", "profile_id": "profile-a", "origin": "default"]
        snapshotValue["default_chat_model_revision"] = 3
        let snapshot = try MacRemoteModelCatalog.decodeSnapshot(settings: snapshotValue, expectedHostID: "host-a")
        XCTAssertEqual(snapshot.defaultResolution, .resolved(profileID: "profile-a", origin: .default))
        XCTAssertEqual(snapshot.defaultRevision, 3)
        snapshotValue["default_chat_model_resolution"] = ["kind": "unavailable", "profile_id": "removed:same-name", "origin": "default", "reason": "missing"]
        XCTAssertEqual(try MacRemoteModelCatalog.decodeSnapshot(settings: snapshotValue, expectedHostID: "host-a").defaultResolution,
                       .unavailable(profileID: "removed:same-name", origin: .default, reason: .missing))
        snapshotValue["default_chat_model_resolution"] = ["kind": "resolved", "profile_id": "missing:same-name", "origin": "default"]
        XCTAssertThrowsError(try MacRemoteModelCatalog.decodeSnapshot(settings: snapshotValue, expectedHostID: "host-a"))
        snapshotValue.removeValue(forKey: "default_chat_model_resolution")
        XCTAssertThrowsError(try MacRemoteModelCatalog.decodeSnapshot(settings: snapshotValue, expectedHostID: "host-a"))
    }

    func testSparseOverridesNativePayloadAndExternalBackendBoundaries() throws {
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

        let conciseIntent = try MacRemoteModelIntent(profile: profile, tier: "concise")
        var nativePayload: [String: Any] = [:]
        try MacRemoteModelPayload.apply(to: &nativePayload, intent: conciseIntent, settings: settings(), expectedHostID: "host-a",
            executionBackend: "nola", modelID: "unrelated-local-profile", modelProviderID: "local-provider")
        XCTAssertEqual(nativePayload["expected_host_id"] as? String, "host-a")
        XCTAssertEqual((nativePayload["model_intent"] as? [String: Any])?["profile_id"] as? String, "profile-a")
        XCTAssertNil(nativePayload["model"])
        XCTAssertNil(nativePayload["execution_model"])
        for changedSettings in [settings(revision: 8), settings(hostID: "host-b")] {
            var rejected: [String: Any] = [:]
            XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &rejected, intent: conciseIntent,
                settings: changedSettings, expectedHostID: nil, executionBackend: "nola",
                modelID: "same-model", modelProviderID: nil))
            XCTAssertTrue(rejected.isEmpty)
        }
        XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &nativePayload, intent: nil,
            settings: settings(), expectedHostID: "host-a", executionBackend: "nola",
            modelID: "same-model", modelProviderID: nil))

        var extPayload: [String: Any] = [:]
        try MacRemoteModelPayload.apply(to: &extPayload, intent: nil, settings: [:], expectedHostID: nil,
            executionBackend: "external", modelID: "external-model", modelProviderID: "external-provider")
        XCTAssertEqual(extPayload["execution_model"] as? String, "external-model")
        XCTAssertNil(extPayload["model_intent"])
        XCTAssertThrowsError(try MacRemoteModelPayload.apply(to: &extPayload,
            intent: MacRemoteModelIntent(profile: profile, tier: "detailed"), settings: settings(), expectedHostID: nil,
            executionBackend: "external", modelID: "external-model", modelProviderID: nil))
    }
}

final class MacRemoteRunObservationAndProjectionEventTests: XCTestCase {
    private func observationFixture(_ edit: (inout [String: Any]) -> Void = { _ in }) throws -> MacRemoteRunObservation {
        var value: [String: Any] = [
            "protocol_version": 1, "conversation_id": "conversation", "client_id": "client",
            "run_id": "run", "assistant_message_id": "root", "status": "running",
            "worker": ["state": "running", "generation": "7"], "recovery": "observe",
            "admission": ["identity": ["source_store_epoch": "epoch", "conversation_id": "conversation",
                "run_id": "run", "assistant_message_id": "root"], "sequence": "0", "record_digest": "digest"],
            "terminal_seal": NSNull(), "permissions": [], "can_submit": false,
        ]
        edit(&value)
        return try JSONDecoder().decode(MacRemoteRunObservation.self,
            from: JSONSerialization.data(withJSONObject: value))
    }

    private func validate(_ value: MacRemoteRunObservation) throws {
        try value.validate(conversationID: "conversation", clientID: "client",
            expectedRunID: "run", assistantMessageID: "root")
    }

    func testRemoteRunObservationIdentityWorkerRecoveryAndContextUsageContracts() throws {
        let base = try observationFixture()
        try validate(base)
        XCTAssertEqual(base.recovery, .observe)
        XCTAssertFalse(base.can_submit)

        try validate(observationFixture {
            $0["worker"] = ["state": "absent", "generation": NSNull()]
            $0["recovery"] = "resume_required"
        })
        XCTAssertThrowsError(try validate(observationFixture { $0["recovery"] = "resume_required" }))
        XCTAssertThrowsError(try validate(observationFixture { $0["can_submit"] = true }))

        for key in ["conversation_id", "client_id", "run_id", "assistant_message_id"] {
            XCTAssertThrowsError(try validate(observationFixture { $0[key] = "foreign" }), key)
        }
        XCTAssertThrowsError(try validate(observationFixture { $0["recovery"] = "terminal" }))
        XCTAssertThrowsError(try validate(observationFixture { $0["protocol_version"] = 2 }))

        try validate(observationFixture {
            $0["worker"] = ["state": "unknown", "generation": "8"]
            $0["recovery"] = "inconsistent"
        })
        XCTAssertThrowsError(try validate(observationFixture {
            $0["worker"] = ["state": "unknown", "generation": "8"]
            $0["recovery"] = "inconsistent"
            $0["permissions"] = [["nonce": "stale"]]
        }))

        let validUsage = try observationFixture { value in
            let admission = value["admission"] as! [String: Any]
            value["context_usage"] = ["protocol_version": 1, "identity": admission["identity"]!,
                "attempt_sequence": "1", "run_cache_usage": NSNull(),
                "admission": admission, "event_sequence": "9007199254740993",
                "record_digest": String(repeating: "a", count: 64),
                "usage": ["used_tokens": "7500", "context_window_tokens": "32768",
                    "prompt_token_limit": "28672", "estimated": false]]
        }
        try validate(validUsage)
        XCTAssertEqual(validUsage.context_usage?.event_sequence, "9007199254740993")
        for key in ["source_store_epoch", "conversation_id", "run_id", "assistant_message_id"] {
            let encoded = try JSONEncoder().encode(XCTUnwrap(validUsage.context_usage))
            var usage = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
            var identity = try XCTUnwrap(usage["identity"] as? [String: Any])
            identity[key] = "foreign"
            usage["identity"] = identity
            XCTAssertThrowsError(try validate(observationFixture { $0["context_usage"] = usage }), key)
        }
    }

    private func userInputWaitLine(conversationID: String = "conversation-1", state: String = "open", revision: Int = 1) throws -> String {
        let object: [String: Any] = ["type": "user_input_wait_changed", "record": [
            "conversation_id": conversationID, "run_id": "run-1", "wait": [
                "wait_id": "input-1", "core_session_id": "core-session", "revision": revision,
                "response_schema_digest": String(repeating: "a", count: 64), "state": ["type": state],
                "question_set": ["questions": [["id": "workspace", "prompt": "Which workspace should I use?",
                    "options": [["id": "current", "label": "Current", "description": "Use the open workspace"],
                                ["id": "choose", "label": "Choose", "description": "Select another folder"]],
                    "allow_freeform": true, "required": true, "max_text_chars": 4096]]]]]]
        return String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
    }

    func testTerminalHintsRemovedProtocolsProgressAndImportInvalidation() throws {
        let valid: [String: Any] = ["type": "assistant_run_terminal_committed_v1",
            "session_id": "conversation", "source_host_id": "host", "run_id": "run",
            "seal_digest": String(repeating: "a", count: 64)]
        let invalidFields: [(String, Any)] = [
            ("session_id", "foreign"), ("source_host_id", ""), ("run_id", " "),
            ("seal_digest", String(repeating: "A", count: 64)), ("seal_digest", true),
            ("canonical_run_id", "another-run"),
        ]
        for (key, value) in invalidFields {
            var object = valid
            object[key] = value
            let line = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
            let event = try XCTUnwrap(MacAgentHostRuntime.decode(line: line, conversationID: "conversation"))
            XCTAssertTrue(event.isError, key)
            XCTAssertNil(event.assistantRunTerminalSealDigest, key)
            XCTAssertNil(event.coreTurnTerminal, key)
        }

        for type in ["core_session_projection_updated", "conversation_projection_committed_v1",
                     "remote_assistant_projection_committed_v1"] {
            let data = try JSONSerialization.data(withJSONObject: ["type": type, "text": "untrusted"])
            let event = try XCTUnwrap(MacAgentHostRuntime.decode(
                line: String(decoding: data, as: UTF8.self), conversationID: "conversation"))
            XCTAssertTrue(event.isError)
            XCTAssertNil(event.assistantJournalCursor)
            XCTAssertNil(event.assistantRunTerminalSealDigest)
            XCTAssertNil(event.coreTurnTerminal)
        }

        let progressLine = #"{"type":"tool_progress","trace":{"schema_version":1,"trace_id":"trace-1","run_id":"run-1","conversation_id":"conversation-1","message_id":"message-1","turn_id":"turn-1","event_id":"event-1","sequence":1,"emitted_at":"2026-09-13T00:00:00Z","kind":"tool.progress","span_id":"effect-1","parent_span_id":null,"status":"running","visibility":"summary","payload":{"phase":"working","summary":"model_retry","detail":"primary/model → fallback/model · quota exhausted","metadata":{"attempt":2,"target_exhausted":true,"failure_code":"quota_exhausted","failure_message":"The provider quota is exhausted."}}}}"#
        let progressEvent = try XCTUnwrap(MacAgentHostRuntime.decode(line: progressLine, conversationID: "conversation-1"))
        XCTAssertEqual(progressEvent.kind, .status)
        XCTAssertEqual(progressEvent.title, "model_retry")
        XCTAssertEqual(progressEvent.detail, "primary/model → fallback/model · quota exhausted")
        XCTAssertEqual(progressEvent.trace?.payload.metadata?.objectValue?["attempt"]?.integerValue, 2)
        XCTAssertEqual(progressEvent.trace?.payload.metadata?.objectValue?["target_exhausted"]?.booleanValue, true)
        XCTAssertEqual(progressEvent.trace?.payload.metadata?.objectValue?["failure_message"]?.stringValue, "The provider quota is exhausted.")

        let wire = #"{"scope":{"type":"conversation","conversation_id":"history"},"cursor":{"scope":{"type":"conversation","conversation_id":"history"},"sequence":1},"identity":{"event_id":"import-event","conversation_id":"history"},"mutation":{"type":"conversation","mutation":{"type":"invalidate_projection","revision":2}},"occurred_at":"2026-09-08T00:00:00Z"}"#
        let importEvent = try JSONDecoder().decode(MacCanonicalConversationProjectionEvent.self, from: Data(wire.utf8))
        XCTAssertEqual(importEvent.mutation, .invalidateProjection(revision: 2))
        XCTAssertEqual(importEvent.sequence, 1)
        XCTAssertEqual(importEvent.conversationID, "history")
        let invalidWire = wire.replacingOccurrences(of: #""event_id":"import-event","conversation_id":"history""#,
            with: #""event_id":"import-event","conversation_id":"foreign""#)
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalConversationProjectionEvent.self, from: Data(invalidWire.utf8)))
    }

    func testUserInputWaitLifecycleRetryStateAndDynamicJSONBounds() throws {
        let openLine = try userInputWaitLine()
        let openEvent = try XCTUnwrap(MacAgentHostRuntime.decode(line: openLine, conversationID: "conversation-1", coreSessionID: "core-session"))
        XCTAssertEqual(openEvent.kind, .userInputRequested)
        XCTAssertFalse(openEvent.isError)
        XCTAssertEqual(openEvent.userInputRequest?.id, "input-1")
        XCTAssertEqual(openEvent.userInputRequest?.runID, "run-1")
        XCTAssertEqual(openEvent.userInputRequest?.questions.first?.id, "workspace")
        XCTAssertEqual(openEvent.userInputRequest?.questions.first?.options.map(\.id), ["current", "choose"])
        XCTAssertEqual(openEvent.userInputRequest?.questions.first?.options.last?.detail, "Select another folder")
        XCTAssertEqual(openEvent.userInputRequest?.questions.first?.allowsFreeform, true)
        XCTAssertEqual(openEvent.userInputRequest?.isOpen, true)
        XCTAssertNil(openEvent.trace)

        let wait = try XCTUnwrap(openEvent.userInputRequest)
        let answer = MacUserInputAnswer(questionID: "workspace", optionID: "current", text: nil)
        let intent = MacUserInputSubmissionIntent(commandID: "stable-answer-command", request: wait, answers: [answer], decline: false)
        XCTAssertTrue(intent.matches(request: wait, answers: [answer], decline: false))
        XCTAssertFalse(intent.matches(request: wait, answers: [answer], decline: true))
        XCTAssertFalse(intent.matches(request: wait, answers: [.init(questionID: "workspace", optionID: "choose", text: nil)], decline: false))
        XCTAssertEqual(try XCTUnwrap(answer.object["value"] as? [String: String]), ["type": "option", "option_id": "current"])

        let foreignEvent = MacAgentHostRuntime.decode(line: try userInputWaitLine(conversationID: "conversation-2"),
            conversationID: "conversation-1", coreSessionID: "core-session")
        XCTAssertEqual(foreignEvent?.kind, .failed)
        XCTAssertTrue(foreignEvent?.isError == true)
        XCTAssertNil(foreignEvent?.userInputRequest)

        let closedEvent = try XCTUnwrap(MacAgentHostRuntime.decode(line: userInputWaitLine(state: "resolved", revision: 2),
            conversationID: "conversation-1", coreSessionID: "core-session"))
        XCTAssertEqual(closedEvent.userInputRequest?.isOpen, false)
        XCTAssertEqual(closedEvent.userInputRequest?.revision, 2)
        XCTAssertEqual(closedEvent.eventID, "user-input:input-1:2")

        let state = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.ActivityState.self,
            from: Data(#""waiting_for_input""#.utf8))
        XCTAssertEqual(state, .waitingForInput)
        XCTAssertNotEqual(state, .waitingForPermission)

        var nested = "null"
        for _ in 0..<(MacCanonicalJSONValue.maximumNestingDepth + 8) {
            nested = "[\(nested)]"
        }
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(nested.utf8)))
    }
}

@MainActor
final class MacRemoteSubmissionIntentTests: XCTestCase {
    private func input(submissionID: String = "run") -> MacPrepareRemoteSubmissionInput {
        .init(conversationID: "conversation", conversationTitle: "Intent", submissionID: submissionID,
            sourceHostID: "fixture-host", clientID: "fixture-client", userMessageID: "\(submissionID):user",
            userPartID: "\(submissionID):user:text", userText: "Original request", userContentJSON: nil,
            assistantMessageID: "\(submissionID):assistant", assistantPartID: "\(submissionID):assistant:text",
            contextCheckpointID: nil, createdAtMs: 1)
    }

    func testPreparedAbandonmentAndBoundSourceAdmissionAcrossColdReload() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("receiver.db")
        let port = try MacApplicationPort(databaseURL: url)

        // Phase 1: Prepared intent has no run or messages and can be abandoned idempotently
        let prepared = try port.prepareRemoteSubmission(input(submissionID: "prepared-run"))
        XCTAssertEqual(prepared.phase, .prepared)
        XCTAssertNil(prepared.request_digest)
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        XCTAssertTrue(try port.loadConversationWindow("conversation").projection.messages.isEmpty)
        let cancelled = try port.abandonPreparedRemoteSubmission(prepared.identity)
        XCTAssertEqual(cancelled.phase, .cancelled)
        XCTAssertNil(try port.remoteSubmissionIntent(conversationID: "conversation"))
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "prepared-run"))
        XCTAssertEqual(try port.abandonPreparedRemoteSubmission(prepared.identity).phase, .cancelled)

        // Phase 2: Bound request needs exact source admission and survives cold reload
        let bound = try port.prepareFixtureRemoteSubmission(input())
        XCTAssertEqual(bound.phase, .bound)
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        XCTAssertTrue(try port.loadConversationWindow("conversation").projection.messages.isEmpty)
        XCTAssertThrowsError(try port.abandonPreparedRemoteSubmission(bound.identity))
        let reopened = try MacApplicationPort(databaseURL: url)
        let restored = try XCTUnwrap(reopened.remoteSubmissionIntent(conversationID: "conversation"))
        XCTAssertEqual(restored.identity, bound.identity)
        XCTAssertEqual(restored.request_digest, bound.request_digest)
        let frozen = try reopened.remoteSubmissionRequest(identity: bound.identity,
            digest: XCTUnwrap(bound.request_digest))
        let semantic = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(frozen)) as? [String: Any])
        let source = try MacAssistantJournalTestFixture(conversationID: "conversation", runID: "run",
            assistantMessageID: "run:assistant", userText: "Original request", userMessageID: "run:user",
            clientID: "fixture-client", semanticRequest: semantic)
        let receipt = try source.submissionReceipt()
        XCTAssertEqual(receipt.request_digest, bound.request_digest)
        let wrong = MacSourceSubmissionReceipt(protocol_version: receipt.protocol_version,
            identity: .init(source_host_id: "foreign-host", client_id: receipt.identity.client_id,
                conversation_id: receipt.identity.conversation_id, submission_id: receipt.identity.submission_id,
                user_message_id: receipt.identity.user_message_id, assistant_message_id: receipt.identity.assistant_message_id),
            request_digest: receipt.request_digest, phase: receipt.phase, admission: receipt.admission,
            rejection_code: receipt.rejection_code)
        XCTAssertThrowsError(try reopened.acceptRemoteSubmissionReceipt(wrong))
        XCTAssertNil(try reopened.activeRunIdentity(conversationID: "conversation"))
        XCTAssertEqual(try reopened.acceptRemoteSubmissionReceipt(receipt).phase, .admitted)
        XCTAssertEqual(try reopened.acceptRemoteSubmissionReceipt(receipt).phase, .admitted)
        XCTAssertEqual(try reopened.activeRunIdentity(conversationID: "conversation")?.runID, "run")
        let wrongDigest = MacSourceSubmissionReceipt(protocol_version: receipt.protocol_version,
            identity: receipt.identity, request_digest: String(repeating: "f", count: 64),
            phase: receipt.phase, admission: receipt.admission, rejection_code: receipt.rejection_code)
        XCTAssertThrowsError(try reopened.adoptObservedRemoteRun(wrongDigest))
        XCTAssertThrowsError(try reopened.adoptObservedRemoteRun(wrong))
        XCTAssertNoThrow(try reopened.adoptObservedRemoteRun(receipt))
        XCTAssertEqual(try reopened.remoteSubmissionIntent(conversationID: "conversation")?.request_digest, bound.request_digest)
        try source.ingestAdmissions(into: reopened)
        _ = try source.finish("Actual source completion")
        _ = try source.commitTerminal(into: reopened)
        XCTAssertNotNil(try reopened.canonicalRunTerminal(conversationID: "conversation", runID: "run"))
        XCTAssertNil(try reopened.remoteSubmissionIntent(conversationID: "conversation"))
    }
}

@MainActor
final class MacRemoteAssistantProjectionTests: XCTestCase {
    func testRejectedTraceCannotCreateMessageOrChangeCallerResult() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: directory.appendingPathComponent("store.db"))),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        let conversation = try XCTUnwrap(model.selectedConversationID)
        func event(_ sequence: Int, run: String = "run", conversationID: String? = nil,
                   kind: String = "phase.changed", traceID: String = "trace") throws -> MacLocalAgentEvent {
            let trace = try XCTUnwrap(MacAgentTraceEnvelope.decode([
                "schema_version": 1, "trace_id": traceID, "run_id": run,
                "conversation_id": conversationID ?? conversation, "turn_id": "turn",
                "event_id": "event-\(sequence)", "sequence": sequence,
                "emitted_at": "2026-10-12T00:00:00Z", "kind": kind,
                "status": "failed", "visibility": "summary", "payload": ["phase": "compacting"],
            ]))
            return MacLocalAgentEvent(kind: .status, text: "", title: nil, detail: nil,
                isError: true, trace: trace)
        }
        var response = "unchanged"
        var assembler = MacSemanticStreamAssembler(identitySeed: "admission")
        var succeeded = true
        func consume(_ event: MacLocalAgentEvent, generation: UInt64 = 0) async {
            await model.consumeTracedAgentEvent(event, prompt: "", conversationID: conversation,
                modelID: "", assistantMessageID: UUID(), generationID: generation,
                permissionUsesLocalRuntime: true, permissionAttachmentURLs: [],
                response: &response, assembler: &assembler, succeeded: &succeeded)
        }
        await consume(try event(1))
        let count = model.selectedMessages.count
        let context = model.contextUsed
        for rejected in [try event(2, run: "foreign", kind: "run.failed"),
                         try event(2, conversationID: "foreign", kind: "run.failed"),
                         try event(1, kind: "run.failed")] {
            await consume(rejected)
            XCTAssertTrue(succeeded)
            XCTAssertEqual(model.selectedMessages.count, count)
            XCTAssertEqual(model.contextUsed, context)
            XCTAssertEqual(response, "unchanged")
        }
        await consume(try event(2, kind: "run.failed"), generation: 99)
        XCTAssertTrue(succeeded)
        XCTAssertEqual(model.selectedMessages.count, count)
        await consume(try event(2, kind: "run.failed"))
        XCTAssertFalse(succeeded, "Rejected input must not consume the legal successor sequence")
        succeeded = true
        await consume(try event(1, run: "next-turn", traceID: "next-trace"))
        let afterSwitch = model.selectedMessages.count
        await consume(try event(3, kind: "run.failed"))
        XCTAssertTrue(succeeded, "A retired Turn cannot fail the current consumer")
        XCTAssertEqual(model.selectedMessages.count, afterSwitch)
        await consume(try event(2, run: "next-turn", kind: "run.failed", traceID: "next-trace"))
        XCTAssertFalse(succeeded)
    }

    private func open(_ port: MacApplicationPort, conversation: String, run: String) throws {
        _ = try port.prepareFixtureRemoteSubmission(.init(conversationID: conversation, conversationTitle: "Live",
            submissionID: run, sourceHostID: "fixture-host", clientID: "fixture-client", userMessageID: "\(run):user",
            userPartID: "\(run):user:text", userText: "Original request", userContentJSON: nil,
            assistantMessageID: "\(run):assistant", assistantPartID: "\(run):assistant:text",
            contextCheckpointID: nil, createdAtMs: 1))
    }

    func testRunningJournalSurvivesColdOpenWithoutEndingRunAndRejectsWrongHost() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("store.db")
        let port = try MacApplicationPort(databaseURL: database)
        let conversation = "live-conversation", run = "live-run"
        try open(port, conversation: conversation, run: run)
        let source = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant")
        let cursor = try source.append("Committed running text")
        try source.ingest(into: port, through: cursor)
        let revision = try port.loadConversationWindow(conversation).projection.revision
        try source.ingest(into: port, through: cursor)
        XCTAssertEqual(try port.loadConversationWindow(conversation).projection.revision, revision,
                       "Replayed committed records must be idempotent")
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, run)
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: conversation, runID: run))
        let cold = try MacApplicationPort(databaseURL: database)
        let window = try cold.loadConversationWindow(conversation)
        XCTAssertTrue(window.assistantJournalHeads.contains(cursor))
        let reference = MacJournalMessageReference(databaseURL: database, cursor: cursor)
        let text = try await MacJournalTextRead.markdown(reference, historicalFallback: "wrong fallback")
        XCTAssertEqual(text, "Committed running text")

        let foreign = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant", sourceHostID: "another-host")
        let foreignCursor = try foreign.append("Must not replace")
        XCTAssertThrowsError(try foreign.ingest(into: cold, through: foreignCursor))
        let unrelated = try MacAssistantJournalTestFixture(conversationID: conversation, runID: "another-run",
            assistantMessageID: "another-assistant")
        let unrelatedCursor = try unrelated.append("Another Run")
        XCTAssertThrowsError(try unrelated.ingest(into: cold, through: unrelatedCursor))
        XCTAssertEqual(try cold.loadConversationWindow(conversation).projection.revision, revision)
    }

    func testDetachedRunningJournalResumesWithoutStartingAnotherRunAndIgnoresLateHint() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("store.db")
        let port = try MacApplicationPort(databaseURL: database)
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        host.allowsEmptyUIQueue = true
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        model.presentationDidAttach()
        let conversation = try XCTUnwrap(model.selectedConversationID)
        model.executionHost = .remote
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([host.fixtureProfile])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
            try await model.waitForModelConfigurationCommit()
        model.draft = "Continue a running answer"
        model.send()
        try await waitUntil { host.runContinuation != nil }
        let continuation = try XCTUnwrap(host.runContinuation)
        let run = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
        let revision = model.selectedTerminalTranscriptRevision
        let text = (1...16).map { "Paragraph \($0) contains enough text to pause the live renderer safely.\n\n" }.joined()
        let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant")
        let cursor = try source.append(text)
        try source.ingest(into: port, through: cursor)
        continuation.yield(source.event(through: cursor))
        try await waitUntil {
            model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference?.cursor == cursor
        }
        model.presentationDidDetach()
        model.presentationDidAttach()
        try await waitUntil {
            model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference?.cursor == cursor
        }
        let reference = try XCTUnwrap(model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference)
        let restored = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
        XCTAssertEqual(restored, text)
        XCTAssertTrue(model.isRunning)
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, revision)
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, run)
        XCTAssertTrue(host.cancellationRequests.isEmpty)
        XCTAssertEqual(host.runRequests, 1)
        _ = try source.finish()
        let seal = try source.commitTerminal(into: port)
        continuation.yield(source.terminalEvent(seal: seal))
        continuation.finish()
        try await waitUntil { !model.isRunning }
        let terminalRevision = model.selectedTerminalTranscriptRevision
        let canonicalRevision = try port.loadConversationWindow(conversation).projection.revision
        let late = source.event(through: cursor)
        var response = text
        var assembler = MacSemanticStreamAssembler(existingText: text, identitySeed: "late-test")
        var succeeded = true
        await model.consumeTracedAgentEvent(late, prompt: "", conversationID: conversation,
            modelID: "", assistantMessageID: try XCTUnwrap(MacCanonicalMessageIdentity.stableID("\(run):assistant")),
            generationID: 1, permissionUsesLocalRuntime: false, permissionAttachmentURLs: [],
            response: &response, assembler: &assembler, succeeded: &succeeded)
        XCTAssertFalse(model.isRunning)
        XCTAssertNil(model.remoteRunRecovery)
        XCTAssertNil(model.remoteRecoveryError)
        XCTAssertTrue(succeeded)
        XCTAssertEqual(response, text)
        XCTAssertEqual(model.selectedTerminalTranscriptRevision, terminalRevision)
        XCTAssertEqual(try port.loadConversationWindow(conversation).projection.revision, canonicalRevision)
        XCTAssertEqual(try repository.canonicalRunTerminal(conversationID: conversation, runID: run)?.result.value.kind, .answered)
    }

    private func waitUntil(_ predicate: () -> Bool, file: StaticString = #filePath,
                           line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while !predicate() && Date() < deadline { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(predicate(), "Expected canonical projection transition", file: file, line: line)
    }
}

@MainActor
final class MacRemoteCancellationReceiptTests: XCTestCase {
    func testColdRoomHTTPRejectionObservesCapturedSourceWithoutResubmitting() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 409, delayed: false, mode: "room_rejected")
        defer { server.terminate() }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("receiver.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        let target = try service.captureSubmissionTarget()
        let bound = try port.prepareFixtureRemoteSubmission(.init(conversationID: "conversation",
            conversationTitle: "Room", submissionID: "original", sourceHostID: "fixture-host", clientID: target.clientID,
            userMessageID: "original:user", userPartID: "original:user:text", userText: "fixture", userContentJSON: nil,
            assistantMessageID: "original:assistant", assistantPartID: "original:assistant:text", contextCheckpointID: nil, createdAtMs: 1))
        // The private Room owner must keep its captured source even if settings
        // change while this immutable execution remains pending.
        try configuration.save(url: "https://changed.invalid", token: "changed-token", hostID: "changed-host")
        var rejected = 0
        let digest = try XCTUnwrap(bound.request_digest)
        let stream = await service.runBoundRoomRequest(identity: bound.identity,
            digest: digest, target: target)
        for try await event in stream {
            if event.remoteSubmissionIntent?.phase == .rejected { rejected += 1 }
        }
        XCTAssertEqual(rejected, 1)
        XCTAssertNil(try port.remoteSubmissionIntent(conversationID: "conversation"))
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "original"))
        let counts = try await requestCounts(baseURL)
        XCTAssertEqual(counts["streams"], 1)
    }

    private func fixturePermissionEvidence() throws -> MacRemotePermissionToolEvidence {
        try XCTUnwrap(MacRemotePermissionToolEvidence.decode([
            "catalog_revision": 2, "tool": ["tool_id": "web_search", "route": "agent_host",
                "schema": ["type": "function", "function": ["name": "web_search", "parameters": ["type": "object"]]]]
        ], toolName: "web_search"))
    }

    func testColdHTTPObservationImportsAdmissionAndOriginalNonceThenActualTerminalWithoutSubmit() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 202, delayed: false, mode: "cold_observe")
        defer { server.terminate() }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("receiver.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        let target = try service.captureSubmissionTarget()
        let first = try await service.observeRun(conversationID: "conversation", expectedRunID: nil,
            assistantMessageID: nil, target: target)
        XCTAssertEqual(first.observation?.run_id, "original")
        let permission = try XCTUnwrap(first.events.compactMap(\.permissionRequest).first)
        XCTAssertEqual(permission.nonce, "original-cold-nonce")
        XCTAssertTrue(first.events.allSatisfy { $0.trace == nil })
        XCTAssertTrue(permission.reason.contains("Review this search"))
        // Configuration edits cannot retarget this admitted Run or nonce.
        try configuration.save(url: "https://changed.invalid", token: "changed-token", hostID: "changed-host")
        // No stream receipt exists. The real decision path must acquire a
        // original Run catalog without consulting current settings or inventing a challenge.
        try await service.resolvePermission(request: permission, allow: false)
        do {
            _ = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
                assistantMessageID: "foreign-root", target: target)
            XCTFail("Observation cannot replace the pinned assistant owner")
        } catch MacRemoteHostError.invalidResponse {}
        let second = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
            assistantMessageID: "original:assistant", target: target)
        XCTAssertEqual(second.observation?.recovery, .terminal)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "original"))
        let cursor = try XCTUnwrap(second.events.compactMap(\.assistantJournalCursor).last)
        let text = try await MacJournalTextRead.markdown(.init(databaseURL: port.databaseURL, cursor: cursor), historicalFallback: "")
        XCTAssertEqual(text, "Cold final body")
        let committedUsage = try XCTUnwrap(second.observation?.context_usage)
        XCTAssertEqual(committedUsage.usage.used_tokens, "7500")
        XCTAssertFalse(committedUsage.usage.estimated)
        let reopened = try MacApplicationPort(databaseURL: port.databaseURL)
        let repository = MacCanonicalConversationRepository(store: reopened)
        let restored = MacWorkspaceModel(initialConversationID: "conversation", canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        restored.remoteObservationForegroundChanged(false)
        await restored.loadSelectedConversationIfNeeded()
        XCTAssertEqual(restored.contextUsed, 7_500)
        XCTAssertEqual(restored.contextWindowTokens, 32_768)
        XCTAssertEqual(restored.contextPromptLimit, 28_672)
        XCTAssertFalse(restored.contextUsageIsEstimated)
        let (data, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("counts"))
        let counts = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Int])
        XCTAssertEqual(counts["streams"], 0)
        XCTAssertEqual(counts["settings"], 0)
        XCTAssertEqual(counts["catalog_reads"], 1)
        let (requestData, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("requests"))
        let envelopes = try XCTUnwrap(JSONSerialization.jsonObject(with: requestData) as? [[String: Any]])
        let decision = try decryptFixtureEnvelope(XCTUnwrap(envelopes.first))
        XCTAssertEqual(decision["nonce"] as? String, "original-cold-nonce")
    }
    func testHTTPRunSendsExplicitMemoryBooleans() async throws {
        for enabled in [false, true] {
            try await exerciseRunningProjection(mode: "partial", expectedBodies: ["Running body"], memoryEnabled: enabled)
        }
    }

    func testRunningHTTPProjectionYieldsWithoutRetiringPermissionReceipt() async throws {
        try await exerciseRunningProjection(mode: "partial", expectedBodies: ["Running body"])
    }

    func testWrongRunAndSessionHTTPProjectionsNeverYield() async throws {
        for mode in ["partial_wrong_run", "partial_wrong_session"] {
            try await exerciseRunningProjection(mode: mode, expectedBodies: [])
        }
    }

    func testWrongHostHTTPProjectionRejectsStreamWithoutRetiringReceipt() async throws {
        try await exerciseRunningProjection(mode: "partial_wrong_host", expectedBodies: [])
    }

    func testTerminalThenLateHTTPPartialNeverYieldsLateBody() async throws {
        try await exerciseRunningProjection(mode: "partial_terminal_late", expectedBodies: ["Running body"])
    }

    func testVerifiedTerminalClosesExecutionBeforeForeignTrailingFrame() async throws {
        try await exerciseRunningProjection(mode: "partial_terminal_foreign_late", expectedBodies: ["Running body"])
    }

    private func exerciseRunningProjection(mode: String, expectedBodies: [String], memoryEnabled: Bool = false) async throws {
        let expectsTerminal = ["partial_terminal_late", "partial_terminal_foreign_late"].contains(mode)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 202, delayed: false, mode: mode)
        defer { if server.isRunning { server.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: "original", generationID: 1))
        try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
        let stream = await service.run(conversationID: "conversation", submissionID: "original",
            userMessageID: "user", assistantMessageID: "original:assistant", prompt: "fixture",
            modelID: "fixture", modelProviderID: nil, modelIntent: nil, memoryEnabled: memoryEnabled, executionBackend: "codex",
            externalSessionID: nil, submission: .init(target: try service.captureSubmissionTarget(), ticket: ticket,
                beforeDispatch: {
                    try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
                    _ = try port.prepareRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Fixture",
                        submissionID: "original", sourceHostID: "fixture-host", clientID: try configuration.clientID(), userMessageID: "user",
                        userPartID: "user:text", userText: "fixture", userContentJSON: nil,
                        assistantMessageID: "original:assistant", assistantPartID: "original:assistant:text",
                        contextCheckpointID: nil, createdAtMs: 1))
                }))
        var bodies: [String] = []
        var terminals = 0
        do {
            for try await event in stream {
                if let cursor = event.assistantJournalCursor {
                    bodies.append(try await MacJournalTextRead.markdown(
                        .init(databaseURL: directory.appendingPathComponent("test.db"), cursor: cursor),
                        historicalFallback: ""))
                }
                if event.assistantRunTerminalSealDigest != nil { terminals += 1 }
            }
            XCTAssertTrue(expectsTerminal, "Nonterminal EOF must not become completion")
        } catch MacRemoteHostError.transportInterrupted {
            XCTAssertFalse(expectsTerminal)
            XCTAssertNotEqual(mode, "partial_wrong_host")
        } catch MacRemoteHostError.invalidResponse {
            XCTAssertTrue(mode.hasPrefix("partial_wrong_"))
        }
        XCTAssertEqual(bodies, expectedBodies)
        let (recorded, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("requests"))
        let requests = try XCTUnwrap(JSONSerialization.jsonObject(with: recorded) as? [[String: Any]])
        let sent = try decryptFixtureEnvelope(XCTUnwrap(requests.first))
        XCTAssertEqual(sent["memory_enabled"] as? Bool, memoryEnabled)
        XCTAssertEqual(terminals, expectsTerminal ? 1 : 0)
        let request = MacRemotePermissionRequest(id: "request", allowConversation: true, protocolVersion: MacRemotePermissionRequest.supportedProtocolVersion,
            nonce: "nonce", hostID: "fixture-host", clientID: try configuration.clientID(), sessionID: "conversation",
            toolName: "web_search", arguments: "{}", callID: "call", reason: "fixture", message: "fixture", expiresAt: nil,
            toolCatalogEvidence: try fixturePermissionEvidence())
        if expectsTerminal {
            do {
                try await service.resolvePermission(request: request, allow: false)
                XCTFail("Only the matching terminal retires the receipt")
            } catch MacRemoteHostError.invalidResponse {
                let counts = try await requestCounts(baseURL)
                XCTAssertEqual(counts["permissions"], 0, "A terminal Run cannot send permission decisions")
            }
        } else {
            // Goes through the retained production receipt and real HTTP path.
            try await service.resolvePermission(request: request, allow: false)
            let counts = try await requestCounts(baseURL)
            XCTAssertEqual(counts["permissions"], 1)
            XCTAssertEqual(counts["catalog_reads"], 1)
        }
    }

    func testCancellationDuringSettingsFetchNeverDispatchesOrCommitsMirror() async throws {
        try await exerciseSubmissionBoundary(failMirror: false)
    }

    func testFailedMirrorCommitNeverSendsHTTPRequest() async throws {
        try await exerciseSubmissionBoundary(failMirror: true)
    }

    func testUnfinishedRunRejectsReplacementAndPreservesExistingPermissionReceipt() async throws {
        try await exerciseSubmissionBoundary(failMirror: false, preserveReceipt: true)
    }

    private func exerciseSubmissionBoundary(failMirror: Bool, preserveReceipt: Bool = false) async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 202, delayed: false,
            mode: preserveReceipt ? "settings_gate_after_first" : "settings_gate")
        defer { if server.isRunning { server.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        if preserveReceipt { try await establishReceipt(service, port: port) }
        let submissionID = preserveReceipt ? "replacement" : "original"
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: submissionID, generationID: 1))
        var mirrorCommits = 0
        let control = MacRemoteSubmissionControl(target: try service.captureSubmissionTarget(), ticket: ticket) {
            mirrorCommits += 1
            throw MacRemoteHostError.invalidResponse
        }
        try port.ensureFixtureConversation(id: "conversation", title: "Receipt")
        let stream = await service.run(conversationID: "conversation", submissionID: submissionID,
            userMessageID: "user", assistantMessageID: "assistant", prompt: "fixture", contextComponents: [],
            modelID: "fixture", modelProviderID: nil, modelIntent: nil, memoryEnabled: false, executionBackend: "codex",
            externalSessionID: nil, submission: control)
        // A running original has no terminal frontier and must reject a new
        // submission before contacting settings. The other cases exercise
        // cancellation and mirror failure after preparation actually suspends.
        if !preserveReceipt {
            try await waitForObservation(baseURL)
            if !failMirror { XCTAssertTrue(ticket.cancelPreparation()) }
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("release"))
        }
        do {
            for try await _ in stream {}
            XCTFail("Submission should fail before transport")
        } catch {
            XCTAssertEqual(ticket.phase, (failMirror || preserveReceipt) ? .rejectedBeforeDispatch : .abandoned)
        }
        XCTAssertEqual(mirrorCommits, failMirror ? 1 : 0)
        let counts = try await requestCounts(baseURL)
        XCTAssertEqual(counts["streams"], preserveReceipt ? 1 : 0)
        XCTAssertEqual(counts["cancels"], 0)
        if preserveReceipt {
            XCTAssertEqual(counts["settings"], 1, "No settings request after an unfinished Run frontier")
            let request = MacRemotePermissionRequest(id: "request", allowConversation: true, protocolVersion: MacRemotePermissionRequest.supportedProtocolVersion,
                nonce: "nonce", hostID: "fixture-host", clientID: try configuration.clientID(),
                sessionID: "conversation", toolName: "web_search", arguments: "{}",
                callID: "call", reason: "fixture", message: "fixture", expiresAt: nil,
                toolCatalogEvidence: try fixturePermissionEvidence())
            try await service.resolvePermission(request: request, allow: false)
            let counts = try await requestCounts(baseURL)
            XCTAssertEqual(counts["permissions"], 1)
        }
    }

    func testFrozenTargetAlsoOwnsSubsequentCancellationAfterDefaultChanges() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (first, firstURL) = try startServer(status: 202, delayed: false)
        let (second, secondURL) = try startServer(status: 202, delayed: false)
        defer { if first.isRunning { first.terminate() }; if second.isRunning { second.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: firstURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        try port.ensureFixtureConversation(id: "conversation", title: "Frozen")
        let target = try service.captureSubmissionTarget()
        try configuration.save(url: secondURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: "original", generationID: 1))
        let stream = await service.run(conversationID: "conversation", submissionID: "original",
            userMessageID: "user", assistantMessageID: "assistant", prompt: "fixture", contextComponents: [],
            modelID: "fixture", modelProviderID: nil, modelIntent: nil, memoryEnabled: false, executionBackend: "codex",
            externalSessionID: nil, submission: .init(target: target, ticket: ticket, beforeDispatch: {
                try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
                _ = try port.prepareRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Frozen",
                    submissionID: "original", sourceHostID: "fixture-host", clientID: target.clientID,
                    userMessageID: "user", userPartID: "user:text", userText: "fixture", userContentJSON: nil,
                    assistantMessageID: "assistant", assistantPartID: "assistant:text", contextCheckpointID: nil, createdAtMs: 1))
            }))
        do { for try await _ in stream {} } catch MacRemoteHostError.transportInterrupted {}
        XCTAssertEqual(ticket.phase, .accepted, "Only the source admission receipt accepts this submission")
        let intent = try XCTUnwrap(port.remoteSubmissionIntent(conversationID: "conversation"))
        try await service.cancelRun(identity: intent.identity)
        let old = try await requestCounts(firstURL)
        let new = try await requestCounts(secondURL)
        XCTAssertEqual(old["streams"], 1)
        XCTAssertEqual(old["cancels"], 1)
        XCTAssertEqual(new["settings"], 0)
        XCTAssertEqual(new["streams"], 0)
        XCTAssertEqual(new["cancels"], 0)
    }

    private func requestCounts(_ baseURL: URL) async throws -> [String: Int] {
        let (data, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("counts"))
        return try JSONDecoder().decode([String: Int].self, from: data)
    }

    func testRejectedCancellationPreservesPermissionReceipt() async throws {
        try await exercise(status: 503, delayed: false)
    }

    func testAcceptedCancellationIsNotTerminalEvidence() async throws {
        try await exercise(status: 202, delayed: false)
    }

    func testLateCancellationResponseCannotRetireReplacementRunReceipt() async throws {
        try await exercise(status: 202, delayed: true)
    }

    func testReadOnlyObservationCatchesUpRealJournalAndPreservesReceipt() async throws {
        try await exercise(status: 202, delayed: false, mode: "observe")
    }

    func testMatchingTerminalProjectionRetiresReceipt() async throws {
        try await exercise(status: 202, delayed: false, mode: "terminal")
    }

    func testLateOldStreamTerminalCannotRetireReplacementReceipt() async throws {
        try await exercise(status: 202, delayed: false, mode: "late_terminal")
    }

    private func exercise(status: Int, delayed: Bool, mode: String = "cancel") async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nola-cancel-receipt-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: status, delayed: delayed, mode: mode)
        defer { if server.isRunning { server.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        if mode == "late_terminal" {
            let original = Task { try await establishReceipt(service, port: port, expectTerminal: true, allowRejectedTerminal: true) }
            try await waitForObservation(baseURL)
            // The source really completed while its execution response was
            // delayed. Import that receipt/seal before admitting a successor.
            _ = try await service.refreshSubmission(conversationID: "conversation", target: service.captureSubmissionTarget(),
                cancel: false, expectedIdentity: nil)
            try await establishReceipt(service, port: port, submissionID: "replacement")
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("release"))
            try await original.value
        } else {
            try await establishReceipt(service, port: port, expectTerminal: mode == "terminal")
        }
        if mode == "observe" {
            let batch = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
                assistantMessageID: "original:assistant", target: service.captureSubmissionTarget())
            XCTAssertEqual(batch.observation?.recovery, .resume_required)
            let cursor = try XCTUnwrap(batch.events.first?.assistantJournalCursor)
            let text = try await MacJournalTextRead.markdown(
                .init(databaseURL: port.databaseURL, cursor: cursor), historicalFallback: "")
            XCTAssertEqual(text, "Running body")
            XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "original"))
        }
        if mode == "cancel" {
        let intent = try XCTUnwrap(port.remoteSubmissionIntent(conversationID: "conversation"))
        let cancellation = Task { try await service.cancelRun(identity: intent.identity) }
        if delayed {
            try await waitForObservation(baseURL)
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("finish-original"))
            _ = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
                assistantMessageID: "original:assistant", target: service.captureSubmissionTarget())
            try await establishReceipt(service, port: port, submissionID: "replacement")
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("release"))
        }
        do {
            try await cancellation.value
            XCTAssertEqual(status, 202)
        } catch {
            XCTAssertEqual(status, 503, "Unexpected cancellation error: \(error)")
        }
        }
        // A denied decision traverses the actual receipt check and HTTP path;
        // no test setter, synthetic grant, or private dictionary inspection.
        let request = MacRemotePermissionRequest(id: "request", allowConversation: true, protocolVersion: MacRemotePermissionRequest.supportedProtocolVersion,
            nonce: "nonce", hostID: "fixture-host", clientID: try configuration.clientID(),
            sessionID: "conversation", toolName: "web_search", arguments: "{}",
            callID: "call", reason: "fixture", message: "fixture", expiresAt: nil,
            toolCatalogEvidence: try fixturePermissionEvidence())
        if mode == "terminal" {
            do {
                try await service.resolvePermission(request: request, allow: false)
                XCTFail("A terminal Run must retire its receipt")
            } catch MacRemoteHostError.invalidResponse {
                let counts = try await requestCounts(baseURL)
                XCTAssertEqual(counts["permissions"], 0, "A terminal Run cannot send permission decisions")
            }
        } else {
            try await service.resolvePermission(request: request, allow: false)
            let counts = try await requestCounts(baseURL)
            XCTAssertEqual(counts["permissions"], 1)
            XCTAssertEqual(counts["catalog_reads"], 1)
        }
    }

    private func waitForObservation(_ baseURL: URL) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while ContinuousClock.now < deadline {
            let (data, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("observed"))
            if String(decoding: data, as: UTF8.self) == "yes" { return }
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTFail("Expected an in-flight request before replacement")
        throw MacRemoteHostError.invalidResponse
    }

    private func establishReceipt(_ service: MacRemoteHostService, port: MacApplicationPort, submissionID: String = "original",
                                  expectTerminal: Bool = false,
                                  allowRejectedTerminal: Bool = false) async throws {
        try port.ensureFixtureConversation(id: "conversation", title: "Receipt")
        let stream = await service.run(conversationID: "conversation", submissionID: submissionID,
            userMessageID: "\(submissionID):user", assistantMessageID: "\(submissionID):assistant",
            prompt: "fixture", contextComponents: [], modelID: "fixture", modelProviderID: nil,
            modelIntent: nil, memoryEnabled: false, executionBackend: "codex", externalSessionID: nil,
            submission: .init(target: try service.captureSubmissionTarget(),
                ticket: .init(identity: .init(conversationID: "conversation", submissionID: submissionID, generationID: 1)),
                beforeDispatch: {
                    if try port.remoteSubmissionIntent(conversationID: "conversation") == nil {
                        try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
                        _ = try port.prepareRemoteSubmission(.init(conversationID: "conversation",
                            conversationTitle: "Receipt", submissionID: submissionID, sourceHostID: "fixture-host", clientID: try service.captureSubmissionTarget().clientID,
                            userMessageID: "\(submissionID):user", userPartID: "\(submissionID):user:text",
                            userText: "fixture", userContentJSON: nil, assistantMessageID: "\(submissionID):assistant",
                            assistantPartID: "\(submissionID):assistant:text", contextCheckpointID: nil, createdAtMs: 1))
                    }
                }))
        do {
            for try await _ in stream {}
            XCTAssertTrue(expectTerminal, "Fixture deliberately disconnects without a Run terminal")
        } catch MacRemoteHostError.transportInterrupted {
            // A superseded stream may intentionally ignore its old seal and
            // reach EOF without completing the replacement Run. The caller's
            // subsequent permission request verifies that replacement ownership.
            XCTAssertTrue(!expectTerminal || allowRejectedTerminal)
            // The real start path already retained its authenticated tool plan.
        } catch {
            // A late stream with no admitted current Run cannot complete or
            // retire the replacement receipt, even when its source has a seal.
            XCTAssertTrue(allowRejectedTerminal, "Unexpected transport failure: \(error)")
        }
    }

    private func decryptFixtureEnvelope(_ envelope: [String: Any]) throws -> [String: Any] {
        XCTAssertEqual(envelope["v"] as? Int, 1)
        XCTAssertEqual(envelope["alg"] as? String, "AES-256-GCM")
        func decode(_ field: String) throws -> Data {
            let text = try XCTUnwrap(envelope[field] as? String)
            var base64 = text.replacingOccurrences(of: "-", with: "+")
                .replacingOccurrences(of: "_", with: "/")
            base64 += String(repeating: "=", count: (4 - base64.count % 4) % 4)
            return try XCTUnwrap(Data(base64Encoded: base64))
        }
        let nonce = try decode("nonce")
        XCTAssertEqual(nonce.count, 12)
        let tag = try decode("tag")
        XCTAssertEqual(tag.count, 16)
        let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: nonce),
            ciphertext: decode("ciphertext"), tag: tag)
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data("fixture-token".utf8))))
        let plaintext = try AES.GCM.open(box, using: key,
            authenticating: Data("nola-mobile-bridge-v1".utf8))
        return try XCTUnwrap(JSONSerialization.jsonObject(with: plaintext) as? [String: Any])
    }

    private func startServer(status: Int, delayed: Bool, mode: String = "cancel") throws -> (CancellationJournalHTTPServer, URL) {
        let server = try CancellationJournalHTTPServer(status: status, delayed: delayed, mode: mode)
        return (server, try server.start())
    }
}

// Actual source Application owns all journal/admission/seal reads. HTTP gates
// control only delivery timing; they never manufacture completion evidence.
nonisolated final class CancellationJournalHTTPServer: @unchecked Sendable {
    private let listener: NWListener
    private let queue = DispatchQueue(label: "nola.cancellation-journal-http")
    private let status: Int
    private let delayed: Bool
    private let mode: String
    private var streams = 0, cancels = 0, settings = 0, observations = 0, permissionDecisions = 0
    private var observed = false, released = false
    private var pending: [() -> Void] = []
    private var requests: [[String: Any]] = []
    private var decodedStreamRequests: [[String: Any]] = []
    private var modelSettings: [String: Any]?
    private var selectionRevisions: [String: Int64] = [:]
    private var selections: [String: String] = [:]
    private var fixtures: [String: MacAssistantJournalTestFixture] = [:]
    private var preparedPlans: [String: [String: Any]] = [:]
    private var runPlans: [String: [String: Any]] = [:]
    private let rejectionDirectory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    private var rejectionSource: MacApplicationServiceClient?
    private var catalogReads = 0
    private var running = true
    private let key = SymmetricKey(data: Data(SHA256.hash(data: Data("fixture-token".utf8))))
    init(status: Int, delayed: Bool, mode: String) throws {
        self.status = status; self.delayed = delayed; self.mode = mode
        listener = try NWListener(using: .tcp, on: .any)
    }
    var isRunning: Bool { queue.sync { running } }
    func replaceModelSettings(_ value: [String: Any]) { queue.sync { modelSettings = value } }
    func terminate() {
        queue.sync { running = false; listener.cancel(); pending.removeAll(); rejectionSource = nil }
        if FileManager.default.fileExists(atPath: rejectionDirectory.path) {
            try? FileManager.default.removeItem(at: rejectionDirectory)
        }
    }
    func start() throws -> URL {
        let ready = DispatchSemaphore(value: 0)
        listener.stateUpdateHandler = { state in
            if case .ready = state { ready.signal() }
            if case .failed = state { ready.signal() }
        }
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { connection.cancel(); return }
            connection.start(queue: self.queue)
            self.read(connection, buffer: Data())
        }
        listener.start(queue: queue)
        guard ready.wait(timeout: .now() + 5) == .success, let port = listener.port,
              let url = URL(string: "http://127.0.0.1:\(port.rawValue)") else {
            listener.cancel(); throw MacRemoteHostError.invalidResponse
        }
        return url
    }
    private func read(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, done, _ in
            guard let self else { connection.cancel(); return }
            var buffer = buffer
            if let data { buffer.append(data) }
            guard buffer.count <= 2 * 1024 * 1024 else { connection.cancel(); return }
            if let split = buffer.range(of: Data("\r\n\r\n".utf8)),
               let header = String(data: buffer[..<split.lowerBound], encoding: .utf8) {
                let lines = header.components(separatedBy: "\r\n")
                let requestLine = lines[0].split(separator: " ")
                let fields = Dictionary(lines.dropFirst().compactMap { line -> (String, String)? in
                    guard let colon = line.firstIndex(of: ":") else { return nil }
                    return (String(line[..<colon]).lowercased(),
                            String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces))
                }, uniquingKeysWith: { _, last in last })
                let count = Int(fields["content-length"] ?? "0") ?? 0
                if requestLine.count >= 2, count >= 0, buffer.count >= split.upperBound + count {
                    self.route(connection, method: String(requestLine[0]), path: String(requestLine[1]),
                               headers: fields, body: buffer.subdata(in: split.upperBound..<(split.upperBound + count)))
                    return
                }
            }
            if done { connection.cancel() } else { self.read(connection, buffer: buffer) }
        }
    }
    private func gate(_ action: @escaping () -> Void) {
        observed = true
        if released { action() } else { pending.append(action) }
    }
    private func route(_ connection: NWConnection, method: String, path: String, headers: [String: String], body: Data) {
        do {
            if method == "GET" {
                switch path {
                case "/requests": try json(connection, value: requests)
                case "/decoded-stream-requests": try json(connection, value: decodedStreamRequests)
                case "/counts": try json(connection, value: ["streams": streams, "cancels": cancels, "settings": settings, "catalog_reads": catalogReads, "permissions": permissionDecisions])
                case "/observed": send(connection, body: Data((observed ? "yes" : "no").utf8))
                case "/finish-original":
                    _ = try fixtures["original"]?.finish()
                    try json(connection, value: [:] as [String: String])
                case "/release":
                    released = true
                    let actions = pending; pending.removeAll()
                    actions.forEach { $0() }
                    try json(connection, value: [:] as [String: String])
                case "/api/mobile/agent/settings":
                    settings += 1
                    let response: () -> Void = { [self] in
                        try? json(connection, value: modelSettings ?? ["host_identity": ["id": "fixture-host"],
                            "network_policy": "allow", "enable_local_dev_tools": false] as [String: Any])
                    }
                    if mode == "settings_gate" || (mode == "settings_gate_after_first" && streams > 0) { gate(response) }
                    else { response() }
                default: send(connection, status: 404, body: Data())
                }
                return
            }
            guard headers["x-nola-mobile-auth"] != nil else { send(connection, status: 401, body: Data()); return }
            let envelope = try JSONSerialization.jsonObject(with: body) as! [String: Any]
            let input = try decrypt(envelope)
            if path.hasSuffix("/submission-frontier") {
                guard input["expected_host_id"] as? String == "fixture-host",
                      let client = input["client_id"] as? String, !client.isEmpty,
                      let conversation = input["conversation_id"] as? String else { throw MacRemoteHostError.invalidResponse }
                // Script a stable source frontier for these transport races.
                // Parent-change/CAS policy has separate paired-source tests;
                // every admission and terminal below still comes from Core.
                let parent = input["assistant_message_id"] as? String
                try encrypted(connection, result: ["protocol_version": 1, "source_host_id": "fixture-host",
                    "client_id": client, "conversation_id": conversation,
                    "configuration_revision": selectionRevisions[conversation] as Any? ?? (parent == nil ? NSNull() : 1 as Any),
                    "parent_message_id": parent as Any? ?? NSNull()])
            } else if path.hasSuffix("/execution-selection") {
                guard input["expected_host_id"] as? String == "fixture-host",
                      let conversation = input["conversation_id"] as? String,
                      let action = input["action"] as? [String: Any], let type = action["type"] as? String else {
                    throw MacRemoteHostError.invalidResponse
                }
                var result: [String: Any]
                if type == "read" {
                    result = selectionRevisions[conversation].map { ["status": "read", "configuration_revision": $0,
                        "model_selection_id": selections[conversation] as Any? ?? NSNull()] } ?? ["status": "not_found"]
                } else {
                    guard ["create", "select"].contains(type), let selection = action["selection"] as? [String: Any] else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    let current = selectionRevisions[conversation]
                    guard type == "create" ? current == nil : (action["expected_revision"] as? NSNumber)?.int64Value == current else {
                        send(connection, status: 409, body: Data()); return
                    }
                    if selection["kind"] as? String == "model" {
                        guard let profile = selection["profile_id"] as? String,
                              let revision = selection["profile_revision"] as? NSNumber,
                              (modelSettings?["model_profiles"] as? [[String: Any]])?.contains(where: {
                                  $0["profile_id"] as? String == profile && ($0["revision"] as? NSNumber) == revision
                              }) == true else { throw MacRemoteHostError.invalidResponse }
                        selections[conversation] = "configured:" + profile
                    } else {
                        guard selection["kind"] as? String == "inherit" else { throw MacRemoteHostError.invalidResponse }
                        selections[conversation] = nil
                    }
                    let next = (current ?? 0) + 1; selectionRevisions[conversation] = next
                    result = ["status": type == "create" ? "created" : "committed", "configuration_revision": next,
                        "model_selection_id": selections[conversation] as Any? ?? NSNull()]
                }
                try encrypted(connection, result: ["source_host_id": "fixture-host", "conversation_id": conversation, "result": result], wrapResult: false)
            } else if path.hasSuffix("/tool-plan/prepare") {
                guard input["expected_host_id"] as? String == "fixture-host", let client = input["client_id"] as? String,
                      let session = input["session_id"] as? String else { throw MacRemoteHostError.invalidResponse }
                let plan = toolPlan(clientID: client, sessionID: session)
                let reference = plan["reference"] as! [String: Any]
                preparedPlans[reference["manifest_id"] as! String] = plan
                try encrypted(connection, result: plan)
            } else if path.hasSuffix("/tool-plan/read") {
                guard input["expected_host_id"] as? String == "fixture-host", input["session_id"] as? String == "conversation",
                      let run = input["expected_run_id"] as? String, let source = fixtures[run],
                      input["assistant_message_id"] as? String == source.assistantMessageID, let plan = runPlans[run],
                      (plan["reference"] as? [String: Any])?["client_id"] as? String == input["client_id"] as? String else {
                    throw MacRemoteHostError.invalidResponse
                }
                catalogReads += 1
                try encrypted(connection, result: plan)
            } else if path.hasSuffix("/stream") {
                streams += 1; requests.append(envelope)
                decodedStreamRequests.append(input)
                let run = input["submission_id"] as? String ?? "original"
                if mode == "room_rejected" {
                    let source = try MacApplicationServiceClient(databaseURL: rejectionDirectory.appendingPathComponent("source.db"))
                    rejectionSource = source
                    let identity: [String: Any] = ["source_host_id": input["expected_host_id"]!, "client_id": input["client_id"]!,
                        "conversation_id": input["conversationId"]!, "submission_id": run,
                        "user_message_id": input["user_message_id"]!, "assistant_message_id": input["assistant_message_id"]!]
                    var semantic = input; semantic.removeValue(forKey: "request_digest")
                    _ = try source.execute(commandData: JSONSerialization.data(withJSONObject: [
                        "type": "register_source_submission", "identity": identity, "request": semantic]))
                    _ = try source.execute(commandData: JSONSerialization.data(withJSONObject: [
                        "type": "reject_source_submission", "identity": identity, "request_digest": input["request_digest"]!,
                        "rejection_code": "dispatch_not_admitted"]))
                    send(connection, status: 409, body: Data("{\"code\":\"host_tool_plan_rejected\"}".utf8))
                    return
                }
                guard let ref = input["tool_plan_ref"] as? [String: Any], let manifest = ref["manifest_id"] as? String,
                      let plan = preparedPlans[manifest], NSDictionary(dictionary: ref).isEqual(to: plan["reference"] as! [String: Any]) else {
                    throw MacRemoteHostError.invalidResponse
                }
                runPlans[run] = plan
                var semantic = input; semantic.removeValue(forKey: "request_digest")
                let source = try MacAssistantJournalTestFixture(conversationID: "conversation", runID: run,
                    assistantMessageID: input["assistant_message_id"] as? String ?? "\(run):assistant",
                    userText: "fixture", userMessageID: input["user_message_id"] as? String ?? "\(run):user",
                    clientID: input["client_id"] as? String ?? "", semanticRequest: semantic)
                guard input["request_digest"] as? String == source.requestDigest else { throw MacRemoteHostError.invalidResponse }
                fixtures[run] = source
                let receiptFrame: [String: Any] = ["type":"submission_receipt", "receipt":try object(source.submissionReceipt())]
                if mode.hasPrefix("partial") || mode == "observe" || mode == "terminal" || (mode == "late_terminal" && streams == 1) {
                    let cursor = try source.append("Running body")
                    var identity = try object(cursor.identity)
                    var live: [String: Any] = ["type": "assistant_journal_committed_v1",
                        "session_id": "conversation", "source_host_id": "fixture-host", "through": try object(cursor)]
                    if mode == "partial_wrong_run" { identity["run_id"] = "other-run" }
                    if mode == "partial_wrong_session" {
                        identity["conversation_id"] = "other-conversation"; live["session_id"] = "other-conversation"
                    }
                    var through = try object(cursor); through["identity"] = identity; live["through"] = through
                    if mode == "partial_wrong_host" { live["source_host_id"] = "wrong-host" }
                    var frames: [[String: Any]] = [receiptFrame] + (mode.hasPrefix("partial") ? [live] : [])
                    if ["partial_terminal_late", "partial_terminal_foreign_late", "terminal", "late_terminal"].contains(mode) {
                        let seal = try source.finish()
                        frames.append(["type": "assistant_run_terminal_committed_v1", "session_id": "conversation",
                            "source_host_id": "fixture-host", "run_id": run, "seal_digest": seal.seal_digest])
                        if mode == "partial_terminal_late" { frames.append(live) }
                        if mode == "partial_terminal_foreign_late" {
                            var foreign = live
                            foreign["source_host_id"] = "wrong-host"
                            frames.append(foreign)
                        }
                    }
                    let bytes = try frames.reduce(into: Data()) { output, frame in
                        output.append(Data("data: ".utf8))
                        output.append(try JSONSerialization.data(withJSONObject: frame))
                        output.append(Data("\n\n".utf8))
                    }
                    let response: () -> Void = { [self] in send(connection, body: bytes, content: "text/event-stream") }
                    if mode == "late_terminal" && streams == 1 { gate(response) } else { response() }
                } else {
                    var bytes = Data("data: ".utf8)
                    bytes.append(try JSONSerialization.data(withJSONObject: receiptFrame)); bytes.append(Data("\n\n".utf8))
                    send(connection, body: bytes, content: "text/event-stream")
                }
            } else if path.hasSuffix("/journal") {
                var command = input
                let operation = command.removeValue(forKey: "operation") as? String ?? ""
                let id = (command["through"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? (command["at"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? ((command["snapshot"] as? [String: Any])?["at"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? (command["seal"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? command["identity"] as? [String: Any]
                let run = command["run_id"] as? String ?? id?["run_id"] as? String ?? "original"
                guard let source = fixtures[run] else {
                    if operation == "terminal_seal" { try encrypted(connection, result: NSNull()); return }
                    throw MacRemoteHostError.invalidResponse
                }
                let operations = ["admission_head": "local_run_message_admission_head", "admissions": "read_run_message_admissions",
                    "context_usage": "local_run_context_usage",
                    "read": "read_assistant_journal", "heads": "local_run_assistant_journal_heads",
                    "open_snapshot": "open_assistant_journal_snapshot", "record_chunk": "read_assistant_journal_record_chunk",
                    "release_snapshot": "release_assistant_journal_snapshot", "terminal_seal": "local_assistant_run_terminal_seal",
                    "terminal_manifest": "read_local_assistant_run_terminal_manifest"]
                guard let type = operations[operation] else { throw MacRemoteHostError.invalidResponse }
                command["type"] = type
                let result = try JSONSerialization.jsonObject(with: source.source.execute(
                    commandData: JSONSerialization.data(withJSONObject: command)), options: [.fragmentsAllowed])
                try encrypted(connection, result: result)
            } else if path.hasSuffix("/submission") {
                if mode == "room_rejected", let source = rejectionSource {
                    let receipt = try JSONSerialization.jsonObject(with: source.execute(commandData: JSONSerialization.data(withJSONObject: [
                        "type": "read_source_submission_receipt", "identity": input["identity"]!,
                        "request_digest": input["request_digest"]!])), options: [.fragmentsAllowed])
                    try encrypted(connection, result: receipt)
                    return
                }
                let identity = input["identity"] as? [String: Any]
                guard let run = identity?["submission_id"] as? String, let source = fixtures[run] else {
                    try encrypted(connection, result: NSNull()); return
                }
                guard input["request_digest"] as? String == source.requestDigest else { throw MacRemoteHostError.invalidResponse }
                try encrypted(connection, result: object(source.submissionReceipt()))
            } else if path.hasSuffix("/observe") {
                observations += 1
                let run = input["expected_run_id"] as? String ?? "original"
                if mode == "cold_observe", fixtures[run] == nil {
                    fixtures[run] = try MacAssistantJournalTestFixture(conversationID: "conversation", runID: run,
                        assistantMessageID: "\(run):assistant", clientID: input["client_id"] as? String ?? "",
                        contextUsage: .init(used: 7_500, window: 32_768, promptLimit: 28_672))
                    runPlans[run] = toolPlan(clientID: input["client_id"] as? String ?? "", sessionID: "conversation")
                }
                guard input["expected_host_id"] as? String == "fixture-host", let source = fixtures[run] else {
                    send(connection, status: 409, body: Data()); return
                }
                let admission = try JSONSerialization.jsonObject(with: source.source.execute(commandData:
                    JSONSerialization.data(withJSONObject: ["type":"local_run_message_admission_head",
                        "conversation_id":"conversation", "run_id":run]))) as! [String: Any]
                let identity = admission["identity"] as! [String: Any]
                if mode == "cold_observe" && observations > 1 { _ = try source.finish("Cold final body") }
                let seal = try JSONDecoder().decode(MacAssistantRunTerminalSeal?.self, from: source.source.execute(commandData:
                    JSONSerialization.data(withJSONObject: ["type":"local_assistant_run_terminal_seal",
                        "conversation_id":"conversation", "run_id":run])))
                let permissions: [[String: Any]] = mode == "cold_observe" && seal == nil ? [[
                    "type":"permission_request", "allow_conversation": true, "protocol_version": 1, "id":"cold-request", "request_id":"cold-request",
                    "nonce":"original-cold-nonce", "host_id":"fixture-host", "client_id":input["client_id"]!,
                    "session_id":"conversation", "canonical_run_id":run, "call_id":"call", "tool_name":"web_search",
                    "expires_at":Int64(Date().timeIntervalSince1970) + 300, "safe_review_text":"Review this search",
                    "tool_catalog_evidence": ["catalog_revision": 2, "tool": ["tool_id": "web_search", "route": "agent_host",
                        "schema": ["type": "function", "function": ["name": "web_search", "parameters": ["type": "object"]]]]],
                    "permission_instructions":[]]] : []
                let encodedSeal: Any = try seal.map { try object($0) as Any } ?? NSNull()
                let contextUsage: Any = try source.committedContextUsage().map { try object($0) as Any } ?? NSNull()
                let generation: Any = permissions.isEmpty ? NSNull() : "1" as Any
                try encrypted(connection, result: ["protocol_version":1, "conversation_id":"conversation",
                    "client_id":input["client_id"]!, "run_id":run,
                    "assistant_message_id":identity["assistant_message_id"]!, "status":seal == nil ? "running" : "sealed",
                    "worker":["state":permissions.isEmpty ? "absent" : "running", "generation":generation],
                    "recovery":seal != nil ? "terminal" : permissions.isEmpty ? "resume_required" : "observe",
                    "admission":admission, "terminal_seal":encodedSeal,
                    "context_usage":contextUsage,
                    "permissions":permissions, "can_submit":seal != nil,
                    "submission_receipt":try object(source.submissionReceipt())] as [String: Any])
            } else if path.hasSuffix("/cancel") {
                guard let identity = input["identity"] as? [String: Any],
                      let run = identity["submission_id"] as? String, let source = fixtures[run],
                      identity["assistant_message_id"] as? String == source.assistantMessageID else {
                    send(connection, status: 409, body: Data()); return
                }
                cancels += 1; observed = true
                let response: () -> Void = { [self] in send(connection, status: status, body: Data("{}".utf8)) }
                if delayed { gate(response) } else { response() }
            } else if path.hasSuffix("/permission") { permissionDecisions += 1; requests.append(envelope); try json(connection, value: [:] as [String: String]) }
            else { send(connection, status: 404, body: Data()) }
        } catch { send(connection, status: 500, body: Data(String(describing: error).utf8)) }
    }
    private func toolPlan(clientID: String, sessionID: String) -> [String: Any] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        return ["schema_version": 1,
                "reference": ["manifest_id": UUID().uuidString, "host_id": "fixture-host", "client_id": clientID,
                              "session_id": sessionID, "catalog_revision": String(repeating: "a", count: 64)],
                "issued_at_ms": now, "expires_at_ms": now + 60_000, "tools": []]
    }

    private func object<T: Encodable>(_ value: T) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as! [String: Any]
    }
    private func decrypt(_ envelope: [String: Any]) throws -> [String: Any] {
        let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: decode(envelope["nonce"] as! String)),
            ciphertext: decode(envelope["ciphertext"] as! String), tag: decode(envelope["tag"] as! String))
        return try JSONSerialization.jsonObject(with: AES.GCM.open(box, using: key,
            authenticating: Data("nola-mobile-bridge-v1".utf8))) as! [String: Any]
    }
    private func encrypted(_ connection: NWConnection, result: Any, wrapResult: Bool = true) throws {
        let value: Any = wrapResult ? ["source_host_id": "fixture-host", "result": result] : result
        let sealed = try AES.GCM.seal(JSONSerialization.data(withJSONObject: value), using: key,
            authenticating: Data("nola-mobile-bridge-v1".utf8))
        try json(connection, value: ["type": "encrypted", "v": 1, "alg": "AES-256-GCM",
            "nonce": encode(Data(sealed.nonce)), "ciphertext": encode(sealed.ciphertext), "tag": encode(sealed.tag)])
    }
    private func encode(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
    private func decode(_ string: String) throws -> Data {
        let base64 = string.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        guard let data = Data(base64Encoded: base64 + String(repeating: "=", count: (4-base64.count%4)%4)) else {
            throw MacRemoteHostError.invalidResponse
        }
        return data
    }
    private func json(_ connection: NWConnection, value: Any) throws {
        send(connection, body: try JSONSerialization.data(withJSONObject: value))
    }
    private func send(_ connection: NWConnection, status: Int = 200, body: Data, content: String = "application/json") {
        var response = Data("HTTP/1.1 \(status) Fixture\r\nContent-Type: \(content)\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n".utf8)
        response.append(body)
        connection.send(content: response, completion: .contentProcessed { _ in connection.cancel() })
    }
}

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
    func cancelRun(identity: MacRemoteSubmissionIdentity) async throws {
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
