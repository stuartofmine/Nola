import XCTest
@testable import Nola

@MainActor
final class MacWorkspaceLayoutContractTests: XCTestCase {
    func testExpandedSidebarUsesOverlayUntilAllThreeColumnsFit() {
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_101, sidebarWidth: 360), .inspectorOverlay)
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_000, sidebarWidth: 360), .inspectorOverlay)
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_102, sidebarWidth: 360), .threeColumn)
    }

    func testEveryThreeColumnLayoutIncludesDividerBudget() {
        for sidebar in [CGFloat(220), 280, 360] {
            for width in stride(from: CGFloat(1_200), through: 1_440, by: 1) {
                guard MacWorkspaceLayoutMode.resolve(width: width, sidebarWidth: sidebar) == .threeColumn else { continue }
                for requestedInspector in [CGFloat(340), 420, 480] {
                    let inspector = MacWorkspaceColumnMetrics.inspectorWidth(
                        availableWidth: width, sidebarWidth: sidebar, storedWidth: requestedInspector
                    )
                    XCTAssertGreaterThanOrEqual(width - sidebar - inspector - 2, MacWorkspaceColumnMetrics.minimumConversationWidth,
                        "Columns overflow at width \(width), sidebar \(sidebar), inspector \(inspector)")
                }
            }
        }
    }

    func testHiddenSidebarDoesNotReserveItsStoredWidth() {
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_280, sidebarWidth: 0), .threeColumn)
        XCTAssertEqual(MacWorkspaceColumnMetrics.inspectorWidth(
            availableWidth: 1_280, sidebarWidth: 0, storedWidth: 480
        ), 480)
    }

    func testNativeV5ChromeKeepsReadableInspectorAndSoftControlTargets() {
        XCTAssertEqual(MacWorkspaceColumnMetrics.preferredInspectorWidth,
                       MacDesignTokens.Layout.inspectorWidth)
        XCTAssertGreaterThanOrEqual(MacDesignTokens.Control.iconButton, 36)
        XCTAssertGreaterThanOrEqual(MacDesignTokens.Control.compactRow, 36)
        XCTAssertGreaterThan(MacDesignTokens.Radius.panel, MacDesignTokens.Radius.control)
        XCTAssertGreaterThan(MacDesignTokens.Radius.floating, MacDesignTokens.Radius.panel)
    }

    func testModuleNavigationBelongsToEachWindow() {
        let data = MacDataCenterNavigationState()
        data.section = .usage
        data.showWebSources()
        let otherData = MacDataCenterNavigationState()
        XCTAssertEqual(data.section, .usage)
        XCTAssertEqual(otherData.section, .sources)
        XCTAssertEqual(otherData.route, .overview)

        let automations = MacAutomationCenterNavigationState()
        automations.showRunHistory()
        automations.searchText = "daily"
        automations.selectedFilter = .paused
        let otherAutomations = MacAutomationCenterNavigationState()
        XCTAssertFalse(otherAutomations.showsRunHistory)
        XCTAssertEqual(otherAutomations.searchText, "")
        XCTAssertEqual(otherAutomations.selectedFilter, .all)

        let extensions = MacExtensionsNavigationState()
        extensions.selectedID = "fixture-extension"
        extensions.query = "fixture"
        extensions.sourceFilter = .connectedHost
        let otherExtensions = MacExtensionsNavigationState()
        XCTAssertNil(otherExtensions.selectedID)
        XCTAssertEqual(otherExtensions.query, "")
        XCTAssertEqual(otherExtensions.sourceFilter, .thisMac)
    }

    func testMinimumThreeColumnWindowKeepsConversationWiderThanInspector() {
        let availableWidth: CGFloat = 1_280
        let inspectorWidth = MacWorkspaceColumnMetrics.inspectorWidth(
            availableWidth: availableWidth,
            sidebarWidth: MacDesignTokens.Layout.sidebarWidth,
            storedWidth: 900
        )
        let conversationWidth = availableWidth
            - MacDesignTokens.Layout.sidebarWidth
            - inspectorWidth

        XCTAssertGreaterThanOrEqual(
            conversationWidth,
            MacWorkspaceColumnMetrics.minimumConversationWidth
        )
        XCTAssertGreaterThan(conversationWidth, inspectorWidth)
        XCTAssertLessThanOrEqual(
            inspectorWidth,
            MacWorkspaceColumnMetrics.maximumInspectorWidth
        )
    }

    func testOversizedInspectorPreferenceIsClampedWithoutDiscardingResizeSupport() {
        let width = MacWorkspaceColumnMetrics.inspectorWidth(
            availableWidth: 1_440,
            sidebarWidth: MacDesignTokens.Layout.sidebarWidth,
            storedWidth: 900
        )

        XCTAssertEqual(width, MacWorkspaceColumnMetrics.maximumInspectorWidth)
        XCTAssertGreaterThan(width, MacWorkspaceColumnMetrics.minimumInspectorWidth)
    }
}
