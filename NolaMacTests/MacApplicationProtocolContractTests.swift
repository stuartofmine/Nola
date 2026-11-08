import XCTest
@testable import Nola

final class MacApplicationProtocolContractTests: XCTestCase {
    func testCurrentLinkedContractIsAcceptedWithoutOpeningStore() throws {
        XCTAssertEqual(MacApplicationServiceClient.protocolVersion, 52)
        try MacApplicationServiceClient.validateLinkedContract(abiVersion: 1, serviceProtocol: 52)
    }

    func testOlderOrNewerFrameworkCannotSelectClientWireProtocol() {
        for version in [UInt32(0), 51, 53, UInt32.max] {
            XCTAssertThrowsError(try MacApplicationServiceClient.validateLinkedContract(
                abiVersion: 1, serviceProtocol: version))
        }
        for abi in [UInt32(0), 2, UInt32.max] {
            XCTAssertThrowsError(try MacApplicationServiceClient.validateLinkedContract(
                abiVersion: abi, serviceProtocol: 52))
        }
        XCTAssertEqual(MacApplicationServiceClient.protocolVersion, 52,
            "Rejected linked frameworks must not mutate the expected client protocol")
    }
}
