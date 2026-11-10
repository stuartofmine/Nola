// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class RemoteHostToolPlanTests: XCTestCase {
    func testExecutionHostReferenceRequiresIdentityBoundSchemaWithoutClientClock() throws {
        let object: [String: Any] = [
            "schema_version": 1,
            "reference": ["manifest_id": "issued", "host_id": "linux-host", "client_id": "ios",
                          "session_id": "conversation", "catalog_revision": String(repeating: "a", count: 64)],
            "issued_at_ms": 1000, "expires_at_ms": 3000,
            "tools": [["tool_id": "custom_mcp", "route": "agent_host",
                       "schema": ["type": "function", "function": ["name": "custom_mcp", "parameters": ["type": "object"]]]]]
        ]
        let plan = try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: object))
        let ref = try plan.validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation")
        XCTAssertEqual(ref["manifest_id"] as? String, "issued")
        XCTAssertThrowsError(try plan.validatedReference(hostID: "ios", clientID: "ios", sessionID: "conversation"))
        XCTAssertThrowsError(try plan.validatedReference(hostID: "linux-host", clientID: "other", sessionID: "conversation"))
        XCTAssertThrowsError(try plan.validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "other"))
        for (key, value) in [("legacy", true as Any), ("schema_version", "1" as Any), ("schema_version", true as Any),
                             ("issued_at_ms", "1000" as Any), ("expires_at_ms", "3000" as Any),
                             ("issued_at_ms", 0 as Any), ("issued_at_ms", 3000 as Any),
                             ("expires_at_ms", 301001 as Any), ("expires_at_ms", 9_007_199_254_740_992 as Any)] {
            var changed = object; changed[key] = value
            XCTAssertThrowsError(try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation"), key)
        }
        // Both long-past and far-future Host timestamps are structurally valid;
        // freshness is checked only by the execution Host, not this device clock.
        for issued: Int64 in [1_000, 8_000_000_000_000_000] {
            var timed = object; timed["issued_at_ms"] = issued; timed["expires_at_ms"] = issued + 300_000
            _ = try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: timed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation")
        }
        let originalTools = try XCTUnwrap(object["tools"] as? [[String: Any]])
        var duplicate = object; duplicate["tools"] = originalTools + originalTools
        var foreign = object
        var originalReference = try XCTUnwrap(object["reference"] as? [String: Any])
        originalReference["legacy"] = true; foreign["reference"] = originalReference
        var malformedTool = originalTools[0]; malformedTool["legacy"] = true
        var unknownTool = object; unknownTool["tools"] = [malformedTool]
        for changed in [duplicate, foreign, unknownTool] {
            XCTAssertThrowsError(try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation"))
        }
        for invalid in [" \n", "id\u{0000}suffix", "id\u{0085}suffix", String(repeating: "x", count: 513)] {
            var changed = object
            var reference = try XCTUnwrap(object["reference"] as? [String: Any]); reference["manifest_id"] = invalid
            changed["reference"] = reference
            XCTAssertThrowsError(try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation"))
        }
    }
}
