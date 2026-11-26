// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

final class NolaTailcatTransportTests: XCTestCase {
    private final class AtomicCounter: @unchecked Sendable {
        private let lock = NSLock()
        private var value = 0
        func increment() -> Int { lock.withLock { value += 1; return value } }
        var current: Int { lock.withLock { value } }
    }

    private final class RecordingURLProtocolClient: NSObject, URLProtocolClient {
        private let lock = NSLock()
        private(set) var didFailCount = 0
        private(set) var didFinishCount = 0

        func urlProtocol(_ protocol: URLProtocol, wasRedirectedTo request: URLRequest, redirectResponse: URLResponse) {}
        func urlProtocol(_ protocol: URLProtocol, cachedResponseIsValid cachedResponse: CachedURLResponse) {}
        func urlProtocol(_ protocol: URLProtocol, didReceive response: URLResponse, cacheStoragePolicy policy: URLCache.StoragePolicy) {}
        func urlProtocol(_ protocol: URLProtocol, didLoad data: Data) {}
        func urlProtocolDidFinishLoading(_ protocol: URLProtocol) {
            lock.withLock { didFinishCount += 1 }
        }
        func urlProtocol(_ protocol: URLProtocol, didFailWithError error: Error) {
            lock.withLock { didFailCount += 1 }
        }
        func urlProtocol(_ protocol: URLProtocol, didReceive challenge: URLAuthenticationChallenge) {}
        func urlProtocol(_ protocol: URLProtocol, didCancel challenge: URLAuthenticationChallenge) {}
    }

    func testConcurrentLeaseExpiryRotatesWithoutCancellation() async throws {
        let coordinator = NolaTailcatLeaseCoordinator<Int>()
        let origin = URL(string: "http://192.168.1.50:5001")!
        let config = NolaTailcatTransport.Configuration(
            origin: origin,
            token: "tok-concurrent",
            hostID: "host-concurrent",
            generation: UUID()
        )
        let openCount = AtomicCounter()
        let closeCount = AtomicCounter()

        // First call opens a lease that is already inside the 30s expiry cutoff (10s remaining).
        let firstEndpoint = try await coordinator.endpoint(
            for: config,
            isParked: { false },
            open: { _ in
                let seq = openCount.increment()
                let url = URL(string: "http://127.0.0.1:\(40000 + seq)")!
                let ttl: TimeInterval = seq == 1 ? 10 : 600
                let snapshot = NolaTailcatLeaseCoordinator<Int>.LeaseSnapshot(
                    endpoint: url,
                    isTailcat: true,
                    leaseID: "lease-\(seq)",
                    expiresAt: Date().addingTimeInterval(ttl),
                    expiresIn: 600,
                    handle: seq
                )
                return (snapshot, { true }, { _ = closeCount.increment() })
            },
            renew: { _, _ in 600 },
            onModeChange: { _, _ in }
        )
        XCTAssertEqual(firstEndpoint.port, 40001)

        // Fire 6 concurrent callers while the first lease is expired (<= 30s).
        // None of them should fail with URLError(.cancelled); all should receive the rotated endpoint.
        let results = try await withThrowingTaskGroup(of: URL.self) { group in
            for _ in 0..<6 {
                group.addTask {
                    try await coordinator.endpoint(
                        for: config,
                        isParked: { false },
                        open: { _ in
                            try await Task.sleep(for: .milliseconds(30))
                            let seq = openCount.increment()
                            let url = URL(string: "http://127.0.0.1:\(40000 + seq)")!
                            let snapshot = NolaTailcatLeaseCoordinator<Int>.LeaseSnapshot(
                                endpoint: url,
                                isTailcat: true,
                                leaseID: "lease-\(seq)",
                                expiresAt: Date().addingTimeInterval(600),
                                expiresIn: 600,
                                handle: seq
                            )
                            return (snapshot, { true }, { _ = closeCount.increment() })
                        },
                        renew: { _, _ in 600 },
                        onModeChange: { _, _ in }
                    )
                }
            }
            var urls: [URL] = []
            for try await url in group { urls.append(url) }
            return urls
        }

        XCTAssertEqual(results.count, 6)
        XCTAssertTrue(results.allSatisfy { $0.port == 40002 })
        XCTAssertEqual(openCount.current, 2)
        XCTAssertEqual(closeCount.current, 1)
    }

