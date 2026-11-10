import XCTest
@testable import Nola

final class MobileHostTransportPolicyTests: XCTestCase {
    func testAllowsHttpsAndPrivateLanHttp() throws {
        XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "https://host.example"))))
        XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://127.0.0.1:5001"))))
        XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://10.0.2.2:5001"))))
        XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://172.31.4.2:5001"))))
        XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://192.168.1.8:5001"))))
        XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://nola.local:5001"))))
        XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://[fd00::1]:5001"))))
    }

    func testRejectsPublicHttpAndCredentialBearingURLs() throws {
        XCTAssertFalse(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://example.com:5001"))))
        XCTAssertFalse(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://8.8.8.8:5001"))))
        XCTAssertFalse(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "http://10.0.0.1.example.com:5001"))))
        XCTAssertFalse(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: "https://user:secret@example.com"))))
    }
}
