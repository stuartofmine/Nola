// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacEditedResendTests: XCTestCase {
    func testExpertConfigurationOnlyChangesExplicitSelections() throws {
        let original: MacCanonicalJSONValue = .object([
            "modelSelectionID": .string("configured:original"),
            "selectedPromptTemplateID": .string(UUID().uuidString),
            "modelOverrides": .object(["configured:original": .object(["temperature": .number(0.3)])]),
            "language": .string("zh-Hans"), "memoryEnabledOverride": .bool(false),
            "retrievalEnabledOverride": .bool(true), "contextLimitOverride": .number(16000)
        ])
        var draft = try AppleExpertConfigurationDraft(defaults: original)
        XCTAssertEqual(try draft.applying(to: original), original)
        draft.modelID = "configured:changed"
        draft.templateID = ""
        guard case .object(let updated) = try draft.applying(to: original), case .object(let fields) = original else {
            return XCTFail("Expected a defaults object")
        }
        XCTAssertEqual(updated["modelSelectionID"], .string("configured:changed"))
        XCTAssertNil(updated["selectedPromptTemplateID"])
        for key in ["modelOverrides", "language", "memoryEnabledOverride", "retrievalEnabledOverride", "contextLimitOverride"] {
            XCTAssertEqual(updated[key], fields[key])
        }
        draft.modelID = ""; draft.templateID = ""
        guard case .object(let inherited) = try draft.applying(to: original) else { return XCTFail() }
        XCTAssertNil(inherited["modelSelectionID"])
        XCTAssertNil(inherited["selectedPromptTemplateID"])
        XCTAssertEqual(inherited["modelOverrides"], fields["modelOverrides"])
        XCTAssertThrowsError(try draft.applying(to: .null))
    }

    func testExpertConfigurationPreservesExactAbsentAndNullTemplate() throws {
        for initial: MacCanonicalJSONValue in [.object([:]), .object(["selectedPromptTemplateID": .null])] {
            XCTAssertEqual(try AppleExpertConfigurationDraft(defaults: initial).applying(to: initial), initial)
        }
    }

    func testExpertModelAndTemplateUpdateUsesCoreCASAndPreservesExistingContexts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("experts.db"))
        for id in ["original", "changed"] {
            let source: [String: Any] = ["schemaVersion": 1, "endpointID": "fixture-endpoint", "endpointName": "Fixture",
                "baseURL": "https://fixture.invalid/v1", "endpointEnabled": true, "endpointOrder": 0,
                "modelEnabled": true, "modelOrder": 0, "priority": 100, "capabilities": ["chat"],
                "wireProtocol": "chat_completions", "credentialAccount": "fixture-account"]
            try port.upsertModelProfile(.init(id: id, displayName: id, provider: "openai-compatible", modelName: id,
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: "{}", selected: false, enabled: true, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil))
        }
        let service = MacAgentRoomService(runtime: port)
        let initial: MacCanonicalJSONValue = .object(["modelSelectionID": .string("configured:original"), "language": .string("zh-Hans")])
        let directory = try service.createAgent(name: "Original", description: "role", defaults: initial, directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let room = try service.create(title: "Existing context")
        let joined = try service.join(agent: agent, room: room)
        let context = try XCTUnwrap(joined.room.participants.first?.contextConversationID)
        let before = try port.loadConversationMetadata(context)
        var draft = try AppleExpertConfigurationDraft(defaults: initial)
        draft.modelID = "configured:changed"; draft.templateID = UUID().uuidString
        let updatedDefaults = try draft.applying(to: initial)
        let updated = try service.updateAgent(agent, name: "Edited", description: "role", availability: "active",
            directory: directory, defaults: updatedDefaults)
        XCTAssertEqual(updated.agents.first?.defaults, updatedDefaults)
        XCTAssertEqual(try port.loadConversationMetadata(context)?.settingsJSON, before?.settingsJSON)
        XCTAssertThrowsError(try service.updateAgent(agent, name: "Stale", description: "role", availability: "active",
            directory: directory, defaults: initial))
        XCTAssertEqual(try service.directory().agents.first?.defaults, updatedDefaults)
    }

    private func credential(profile: String = "profile", endpoint: String = "https://fixture.invalid/v1?token=scope", model: String = "model", secret: String = "fixture-key") -> MacRunModelCredential {
        .init(profileID: profile, endpointID: "endpoint", account: "private-account", endpoint: endpoint,
              model: model, wireProtocol: .chatCompletions, secretDigest: MacRunModelCredential.digest(secret))
    }
    private var route: MacCanonicalJSONValue {
        .object(["kind": .string("http"), "siwc_account": .null, "protocol": .string(MacModelWireProtocol.chatCompletions.rawValue),
                 "model": .string("model"), "endpoint": .string("https://fixture.invalid/v1"),
                 "endpoint_query_sha256": .string(MacRunModelCredential.digest("token=scope")),
                 "credential_sha256": .string(MacRunModelCredential.digest("fixture-key")), "output_token_field": .string("max_tokens"),
                 "provider": .null, "credential_source": .object(["kind": .string("ephemeral")]),
                 "endpoint_query_source": .object(["kind": .string("ephemeral")])])
    }
    func testFrozenRouteRequiresCurrentExactPrivateCredentialAndUniqueProfile() throws {
        XCTAssertEqual(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: [credential()]), 0)
        for changed in [credential(endpoint: "https://elsewhere.invalid/v1?token=scope"), credential(model: "other"),
                        credential(secret: "rotated"), credential(endpoint: "https://fixture.invalid/v1?token=changed") ] {
            XCTAssertFalse(MacEditedResendModelResolver.matchesHTTP(route, credential: changed))
            XCTAssertThrowsError(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: [changed]))
        }
        XCTAssertThrowsError(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: [credential(), credential(profile: "another")]))
        XCTAssertThrowsError(try MacEditedResendModelResolver.uniqueHTTPMatch(route, credentials: []))
    }

    func testChatGPTResendRequiresTheExactReadyAccountWithoutAnAPIKey() throws {
        let id = UUID().uuidString.lowercased()
        let raw: MacCanonicalJSONValue = .object(["account_id": .string(id), "account_revision": .number(3),
            "credential_ref": .string("siwc:" + id), "client_id": .string("fixture-client"), "subject": .string("fixture-subject")])
        func account(revision: Int64 = 3, state: String = "ready", source: String = "nola_chatgpt", reference: String? = nil) -> MacModelAccountSnapshot {
            .init(revision: revision, account: .init(id: id, display_name: "Fixture",
                source: source, credential_ref: reference ?? "siwc:" + id, state: state))
        }
        XCTAssertTrue(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account()))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(revision: 4)))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(state: "needs_reauthorization")))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(source: "api")))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(raw, account: account(reference: "siwc:foreign")))
        XCTAssertFalse(MacEditedResendModelResolver.matchesSiwcAccount(.null, account: account()))
    }

    func testEditedNativePayloadUsesNewOwnerAndOnlyFrozenModelFields() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let operation = UUID().uuidString.lowercased(), target = UUID().uuidString.lowercased()
        var request = try MacLocalAgentRunRequest(conversationID: target, submissionID: operation,
            userMessageID: operation + ":user", assistantMessageID: operation + ":assistant", prompt: " edited text \n",
            projectURL: environment.sessionWorkspaceURL(for: target), attachmentURLs: [], modelID: "model",
            endpoint: "https://fixture.invalid/v1?token=scope", apiKey: "fixture-key",
            reasoningPreference: "automatic", modelParameters: .webAlignedCloudDefaults, wireProtocol: .chatCompletions,
            permissionMode: .cautious, databaseURLOverride: port.databaseURL, appEnvironment: environment, applicationPort: port)
        request.editedResendOperationID = operation
        request.editedResendModel = .init(version: 1, primary: route, fallbacks: .array([]), generation: .object([
            "max_retries": .number(1), "parallel_tool_calls": .null, "reasoning_selection": .null,
            "reasoning_tier": .string("balanced"), "sampling_override": .bool(false), "generation_options": .object([:]),
            "temperature": .number(0.2), "top_p": .number(0.8), "max_output_tokens": .number(2000),
            "thinking_enabled": .bool(false), "prompt_cache": .bool(false), "prompt_cache_retention": .null,
            "prompt_cache_key_sha256": .null, "prompt_cache_key_prefix_sha256": .null
        ]), output_admission: .object(["version": .number(1), "preference": .string("default"), "configured_context_window": .number(32000),
            "routes": .array([.object(["route": route, "max_input_tokens": .null, "max_output_tokens": .number(2000), "max_total_tokens": .null])]),
            "resolved": .object(["tokens": .number(2000), "source": .string("known_capability_default")])]), model_capability: .object([
            "profile_id": .string("fixture-model"), "revision": .number(1), "instruction_profile": .string("generic"),
            "native_tool_calls": .bool(true), "parallel_tool_calls": .bool(false), "reasoning": .bool(false),
            "reasoning_continuity": .bool(false), "prompt_cache": .bool(false), "vision_input": .bool(false),
            "context_window_tokens": .number(32000), "max_output_tokens": .number(2000),
            "request_continuation": .bool(false), "max_request_continuations": .number(0),
            "stream_text": .bool(true), "stream_reasoning": .bool(true), "stream_tool_calls": .bool(true),
            "direct_tool_capability_ids": .array([])
        ]))
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(payload["edited_resend_operation_id"] as? String, operation)
        XCTAssertEqual(payload["user_message_id"] as? String, operation + ":user")
        XCTAssertEqual(payload["assistant_message_id"] as? String, operation + ":assistant")
        XCTAssertEqual(payload["prompt"] as? String, " edited text \n")
        XCTAssertEqual((payload["context_components"] as? [Any])?.count, 0)
        XCTAssertEqual((payload["attachments"] as? [Any])?.count, 0)
        XCTAssertNil(payload["external_session_id"])
        XCTAssertNil(payload["expected_parent_message_id"] as? String)
        XCTAssertNil(payload["max_output_tokens"], "A default admission must not turn into an explicit preference")
        XCTAssertEqual(payload["temperature"] as? Double, 0.2)
        XCTAssertEqual(payload["context_window_tokens"] as? Int, 32000)
        XCTAssertEqual((payload["mac_launch"] as? [String: Any])?["project_path"] as? String,
                       environment.sessionWorkspaceURL(for: target).standardizedFileURL.resolvingSymlinksInPath().path)
        XCTAssertNil(payload["resume"])
        XCTAssertNil(payload["fresh_resend_policy"], "Only the trusted Host may attest its actual Core configuration")
        for field in ["max_output_tokens", "omitted"] {
            var frozenRoute = try XCTUnwrap(request.editedResendModel)
            guard case .object(var fields) = frozenRoute.primary else { return XCTFail() }
            fields["output_token_field"] = .string(field)
            frozenRoute = .init(version: frozenRoute.version, primary: .object(fields), fallbacks: frozenRoute.fallbacks,
                generation: frozenRoute.generation, output_admission: frozenRoute.output_admission, model_capability: frozenRoute.model_capability)
            request.editedResendModel = frozenRoute
            let adapted = try await MacAgentHostRuntime().initialRequestData(for: request)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: adapted) as? [String: Any])
            XCTAssertEqual(object["chat_output_token_field"] as? String, "auto")
        }

    }
}
