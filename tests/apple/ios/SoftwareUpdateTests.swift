// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class SoftwareUpdateTests: XCTestCase {
    func testUpdateAbiRejectsInstallAndUnknownOperations() async {
        do {
            let _: SoftwareUpdateSnapshot = try await AppleSoftwareUpdateClient.call("install")
            XCTFail("The check/download ABI must never install or quit the Host")
        } catch {
            XCTAssertTrue(error is AppleSoftwareUpdateClient.UpdateFailure)
        }
    }

    func testOfferPreservesExactReleaseIdentityAndLiteralNotes() throws {
        let data = Data(#"{"status":"available","current_version":"1.0.0","current_build":1,"channel":"internal","target":"ios","release":{"release_id":"signed-payload-digest","version":"1.0.1","build":2,"notes":"<literal release notes>","artifact":{"kind":"store","url":"https://apps.apple.com/app/id123"}}}"#.utf8)
        let snapshot = try JSONDecoder().decode(SoftwareUpdateSnapshot.self, from: data)
        XCTAssertEqual(snapshot.release?.release_id, "signed-payload-digest")
        XCTAssertEqual(snapshot.release?.notes, "<literal release notes>")
        XCTAssertEqual(snapshot.release?.artifact.kind, "store")
    }
}
