// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacInspectorInteractionTests: XCTestCase {
    func testOpeningSectionsKeepsOtherSectionsExpanded() {
        let state = MacInspectorNavigationState()
        state.selectedTab = .context
        state.selectedTab = .progress
        XCTAssertEqual(state.expandedSections, [.overview, .context, .progress])
        state.expandedSections.remove(.context)
        let revision = state.revealRevision
        state.selectedTab = .context
        XCTAssertTrue(state.expandedSections.contains(.context))
        XCTAssertGreaterThan(state.revealRevision, revision)
    }

    func testDirectKnowledgeEntryExpandsTheRequestedEditor() {
        let state = MacInspectorNavigationState(initialPane: "wiki")
        XCTAssertEqual(state.selectedTab, .context)
        XCTAssertTrue(state.expandedSections.contains(.context))
        XCTAssertTrue(state.expandedKnowledge.contains(.wiki))
        state.knowledgePane = .recall
        XCTAssertTrue(state.expandedKnowledge.contains(.wiki))
        XCTAssertTrue(state.expandedKnowledge.contains(.recall))
    }
}
