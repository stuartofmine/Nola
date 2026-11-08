// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises the real Rust/SQLite canonical store through the production Mac ABI.

import Foundation
import Darwin
import XCTest
@testable import Nola

final class MacCanonicalStoreEndToEndTests: XCTestCase {
    func testMemorySelectionRequiresCurrentOwnerAwareABI() {
        XCTAssertTrue(MacPersonalIntelligenceCore.isMemorySelectionAvailable)
    }

    func testGlobalStopRejectsConcurrentStartupAndPreservesLiveClientReferences() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-stop-race-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        var first: MacApplicationServiceClient? = .deferred(databaseURL: database)
        var second: MacApplicationServiceClient? = .deferred(databaseURL: database)
        defer { first = nil; second = nil }
        let supervisor = MacApplicationServiceSupervisor.shared
        try XCTUnwrap(first).prepare()
        let socket = try XCTUnwrap(first).socketURL
        let pid = try XCTUnwrap(supervisor.ownedProcessIdentifier(socketURL: socket))
        XCTAssertEqual(Darwin.kill(pid, SIGSTOP), 0)
        defer { Darwin.kill(pid, SIGCONT) }
        let finished = expectation(description: "Global stop completes")
        DispatchQueue.global(qos: .userInitiated).async {
            supervisor.stopOwnedProcesses()
            finished.fulfill()
        }
        let deadline = Date().addingTimeInterval(1)
        while !supervisor.isStoppingServices, Date() < deadline {
            Thread.sleep(forTimeInterval: 0.005)
        }
        XCTAssertTrue(supervisor.isStoppingServices)
        let other = MacApplicationServiceClient.deferred(databaseURL: root.appendingPathComponent("other.db"))
        XCTAssertThrowsError(try other.prepare()) { error in
            XCTAssertTrue(error is CancellationError, "Unexpected concurrent startup result: \(error)")
        }
        XCTAssertNil(supervisor.ownedProcessIdentifier(socketURL: other.socketURL))
        wait(for: [finished], timeout: 5)
        XCTAssertFalse(supervisor.isStoppingServices)
        try XCTUnwrap(second).prepare()
        let restartedPID = try XCTUnwrap(supervisor.ownedProcessIdentifier(socketURL: socket))
        XCTAssertNotEqual(restartedPID, pid)
        first = nil
        XCTAssertEqual(supervisor.ownedProcessIdentifier(socketURL: socket), restartedPID)
        try XCTUnwrap(second).healthCheck()
        second = nil
        XCTAssertNil(supervisor.ownedProcessIdentifier(socketURL: socket))
    }

    func testUnresponsiveDatabaseDoesNotSerializeAnotherDatabaseRequest() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-independent-writers-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let paused = MacApplicationServiceClient.deferred(databaseURL: root.appendingPathComponent("paused.db"))
        let healthy = MacApplicationServiceClient.deferred(databaseURL: root.appendingPathComponent("healthy.db"))
        try paused.prepare()
        try healthy.prepare()
        let pid = try XCTUnwrap(MacApplicationServiceSupervisor.shared.ownedProcessIdentifier(socketURL: paused.socketURL))
        XCTAssertEqual(Darwin.kill(pid, SIGSTOP), 0)
        defer { Darwin.kill(pid, SIGCONT) }
        let entered = DispatchSemaphore(value: 0)
        let finished = expectation(description: "Paused writer readiness check ends")
        DispatchQueue.global(qos: .userInitiated).async {
            entered.signal()
            do {
                try paused.prepare()
                XCTFail("Paused writer must fail its readiness check")
            } catch {
                // A failed probe must not hold the global lifecycle lock.
            }
            finished.fulfill()
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 1), .success)
        Thread.sleep(forTimeInterval: 0.35)
        let started = DispatchTime.now().uptimeNanoseconds
        let result = try healthy.execute(commandData: Data(#"{"type":"list_conversations","query":{"project_root":null,"non_empty":false,"limit":1024,"offset":0}}"#.utf8))
        XCTAssertNotNil(try JSONSerialization.jsonObject(with: result) as? [Any])
        XCTAssertLessThan(Double(DispatchTime.now().uptimeNanoseconds - started) / 1_000_000_000, 1)
        wait(for: [finished], timeout: 7)
    }

    func testHealthRecheckDoesNotTerminateOrReplaceAnExistingWriter() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-paused-writer-\(UUID().uuidString)")
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }
        let client = MacApplicationServiceClient.deferred(databaseURL: database)
        try client.prepare()
        let supervisor = MacApplicationServiceSupervisor.shared
        let pid = try XCTUnwrap(supervisor.ownedProcessIdentifier(socketURL: client.socketURL))
        XCTAssertEqual(Darwin.kill(pid, SIGSTOP), 0)
        defer { Darwin.kill(pid, SIGCONT) }
        let started = Date()
        XCTAssertThrowsError(try client.prepare())
        XCTAssertLessThan(Date().timeIntervalSince(started), 7)
        XCTAssertEqual(supervisor.ownedProcessIdentifier(socketURL: client.socketURL), pid)
        XCTAssertEqual(Darwin.kill(pid, SIGCONT), 0)
        // A pending TERM would take effect after CONT, making the original
        // writer disappear. Repeated probes must retain the original process.
        for _ in 0..<3 {
            Thread.sleep(forTimeInterval: 0.05)
            try client.healthCheck()
            XCTAssertEqual(supervisor.ownedProcessIdentifier(socketURL: client.socketURL), pid)
        }
        let result = try client.execute(commandData: Data(#"{"type":"list_conversations","query":{"project_root":null,"non_empty":false,"limit":1024,"offset":0}}"#.utf8))
        XCTAssertNotNil(try JSONSerialization.jsonObject(with: result) as? [Any])
    }

    func testOwnedProcessIgnoringTerminateIsKilledWithoutUnboundedWait() throws {
        let registry = MacOwnedChildProcessRegistry()
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ignore-term-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let ready = root.appendingPathComponent("ready")
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = ["-c", "trap '' TERM; : > \"$1\"; exec /bin/sleep 30", "fixture", ready.path]
        try process.run()
        registry.register(process)
        defer { registry.stopAll(gracePeriod: 0) }
        let readyDeadline = Date().addingTimeInterval(2)
        while !FileManager.default.fileExists(atPath: ready.path), Date() < readyDeadline {
            Thread.sleep(forTimeInterval: 0.01)
        }
        XCTAssertTrue(FileManager.default.fileExists(atPath: ready.path))
        let started = DispatchTime.now().uptimeNanoseconds
        registry.stop(process, gracePeriod: 0.05)
        let elapsed = Double(DispatchTime.now().uptimeNanoseconds - started) / 1_000_000_000
        XCTAssertLessThan(elapsed, 0.5)
        let exitDeadline = Date().addingTimeInterval(2)
        while process.isRunning, Date() < exitDeadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        XCTAssertFalse(process.isRunning)
        if !process.isRunning {
            XCTAssertEqual(process.terminationReason, .uncaughtSignal)
            XCTAssertEqual(process.terminationStatus, SIGKILL)
        }
    }

    func testFullSocketBacklogCannotEscapeConnectionDeadline() throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-full-backlog-\(UUID().uuidString).db")
        let client = MacApplicationServiceClient.deferred(databaseURL: database)
        let listener = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        XCTAssertGreaterThanOrEqual(listener, 0)
        guard listener >= 0 else { return }
        var peers: [Int32] = []
        defer {
            for peer in peers { Darwin.close(peer) }
            Darwin.close(listener)
            Darwin.unlink(client.socketURL.path)
        }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let path = Array(client.socketURL.path.utf8CString)
        withUnsafeMutableBytes(of: &address.sun_path) { bytes in
            bytes.initializeMemory(as: UInt8.self, repeating: 0)
            path.withUnsafeBytes { bytes.copyBytes(from: $0) }
        }
        func withAddress(_ operation: (UnsafePointer<sockaddr>, socklen_t) -> Int32) -> Int32 {
            withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    operation($0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
        }
        XCTAssertEqual(withAddress { Darwin.bind(listener, $0, $1) }, 0)
        XCTAssertEqual(Darwin.listen(listener, 1), 0)
        var saturated = false
        var saturationError: Int32 = 0
        var connectedPeers = 0
        for _ in 0..<32 {
            let peer = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
            XCTAssertGreaterThanOrEqual(peer, 0)
            guard peer >= 0 else { return }
            peers.append(peer)
            let flags = Darwin.fcntl(peer, F_GETFL)
            XCTAssertEqual(Darwin.fcntl(peer, F_SETFL, flags | O_NONBLOCK), 0)
            let result = withAddress { Darwin.connect(peer, $0, $1) }
            let connectionError = errno
            if result != 0 {
                XCTAssertTrue([EINPROGRESS, EAGAIN, EWOULDBLOCK, ECONNREFUSED].contains(connectionError))
                saturationError = connectionError
                saturated = true
                break
            }
            connectedPeers += 1
        }
        XCTAssertGreaterThan(connectedPeers, 0, "Listener must accept connections before saturation")
        XCTAssertTrue(saturated, "Fixture must actually fill the connection queue")
        guard saturated else { return }
        let started = DispatchTime.now().uptimeNanoseconds
        XCTAssertThrowsError(try client.healthCheck()) { error in
            guard case MacApplicationServiceError.connectionFailed(let detail) = error else {
                return XCTFail("Expected connection deadline, received \(error)")
            }
            if saturationError == ECONNREFUSED {
                XCTAssertEqual(detail, String(cString: strerror(ECONNREFUSED)))
            } else {
                XCTAssertEqual(detail, "service request timed out")
            }
        }
        let elapsed = Double(DispatchTime.now().uptimeNanoseconds - started) / 1_000_000_000
        XCTAssertLessThan(elapsed, 0.8)
    }

    func testRecoveredAdmissionSurvivesRealServiceRestartWithoutReexecution() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-pending-receipt-\(UUID().uuidString)")
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }
        do {
            let initializer = MacApplicationServiceClient.deferred(databaseURL: database)
            try initializer.prepare()
        }
        let socket = MacApplicationServiceClient.serviceSocketURL(for: database)
        XCTAssertFalse(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(socketURL: socket))
        let clientID = MacApplicationServiceClient.serviceClientID(for: database)
        // Read-only commands intentionally bypass durable admission and return a
        // fresh result. Seed a real write admission while the product writer is
        // stopped so retrying it proves that recovery never reexecutes a mutation.
        let interruptedArguments = #"{"key":"interrupted-write","updated_at_ms":1,"value_json":"true"}"#
        let seed = Process()
        seed.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        seed.arguments = [database.path, """
        INSERT INTO command_receipts
        (client_id,request_id,command_type,command_json,expected_revision,status,received_at)
        VALUES ('\(clientID)','interrupted','put_product_setting','\(interruptedArguments)',NULL,'accepted','2026-09-06T00:00:00Z');
        """]
        try seed.run()
        seed.waitUntilExit()
        XCTAssertEqual(seed.terminationStatus, 0)
        guard seed.terminationStatus == 0 else { return }
        for _ in 0..<2 {
            do {
                let client = MacApplicationServiceClient.deferred(databaseURL: database)
                try client.prepare()
                let started = Date()
                XCTAssertThrowsError(try client.execute(
                    commandData: Data(
                        #"{"type":"put_product_setting","key":"interrupted-write","value_json":"true","updated_at_ms":1}"#.utf8
                    ),
                    requestID: "interrupted"
                )) { error in
                    guard case MacApplicationServiceError.outcomePending(let requestID) = error else {
                        return XCTFail("Expected pending durable outcome, received \(error)")
                    }
                    XCTAssertEqual(requestID, "interrupted")
                }
                XCTAssertLessThan(Date().timeIntervalSince(started), 3.5)
                let data = try client.execute(commandData: Data(
                    #"{"type":"command_receipt","request_id":"interrupted"}"#.utf8
                ))
                let receipt = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                XCTAssertEqual(receipt["status"] as? String, "accepted")
                XCTAssertEqual(receipt["request_id"] as? String, "interrupted")
                XCTAssertNil(receipt["completed_at"])

                let settingData = try client.execute(commandData: Data(
                    #"{"type":"get_product_setting","key":"interrupted-write"}"#.utf8
                ))
                let setting = try JSONSerialization.jsonObject(
                    with: settingData,
                    options: [.fragmentsAllowed]
                )
                XCTAssertTrue(setting is NSNull, "Recovered admission must not execute the pending write")
            }
            XCTAssertFalse(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(socketURL: socket))
        }
    }

    func testPendingOutcomeRetainsRequestIdentityInUserMessage() {
        let error = MacApplicationServiceError.outcomePending(requestID: "pending-request-123")
        XCTAssertTrue(error.localizedDescription.contains("pending-request-123"))
        XCTAssertNotEqual(error.localizedDescription, MacApplicationServiceError.invalidResponse.localizedDescription)
    }
    func testSilentConnectedHealthPeerCannotBlockStartupProbe() throws {
        try assertHealthProbeDeadline(trickles: false)
    }

    func testTricklingHealthPeerCannotExtendProbeDeadline() throws {
        try assertHealthProbeDeadline(trickles: true)
    }

    func testSilentOrdinaryRequestHasAnAbsoluteDeadline() throws {
        try assertHealthProbeDeadline(trickles: false, ordinaryRequest: true)
    }

    func testTricklingOrdinaryRequestCannotRenewItsDeadline() throws {
        try assertHealthProbeDeadline(trickles: true, ordinaryRequest: true)
    }

    func testNonReadingPeerCannotBlockLargeRequestWrite() throws {
        try assertHealthProbeDeadline(trickles: false, ordinaryRequest: true, blockedWrite: true)
    }

    func testCancellationInterruptsSilentResponseWithoutWaitingForDeadline() throws {
        try assertHealthProbeDeadline(trickles: false, ordinaryRequest: true, cancelRequest: true)
    }

    func testCancellationInterruptsBlockedWriteWithoutReportingConnectionFailure() throws {
        try assertHealthProbeDeadline(trickles: false, ordinaryRequest: true, blockedWrite: true, cancelRequest: true)
    }

    private func assertHealthProbeDeadline(trickles: Bool, ordinaryRequest: Bool = false, blockedWrite: Bool = false, cancelRequest: Bool = false) throws {
        let database = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-health-deadline-\(UUID().uuidString).db")
        let client = MacApplicationServiceClient.deferred(databaseURL: database)
        let listener = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        XCTAssertGreaterThanOrEqual(listener, 0)
        guard listener >= 0 else { return }
        defer {
            Darwin.close(listener)
            Darwin.unlink(client.socketURL.path)
        }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let path = Array(client.socketURL.path.utf8CString)
        withUnsafeMutableBytes(of: &address.sun_path) { bytes in
            bytes.initializeMemory(as: UInt8.self, repeating: 0)
            path.withUnsafeBytes { bytes.copyBytes(from: $0) }
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        XCTAssertEqual(bound, 0)
        guard bound == 0 else { return }
        XCTAssertEqual(Darwin.listen(listener, 1), 0)
        let finished = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .userInitiated).async {
            defer { finished.signal() }
            var readiness = pollfd(fd: listener, events: Int16(POLLIN), revents: 0)
            guard Darwin.poll(&readiness, 1, 2_000) > 0 else { return }
            let peer = Darwin.accept(listener, nil, nil)
            guard peer >= 0 else { return }
            defer { Darwin.close(peer) }
            var enabled: Int32 = 1
            _ = Darwin.setsockopt(peer, SOL_SOCKET, SO_NOSIGPIPE, &enabled, socklen_t(MemoryLayout<Int32>.size))
            // Close after one second even if the implementation regresses,
            // so this fixture never strands the test in a blocking read.
            for _ in 0..<20 {
                if trickles {
                    var byte: UInt8 = 32
                    if Darwin.write(peer, &byte, 1) != 1 { break }
                }
                Thread.sleep(forTimeInterval: 0.05)
            }
        }
        let started = DispatchTime.now().uptimeNanoseconds
        let cancellation = MacApplicationQueryCancellation()
        if cancelRequest {
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.1) {
                cancellation.cancel()
            }
        }
        func performRequest() throws {
            if ordinaryRequest {
                _ = try client.send([
                    "protocol_version": MacApplicationServiceClient.protocolVersion,
                    "client_id": client.clientID,
                    "request_id": "deadline-fixture",
                    "command": [
                        "type": "transport_fixture",
                        "arguments": ["payload": blockedWrite ? String(repeating: "x", count: 2 * 1_024 * 1_024) : ""],
                    ] as [String: Any],
                ], queryCancellation: cancelRequest ? cancellation : nil,
                   deadline: cancelRequest ? nil : DispatchTime.now().uptimeNanoseconds + 250_000_000)
            } else {
                try client.healthCheck()
            }
        }
        XCTAssertThrowsError(try performRequest()) { error in
            if cancelRequest {
                XCTAssertTrue(error is CancellationError, "Expected cancellation, received \(error)")
                return
            }
            guard case MacApplicationServiceError.connectionFailed(let detail) = error else {
                return XCTFail("Expected bounded probe timeout, received \(error)")
            }
            XCTAssertEqual(detail, "service request timed out")
        }
        let elapsed = Double(DispatchTime.now().uptimeNanoseconds - started) / 1_000_000_000
        XCTAssertLessThan(elapsed, 0.8)
        XCTAssertEqual(finished.wait(timeout: .now() + 3), .success)
    }

    func testVerboseUnreadyServiceHasBoundedDiagnosticsAndIsStopped() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-verbose-startup-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let executable = root.appendingPathComponent("verbose-service")
        let script = """
        #!/bin/sh
        i=0
        while [ "$i" -lt 512 ]; do
            printf '%s' '\(String(repeating: "x", count: 1024))' >&2
            i=$((i + 1))
        done
        exec /bin/sleep 30
        """
        try Data(script.utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let key = "NOLA_APPLICATION_SERVICE_EXECUTABLE"
        let previous = ProcessInfo.processInfo.environment[key]
        setenv(key, executable.path, 1)
        defer {
            if let previous { setenv(key, previous, 1) } else { unsetenv(key) }
        }
        let client = MacApplicationServiceClient.deferred(
            databaseURL: root.appendingPathComponent("product.db")
        )
        let started = Date()
        XCTAssertThrowsError(try client.prepare()) { error in
            guard case MacApplicationServiceError.launchFailed(let detail) = error else {
                return XCTFail("Unexpected startup error: \(error)")
            }
            XCTAssertEqual(detail.utf8.count, 16 * 1024)
            XCTAssertTrue(detail.allSatisfy { $0 == "x" })
        }
        XCTAssertLessThan(Date().timeIntervalSince(started), 10)
        XCTAssertFalse(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(
            socketURL: client.socketURL
        ))
    }

    func testDeferredServiceClientDoesNotCreateDatabaseOrLaunchProcess() {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-deferred-startup-\(UUID().uuidString)")
        let database = root.appendingPathComponent("product.db")
        let client = MacApplicationServiceClient.deferred(databaseURL: database)
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.path))
        XCTAssertFalse(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(
            socketURL: client.socketURL
        ))
    }

    func testFailedStartupPreservesDatabaseAndSameClientCanRetry() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-startup-retry-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        let original = Data("invalid isolated database fixture".utf8)
        try original.write(to: database)
        var client: MacApplicationServiceClient? = .deferred(databaseURL: database)
        let socket = try XCTUnwrap(client).socketURL
        defer { client = nil }

        for _ in 0..<2 {
            let started = Date()
            XCTAssertThrowsError(try XCTUnwrap(client).prepare())
            XCTAssertLessThan(Date().timeIntervalSince(started), 10)
            XCTAssertEqual(try Data(contentsOf: database), original)
            XCTAssertFalse(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(
                socketURL: socket
            ))
        }

        // Explicit fixture repair, not product recovery or a schema migration.
        try FileManager.default.removeItem(at: database)
        try XCTUnwrap(client).prepare()
        XCTAssertTrue(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(socketURL: socket))
        do {
            let port = try MacApplicationPort(databaseURL: database)
            _ = try port.putProductSetting(
                key: "startup-retry-fixture", valueJSON: "true", updatedAtMs: 1
            )
            try XCTUnwrap(client).prepare()
            XCTAssertEqual(try port.productSetting(key: "startup-retry-fixture")?.valueJSON, "true")
        }
        client = nil
        XCTAssertFalse(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(socketURL: socket))
    }

    func testProductionServiceLaunchRequiresTheCurrentCanonicalSchema() {
        let arguments = MacApplicationServiceSupervisor.serviceArguments(
            databaseURL: URL(fileURLWithPath: "/tmp/nola/product.db"),
            socketURL: URL(fileURLWithPath: "/tmp/nola/product.sock"),
            parentPID: 42
        )

        XCTAssertEqual(arguments, [
            "--database", "/tmp/nola/product.db",
            "--socket", "/tmp/nola/product.sock",
            "--parent-pid", "42",
        ])
        XCTAssertFalse(arguments.contains("--recreate-incompatible-store"))
    }

    func testExplicitRecoveryLaunchCarriesAUniqueArchiveDestination() {
        let arguments = MacApplicationServiceSupervisor.serviceArguments(
            databaseURL: URL(fileURLWithPath: "/tmp/nola/product.db"),
            socketURL: URL(fileURLWithPath: "/tmp/nola/product.sock"),
            parentPID: 42,
            archiveIncompatibleStoreAt: URL(fileURLWithPath: "/tmp/nola/recovered/old")
        )

        XCTAssertEqual(Array(arguments.suffix(2)), [
            "--archive-incompatible-store", "/tmp/nola/recovered/old",
        ])
        XCTAssertFalse(arguments.contains("--recreate-incompatible-store"))
    }

    func testNormalLaunchUsesOnlyExplicitCurrentStoreArguments() {
        XCTAssertEqual(MacApplicationServiceSupervisor.serviceArguments(
            databaseURL: URL(fileURLWithPath: "/tmp/product.db"),
            socketURL: URL(fileURLWithPath: "/tmp/service.sock"),
            parentPID: 42
        ), ["--database", "/tmp/product.db", "--socket", "/tmp/service.sock", "--parent-pid", "42"])
    }

    func testOnlyDirectStructuredStartupFailureEnablesIncompatibleStoreRecovery() throws {
        let envelope = #"{"type":"nola.service.startup_failure","protocol_version":1,"code":"incompatible_canonical_store","detail":"schema 11"}"#
        let classified = try XCTUnwrap(
            MacApplicationServiceSupervisor.classifiedStartupFailure(from: envelope)
        )
        guard case .incompatibleStore(let detail) = classified else {
            return XCTFail("Unexpected startup failure: \(classified)")
        }
        XCTAssertEqual(detail, "schema 11")

        XCTAssertNil(MacApplicationServiceSupervisor.classifiedStartupFailure(
            from: "user text mentions incompatible_canonical_store"
        ))
        XCTAssertNil(MacApplicationServiceSupervisor.classifiedStartupFailure(
            from: #"{"type":"log","protocol_version":1,"code":"incompatible_canonical_store","detail":"nested diagnostic"}"#
        ))
        XCTAssertNil(MacApplicationServiceSupervisor.classifiedStartupFailure(
            from: #"{"type":"nola.service.startup_failure","protocol_version":1,"code":"unknown","detail":"incompatible_canonical_store"}"#
        ))
    }

    func testExplicitMacRecoveryArchivesOldStoreBeforeStartingFreshService() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-incompatible-recovery-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        let sqlite = Process()
        sqlite.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        sqlite.arguments = [
            database.path,
            "CREATE TABLE legacy(value TEXT); INSERT INTO legacy VALUES('keep'); "
                + "PRAGMA application_id=1313819713; PRAGMA user_version=11;",
        ]
        try sqlite.run()
        sqlite.waitUntilExit()
        XCTAssertEqual(sqlite.terminationStatus, 0)
        let original = try Data(contentsOf: database)
        var client: MacApplicationServiceClient? = .deferred(databaseURL: database)
        defer { client = nil }

        XCTAssertThrowsError(try XCTUnwrap(client).prepare()) { error in
            guard let serviceError = error as? MacApplicationServiceError,
                  case .incompatibleStore = serviceError else {
                return XCTFail("Unexpected startup error: \(error)")
            }
        }
        XCTAssertEqual(try Data(contentsOf: database), original)

        let archive = try XCTUnwrap(
            try XCTUnwrap(client).archiveIncompatibleStoreAndPrepare()
        )
        XCTAssertEqual(
            try Data(contentsOf: archive.appendingPathComponent("product.db")),
            original
        )
        XCTAssertNotEqual(try Data(contentsOf: database), original)
        try XCTUnwrap(client).healthCheck()
    }

    func testNativeClientUsesSharedApplicationServiceSocketProtocol() {
        let databaseURL = URL(fileURLWithPath: "/tmp/nola socket contract/product.db")
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in databaseURL.standardizedFileURL.path.utf8 {
            hash ^= UInt64(byte)
            hash &*= 1_099_511_628_211
        }
        let expected = String(
            format: "/tmp/nola-application-v%d-%016llx.sock",
            MacApplicationServiceClient.protocolVersion,
            hash
        )

        XCTAssertEqual(
            MacApplicationServiceClient.serviceSocketURL(for: databaseURL).path,
            expected
        )
        XCTAssertEqual(
            MacApplicationServiceClient.serviceClientID(for: databaseURL),
            String(format: "nola-local-%016llx", hash)
        )
    }

    func testModelProfileSelectionAndRuntimeSurviveRealCoreABIReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-runtime-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db", isDirectory: false)
        defer { try? FileManager.default.removeItem(at: root) }

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            try store.upsertModelProfile(modelProfile(id: "small", selected: true, now: 1_000))
            try store.upsertModelProfile(modelProfile(id: "large", selected: true, now: 2_000))
            try store.reportModelRuntime(MacCanonicalModelRuntime(
                hostID: "mac:test-host",
                profileID: "large",
                phase: "ready",
                endpointJSON: #"{"base_url":"http://127.0.0.1:18080/v1"}"#,
                processID: 42,
                error: nil,
                startedAtMs: 2_100,
                stoppedAtMs: nil,
                updatedAtMs: 2_200
            ))
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let profiles = try reopened.listModelProfiles()
        XCTAssertEqual(profiles.filter(\.selected).map(\.id), ["large"])
        let runtime = try XCTUnwrap(reopened.modelRuntime(hostID: "mac:test-host"))
        XCTAssertEqual(runtime.profileID, "large")
        XCTAssertEqual(runtime.phase, "ready")
        XCTAssertTrue(runtime.endpointJSON?.contains("127.0.0.1:18080") == true)
    }

    func testConversationPlanSurvivesRealCoreABIReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-conversation-plan-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db", isDirectory: false)
        defer { try? FileManager.default.removeItem(at: root) }

        func planStep(status: String) -> MacCanonicalJSONValue {
            var fields: [String: MacCanonicalJSONValue] = [
                "id": .string("inspect"),
                "content": .string("Inspect"),
                "activeForm": .string("Inspecting"),
                "status": .string(status),
                "priority": .string("medium"),
                "dependencyIds": .array([]),
                "approvalStatus": .string("not_required"),
                "capabilityRefs": .array([]),
                "expectedArtifactRefs": .array([]),
                "artifactRefs": .array([]),
                "workStepTransactionRefs": .array([]),
                "attempt": .number(0),
                "successCriteria": .array([.string("Inspection completed")]),
                "evidenceRefs": .array(status == "completed" ? [.string("inspection-log")] : []),
                "verificationStatus": .string(status == "completed" ? "passed" : "pending"),
            ]
            if status == "completed" {
                fields["terminationReason"] = .string("success")
            }
            return .object(fields)
        }

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            let first = try store.saveConversationPlan(
                conversationID: "plan-conversation",
                expectedRevision: nil,
                todos: .array([planStep(status: "in_progress")]),
                updatedAtMs: 100
            )
            XCTAssertEqual(first.revision, 1)
            let second = try store.saveConversationPlan(
                conversationID: "plan-conversation",
                expectedRevision: first.revision,
                todos: .array([planStep(status: "completed")]),
                updatedAtMs: 200
            )
            XCTAssertEqual(second.revision, 2)
            XCTAssertThrowsError(try store.saveConversationPlan(
                conversationID: "plan-conversation", expectedRevision: first.revision,
                todos: .array([planStep(status: "in_progress")]), updatedAtMs: 300
            ))
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let restored = try XCTUnwrap(reopened.conversationPlan("plan-conversation"))
        XCTAssertEqual(restored.revision, 2)
        XCTAssertEqual(
            restored.todos,
            .array([planStep(status: "completed")])
        )
    }

    func testProductSettingRevisionAndDeletionSurviveRealCoreABIReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-product-setting-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db", isDirectory: false)
        defer { try? FileManager.default.removeItem(at: root) }

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            let saved = try store.putProductSetting(
                key: MacProductSettingKeys.memoryEnabled,
                valueJSON: #"{"schemaVersion":1,"value":false}"#,
                updatedAtMs: 100
            )
            XCTAssertEqual(saved.revision, 1)
        }

        do {
            let reopened = try MacApplicationPort(databaseURL: databaseURL)
            XCTAssertTrue(try XCTUnwrap(
                reopened.productSetting(key: MacProductSettingKeys.memoryEnabled)
            ).valueJSON.contains("false"))
            let updated = try reopened.putProductSetting(
                key: MacProductSettingKeys.memoryEnabled,
                valueJSON: #"{"schemaVersion":1,"value":true}"#,
                updatedAtMs: 200
            )
            XCTAssertEqual(updated.revision, 2)
            XCTAssertTrue(try reopened.deleteProductSetting(key: MacProductSettingKeys.memoryEnabled))
        }

        let final = try MacApplicationPort(databaseURL: databaseURL)
        XCTAssertNil(try final.productSetting(key: MacProductSettingKeys.memoryEnabled))
    }

    func testAssistantDefaultsUseOnlyCanonicalProductSettingsAcrossReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-assistant-defaults-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db", isDirectory: false)
        defer { try? FileManager.default.removeItem(at: root) }

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            MacAssistantPreferences.setMemoryEnabled(false, in: store)
            MacAssistantPreferences.setUserPreferences("Prefer concise answers", in: store)
            MacAssistantPreferences.setRetrievalEnabled(false, in: store)
            MacAssistantPreferences.setContextLimit(96_000, in: store)
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        XCTAssertFalse(MacAssistantPreferences.memoryEnabled(in: reopened))
        XCTAssertEqual(MacAssistantPreferences.userPreferences(in: reopened), "Prefer concise answers")
        XCTAssertFalse(MacAssistantPreferences.retrievalEnabled(in: reopened))
        XCTAssertEqual(MacAssistantPreferences.contextLimit(in: reopened), 96_000)
    }

    func testLongTermMemoryRequiresExplicitOptIn() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-memory-default-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("product.db"))
        XCTAssertFalse(MacAssistantPreferences.memoryEnabled(in: store))
        MacAssistantPreferences.setMemoryEnabled(true, in: store)
        XCTAssertTrue(MacAssistantPreferences.memoryEnabled(in: store))
    }

    func testModelParametersUseCanonicalModelProfileAcrossColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-parameters-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db", isDirectory: false)
        defer { try? FileManager.default.removeItem(at: root) }

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            let parameters = ModelParameters(
                maxContextLength: 131_072,
                temperature: 0.23,
                responseStyle: .professional
            )
            try store.upsertModelProfile(modelProfile(id: "qwen3.5-4b", selected: true, now: 1))
            try ModelParametersStore(applicationPort: store)
                .saveParameters(parameters, for: "qwen3.5-4b", isLocal: true)
        }

        let reopenedStore = try MacApplicationPort(databaseURL: databaseURL)
        let reopened = ModelParametersStore(applicationPort: reopenedStore)
            .loadParameters(for: "qwen3.5-4b", isLocal: true)
        XCTAssertEqual(reopened.temperature, 0.23)
        XCTAssertEqual(reopened.maxContextLength, 131_072)
        XCTAssertEqual(reopened.responseStyle, .professional)
        XCTAssertFalse(try XCTUnwrap(reopenedStore.modelProfile("qwen3.5-4b")).parametersJSON.isEmpty)
    }

    func testRemoteModelParametersResolveCanonicalProfileByModelNameWithoutDuplicate() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-remote-model-parameters-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db", isDirectory: false)
        defer { try? FileManager.default.removeItem(at: root) }

        let store = try MacApplicationPort(databaseURL: databaseURL)
        try store.upsertModelProfile(MacCanonicalModelProfile(
            id: "remote:qwen-vl",
            displayName: "Qwen VL",
            provider: "api",
            modelName: "qwen-vl",
            runtime: "remote",
            sourceJSON: #"{"baseURL":"https://example.invalid/v1"}"#,
            parametersJSON: "{}",
            selected: true,
            enabled: true,
            revision: 1,
            createdAtMs: 1_000,
            updatedAtMs: 1_000,
            deletedAtMs: nil
        ))
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.19
        try ModelParametersStore(applicationPort: store)
            .saveParameters(parameters, for: "qwen-vl", isLocal: false)

        XCTAssertNil(try store.modelProfile("qwen-vl"))
        let canonical = try XCTUnwrap(store.modelProfile("remote:qwen-vl"))
        let decoded = try JSONDecoder().decode(
            ModelParameters.self,
            from: try XCTUnwrap(canonical.parametersJSON.data(using: .utf8))
        )
        XCTAssertEqual(decoded.temperature, 0.19)
    }

    func testMissingModelParameterSaveDoesNotCreateProfileOrPublishSuccess() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-missing-model-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try MacApplicationPort(databaseURL: root.appendingPathComponent("product.db"))
        let parameters = ModelParametersStore(applicationPort: runtime)
        XCTAssertThrowsError(try parameters.saveParameters(.localDefaults, for: "missing", isLocal: true))
        XCTAssertNil(parameters.lastUpdated)
        XCTAssertTrue(try runtime.listModelProfiles().isEmpty)
    }

    private func modelProfile(id: String, selected: Bool, now: Int64) -> MacCanonicalModelProfile {
        MacCanonicalModelProfile(
            id: id,
            displayName: id.capitalized,
            provider: "local",
            modelName: id,
            runtime: "mlx",
            sourceJSON: #"{"format":"mlx"}"#,
            parametersJSON: #"{"maxContextLength":65536}"#,
            selected: selected,
            enabled: true,
            revision: 1,
            createdAtMs: now,
            updatedAtMs: now,
            deletedAtMs: nil
        )
    }
}
