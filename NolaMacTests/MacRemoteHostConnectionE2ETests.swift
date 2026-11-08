// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises the macOS client-to-Agent-Host handshake over a real loopback HTTP connection.

import Foundation
import XCTest
@testable import Nola

final class MacRemoteHostConnectionE2ETests: XCTestCase {
    func testRemoteModelCatalogUsesAuthenticatedSettingsAndHostOwnedIdentities() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-model-catalog-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let (server, port) = try startHostServer(reportURL: root.appendingPathComponent("request.json"))
        arrangeServerCleanup(server)
        let catalog = try await MacRemoteHostService.shared.fetchRemoteModelCatalog(configuration: .init(
            baseURL: XCTUnwrap(URL(string: "http://127.0.0.1:\(port)")),
            token: "valid-token", expectedHostID: "studio-host"
        ))
        XCTAssertEqual(catalog.profiles, [.init(hostID: "studio-host", profileID: "server-profile",
            revision: 11, displayName: "Host model", modelName: "same-name")])
        XCTAssertEqual(catalog.defaultResolution, .resolved(profileID: "server-profile", origin: .automatic))
        XCTAssertNil(catalog.defaultRevision)
    }
    func testConnectionUsesMobileSettingsContractAndParsesStableHostIdentity() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-host-handshake-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let reportURL = root.appendingPathComponent("request.json")
        let (server, port) = try startHostServer(reportURL: reportURL)
        arrangeServerCleanup(server)

        let report = try await MacRemoteHostService.shared.testConnection(
            configuration: MacRemoteHostConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/v1")),
                token: "valid-token"
            )
        )

        XCTAssertEqual(report.hostID, "studio-host")
        XCTAssertEqual(report.displayName, "Studio Mac")
        XCTAssertEqual(report.origin, "127.0.0.1")
        XCTAssertTrue(report.messagingWorkerEnabled)
        XCTAssertEqual(report.messagingChannelCount, 2)
        XCTAssertEqual(report.messagingConnectedCount, 1)
        XCTAssertEqual(report.messagingManagementURL?.path, "/settings")
        XCTAssertEqual(report.messagingManagementURL?.fragment, "channels")
        XCTAssertEqual(report.messagingManagementURL?.port, port)
        XCTAssertEqual(report.messagingManagementURL?.host, "127.0.0.1")
        let deadline = ContinuousClock.now + .seconds(2)
        while !FileManager.default.fileExists(atPath: reportURL.path), ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        let request = try JSONSerialization.jsonObject(with: Data(contentsOf: reportURL)) as? [String: String]
        XCTAssertEqual(request?["path"], "/api/mobile/agent/settings")
        XCTAssertEqual(
            request?["body_sha256"],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        )
        XCTAssertFalse(request?["timestamp"]?.isEmpty ?? true)
        XCTAssertFalse(request?["nonce"]?.isEmpty ?? true)
        XCTAssertFalse(request?["proof"]?.isEmpty ?? true)
    }

    func testControlPlaneURLDoesNotInheritHostAPIPath() throws {
        let base = try XCTUnwrap(URL(string: "https://studio.local:7443/api/mobile/agent/settings"))
        XCTAssertEqual(
            macRemoteHostControlPlaneURL(baseURL: base, path: "settings/channels?tab=active#telegram")?.absoluteString,
            "https://studio.local:7443/settings/channels?tab=active#telegram"
        )
    }

    func testRejectedTokenProducesLocalizedActionableError() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-host-auth-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let (server, port) = try startHostServer(reportURL: root.appendingPathComponent("request.json"))
        arrangeServerCleanup(server)

        do {
            _ = try await MacRemoteHostService.shared.testConnection(
                configuration: MacRemoteHostConfiguration(
                    baseURL: try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)")),
                    token: "expired-token"
                )
            )
            XCTFail("An invalid token must not produce a connected state")
        } catch {
            XCTAssertEqual(
                macRemoteHostUserFacingError(error),
                String(localized: "mac.host.error.unauthorized")
            )
        }
    }

    func testRemoteRunCancellationUsesMobileHostContract() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-host-cancel-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let reportURL = root.appendingPathComponent("request.json")
        let (server, port) = try startCancellationServer(reportURL: reportURL)
        arrangeServerCleanup(server)

        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("receiver.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: application)
        let source = try MacAssistantJournalTestFixture(conversationID: "remote-conversation", runID: "cancel-run",
            assistantMessageID: "cancel-run:assistant")
        try source.ingestAdmissions(into: application)
        let receipt = try source.submissionReceipt()
        let service = await MacRemoteHostService(configurationStore: configuration)
        try await service.cancelRun(
            identity: receipt.identity, requestDigest: receipt.request_digest,
            configuration: MacRemoteHostConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/v1")),
                token: "valid-token", expectedHostID: "fixture-host"
            )
        )

        let request = try JSONSerialization.jsonObject(
            with: Data(contentsOf: reportURL)
        ) as? [String: String]
        XCTAssertEqual(request?["method"], "POST")
        XCTAssertEqual(request?["path"], "/api/mobile/chat/agent/v2/cancel")
        XCTAssertFalse(request?["body_sha256"]?.isEmpty ?? true)
        XCTAssertFalse(request?["proof"]?.isEmpty ?? true)
    }

    func testOnlyActiveUncancelledCoreTaskOffersCancellation() {
        func task(state: MacCoreTaskState, cancelRequested: Bool) -> MacCoreTaskHandle {
            MacCoreTaskHandle(
                id: UUID().uuidString,
                ownerSessionID: "session",
                ownerTurnID: "turn",
                kind: .agent,
                state: state,
                progress: nil,
                currentStep: nil,
                resultRef: nil,
                cancelRequested: cancelRequested,
                lineage: nil,
                durability: nil
            )
        }

        XCTAssertTrue(MacCoreTaskUIModel(task: task(state: .running, cancelRequested: false)).canCancel)
        XCTAssertFalse(MacCoreTaskUIModel(task: task(state: .running, cancelRequested: true)).canCancel)
        XCTAssertFalse(MacCoreTaskUIModel(task: task(state: .completed, cancelRequested: false)).canCancel)
    }

    private func arrangeServerCleanup(_ server: Process) {
        let exited = XCTestExpectation(description: "Local test host terminated")
        server.terminationHandler = { _ in exited.fulfill() }
        addTeardownBlock {
            if server.isRunning {
                server.terminate()
            } else {
                exited.fulfill()
            }
            // Waiting synchronously here can prevent Foundation from delivering
            // the termination callback. Keep teardown bounded and cooperative.
            let result = await XCTWaiter.fulfillment(of: [exited], timeout: 5)
            guard result == .completed else {
                XCTFail("Local test host did not report termination within five seconds.")
                if server.isRunning {
                    kill(server.processIdentifier, SIGKILL)
                }
                return
            }
        }
    }

    private func startHostServer(reportURL: URL) throws -> (Process, Int) {
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [
            "-u",
            "-c",
            """
            import base64, hashlib, hmac, http.server, json, os, socketserver

            report_path = os.environ["NOLA_HOST_REPORT"]

            class Handler(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    timestamp = self.headers.get("X-Nola-Mobile-Timestamp", "")
                    nonce = self.headers.get("X-Nola-Mobile-Nonce", "")
                    body_sha256 = self.headers.get("X-Nola-Mobile-Body-SHA256", "")
                    proof = self.headers.get("X-Nola-Mobile-Auth", "")
                    with open(report_path, "w", encoding="utf-8") as handle:
                        json.dump({
                            "path": self.path,
                            "timestamp": timestamp,
                            "nonce": nonce,
                            "body_sha256": body_sha256,
                            "proof": proof,
                        }, handle)
                    message = f"v1\\nGET\\n{self.path}\\n{timestamp}\\n{nonce}\\n{body_sha256}".encode()
                    expected = base64.urlsafe_b64encode(
                        hmac.new(b"valid-token", message, hashlib.sha256).digest()
                    ).decode().rstrip("=")
                    if not timestamp or not nonce or not hmac.compare_digest(proof, expected):
                        body = json.dumps({"error": "unauthorized"}).encode()
                        self.send_response(401)
                    elif self.path != "/api/mobile/agent/settings":
                        body = json.dumps({"error": "wrong path"}).encode()
                        self.send_response(404)
                    else:
                        body = json.dumps({
                            "host_identity": {"id": "studio-host", "display_name": "Studio Mac"},
                            "default_chat_model_resolution": {"kind": "resolved", "profile_id": "server-profile", "origin": "automatic"},
                            "default_chat_model_revision": None,
                            "model_profiles": [{"profile_id": "server-profile", "revision": 11,
                                                "display_name": "Host model", "model_name": "same-name"}],
                            "messaging": {
                                "worker": {"enabled": True},
                                "management_path": "/settings#channels",
                                "channels": [
                                    {"kind": "telegram", "enabled": True, "state": "connected"},
                                    {"kind": "whatsapp", "enabled": True, "state": "degraded"}
                                ]
                            }
                        }).encode()
                        self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)

                def log_message(self, format, *args):
                    pass

            server = socketserver.TCPServer(("127.0.0.1", 0), Handler)
            print(server.server_address[1], flush=True)
            server.serve_forever()
            """,
        ]
        process.environment = ProcessInfo.processInfo.environment.merging([
            "NOLA_HOST_REPORT": reportURL.path,
        ]) { _, new in new }
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice
        try process.run()

        var bytes = Data()
        while !bytes.contains(0x0A) {
            let chunk = output.fileHandleForReading.availableData
            guard !chunk.isEmpty else {
                process.terminate()
                throw NSError(
                    domain: "MacRemoteHostConnectionE2ETests",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Local Agent Host exited before reporting its port."]
                )
            }
            bytes.append(chunk)
        }
        guard let line = String(decoding: bytes, as: UTF8.self).split(separator: "\n").first,
              let port = Int(line) else {
            process.terminate()
            throw NSError(
                domain: "MacRemoteHostConnectionE2ETests",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "Local Agent Host returned an invalid port."]
            )
        }
        return (process, port)
    }

    private func startCancellationServer(reportURL: URL) throws -> (Process, Int) {
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [
            "-u",
            "-c",
            """
            import hashlib, http.server, json, os, socketserver

            report_path = os.environ["NOLA_HOST_REPORT"]

            class Handler(http.server.BaseHTTPRequestHandler):
                def do_POST(self):
                    length = int(self.headers.get("Content-Length", "0"))
                    body = self.rfile.read(length)
                    with open(report_path, "w", encoding="utf-8") as handle:
                        json.dump({
                            "method": "POST",
                            "path": self.path,
                            "body_sha256": hashlib.sha256(body).hexdigest(),
                            "proof": self.headers.get("X-Nola-Mobile-Auth", ""),
                        }, handle)
                    payload = json.dumps({"accepted": True}).encode()
                    self.send_response(202)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(payload)))
                    self.end_headers()
                    self.wfile.write(payload)

                def log_message(self, format, *args):
                    pass

            server = socketserver.TCPServer(("127.0.0.1", 0), Handler)
            print(server.server_address[1], flush=True)
            server.serve_forever()
            """,
        ]
        process.environment = ProcessInfo.processInfo.environment.merging([
            "NOLA_HOST_REPORT": reportURL.path,
        ]) { _, new in new }
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice
        try process.run()

        var bytes = Data()
        while !bytes.contains(0x0A) {
            let chunk = output.fileHandleForReading.availableData
            guard !chunk.isEmpty else {
                process.terminate()
                throw NSError(
                    domain: "MacRemoteHostConnectionE2ETests",
                    code: 3,
                    userInfo: [NSLocalizedDescriptionKey: "Cancellation host exited before reporting its port."]
                )
            }
            bytes.append(chunk)
        }
        guard let line = String(decoding: bytes, as: UTF8.self).split(separator: "\n").first,
              let port = Int(line) else {
            process.terminate()
            throw NSError(
                domain: "MacRemoteHostConnectionE2ETests",
                code: 4,
                userInfo: [NSLocalizedDescriptionKey: "Cancellation host returned an invalid port."]
            )
        }
        return (process, port)
    }
}
