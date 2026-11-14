import Foundation
import XCTest
@testable import Nola

final class MacLocalAgentResumeFrameTests: XCTestCase {
    private func rule(_ resource: String, decision: String = "allow") -> MacCanonicalJSONValue {
        .object(["capability_pattern": .string("render_ui"), "resource_pattern": .string(resource),
                 "decision": .string(decision), "source": .string("conversation")])
    }

    func testColdResumeRetainsFrozenAuthorityAfterAdditionalConversationApprovals() throws {
        let initial = rule("create")
        let later = rule("update")
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial], current: [later, initial])
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial, later], current: [later, initial])
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial, initial], current: [initial])
        try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial], current: [initial, initial])
        // The validation API returns no replacement rules: launch must retain
        // descriptor.permission_rules rather than importing the latest grants.
    }

    func testColdResumeRejectsRevokedChangedAndNewRestrictiveRules() {
        let initial = rule("create")
        let changedSource: MacCanonicalJSONValue = .object(["capability_pattern": .string("render_ui"),
            "resource_pattern": .string("create"), "decision": .string("allow"), "source": .string("global")])
        for current in [[], [changedSource], [rule("update")], [initial, rule("*", decision: "deny")],
                        [initial, rule("*", decision: "ask")], [initial, .null]] as [[MacCanonicalJSONValue]] {
            XCTAssertThrowsError(try MacFrozenLaunchDescriptor.validatePermissionRules(frozen: [initial], current: current))
        }
    }

    private func original() -> [String: Any] {
        ["prompt": "Do not submit this twice", "submission_id": "run",
         "user_message_id": "user", "assistant_message_id": "run:assistant",
         "context_components": [["kind": "platform_data", "content": "old context"]],
         "input_origin": "interactive_user", "attachments": [["path": "/fixture/a.txt", "byte_count": 10]],
         "permission_profile": "safe", "permission_rules": [],
         "model_protocol": "responses", "max_output_tokens": 64000,
         "reasoning_selection": ["kind": "effort", "effort": "high"],
         "reasoning_tier": "concise",
         "external_session_id": "original-executor-session", "api_key": "fixture-not-a-secret"]
    }

    func testResumeFrameCarriesConfigurationWithoutNewUserSubmission() throws {
        let source = try JSONSerialization.data(withJSONObject: original())
        let frame = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "run", assistantMessageID: "run:assistant")
        XCTAssertEqual(frame.last, 0x0a, "desktop transport consumes one newline-delimited frame")
        let envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: frame) as? [String: Any])
        XCTAssertEqual(Set(envelope.keys), ["operation", "session_id", "expected_run_id", "assistant_message_id", "configuration", "expert_template_authority"])
        XCTAssertEqual(envelope["operation"] as? String, "resume")
        XCTAssertEqual(envelope["session_id"] as? String, "conversation")
        XCTAssertEqual(envelope["expected_run_id"] as? String, "run")
        let configuration = try XCTUnwrap(envelope["configuration"] as? [String: Any])
        for key in ["prompt", "submission_id", "user_message_id", "assistant_message_id", "context_components", "selected_experts", "expected_source_store_epoch", "expert_template_authority", "input_origin", "attachments"] {
            XCTAssertNil(configuration[key], key)
        }
        XCTAssertEqual(configuration["max_output_tokens"] as? Int, 64000)
        XCTAssertEqual(configuration["reasoning_tier"] as? String, "concise")
        XCTAssertEqual(configuration["external_session_id"] as? String, "original-executor-session")
        XCTAssertEqual(configuration["permission_profile"] as? String, "safe")
        XCTAssertEqual(configuration["host_attachments"] as? [[String: AnyHashable]], original()["attachments"] as? [[String: AnyHashable]])
        XCTAssertEqual((try JSONSerialization.jsonObject(with: source) as? [String: Any])?["prompt"] as? String,
                       "Do not submit this twice", "transformation must not mutate the owned original frame")
    }

    func testResumeSuppliesCurrentNativeAuthorityOutsideFrozenConfiguration() throws {
        var frozen = original()
        frozen["expert_template_authority"] = ["actor": ["kind": "forged_web_actor"], "request_host_id": "foreign"]
        frozen["selected_experts"] = [["agent_id": "expert", "agent_revision": 1]]
        frozen["expected_source_store_epoch"] = "old-host-store"
        let frame = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: JSONSerialization.data(withJSONObject: frozen),
            conversationID: "conversation", runID: "run", assistantMessageID: "run:assistant")
        let value = try XCTUnwrap(JSONSerialization.jsonObject(with: frame) as? [String: Any])
        let authority = try XCTUnwrap(value["expert_template_authority"] as? [String: Any])
        XCTAssertTrue(authority["request_host_id"] is NSNull)
        let actor = try XCTUnwrap(authority["actor"] as? [String: Any])
        XCTAssertEqual(actor["kind"] as? String, "local_user")
        XCTAssertFalse((actor["id"] as? String ?? "").isEmpty)
        let configuration = try XCTUnwrap(value["configuration"] as? [String: Any])
        XCTAssertNil(configuration["expert_template_authority"]); XCTAssertNil(configuration["selected_experts"])
        XCTAssertNil(configuration["expected_source_store_epoch"])
    }

    func testResumeFrameRejectsDifferentRunOrAssistant() throws {
        let source = try JSONSerialization.data(withJSONObject: original())
        XCTAssertThrowsError(try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "replacement", assistantMessageID: "run:assistant"))
        XCTAssertThrowsError(try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "run", assistantMessageID: "other:assistant"))
    }

    func testAutomationResumeDropsAdmissionAuthorityAndArtifactPaths() throws {
        var input = original()
        input["automation_execution_lease"] = ["lease_token": "one-admission-only"]
        input["automation_execution_spec"] = ["prompt": "original"]
        input["automation_artifacts"] = [["path": "/mutable/source", "sha256": "original"]]
        input["mac_launch"] = ["model_profile_id": "frozen-profile", "model_profile_revision": NSNull(), "input_origin": "scheduled_action"]
        let source = try JSONSerialization.data(withJSONObject: input)
        let frame = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "run", assistantMessageID: "run:assistant")
        let decoded = try XCTUnwrap(JSONSerialization.jsonObject(with: frame) as? [String: Any])
        let configuration = try XCTUnwrap(decoded["configuration"] as? [String: Any])
        for key in ["automation_execution_lease", "automation_execution_spec", "automation_artifacts", "prompt"] {
            XCTAssertNil(configuration[key], "Recovery must not claim, resubmit or re-read artifacts")
        }
        XCTAssertEqual((configuration["mac_launch"] as? [String: Any])?["model_profile_id"] as? String, "frozen-profile")
    }

    func testResumeConfigurationRoundTripKeepsEveryFrozenNonInputField() throws {
        var expected = original()
        let source = try JSONSerialization.data(withJSONObject: expected)
        for key in ["prompt", "submission_id", "user_message_id", "assistant_message_id", "context_components", "input_origin"] {
            expected.removeValue(forKey: key)
        }
        expected["host_attachments"] = expected.removeValue(forKey: "attachments")
        let frame = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: source,
            conversationID: "conversation", runID: "run", assistantMessageID: "run:assistant")
        let envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: frame) as? [String: Any])
        let actual = try XCTUnwrap(envelope["configuration"] as? [String: Any])
        XCTAssertEqual(try JSONSerialization.data(withJSONObject: actual, options: .sortedKeys),
                       try JSONSerialization.data(withJSONObject: expected, options: .sortedKeys))
    }
}
