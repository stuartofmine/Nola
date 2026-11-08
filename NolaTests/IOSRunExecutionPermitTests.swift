import XCTest
@testable import Nola

final class IOSRunExecutionPermitTests: XCTestCase {
    func testRevocationRejectsSubsequentAdmissionsWithoutRevokingAnotherRun() throws {
        let old = IOSRunExecutionPermit()
        let current = IOSRunExecutionPermit()
        try old.checkAdmission()
        old.revoke()
        old.revoke()
        XCTAssertThrowsError(try old.checkAdmission()) { error in
            XCTAssertTrue(error is CancellationError)
        }
        XCTAssertNoThrow(try current.checkAdmission())
    }

    func testExecutorRejectsRevokedRunBeforeDispatchingInstalledHandler() async {
        let permit = IOSRunExecutionPermit()
        permit.revoke()
        let executor = IOSAgentHostExecutor()
        // The real memory handler requires a query. Cancellation must win even
        // before argument handling; no shared memory store should be queried.
        let result = await executor.executeBridge(callId: "revoked", toolName: "memory_search",
            capability: .none, arguments: .object([:]), grant: nil, executionPermit: permit)
        XCTAssertTrue(result.isError)
        guard case .object(let output) = result.output else { return XCTFail("Expected rejection") }
        XCTAssertEqual(output["error"], .string(CancellationError().localizedDescription))
    }
}
