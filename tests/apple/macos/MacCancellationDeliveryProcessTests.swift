import Foundation
import XCTest
@testable import Nola

final class MacCancellationDeliveryProcessTests: XCTestCase {
    private func fixture(body: String) throws -> (directory: URL, executable: URL) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-cancel-transport-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let executable = directory.appendingPathComponent("control-fixture")
        try ("#!/bin/sh\nIFS= read -r frame\n" + body + "\n")
            .write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        return (directory, executable)
    }

    private func request() throws -> MacRunCancellationRequest {
        try .init(conversationID: "conversation", expectedRunID: "run",
                  assistantMessageID: "assistant", cancellationID: "cancel", requestedAtMs: 1)
    }

    func testOwnedChildReceivesOnlyControlFrameWithoutCreatingDatabaseOrTerminal() async throws {
        let fixture = try fixture(body: "printf '%s' \"$frame\" > received.json\nprintf '%s\\n' \"$@\" > arguments.txt")
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let database = fixture.directory.appendingPathComponent("isolated.db")
        try await MacCancellationDeliveryProcess.run(executable: fixture.executable,
            databaseURL: database, request: request())
        let data = try Data(contentsOf: fixture.directory.appendingPathComponent("received.json"))
        let frame = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: String])
        XCTAssertEqual(frame, ["operation": "cancel", "session_id": "conversation",
            "expected_run_id": "run", "assistant_message_id": "assistant", "cancellation_id": "cancel"])
        let arguments = try String(contentsOf: fixture.directory.appendingPathComponent("arguments.txt"), encoding: .utf8)
        XCTAssertEqual(arguments.split(separator: "\n").map(String.init),
                       ["run", "--json", "--host-request-stdin", "--session", "conversation"])
        XCTAssertFalse(FileManager.default.fileExists(atPath: database.path),
                       "Transport success itself cannot manufacture a canonical terminal record")
    }

    func testLargeErrorOutputIsDrainedAndRetainedBounded() async throws {
        let fixture = try fixture(body: "i=0; while [ \"$i\" -lt 5000 ]; do printf 'control-fixture-output-0123456789\\n' >&2; i=$((i+1)); done; exit 7")
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        do {
            try await MacCancellationDeliveryProcess.run(executable: fixture.executable,
                databaseURL: fixture.directory.appendingPathComponent("isolated.db"), request: request())
            XCTFail("Expected owned child failure")
        } catch MacLocalAgentRuntimeError.processFailed(let status, let detail) {
            XCTAssertEqual(status, 7)
            XCTAssertEqual(detail.utf8.count, 4_000)
        }
    }

    func testDeadlineStopsOnlyOwnedControlChildAndDoesNotReportSuccess() async throws {
        let fixture = try fixture(body: "exec /bin/sleep 60")
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        do {
            try await MacCancellationDeliveryProcess.run(executable: fixture.executable,
                databaseURL: fixture.directory.appendingPathComponent("isolated.db"),
                request: request(), timeout: .milliseconds(50))
            XCTFail("A timed-out control transport is not success")
        } catch MacLocalAgentRuntimeError.processFailed { }
    }
}
