// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Locks the iOS client to the shared runtime context accounting contract.

import XCTest
@testable import Nola

final class AgentCoreContextContractTests: XCTestCase {
    func testRemoteMaintenanceReceiptRequiresExactOwnerAndCommittedCheckpoint() throws {
        let payload = #"{"conversation_id":"owner","compacted":true,"checkpoint_revision":3,"committed_at_ms":42,"context":{"used_tokens":100,"context_window_tokens":1000,"usage_percent":10,"replaced_item_count":2,"remaining_item_count":4}}"#
        let receipt = try JSONDecoder().decode(IOSRemoteContextCompactionReceipt.self, from: Data(payload.utf8))
        XCTAssertNoThrow(try receipt.validate(conversationID: "owner"))
        XCTAssertThrowsError(try receipt.validate(conversationID: "another-owner"))
        let mismatched = try JSONDecoder().decode(IOSRemoteContextCompactionReceipt.self,
            from: Data(payload.replacingOccurrences(of: "\"compacted\":true", with: "\"compacted\":false").utf8))
        XCTAssertThrowsError(try mismatched.validate(conversationID: "owner"))
    }

    func testMaintenancePhysicalReceiptPreservesActualCompletionEvidence() {
        let unknown = LLMSummaryPhysicalReceipt(output: .empty)
        XCTAssertNil(unknown.finishReason)
        XCTAssertNil(unknown.hasToolCalls)
        let limited = LLMSummaryPhysicalReceipt(output: ModelOutput(items: [], explain: "partial"),
            finishReason: "length", hasToolCalls: false)
        XCTAssertEqual(limited.finishReason, "length")
        XCTAssertEqual(limited.hasToolCalls, false)
        XCTAssertTrue(isModelOutputLimitFinishReason(limited.finishReason!))
    }

