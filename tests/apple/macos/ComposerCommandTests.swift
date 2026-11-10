import XCTest
@testable import Nola

final class ComposerCommandTests: XCTestCase {
    func testExplicitPrefixAndArgumentsDoNotRouteOrdinaryMessages() {
        for text in ["搜索 current", "please /stop", "https://example.org", "模型权限设计"] {
            XCTAssertNil(ComposerCommand.parse(text))
        }
        XCTAssertEqual(ComposerCommand.query("/mo"), "mo")
        XCTAssertNil(ComposerCommand.query("/model "))
        XCTAssertEqual(ComposerCommand.parse("/search 最近变化")?.argument, "最近变化")
        XCTAssertNil(ComposerCommand.parse("/unknown")?.command)
        XCTAssertNil(ComposerCommand.parse("/stop extra")?.command)
        XCTAssertEqual(ComposerCommand.parse("/stop ")?.command, .stop)
    }
}
