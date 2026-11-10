// SPDX-License-Identifier: Apache-2.0

import Foundation
import Testing
@testable import Nola

@Suite("Mac Chrome Messaging End to End", .serialized)
struct MacChromeMessagingEndToEndTests {
    @Test("Exporting an available Playwright runtime never launches its executable")
    func playwrightEnvironmentExportDoesNotLaunchProcess() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let bundleURL = directory.appendingPathComponent("RuntimeFixture.bundle")
        let contents = bundleURL.appendingPathComponent("Contents")
        let runtime = contents.appendingPathComponent("Resources/NolaPlaywrightRuntime")
        try FileManager.default.createDirectory(
            at: runtime.appendingPathComponent("bin"), withIntermediateDirectories: true
        )
        let plist = try PropertyListSerialization.data(fromPropertyList: [
            "CFBundleIdentifier": "com.nola.tests.lazy-browser",
            "CFBundlePackageType": "BNDL"
        ], format: .xml, options: 0)
        try plist.write(to: contents.appendingPathComponent("Info.plist"))
        let executable = runtime.appendingPathComponent("bin/node")
        try Data("#!/bin/sh\n/usr/bin/touch \"$0.started\"\nexit 1\n".utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: executable.path)
        try Data().write(to: runtime.appendingPathComponent("bridge.cjs"))
        let bundle = try #require(Bundle(url: bundleURL))
        #expect(!MacPlaywrightRuntimeService.shared.snapshot(bundle: bundle).isAvailable,
            "A runtime missing its owner queue must not be exported")
        try Data().write(to: runtime.appendingPathComponent("owner_queue.cjs"))
        let broker = MacChromeMessagingConfiguration(
            serverURL: URL(string: "http://127.0.0.1:12345")!,
            token: "fixture-token", hostID: "fixture", localUserID: "fixture",
            fileURL: directory.appendingPathComponent("unused.json")
        )
        for _ in 0..<3 {
            var environment = ["NOLA_PLAYWRIGHT_RPC_URL": "stale-endpoint"]
            MacPlaywrightRuntimeService.shared.apply(to: &environment, broker: broker, bundle: bundle)
            #expect(environment["NOLA_PLAYWRIGHT_AVAILABLE"] == "1")
            #expect(environment["NOLA_PLAYWRIGHT_NODE"] == executable.path)
            #expect(environment["NOLA_PLAYWRIGHT_RPC_URL"] == "http://127.0.0.1:12345/api/native-browser/managed/rpc")
            #expect(environment["NOLA_PLAYWRIGHT_RPC_TOKEN"] == broker.token)
            #expect(!FileManager.default.fileExists(atPath: executable.path + ".started"))
        }
        #expect(MacPlaywrightRuntimeService.shared.snapshot(bundle: bundle).stateRootURL.path
            .hasPrefix(MacAppEnvironment.shared.supportRoot.path + "/"))
    }

    @Test("Managed browser starts only for authenticated tool requests through the app broker")
    func managedBrowserBrokerIsLazyAndPreservesRPCContract() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let calls = MacChromeAssistanceCounter()
        let service = MacChromeMessagingService(installer: nil, managedBrowserAction: { action, args in
            calls.increment()
            if action == "fail" {
                throw NSError(domain: "BrowserTest", code: 1)
            }
            return .object(["action": .string(action), "args": .object(args)])
        })
        let configuration = try service.ensureStarted(configuration: MacChromeMessagingConfiguration(
            serverURL: URL(string: "http://127.0.0.1:0")!,
            token: "isolated-managed-browser-token",
            hostID: "test-host",
            localUserID: "test-user",
            fileURL: directory.appendingPathComponent("broker.json")
        ))
        defer { service.stop() }
        #expect(calls.value == 0)

        // Sending multiple turns only exports metadata, even with a bundled runtime.
        for _ in 0..<3 {
            var environment: [String: String] = [:]
            MacPlaywrightRuntimeService.shared.apply(to: &environment, broker: configuration)
            #expect(environment["NOLA_PLAYWRIGHT_AVAILABLE"] != nil)
        }
        _ = try await URLSession.shared.data(from: configuration.serverURL.appendingPathComponent("api/health"))
        #expect(calls.value == 0)

        func send(token: String, body: [String: Any]) async throws -> (Int, [String: Any]) {
            var request = URLRequest(url: configuration.serverURL
                .appendingPathComponent("api/native-browser/managed/rpc"))
            request.httpMethod = "POST"
            request.setValue(token, forHTTPHeaderField: "X-Nola-Playwright-Token")
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
            let (data, response) = try await URLSession.shared.data(for: request)
            return (
                try #require(response as? HTTPURLResponse).statusCode,
                try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
            )
        }

        let unauthorized = try await send(token: "wrong-token", body: ["action": "start", "args": [:]])
        #expect(unauthorized.0 == 401)
        let malformed = try await send(token: configuration.token, body: ["args": [:]])
        #expect(malformed.0 == 400)
        #expect(calls.value == 0)

        for action in ["start", "snapshot"] {
            let response = try await send(token: configuration.token, body: [
                "action": action, "args": ["session_id": "same-session"]
            ])
            #expect(response.0 == 200)
            let result = try #require(response.1["result"] as? [String: Any])
            #expect(result["action"] as? String == action)
            let args = try #require(result["args"] as? [String: Any])
            #expect(args["session_id"] as? String == "same-session")
        }
        #expect(calls.value == 2)
        let failure = try await send(token: configuration.token, body: ["action": "fail", "args": [:]])
        #expect(failure.0 == 503)
        #expect(failure.1["error"] as? String != nil)
    }

    @Test("XCTest keeps the Chrome native host manifest out of the user Chrome profile")
    func nativeHostManifestIsTestIsolated() {
        let manifestURL = MacChromeExtensionInstaller.nativeManifestURL.standardizedFileURL
        let testSupportRoot = MacAppEnvironment.shared.supportRoot.standardizedFileURL
        let userChromeRoot = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(
                "Library/Application Support/Google/Chrome/NativeMessagingHosts",
                isDirectory: true
            )
            .standardizedFileURL

        #expect(MacProcessRuntimePolicy.isRunningTests)
        #expect(manifestURL.path.hasPrefix(testSupportRoot.path + "/"))
        #expect(!manifestURL.path.hasPrefix(userChromeRoot.path + "/"))
    }

    @Test("Disconnected Chrome returns a recoverable error and debounces setup assistance")
    func disconnectedChromeRecoveryIsStructuredAndDebounced() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let requestedConfiguration = MacChromeMessagingConfiguration(
            serverURL: URL(string: "http://127.0.0.1:0")!,
            token: "native-mac-recovery-token",
            hostID: "chrome-default",
            localUserID: "mac-local-user",
            fileURL: directory.appendingPathComponent("native_browser_host.json")
        )
        let assistance = MacChromeAssistanceCounter()
        let service = MacChromeMessagingService(installer: nil, setupAssistance: {
            assistance.increment()
        })
        let configuration = try service.ensureStarted(configuration: requestedConfiguration)
        defer { service.stop() }

        let first = try await postAllowingFailure(
            configuration: configuration,
            path: "/api/native-browser/command",
            object: ["action": "open_page", "args": ["url": "https://example.com"]]
        )
        let second = try await postAllowingFailure(
            configuration: configuration,
            path: "/api/native-browser/command",
            object: ["action": "inspect", "args": [:]]
        )

        #expect(first.status == 503)
        #expect(first.object["code"] as? String == "chrome_extension_not_connected")
        #expect(first.object["recovery_action"] as? String == "open_browser_setup")
        #expect(second.status == 503)
        #expect(assistance.value == 1)
    }

    @Test("Mac broker carries a command from Agent Host to Chrome and returns its result")
    func brokerRoundTrip() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: directory) }
        let requestedConfiguration = MacChromeMessagingConfiguration(
            serverURL: URL(string: "http://127.0.0.1:0")!,
            token: "native-mac-test-token",
            hostID: "chrome-default",
            localUserID: "mac-local-user",
            fileURL: directory.appendingPathComponent("native_browser_host.json")
        )
        let service = MacChromeMessagingService(installer: nil)
        let configuration = try service.ensureStarted(configuration: requestedConfiguration)
        defer { service.stop() }

        let identity: [String: Any] = [
            "host_id": "chrome-default",
            "instance_id": "extension-instance",
            "manifest_id": "extension-manifest",
            "manifest_version": 1,
            "issued_at_ms": UInt64(Date().timeIntervalSince1970 * 1000),
            "expires_at_ms": UInt64(Date().timeIntervalSince1970 * 1000) + 90_000,
            "capabilities": ["open_page", "inspect"],
            "extension_version": "0.1.0",
            "wait_sec": 3
        ]
        let pollTask = Task {
            try await post(
                configuration: configuration,
                path: "/api/native-browser/host/poll",
                object: identity
            )
        }
        try await Task.sleep(for: .milliseconds(100))
        let commandTask = Task {
            try await post(
                configuration: configuration,
                path: "/api/native-browser/command",
                object: [
                    "host_id": "chrome-default",
                    "action": "open_page",
                    "args": ["url": "https://example.com"],
                    "timeout_sec": 3
                ]
            )
        }

        let pollResponse = try await pollTask.value
        let command = try #require(pollResponse["command"] as? [String: Any])
        #expect(command["action"] as? String == "open_page")
        let commandID = try #require(command["id"] as? String)
        let resultResponse = try await post(
            configuration: configuration,
            path: "/api/native-browser/host/result",
            object: [
                "host_id": "chrome-default",
                "instance_id": "extension-instance",
                "manifest_id": "extension-manifest",
                "command_id": commandID,
                "result": [
                    "ok": true,
                    "final_url": "https://example.com/",
                    "title": "Example Domain"
                ]
            ]
        )
        #expect(resultResponse["accepted"] as? Bool == true)

        let commandResponse = try await commandTask.value
        let result = try #require(commandResponse["result"] as? [String: Any])
        #expect(result["ok"] as? Bool == true)
        #expect(result["title"] as? String == "Example Domain")
        #expect(service.snapshot().isExtensionConnected)

        let statusPoll = Task {
            try await post(
                configuration: configuration,
                path: "/api/native-browser/host/poll",
                object: identity
            )
        }
        try await Task.sleep(for: .milliseconds(100))
        let connectionCheck = Task.detached {
            service.checkConnection(timeout: 3)
        }
        let statusResponse = try await statusPoll.value
        let statusCommand = try #require(statusResponse["command"] as? [String: Any])
        #expect(statusCommand["action"] as? String == "status")
        let statusCommandID = try #require(statusCommand["id"] as? String)
        _ = try await post(
            configuration: configuration,
            path: "/api/native-browser/host/result",
            object: [
                "host_id": "chrome-default",
                "instance_id": "extension-instance",
                "manifest_id": "extension-manifest",
                "command_id": statusCommandID,
                "result": ["ok": true, "connected": true],
            ]
        )
        let check = await connectionCheck.value
        #expect(check.succeeded)
    }

    @Test("Independent brokers bind separate OS-assigned ports")
    func concurrentBrokersDoNotCompeteForGuessedPorts() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = MacChromeMessagingService(installer: nil)
        let second = MacChromeMessagingService(installer: nil)
        defer { first.stop(); second.stop() }
        func requested(_ id: String) -> MacChromeMessagingConfiguration {
            MacChromeMessagingConfiguration(
                serverURL: URL(string: "http://127.0.0.1:0")!, token: "token-\(id)",
                hostID: id, localUserID: "test", fileURL: directory.appendingPathComponent("\(id).json")
            )
        }
        let a = try first.ensureStarted(configuration: requested("first"))
        let b = try second.ensureStarted(configuration: requested("second"))
        #expect(a.serverURL.port != 0)
        #expect(b.serverURL.port != 0)
        #expect(a.serverURL.port != b.serverURL.port)
        for configuration in [a, b] {
            let (_, response) = try await URLSession.shared.data(
                from: configuration.serverURL.appendingPathComponent("api/health")
            )
            #expect((response as? HTTPURLResponse)?.statusCode == 200)
        }
        first.stop()
        let (_, response) = try await URLSession.shared.data(from: b.serverURL.appendingPathComponent("api/health"))
        #expect((response as? HTTPURLResponse)?.statusCode == 200)
    }

    private func post(
        configuration: MacChromeMessagingConfiguration,
        path: String,
        object: [String: Any]
    ) async throws -> [String: Any] {
        let url = try #require(
            URL(string: path, relativeTo: configuration.serverURL)?.absoluteURL
        )
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: object)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(
            configuration.token,
            forHTTPHeaderField: "X-Nola-Native-Browser-Token"
        )
        let (data, response) = try await URLSession.shared.data(for: request)
        let http = try #require(response as? HTTPURLResponse)
        #expect((200..<300).contains(http.statusCode))
        return try #require(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )
    }

    private func postAllowingFailure(
        configuration: MacChromeMessagingConfiguration,
        path: String,
        object: [String: Any]
    ) async throws -> (status: Int, object: [String: Any]) {
        let url = try #require(URL(string: path, relativeTo: configuration.serverURL)?.absoluteURL)
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.httpBody = try JSONSerialization.data(withJSONObject: object)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(configuration.token, forHTTPHeaderField: "X-Nola-Native-Browser-Token")
        let (data, response) = try await URLSession.shared.data(for: request)
        let http = try #require(response as? HTTPURLResponse)
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        return (http.statusCode, object)
    }
}

private final class MacChromeAssistanceCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0

    var value: Int {
        lock.withLock { count }
    }

    func increment() {
        lock.withLock { count += 1 }
    }
}
