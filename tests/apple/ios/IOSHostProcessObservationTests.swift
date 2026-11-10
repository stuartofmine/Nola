// SPDX-License-Identifier: Apache-2.0
import XCTest
import Darwin
@testable import Nola

final class IOSHostProcessObservationTests: XCTestCase {
    func testUnavailableObservationDoesNotInventProcessIdentity() {
        let unknown: AgentCoreJSONValue = .object(["state": .string("unknown"), "pid": .signedInteger(42)])
        XCTAssertEqual(IOSHostProcessObservation.workerIdentity(observation: unknown, nonce: "worker"),
            .object(["kind": .string("unobservable"), "nonce": .string("worker")]))
    }

    func testRunningObservationPreservesExactOSFacts() {
        let observed: AgentCoreJSONValue = .object(["state": .string("running"), "pid": .signedInteger(42),
            "boot_id": .string("boot"), "start_id": .string("123:456")])
        XCTAssertEqual(IOSHostProcessObservation.workerIdentity(observation: observed, nonce: "worker"),
            .object(["kind": .string("process"), "pid": .signedInteger(42), "boot_id": .string("boot"),
                "start_id": .string("123:456"), "nonce": .string("worker")]))
    }

    func testActualCurrentProcessObservationIsAcceptedByCurrentCore() throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("process.db"))
        let identity = IOSHostProcessObservation.currentWorkerIdentity()
        let encoded = try runtime.executeAgentCommand(.object(["type": .string("encode_host_worker_identity"), "identity": identity]))
        guard case .object(let fields) = encoded, case .string(let generation)? = fields["worker_generation"],
              case .string(let owner)? = fields["lease_owner"] else { return XCTFail("Missing Core worker encoding") }
        XCTAssertFalse(generation.isEmpty)
        XCTAssertFalse(owner.isEmpty)
        guard case .object(let facts) = identity, case .string(let kind)? = facts["kind"] else { return XCTFail() }
        XCTAssertTrue(["process", "unobservable"].contains(kind))
        // Simulator success is not evidence of physical-device sysctl access.
        if kind == "unobservable" { XCTAssertEqual(Set(facts.keys), Set(["kind", "nonce"])) }
    }
}
