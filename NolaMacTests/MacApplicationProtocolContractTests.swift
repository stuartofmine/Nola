import XCTest
@testable import Nola

final class MacApplicationProtocolContractTests: XCTestCase {
    func testCurrentLinkedContractIsAcceptedWithoutOpeningStore() throws {
        XCTAssertEqual(MacApplicationServiceClient.protocolVersion, 1)
        try MacApplicationServiceClient.validateLinkedContract(abiVersion: 1, serviceProtocol: 1)
    }

    func testOlderOrNewerFrameworkCannotSelectClientWireProtocol() {
        for version in [UInt32(0), 2, 54, UInt32.max] {
            XCTAssertThrowsError(try MacApplicationServiceClient.validateLinkedContract(
                abiVersion: 1, serviceProtocol: version))
        }
        for abi in [UInt32(0), 2, UInt32.max] {
            XCTAssertThrowsError(try MacApplicationServiceClient.validateLinkedContract(
                abiVersion: abi, serviceProtocol: 1))
        }
        XCTAssertEqual(MacApplicationServiceClient.protocolVersion, 1,
            "Rejected linked frameworks must not mutate the expected client protocol")
    }
}