    func testProactiveRenewalExtendsLeaseAndFallsBackOnUnknownOperation() async throws {
        let coordinator = NolaTailcatLeaseCoordinator<Int>()
        let origin = URL(string: "http://192.168.1.51:5001")!
        let config = NolaTailcatTransport.Configuration(
            origin: origin,
            token: "tok-renew",
            hostID: "host-renew",
            generation: UUID()
        )
        let openCount = AtomicCounter()
        let renewCount = AtomicCounter()

        // Open with 120s remaining out of 1200s (inside 25% / 300s proactive window, above 30s cutoff).
        let ep1 = try await coordinator.endpoint(
            for: config,
            isParked: { false },
            open: { _ in
                let seq = openCount.increment()
                let snapshot = NolaTailcatLeaseCoordinator<Int>.LeaseSnapshot(
                    endpoint: URL(string: "http://127.0.0.1:\(41000 + seq)")!,
                    isTailcat: true,
                    leaseID: "lease-\(seq)",
                    expiresAt: Date().addingTimeInterval(120),
                    expiresIn: 1200,
                    handle: seq
                )
                return (snapshot, { true }, {})
            },
            renew: { _, _ in
                _ = renewCount.increment()
                throw NolaTailcatTransportError.hostError(status: 400, code: "unknown_transport_operation")
            },
            onModeChange: { _, _ in }
        )
        XCTAssertEqual(ep1.port, 41001)

        // Second call triggers proactive renewal in background; host returns 400 unknown_transport_operation.
        let ep2 = try await coordinator.endpoint(
            for: config,
            isParked: { false },
            open: { _ in XCTFail("Should reuse active lease"); throw URLError(.cancelled) },
            renew: { _, _ in
                _ = renewCount.increment()
                throw NolaTailcatTransportError.hostError(status: 400, code: "unknown_transport_operation")
            },
            onModeChange: { _, _ in }
        )
        XCTAssertEqual(ep2, ep1)
        try await Task.sleep(for: .milliseconds(40))
        XCTAssertEqual(renewCount.current, 1)

        // Third call should NOT retry renew because renewUnsupported was recorded.
        let ep3 = try await coordinator.endpoint(
            for: config,
            isParked: { false },
            open: { _ in XCTFail("Should still reuse active lease"); throw URLError(.cancelled) },
            renew: { _, _ in
                _ = renewCount.increment()
                return 1200
            },
            onModeChange: { _, _ in }
        )
        XCTAssertEqual(ep3, ep1)
        try await Task.sleep(for: .milliseconds(40))
        XCTAssertEqual(renewCount.current, 1)
    }

    func testURLProtocolStopGuardSuppressesLateFailureCallbacks() {
        let request = URLRequest(url: URL(string: "http://192.168.1.52:5001/api/me")!)
        let client = RecordingURLProtocolClient()
        let proto = NolaTailcatURLProtocol(request: request, cachedResponse: nil, client: client)

        proto.stopLoading()
        proto.deliverFailure(URLError(.timedOut))

        XCTAssertEqual(client.didFailCount, 0)
    }

    func testTransientRegistrationOverridesAndCleansUp() {
        let origin = URL(string: "http://192.168.1.53:5001")!
        defer { NolaTailcatTransport.unregisterTransient(origin: origin) }

        NolaTailcatTransport.registerTransient(
            origin: origin,
            hostID: "draft-host-id",
            token: "draft-token",
            requireNativeAvailability: false
        )
        let config = NolaTailcatTransport.configuration(for: origin.appendingPathComponent("api/mobile/me"))
        XCTAssertEqual(config?.hostID, "draft-host-id")
        XCTAssertEqual(config?.token, "draft-token")

        NolaTailcatTransport.unregisterTransient(origin: origin)
        let after = NolaTailcatTransport.configuration(for: origin.appendingPathComponent("api/mobile/me"))
        XCTAssertNil(after)
    }