    func testSnapshotUsesTokenUsageInsteadOfMessageCount() throws {
        let json = #"""
        {
          "platform":"ios",
          "execution_mode":"device_local",
          "phase":"running",
          "iteration":7,
          "message_count":320,
          "estimated_context_tokens":98304,
          "context_window_tokens":131072,
          "context_usage_percent":75.0,
          "pending_tool_count":0,
          "awaiting_permission":false,
          "awaiting_host_call":false,
          "usage":{"prompt_tokens":96000,"completion_tokens":2304,"total_tokens":98304}
        }
        """#

        let snapshot = try JSONDecoder().decode(AgentCoreSnapshot.self, from: Data(json.utf8))

        XCTAssertEqual(snapshot.messageCount, 320)
        XCTAssertEqual(snapshot.estimatedContextTokens, 98_304)
        XCTAssertEqual(snapshot.contextWindowTokens, 131_072)
        XCTAssertEqual(snapshot.contextUsagePercent, 75.0)
        XCTAssertNotEqual(snapshot.messageCount, snapshot.estimatedContextTokens)
    }

    func testContextPlanInputEncodesLongHistoryBudgetFields() throws {
        let input = AgentCoreContextPlanInput(
            contextWindowTokens: 262_144,
            tokenThreshold: 157_286,
            promptTokenLimit: 253_952,
            reservedOutputTokens: 8_192,
            estimatedTokens: 180_000,
            messageCount: 480,
            keepRecentMessages: 12,
            adaptiveLongHistory: true
        )

        let data = try JSONEncoder().encode(input)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["estimated_tokens"] as? Int, 180_000)
        XCTAssertEqual(object["message_count"] as? Int, 480)
        XCTAssertEqual(object["keep_recent_messages"] as? Int, 12)
        XCTAssertEqual(object["adaptive_long_history"] as? Bool, true)
    }

    func testContextPrepareInputPreservesStableIDsAndEphemeralMetadata() throws {
        let input = AgentCoreContextPrepareInput(
            messages: [
                AgentCoreContextPrepareMessage(
                    id: "message-1",
                    role: "tool",
                    content: "result",
                    toolCallId: "call-1",
                    ephemeral: true,
                    nolaEphemeral: true
                )
            ],
            platform: .ios,
            budget: AgentCoreContextPrepareBudget(
                contextWindowTokens: 131_072,
                reservedOutputTokens: 8_192,
                keepRecentMessages: 8
            )
        )

        let data = try JSONEncoder().encode(input)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let messages = try XCTUnwrap(object["messages"] as? [[String: Any]])
        XCTAssertEqual(object["schema_version"] as? Int, 2)
        XCTAssertEqual(object["platform"] as? String, "ios")
        XCTAssertEqual(object["mode"] as? String, "normal")
        XCTAssertEqual((object["components"] as? [[String: Any]])?.count, 0)
        XCTAssertEqual((object["tools"] as? [Any])?.count, 0)
        XCTAssertEqual(messages.first?["id"] as? String, "message-1")
        XCTAssertEqual(messages.first?["tool_call_id"] as? String, "call-1")
        XCTAssertEqual(messages.first?["ephemeral"] as? Bool, true)
        XCTAssertEqual(messages.first?["_nola_ephemeral"] as? Bool, true)
    }

    func testContextPrepareResponseDecodesArchiveIDsAndIgnoresExtensions() throws {
        let json = #"""
        {
          "schema_version":2,
          "action":"summary_required",
          "messages":[],
          "tools":[],
          "payload_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "changed":true,
          "reason":"archive_required",
          "metrics":{
            "message_tokens":9000,
            "tool_schema_tokens":1000,
            "total_prompt_tokens":10000,
            "prompt_token_limit":12000,
            "reserved_output_tokens":4096,
            "usage_percent":83.33,
            "before_tokens":14000,
            "after_tokens":10000,
            "trigger_tokens":10800,
            "target_tokens":8400,
            "archive_message_count":2,
            "transaction_count":1,
            "future_metric":1
          },
          "component_metrics":[],
          "archive_request":{
            "archive_id":"archive-1",
            "archive_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "message_indices":[0,1],
            "message_ids":["a","b"],
            "messages":[
              {"id":"a","role":"user","content":"first"},
              {"id":"b","role":"assistant","content":"second"}
            ],
            "target_summary_tokens":1024
          },
          "future_field":{"enabled":true}
        }
        """#

        let response = try JSONDecoder().decode(AgentCoreContextPrepareResponse.self, from: Data(json.utf8))
        XCTAssertEqual(response.action, "summary_required")
        XCTAssertEqual(response.schemaVersion, 2)
        XCTAssertEqual(response.payloadDigest, String(repeating: "a", count: 64))
        XCTAssertEqual(response.metrics.promptTokenLimit, 12_000)
        XCTAssertEqual(response.metrics.totalPromptTokens, 10_000)
        XCTAssertEqual(response.archiveRequest?.archiveId, "archive-1")
        XCTAssertEqual(response.archiveRequest?.messageIds, ["a", "b"])
        XCTAssertEqual(response.archiveRequest?.messages.map(\.content), ["first", "second"])
    }

    func testContextInputDoesNotExposeClientArchivePolicy() throws {
        let input = AgentCoreContextPrepareInput(
            messages: [
                AgentCoreContextPrepareMessage(id: "user-1", role: "user", content: "hello")
            ],
            budget: AgentCoreContextPrepareBudget(
                contextWindowTokens: 32_768,
                reservedOutputTokens: 4_096,
                keepRecentMessages: 8
            )
        )

        let encoded = try JSONEncoder().encode(input)
        let json = try XCTUnwrap(String(data: encoded, encoding: .utf8))
        XCTAssertFalse(json.contains("priority"))
        XCTAssertFalse(json.contains("truncation"))
        XCTAssertFalse(json.contains("archive_boundary"))
    }
}

final class AgentCoreHostInteractionTests: XCTestCase {
    private var responseClock: Int64 = 0
    func testPermissionHostCallAndSessionGrantRoundTrip() async throws {
        guard IOSApplicationRuntime.isAvailable else {
            throw XCTSkip("NolaCore runtime is unavailable")
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("ios-permission-contract-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("product.db"))
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let runtime = IOSCanonicalAgentApplication(store: store, descriptor: .init(
            conversationID: "ios-host-contract", conversationTitle: "Permission contract", identity: identity,
            userText: "Read the selected reference file", userContentJSON: nil, contextCheckpointID: nil))
        responseClock = Int64(Date().timeIntervalSince1970 * 1_000)
        let started = try runtime.begin(coreConfig: makeRuntimeConfig(), submission: .object([
            "type": .string("user_input"),
            "items": .array([
                .object([
                    "type": .string("message"),
                    "id": .string(identity.userMessageID),
                    "role": .string("user"),
                    "content": .string("Read the selected reference file"),
                ]),
            ]),
            "submission_id": .string(identity.submissionID),
            "delivery": .string("current_turn"),
        ]), createdAtMs: responseClock)
        let firstModel = try hostRequest(in: started, type: "model_stream")
        let permissionTransition = try await submitHostResponse(
            runtime,
            request: firstModel,
            payload: modelCompletion(
                callID: "call-1",
                uri: "file:///tmp/reference.txt"
            )
        )
        let permissionRequest = try hostRequest(
            in: permissionTransition,
            type: "request_permission"
        )
        let permissionPayload = try payload(of: permissionRequest)
        XCTAssertEqual(permissionPayload["capability_id"], .string("device_read_file"))
        let permissionID = try string("permission_request_id", in: permissionPayload)

        let grant: AgentCoreJSONValue = .object(["scope": .string("conversation")])
        let allowed = try runtime.submitActive(.object([
            "type": .string("permission_response"),
            "request_id": .string(permissionID),
            "decision": .string("allow_conversation"),
            "grant": grant,
        ]))
        let firstTool = try hostRequest(in: allowed, type: "execute_tool")
        let firstToolPayload = try payload(of: firstTool)
        XCTAssertEqual(firstToolPayload["call_id"], .string("call-1"))
        XCTAssertEqual(firstToolPayload["grant"], grant)

        let resumed = try await submitHostResponse(
            runtime,
            request: firstTool,
            payload: .object([
                "type": .string("tool_completed"),
                "call_id": .string("call-1"),
                "output": .object(["content": .string("reference")]),
                "outcome": .string("succeeded"),
                "full_output_ref": .null,
                "artifacts": .array([]),
            ])
        )
        let secondModel = try hostRequest(in: resumed, type: "model_stream")
        let secondToolTransition = try await submitHostResponse(
            runtime,
            request: secondModel,
            payload: modelCompletion(
                callID: "call-2",
                uri: "file:///tmp/reference.txt"
            )
        )
        let requestTypes = try hostRequestTypes(in: secondToolTransition)
        XCTAssertFalse(
            requestTypes.contains("request_permission"),
            "Unexpected host requests: \(requestTypes)"
        )
        let secondTool = try hostRequest(in: secondToolTransition, type: "execute_tool")
        XCTAssertEqual(try payload(of: secondTool)["call_id"], .string("call-2"))

        let afterSecond = try await submitHostResponse(
            runtime,
            request: secondTool,
            payload: .object([
                "type": .string("tool_completed"),
                "call_id": .string("call-2"),
                "output": .object(["content": .string("reference again")]),
                "outcome": .string("succeeded"),
                "full_output_ref": .null,
                "artifacts": .array([]),
            ])
        )
        let thirdModel = try hostRequest(in: afterSecond, type: "model_stream")
        let differentFile = try await submitHostResponse(
            runtime,
            request: thirdModel,
            payload: modelCompletion(
                callID: "call-3",
                uri: "file:///tmp/second.txt"
            )
        )
        XCTAssertTrue(try hostRequestTypes(in: differentFile).contains("request_permission"))
        XCTAssertFalse(try hostRequestTypes(in: differentFile).contains("execute_tool"))
    }

    func testIOSDeviceExecutorRejectsHostNamespace() async throws {
        let result = await IOSAgentHostExecutor().executeBridge(
            callId: "host-call",
            toolName: "host.shell.exec",
            capability: .process,
            arguments: .object(["command": .string("pwd")]),
            grant: nil
        )
        XCTAssertEqual(result.callId, "host-call")
        XCTAssertTrue(result.isError)
    }

    private func makeRuntimeConfig() -> AgentCoreJSONValue {
        let capabilities: [String: AgentCoreJSONValue] = [
            "model.stream": capability(
                id: "model.stream",
                route: "model",
                permissionDomain: nil,
                supportsStreaming: true
            ),
            "permission.ask": capability(
                id: "permission.ask",
                route: "permission",
                permissionDomain: nil
            ),
            "context.compact": capability(
                id: "context.compact",
                route: "compaction",
                permissionDomain: nil
            ),
            "host.cancel": capability(
                id: "host.cancel",
                route: "tool",
                permissionDomain: nil
            ),
            "device_read_file": capability(
                id: "device_read_file",
                route: "tool",
                permissionDomain: "files_read"
            ),
        ]
        return .object([
            "session_id": .string("ios-host-contract"),
            "model": .string("test-model"),
            "turn_identity": IOSCanonicalHostTestProfile.turnIdentity(toolIDs: ["device_read_file"]),
            "initial_items": .array([]),
            "tools": .array([
                .object([
                    "type": .string("function"),
                    "function": .object([
                        "name": .string("device_read_file"),
                        "description": .string("Read a selected file"),
                        "parameters": .object(["type": .string("object")]),
                    ]),
                ]),
            ]),
            "host_catalog": .object([
                "host_id": .string("ios-test-host"),
                "revision": .number(1),
                "availability_epoch": .number(1),
                "capabilities": .object(capabilities),
            ]),
            "permission_rules": .object(["rules": .array([])]),
        ])
    }

    private func capability(
        id: String,
        route: String,
        permissionDomain: String?,
        supportsStreaming: Bool = false
    ) -> AgentCoreJSONValue {
        .object([
            "id": .string(id),
            "title": .string(id),
            "route": .string(route),
            "availability": .string("available"),
            "exposure": .string("visible"),
            "permission_domain": permissionDomain.map { .string($0) } ?? .null,
            "input_schema": .object(["type": .string("object")]),
            "input_normalization": .object([
                "permission_resource": id == "device_read_file"
                    ? .object([
                        "kind": .string("first_of"),
                        "keys": .array(["path", "url", "uri"].map(AgentCoreJSONValue.string)),
                    ])
                    : .object(["kind": .string("full_input")]),
            ]),
            "output_policy": .object([
                "preview_char_limit": .number(4_000),
                "persist_full_output": .bool(true),
                "allow_artifacts": .bool(true),
            ]),
            "supports_streaming": .bool(supportsStreaming),
            "supports_argument_streaming": .bool(false),
            "parallelism": .string("serial"),
            "cancellation": .string("cooperative"),
            "replay_safety": .string("recoverable"),
        ])
    }

    private func modelCompletion(callID: String, uri: String) -> AgentCoreJSONValue {
        .object([
            "type": .string("model_completed"),
            "items": .array([]),
            "tool_calls": .array([
                .object([
                    "call_id": .string(callID),
                    "name": .string("device_read_file"),
                    "arguments": .object(["uri": .string(uri)]),
                    "permission_scope": .null,
                    "permission_rationale": .null,
                ]),
            ]),
            "usage": .object([
                "input_tokens": .number(8),
                "output_tokens": .number(3),
                "total_tokens": .number(11),
            ]),
        ])
    }

    private func submitHostResponse(
        _ runtime: IOSCanonicalAgentApplication,
        request: AgentCoreJSONValue,
        payload: AgentCoreJSONValue
    ) async throws -> AgentCoreJSONValue {
        responseClock = max(responseClock + 3, Int64(Date().timeIntervalSince1970 * 1_000))
        let receivedAt = responseClock
        guard case .object(let envelope) = request else {
            throw ContractError.invalidEnvelope
        }
        let resumed: Int
        if case .number(let value)? = envelope["resume_after_sequence"] { resumed = Int(value) }
        else { resumed = 0 }
        func response(_ payload: AgentCoreJSONValue, _ sequence: Int) throws -> AgentCoreJSONValue {
            try DeviceAgentCoordinator.hostResponseEnvelope(request: request, payload: payload, sequence: resumed + sequence)
        }
        _ = try runtime.acceptHostResponse(response(.object(["type": .string("execution_started")]), 1), receivedAtMs: receivedAt)
        var finalSequence = 2
        if case .object(let body) = payload, body["type"] == .string("model_completed") {
            _ = try runtime.acceptHostResponse(response(.object(["type": .string("model_attempt_started"),
                "provider": .string("fixture"), "model": .string("test-model"), "reason": .string("initial")]), 2), receivedAtMs: receivedAt + 1)
            finalSequence = 3
        }
        return try runtime.acceptHostResponse(response(payload, finalSequence), receivedAtMs: receivedAt + 2)
    }

    private func hostRequest(
        in envelope: AgentCoreJSONValue,
        type: String
    ) throws -> AgentCoreJSONValue {
        let requests = try hostRequests(in: envelope)
        for request in requests {
            let requestPayload = try payload(of: request)
            if requestPayload["type"] == .string(type) {
                return request
            }
        }
        throw ContractError.missingHostRequest(type)
    }

    private func hostRequestTypes(
        in envelope: AgentCoreJSONValue
    ) throws -> [String] {
        try hostRequests(in: envelope).compactMap { request in
            guard let requestPayload = try? payload(of: request),
                  case .string(let type)? = requestPayload["type"] else {
                return nil
            }
            return type
        }
    }

    private func hostRequests(
        in envelope: AgentCoreJSONValue
    ) throws -> [AgentCoreJSONValue] {
        guard case .object(let transition) = envelope,
              case .array(let requests)? = transition["host_requests"] else {
            throw ContractError.invalidTransition
        }
        return requests
    }

    private func payload(
        of request: AgentCoreJSONValue
    ) throws -> [String: AgentCoreJSONValue] {
        guard case .object(let envelope) = request,
              case .object(let payload)? = envelope["payload"] else {
            throw ContractError.invalidEnvelope
        }
        return payload
    }

    private func string(
        _ key: String,
        in object: [String: AgentCoreJSONValue]
    ) throws -> String {
        guard case .string(let value)? = object[key] else {
            throw ContractError.missingString(key)
        }
        return value
    }

    private enum ContractError: Error {
        case invalidEnvelope
        case invalidTransition
        case missingHostRequest(String)
        case missingString(String)
    }
}

/// Explicit capabilities for canonical Host fixtures, not a production fallback.
/// The advertised tool IDs must exactly match each fixture's model tool surface.
enum IOSCanonicalHostTestProfile {
    static func turnIdentity(toolIDs: [String]) -> AgentCoreJSONValue {
        let profile: AgentCoreJSONValue = .object([
            "profile_id": .string("ios-canonical-host-fixture"), "revision": .number(1),
            "instruction_profile": .string("generic"),
            "native_tool_calls": .bool(!toolIDs.isEmpty), "parallel_tool_calls": .bool(false),
            "reasoning": .bool(false), "reasoning_continuity": .bool(false),
            "prompt_cache": .bool(false), "vision_input": .bool(false),
            "context_window_tokens": .number(128_000), "max_output_tokens": .number(4096),
            "request_continuation": .bool(false), "max_request_continuations": .number(0),
            "stream_text": .bool(true), "stream_reasoning": .bool(false),
            "stream_tool_calls": .bool(!toolIDs.isEmpty),
            "direct_tool_capability_ids": .array(toolIDs.map(AgentCoreJSONValue.string)),
        ])
        let identity = DeviceAgentCoordinator.promptTurnIdentity(stablePromptDigest: nil,
            promptFragmentIds: [], stablePromptFragmentDigests: [:], modelCapability: profile)
        guard case .object(let fields)? = identity else {
            XCTFail("Explicit fixture profile must produce a canonical turn identity")
            return .null
        }
        XCTAssertEqual(fields["model_capability_id"], .string("ios-canonical-host-fixture"))
        XCTAssertEqual(fields["model_capability_revision"], .number(1))
        XCTAssertEqual(fields["model_capability"], profile)
        XCTAssertEqual(Set(toolIDs).count, toolIDs.count, "Fixture tool identities must be unique")
        return .object(fields)
    }
}
