import XCTest
@testable import Nola

final class AgentCorePromptRuntimeStateTests: XCTestCase {
    func testRuntimeStateUsesCoreABIName() {
        XCTAssertEqual(AgentCorePromptLayer.runtimeState.rawValue, "runtime_state")
    }
}
