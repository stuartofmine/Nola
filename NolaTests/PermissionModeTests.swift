// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class PermissionModeTests: XCTestCase {
    func testPermissionModesUseTwoCanonicalWireValuesAndCautiousDefault() throws {
        XCTAssertEqual(IOSPermissionMode.allCases.map(\.rawValue), ["cautious", "allow_all"])
        XCTAssertTrue(IOSPermissionMode.cautious.autoApprove)
        XCTAssertEqual(IOSPermissionMode.cautious.networkPolicy, "ask")
        XCTAssertTrue(IOSPermissionMode.allowAll.autoApprove)
        XCTAssertEqual(IOSPermissionMode.allowAll.networkPolicy, "allow")
        XCTAssertEqual(Conversation().permissionMode, .cautious)
    }

    func testConversationPermissionModePersistsAndAbsentFieldInitializesCautious() throws {
        var conversation = Conversation()
        conversation.permissionMode = .allowAll
        let encoded = try JSONEncoder().encode(conversation)
        XCTAssertEqual(try JSONDecoder().decode(Conversation.self, from: encoded).permissionMode, .allowAll)

        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        object.removeValue(forKey: "permissionMode")
        let absent = try JSONSerialization.data(withJSONObject: object)
        XCTAssertEqual(try JSONDecoder().decode(Conversation.self, from: absent).permissionMode, .cautious)
    }

    func testConversationRejectsInvalidExplicitPermissionWithoutChangingSettings() throws {
        let conversation = Conversation(sessionMemory: "keep this memory")
        let encoded = try JSONEncoder().encode(conversation)
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        let invalidProfiles: [Any] = ["safe", "recommended", "moderate", "allow-all", " AllowAll ", "unknown", "", true, NSNull()]
        for invalid in invalidProfiles {
            object["permissionMode"] = invalid
            let data = try JSONSerialization.data(withJSONObject: object)
            XCTAssertThrowsError(try JSONDecoder().decode(Conversation.self, from: data))
            XCTAssertEqual(object["sessionMemory"] as? String, "keep this memory")
        }
    }

    func testRuntimeConfigDefaultsToCautious() {
        let config = AgentCoreRuntimeConfig()
        XCTAssertEqual(config.permissionProfile, .cautious)
    }
}
