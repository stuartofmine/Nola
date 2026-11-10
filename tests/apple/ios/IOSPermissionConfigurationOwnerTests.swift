// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSPermissionConfigurationOwnerTests: XCTestCase {
    func testFailedHydrationCanRetryButOldAttemptCannotCompleteNewAttempt() throws {
        let identity = IOSRemoteSubmissionIdentity(source_host_id: "host", client_id: "client",
            conversation_id: "conversation", submission_id: "run", user_message_id: "user", assistant_message_id: "assistant")
        var hydration = IOSPermissionHydrationState()
        let first = try XCTUnwrap(hydration.begin(identity))
        XCTAssertNil(hydration.begin(identity), "Only one read may be in flight")
        XCTAssertTrue(hydration.finish(identity, request: first, succeeded: false))
        let retry = try XCTUnwrap(hydration.begin(identity))
        XCTAssertFalse(hydration.finish(identity, request: first, succeeded: true))
        XCTAssertTrue(hydration.finish(identity, request: retry, succeeded: true))
        XCTAssertNil(hydration.begin(identity), "A successful cold read is not repeated per snapshot")
        hydration.confirm(identity)
        XCTAssertNil(hydration.begin(identity), "Source commit alone does not restart ACK polling")
        hydration.invalidate(forgetSuccess: true)
        XCTAssertNotNil(hydration.begin(identity), "Explicit reconnect can recheck the same owner")
    }

    func testPermissionCompletionFenceRejectsAToBToAStaleReplyAndCleanup() {
        let a = UUID(), b = UUID(), first = UUID(), second = UUID()
        func current(_ owner: UUID?, _ token: UUID?) -> Bool {
            IOSPermissionChangeFence.matches(owner: a, changeID: first, selectedOwner: owner, currentChangeID: token)
        }
        XCTAssertTrue(current(a, first))
        XCTAssertFalse(current(b, first))
        XCTAssertFalse(current(a, nil))
        XCTAssertFalse(current(a, second), "Old success/failure must leave the new edit saving")
        XCTAssertTrue(IOSPermissionChangeFence.matches(owner: a, changeID: second, selectedOwner: a, currentChangeID: second))
    }

    private func value(_ input: Any) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: input, options: .fragmentsAllowed))
    }

    func testOwnerUsesOneCoreSnapshotAndPreservesUnresolvedObservation() throws {
        var calls = 0
        let owner = try IOSPermissionConfigurationOwner.read(conversationID: "conversation", hostID: "host", clientID: "client",
            unresolvedRunID: "run", sourceUnconfirmed: true) { command in
            calls += 1
            guard case .object(let input) = command else { throw IOSApplicationRuntimeError.invalidResponse }
            XCTAssertEqual(input["type"], .string("read_permission_configuration"))
            XCTAssertEqual(input["unresolved_run_ids"], .array([.string("run")]))
            XCTAssertEqual(input["source_unconfirmed"], .bool(true))
            return try self.value(["conversation_id":"conversation", "owner":["kind":"remote",
                "identity":["source_host_id":"host", "client_id":"client", "conversation_id":"conversation",
                    "submission_id":"run", "user_message_id":"user", "assistant_message_id":"assistant"]]])
        }
        XCTAssertEqual(calls, 1)
        guard case .remote(let identity) = owner else { return XCTFail("Expected source owner") }
        XCTAssertEqual(identity.submission_id, "run")
    }

    func testFailedSnapshotCannotBeDisplayedAsInactive() {
        XCTAssertThrowsError(try IOSPermissionConfigurationOwner.read(conversationID: "conversation", hostID: nil, clientID: nil,
            unresolvedRunID: nil, sourceUnconfirmed: false) { _ in
                throw IOSApplicationRuntimeError.rejected("owner changed")
            })
        XCTAssertThrowsError(try IOSPermissionConfigurationOwner.read(conversationID: "conversation", hostID: nil, clientID: nil,
            unresolvedRunID: nil, sourceUnconfirmed: false) { _ in
                try self.value(["conversation_id":"foreign", "owner":["kind":"inactive"]])
            })
    }
}
