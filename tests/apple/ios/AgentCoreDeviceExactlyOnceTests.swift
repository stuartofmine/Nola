// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies exactly-once Host response reduction at the canonical iOS Application boundary.

import XCTest
@testable import Nola

final class AgentCoreDeviceExactlyOnceTests: XCTestCase {
    func testCanonicalApplicationDoesNotDuplicateLateHostResponseAndSurvivesColdReopen() throws {
        guard IOSApplicationRuntime.isAvailable else {
            throw XCTSkip("NolaCore runtime is unavailable")
        }

        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-exactly-once-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let conversationID = "ios-device-exactly-once"
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let store = try IOSApplicationRuntime(databaseURL: database)
        let application = IOSCanonicalAgentApplication(
            store: store,
            descriptor: .init(
                conversationID: conversationID,
                conversationTitle: "Exactly once",
                identity: identity,
                userText: "Read the selected file once.",
                userContentJSON: nil,
                contextCheckpointID: nil
            )
        )

        let started = try application.begin(
            coreConfig: runtimeConfig(sessionID: conversationID),
            submission: userInput(identity: identity),
            createdAtMs: 100
        )
        if let checkpointRequest = hostRequest(in: started, type: "persist_checkpoint") {
            _ = try application.acceptHostResponse(
                hostResponse(for: checkpointRequest, payload: .object(["type": .string("execution_started")])),
                receivedAtMs: 105
            )
            _ = try application.acceptHostResponse(
                hostResponse(for: checkpointRequest, payload: .object([
                    "type": .string("completed"), "output": .object(["persisted": .bool(true)]),
                    "usage": .null, "full_output_ref": .null, "artifacts": .array([]),
                ]), sequenceOffset: 2),
                receivedAtMs: 110
            )
        }
        let modelRequest = try XCTUnwrap(hostRequest(in: started, type: "model_stream"))
        let acknowledgement = try application.acceptHostResponse(
            hostResponse(for: modelRequest, payload: .object(["type": .string("execution_started")])),
            receivedAtMs: 115
        )
        if case .object(let delta) = acknowledgement {
            XCTAssertNil(delta["snapshot"], "Execution-start acknowledgement uses the Core delta-only contract")
        } else {
            XCTFail("Missing execution acknowledgement")
        }
        _ = try application.acceptHostResponse(
            hostResponse(for: modelRequest, payload: .object([
                "type": .string("model_attempt_started"), "provider": .string("fixture"),
                "model": .string("fixture-model"), "reason": .string("initial"),
            ]), sequenceOffset: 2), receivedAtMs: 120
        )
        let response = try hostResponse(for: modelRequest, payload: modelCompletion(), sequenceOffset: 3)
        _ = try application.acceptHostResponse(response, receivedAtMs: 125)

        let projectionBeforeDuplicate = try store.loadConversationProjectionV1(id: conversationID)
        do {
            _ = try application.acceptHostResponse(response, receivedAtMs: 130)
        } catch {
            // A terminal or consumed effect may be rejected. Rejection and
            // idempotent acceptance are both legal if the projection is unchanged.
        }
        XCTAssertEqual(
            try store.loadConversationProjectionV1(id: conversationID),
            projectionBeforeDuplicate
        )

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(
            try reopened.loadConversationProjectionV1(id: conversationID),
            projectionBeforeDuplicate
        )
        XCTAssertEqual(try reopened.listConversations().map(\.id), [conversationID])
    }

    private func runtimeConfig(sessionID: String) -> AgentCoreJSONValue {
        .object([
            "session_id": .string(sessionID),
            "model": .string("fixture-model"),
            "turn_identity": IOSCanonicalHostTestProfile.turnIdentity(toolIDs: []),
            "initial_items": .array([]),
            "tools": .array([]),
            "host_catalog": .object([
                "host_id": .string("ios-test-host"),
                "revision": .number(1),
                "availability_epoch": .number(1),
                "capabilities": .object([
                    "model.stream": capability("model.stream", route: "model", streaming: true),
                ]),
            ]),
            "permission_rules": .object(["rules": .array([])]),
        ])
    }

    private func capability(_ id: String, route: String, streaming: Bool = false) -> AgentCoreJSONValue {
        .object([
            "id": .string(id), "title": .string(id), "route": .string(route),
            "availability": .string("available"), "exposure": .string("visible"),
            "permission_domain": .null,
            "input_schema": .object(["type": .string("object")]),
            "output_policy": .object([
                "preview_char_limit": .number(4_000),
                "persist_full_output": .bool(true),
                "allow_artifacts": .bool(true),
            ]),
            "supports_streaming": .bool(streaming),
            "supports_argument_streaming": .bool(false),
            "parallelism": .string("serial"),
            "cancellation": .string("cooperative"),
            "replay_safety": .string("recoverable"),
        ])
    }

    private func userInput(identity: IOSCanonicalAgentTurnIdentity) -> AgentCoreJSONValue {
        .object([
            "type": .string("user_input"),
            "items": .array([.object([
                "type": .string("message"),
                "id": .string(identity.userMessageID),
                "role": .string("user"),
                "content": .string("Read the selected file once."),
            ])]),
            "submission_id": .string(identity.submissionID),
            "delivery": .string("current_turn"),
        ])
    }

    private func modelCompletion() -> AgentCoreJSONValue {
        .object([
            "type": .string("model_completed"),
            "items": .array([.object([
                "type": .string("message"),
                "id": .string("assistant-fixture"),
                "role": .string("assistant"),
                "content": .string("The file was read once."),
            ])]),
            "tool_calls": .array([]),
            "usage": .object([
                "input_tokens": .number(8),
                "output_tokens": .number(5),
                "total_tokens": .number(13),
            ]),
        ])
    }

    private func hostRequest(in transition: AgentCoreJSONValue, type: String) -> AgentCoreJSONValue? {
        guard case .object(let object) = transition,
              case .array(let requests)? = object["host_requests"] else { return nil }
        return requests.reversed().first { request in
            guard case .object(let envelope) = request,
                  case .object(let payload)? = envelope["payload"] else { return false }
            return payload["type"] == .string(type)
        }
    }

    private func hostResponse(for request: AgentCoreJSONValue, payload: AgentCoreJSONValue, sequenceOffset: Int = 1) throws -> AgentCoreJSONValue {
        guard case .object(let envelope) = request else {
            throw DeviceAgentCoordinatorError.invalidTransition
        }
        let resumedSequence: Int
        if case .number(let value)? = envelope["resume_after_sequence"] { resumedSequence = Int(value) }
        else { resumedSequence = 0 }
        // Exercise the production envelope constructor so the claimed delivery
        // lease and attempt identity remain part of duplicate/reopen coverage.
        return try DeviceAgentCoordinator.hostResponseEnvelope(
            request: request, payload: payload, sequence: resumedSequence + sequenceOffset
        )
    }
}
