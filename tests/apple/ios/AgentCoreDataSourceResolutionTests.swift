// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class AgentCoreDataSourceResolutionTests: XCTestCase {
    func testAskWhenRelevantRequiresExplicitConversationConsent() throws {
        XCTAssertTrue(AgentCoreDataSourceResolver.isAvailable)
        let result = try XCTUnwrap(AgentCoreDataSourceResolver.resolve(.init(
            sourceID: "calendar",
            policy: "askWhenRelevant",
            conversationOverride: nil,
            contract: nil,
            now: "2026-08-05T00:00:00Z"
        )))

        XCTAssertFalse(result.enabled)
        XCTAssertFalse(result.usable)
        XCTAssertTrue(result.shouldRequestConsent)
        XCTAssertFalse(result.shouldRefresh)
        XCTAssertEqual(result.reason, "consent_required")
    }
}
