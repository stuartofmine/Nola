import XCTest
@testable import Nola

final class MacCorePromptRuntimeStateTests: XCTestCase {
    func testRuntimeStateUsesCoreABIName() {
        XCTAssertEqual(MacCorePromptLayer.runtimeState.rawValue, "runtime_state")
    }
}