    func testErrorMappingCoversTailcatStagesAndHostErrorCodes() {
        let codes = [
            "dns",
            "certificate",
            "timeout",
            "permission",
            "tailcat_not_available",
            "tailcat_start_timeout",
            "transport_host_identity_mismatch",
            "transport_capacity_reached",
        ]
        for code in codes {
            let err = NolaTailcatTransportError.hostError(status: 503, code: code)
            let mapped = macRemoteHostUserFacingError(err)
            XCTAssertFalse(mapped.isEmpty, "Missing user-facing message for \(code)")
            XCTAssertNotEqual(mapped, err.localizedDescription + "unexpected")
            XCTAssertNotNil(NolaTailcatTransport.userFacingErrorMessage(for: err))
        }
    }

    func testManagedNolaServerTailcatLaunchArgumentsAndValidation() throws {
        // Default disabled config preserves existing args.
        let disabledArgs = try MacManagedNolaServer.launchArguments(
            bindAddress: "0.0.0.0",
            port: 5001,
            tailcat: MacManagedTailcatConfiguration(enabled: false, helperPath: "", helperSHA256: "")
        )
        XCTAssertEqual(disabledArgs, ["server", "--host", "0.0.0.0", "--port", "5001", "--json"])

        // Create a temporary executable helper file and compute its SHA-256.
        let tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }
        let helperURL = tempDir.appendingPathComponent("tailcat-helper")
        try Data("#!/bin/sh\nexit 0\n".utf8).write(to: helperURL)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: helperURL.path)

        let sha256 = try MacManagedTailcatConfiguration.computeSHA256(forFileAt: helperURL)
        XCTAssertEqual(sha256.count, 64)

        let enabledConfig = MacManagedTailcatConfiguration(
            enabled: true,
            helperPath: helperURL.path,
            helperSHA256: sha256.uppercased()
        )
        let enabledArgs = try MacManagedNolaServer.launchArguments(
            bindAddress: "0.0.0.0",
            port: 5001,
            tailcat: enabledConfig
        )
        XCTAssertEqual(enabledArgs, [
            "server",
            "--host", "0.0.0.0",
            "--port", "5001",
            "--tailcat-ingress",
            "--tailcat-helper", helperURL.path,
            "--tailcat-helper-sha256", sha256,
            "--json",
        ])

        // Invalid SHA-256 is rejected before launch.
        let badSHA = MacManagedTailcatConfiguration(enabled: true, helperPath: helperURL.path, helperSHA256: "not-a-sha")
        XCTAssertThrowsError(try MacManagedNolaServer.launchArguments(bindAddress: "0.0.0.0", port: 5001, tailcat: badSHA))
    }

    func testBundledTailcatHelperAutoDiscoveryWithoutManualConfig() throws {
        // NolaMac's Embed Nola Runtime Products build phase embeds nola-tailcat into Contents/MacOS/.
        let bundled = try XCTUnwrap(
            MacManagedTailcatConfiguration.bundledHelperURL(),
            "Expected nola-tailcat to be bundled in Nola.app/Contents/MacOS/nola-tailcat"
        )
        let pinnedSHA256 = try XCTUnwrap(
            MacManagedTailcatConfiguration.bundledHelperSHA256(),
            "Expected build-time nola-tailcat.sha256 in Nola.app/Contents/Resources/nola-tailcat.sha256"
        )
        let expectedSHA256 = try MacManagedTailcatConfiguration.computeSHA256(forFileAt: bundled)
        XCTAssertEqual(pinnedSHA256, expectedSHA256)
        let zeroConfig = MacManagedTailcatConfiguration(enabled: true, helperPath: "", helperSHA256: "")
        let args = try MacManagedNolaServer.launchArguments(
            bindAddress: "0.0.0.0",
            port: 5001,
            tailcat: zeroConfig
        )
        XCTAssertEqual(args, [
            "server",
            "--host", "0.0.0.0",
            "--port", "5001",
            "--tailcat-ingress",
            "--tailcat-helper", bundled.path,
            "--tailcat-helper-sha256", expectedSHA256,
            "--json",
        ])
    }
}
