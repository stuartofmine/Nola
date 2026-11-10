// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacRemoteRunObservationTests: XCTestCase {
    private func fixture(_ edit: (inout [String: Any]) -> Void = { _ in }) throws -> MacRemoteRunObservation {
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

    func testZeroBodyRunningRunRemainsObservable() throws {
        let value = try fixture()
        try validate(value)
        XCTAssertEqual(value.recovery, .observe)
        XCTAssertFalse(value.can_submit)
    }

    func testOnlyAbsentWorkerPermitsExecutionRecovery() throws {
        try validate(fixture {
            $0["worker"] = ["state": "absent", "generation": NSNull()]
            $0["recovery"] = "resume_required"
        })
        XCTAssertThrowsError(try validate(fixture { $0["recovery"] = "resume_required" }))
        XCTAssertThrowsError(try validate(fixture { $0["can_submit"] = true }))
    }

    func testObservationRejectsChangedIdentityAndFalseTerminal() throws {
        for key in ["conversation_id", "client_id", "run_id", "assistant_message_id"] {
            XCTAssertThrowsError(try validate(fixture { $0[key] = "foreign" }), key)
        }
        XCTAssertThrowsError(try validate(fixture { $0["recovery"] = "terminal" }))
        XCTAssertThrowsError(try validate(fixture { $0["protocol_version"] = 2 }))
    }

    func testUnknownWorkerCannotExposePermissionOrBecomeIdle() throws {
        try validate(fixture {
            $0["worker"] = ["state": "unknown", "generation": "8"]
            $0["recovery"] = "inconsistent"
        })
        XCTAssertThrowsError(try validate(fixture {
            $0["worker"] = ["state": "unknown", "generation": "8"]
            $0["recovery"] = "inconsistent"
            $0["permissions"] = [["nonce": "stale"]]
        }))
    }

    func testContextUsageMustHaveTheSameAdmittedRootAndEpoch() throws {
        let valid = try fixture { value in
            let admission = value["admission"] as! [String: Any]
            value["context_usage"] = ["protocol_version": 1, "identity": admission["identity"]!,
                "attempt_sequence": "1", "run_cache_usage": NSNull(),
                "admission": admission, "event_sequence": "9007199254740993",
                "record_digest": String(repeating: "a", count: 64),
                "usage": ["used_tokens": "7500", "context_window_tokens": "32768",
                    "prompt_token_limit": "28672", "estimated": false]]
        }
        try validate(valid)
        XCTAssertEqual(valid.context_usage?.event_sequence, "9007199254740993")
        for key in ["source_store_epoch", "conversation_id", "run_id", "assistant_message_id"] {
            let encoded = try JSONEncoder().encode(XCTUnwrap(valid.context_usage))
            var usage = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
            var identity = try XCTUnwrap(usage["identity"] as? [String: Any])
            identity[key] = "foreign"
            usage["identity"] = identity
            XCTAssertThrowsError(try validate(fixture { $0["context_usage"] = usage }), key)
        }
    }
}
