import XCTest
@testable import Nola

final class MacSecondaryPageLayoutTests: XCTestCase {
    func testWorkspaceCanLaunchEverySecondaryPageForRealVisualAcceptance() {
        for section in MacWorkspaceSection.allCases {
            XCTAssertEqual(
                MacWorkspaceSection.initialSection(environment: ["NOLA_MAC_INITIAL_SECTION": section.rawValue]),
                section
            )
        }
        XCTAssertEqual(MacWorkspaceSection.initialSection(environment: [:]), .chat)
    }
}
