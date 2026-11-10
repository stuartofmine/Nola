// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class RemoteHostModelCatalogTests: XCTestCase {
    @MainActor func testPromptVoiceDoesNotBecomeReasoningTierDuringHostPreparation() async throws {
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:9")!,
            token: "fixture-token", expectedHostId: "host", clientId: "client")) { request in
            let object: [String: Any]
            if request.httpMethod == "GET", request.url?.path.hasSuffix("/agent/settings") == true {
                object = ["host_identity": ["id": "host"],
                    "model_profiles": [["profile_id": "p", "revision": 3, "display_name": "Model", "model_name": "upstream"]],
                    "default_chat_model_resolution": ["kind": "resolved", "profile_id": "p", "origin": "default"],
                    "default_chat_model_revision": 7]
            } else if request.url?.path.hasSuffix("/chat/agent/v2/submission-frontier") == true {
                var encrypted = try WebAgentProvider.encryptMobileJSON(["source_host_id": "host", "result": [
                    "protocol_version": 1, "source_host_id": "host", "client_id": "client", "conversation_id": "conversation",
                    "configuration_revision": NSNull(), "parent_message_id": NSNull(),
                ]], token: "fixture-token")
                encrypted["type"] = "encrypted"
                object = encrypted
            } else {
                XCTAssertTrue(request.url?.path.hasSuffix("/agent/tool-plan/prepare") == true)
                let now = Int64(Date().timeIntervalSince1970 * 1000)
                let plan: [String: Any] = ["schema_version": 1,
                    "reference": ["manifest_id": "plan", "host_id": "host", "client_id": "client", "session_id": "conversation", "catalog_revision": String(repeating: "a", count: 64)],
                    "issued_at_ms": now, "expires_at_ms": now + 60_000, "tools": []]
                var encrypted = try WebAgentProvider.encryptMobileJSON(["source_host_id": "host", "result": plan], token: "fixture-token")
                encrypted["type"] = "encrypted"
                object = encrypted
            }
            let data = try JSONSerialization.data(withJSONObject: object)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream {
                for byte in data { $0.yield(byte) }; $0.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
        }
        for style in PromptResponseStyle.allCases {
            let prepared = try await provider.prepareHostToolPlan(["conversationId": "conversation", "response_style": style.rawValue])
            XCTAssertEqual(prepared["response_style"] as? String, style.rawValue)
            let intent = try XCTUnwrap(prepared["model_intent"] as? [String: Any])
            XCTAssertEqual(intent["tier"] as? String, "balanced")
            XCTAssertEqual(intent["profile_id"] as? String, "p")
        }
    }

    func testSubmissionFrontierRejectsRebasedParentAndForeignOwner() throws {
        func frontier(host: String = "host", revision: Int64? = 7, parent: String? = "observed-parent") -> AppleSubmissionFrontier {
            .init(protocol_version: 1, source_host_id: host, client_id: "client", conversation_id: "conversation",
                  configuration_revision: revision, parent_message_id: parent)
        }
        try frontier().validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent")
        XCTAssertThrowsError(try frontier(parent: "newer-parent").validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        XCTAssertThrowsError(try frontier(host: "foreign").validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        XCTAssertThrowsError(try frontier(revision: nil).validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        XCTAssertThrowsError(try frontier(revision: 0).validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        try frontier(revision: nil, parent: nil).validate(host: "host", client: "client", conversation: "conversation", observedParent: nil)
    }

    private func settings() -> [String: Any] {
        ["host_identity": ["id": "host"],
         "model_profiles": [["profile_id": "p", "revision": 3, "display_name": "Model", "model_name": "upstream"]],
         "default_chat_model_resolution": ["kind": "resolved", "profile_id": "p", "origin": "default"],
         "default_chat_model_revision": 7]
    }
    func testFreezesResolvedHostProfileWithoutProviderSecrets() throws {
        let frozen = try AppleRemoteHostModelCatalog.freezeDefault(settings: settings(), expectedHostID: "host")
        let payload = try frozen.payload(tier: "balanced")
        XCTAssertEqual(Set(payload.keys), ["profile_id", "expected_revision", "tier", "overrides"])
        XCTAssertEqual(payload["profile_id"] as? String, "p")
        XCTAssertEqual(payload["expected_revision"] as? UInt64, 3)
        XCTAssertEqual(frozen.defaultRevision, 7)
        try AppleRemoteHostModelCatalog.validateUnchanged(frozen, settings: settings())
    }
    func testRejectsForeignHostUnavailableOrMissingDefault() {
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: settings(), expectedHostID: "foreign"))
        let resolutions: [[String: Any]] = [[:], ["kind": "no_automatic_candidate"],
            ["kind": "unavailable", "profile_id": "p", "origin": "default", "reason": "disabled"],
            ["kind": "resolved", "profile_id": "missing", "origin": "automatic"],
            ["kind": "resolved", "profile_id": "p", "origin": "conversation"]]
        for resolution in resolutions {
            var value = settings(); value["default_chat_model_resolution"] = resolution
            XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host"))
        }
    }
    func testDefaultAndProfileRevisionChangesCannotBindStaleRequest() throws {
        let frozen = try AppleRemoteHostModelCatalog.freezeDefault(settings: settings(), expectedHostID: "host")
        var value = settings(); value["default_chat_model_revision"] = 8
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.validateUnchanged(frozen, settings: value))
        value = settings(); value["model_profiles"] = [["profile_id": "p", "revision": 4, "display_name": "Model", "model_name": "upstream"]]
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.validateUnchanged(frozen, settings: value))
        XCTAssertEqual(try frozen.payload(tier: "detailed")["expected_revision"] as? UInt64, 3,
                       "frozen payload remains unchanged for original receipt recovery")
    }
    func testMalformedCatalogAndDefaultRevisionAreRejected() {
        for revision in [true, 0, -1, 1.5, "7"] as [Any] {
            var value = settings(); value["default_chat_model_revision"] = revision
            XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host"))
        }
        var value = settings(); let profiles = value["model_profiles"] as! [[String: Any]]
        value["model_profiles"] = profiles + profiles
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host"))
    }
    func testAutomaticDefaultIsHostResolvedAndHasNoInventedRevision() throws {
        var value = settings(); value["default_chat_model_revision"] = NSNull()
        value["default_chat_model_resolution"] = ["kind": "resolved", "profile_id": "p", "origin": "automatic"]
        let frozen = try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host")
        XCTAssertNil(frozen.defaultRevision)
        XCTAssertThrowsError(try frozen.payload(tier: "invented"))
    }
}
