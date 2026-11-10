// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class AppleConversationExecutionOwnershipTests: XCTestCase {
    func testSourceOwnershipRequiresBothHostAndStoreEpoch() throws {
        func decode(_ value: String) throws -> AppleConversationSourceOwner {
            try JSONDecoder().decode(AppleConversationSourceOwner.self, from: Data(value.utf8))
        }
        XCTAssertEqual(try decode(#"{"kind":"remote","source_host_id":"host","source_store_epoch":"epoch"}"#),
            .remote(hostID: "host", storeEpoch: "epoch"))
        XCTAssertNotEqual(AppleConversationSourceOwner.remote(hostID: "host", storeEpoch: "old"),
            .remote(hostID: "host", storeEpoch: "new"))
        XCTAssertEqual(try decode(#"{"kind":"local"}"#), .local)
        XCTAssertEqual(try decode(#"{"kind":"unavailable"}"#), .unavailable)
        for invalid in [#"{"kind":"remote","source_host_id":"host"}"#,
            #"{"kind":"remote","source_host_id":"host","source_store_epoch":" "}"#,
            #"{"kind":"local","source_host_id":"host"}"#,
            #"{"kind":"remote","source_host_id":"host","source_store_epoch":"epoch","legacy":true}"#] {
            XCTAssertThrowsError(try decode(invalid))
        }
    }

    func testCurrentOwnershipIsRequiredAndTaskIdentityIsExact() throws {
        let expected = AppleTaskChildDestination(parentConversationID: "parent", parentRunID: "run",
            taskID: "task", childConversationID: "child")
        let raw = Data(#"{"kind":"task_child","parent_conversation_id":"parent","parent_run_id":"run","task_id":"task","child_conversation_id":"child"}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(AppleConversationExecutionOwnership.self, from: raw), .taskChild(expected))
        let foreign = AppleTaskChildDestination(parentConversationID: "parent", parentRunID: "other",
            taskID: "task", childConversationID: "child")
        XCTAssertNotEqual(AppleConversationExecutionOwnership.taskChild(expected), .taskChild(foreign))
        for invalid in [#"{}"#, #"{"kind":"legacy"}"#, #"{"kind":"task_child","parent_conversation_id":"parent","parent_run_id":"run","task_id":"task"}"#] {
            XCTAssertThrowsError(try JSONDecoder().decode(AppleConversationExecutionOwnership.self, from: Data(invalid.utf8)))
        }
        XCTAssertEqual(try JSONDecoder().decode(AppleConversationExecutionOwnership.self,
            from: Data(#"{"kind":"unavailable"}"#.utf8)), .unavailable)
    }

    func testReadRequestUsesExactScopeRatherThanResultURI() throws {
        let destination = AppleTaskChildDestination(parentConversationID: "parent", parentRunID: "run",
            taskID: "task", childConversationID: "child")
        let request = AppleTaskChildWindowRequest(destination: destination, beforeOrdinal: nil,
            limit: 30, contentByteLimit: 65536)
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertEqual(fields["parent_run_id"] as? String, "run")
        XCTAssertEqual(fields["task_id"] as? String, "task")
        XCTAssertEqual(fields["child_conversation_id"] as? String, "child")
        XCTAssertTrue(fields["before_ordinal"] is NSNull)
        XCTAssertNil(fields["result_ref"])
        XCTAssertFalse(AppleTaskChildDestination(parentConversationID: "same", parentRunID: "run",
            taskID: "task", childConversationID: "same").hasValidIdentity)
    }
}
