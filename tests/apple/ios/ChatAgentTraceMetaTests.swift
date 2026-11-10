// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies that user-visible Agent process state survives message persistence.

import XCTest
@testable import Nola

final class ChatAgentTraceMetaTests: XCTestCase {
    func testVisibleThinkingAndPhaseRoundTrip() {
        let meta = ChatAgentTraceMeta.values(
            thinking: "  Inspecting the selected sources.  ",
            phase: "  searching  "
        )

        XCTAssertEqual(
            ChatAgentTraceMeta.thinking(from: meta),
            "Inspecting the selected sources."
        )
        XCTAssertEqual(ChatAgentTraceMeta.phase(from: meta), "searching")
        XCTAssertTrue(ChatAgentTraceMeta.values(thinking: " ", phase: nil).isEmpty)
    }
}
