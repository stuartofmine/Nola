// SPDX-License-Identifier: Apache-2.0
// File Overview: Locates the Playwright runtime bundled with Nola Mac and exports its Host contract.

import Foundation
import AppKit

struct MacPlaywrightRuntimeSnapshot: Equatable, Sendable {
    let isAvailable: Bool
    let nodeURL: URL?
    let bridgeURL: URL?
    let stateRootURL: URL
}

actor MacPlaywrightRuntimeService {
    static let shared = MacPlaywrightRuntimeService()

    private struct ControlEndpoint: Decodable {
        let rpcURL: String
        let token: String

        private enum CodingKeys: String, CodingKey {
            case rpcURL = "rpc_url"
            case token
        }
    }

    private struct RPCRequest: Encodable {
        let action: String
        let args: [String: MacAgentTraceJSONValue]
    }

    private struct RPCResponse: Decodable {
        let result: MacAgentTraceJSONValue
    }

    private var process: Process?
    private var endpoint: ControlEndpoint?
    private var pendingControlURL: URL?
    private var pendingToken: String?
    private init() {
        _ = NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification,
            object: nil,
            queue: nil
        ) { [weak self] _ in
            Task { await self?.stop() }
        }
    }

    nonisolated func snapshot(bundle: Bundle = .main) -> MacPlaywrightRuntimeSnapshot {
        let runtimeRoot = bundle.resourceURL?.appendingPathComponent("NolaPlaywrightRuntime", isDirectory: true)
        let nodeURL = runtimeRoot?.appendingPathComponent("bin/node")
        let bridgeURL = runtimeRoot?.appendingPathComponent("bridge.cjs")
        let ownerQueueURL = runtimeRoot?.appendingPathComponent("owner_queue.cjs")
        let available = nodeURL.map { FileManager.default.isExecutableFile(atPath: $0.path) } == true
            && bridgeURL.map { FileManager.default.fileExists(atPath: $0.path) } == true
            && ownerQueueURL.map { FileManager.default.fileExists(atPath: $0.path) } == true
        let stateRoot = MacAppEnvironment.shared.supportRoot
            .appendingPathComponent("Browser", isDirectory: true)
        return MacPlaywrightRuntimeSnapshot(
            isAvailable: available,
            nodeURL: available ? nodeURL : nil,
            bridgeURL: available ? bridgeURL : nil,
            stateRootURL: stateRoot
        )
    }

    nonisolated func apply(
        to environment: inout [String: String],
        broker: MacChromeMessagingConfiguration,
        bundle: Bundle = .main
    ) {
        let runtime = snapshot(bundle: bundle)
        // Export a stable app-owned endpoint, not a per-turn browser process.
        // The broker calls perform only when a managed browser action arrives.
        environment.removeValue(forKey: "NOLA_PLAYWRIGHT_RPC_URL")
        environment.removeValue(forKey: "NOLA_PLAYWRIGHT_RPC_TOKEN")
        guard let nodeURL = runtime.nodeURL, let bridgeURL = runtime.bridgeURL else {
            environment["NOLA_PLAYWRIGHT_AVAILABLE"] = "0"
            return
        }
        environment["NOLA_PLAYWRIGHT_AVAILABLE"] = "1"
        environment["NOLA_PLAYWRIGHT_NODE"] = nodeURL.path
        environment["NOLA_PLAYWRIGHT_BRIDGE"] = bridgeURL.path
        environment["NOLA_PLAYWRIGHT_STATE_ROOT"] = runtime.stateRootURL.path
        environment["NOLA_PLAYWRIGHT_RPC_URL"] = broker.serverURL
            .appendingPathComponent("api/native-browser/managed/rpc").absoluteString
        environment["NOLA_PLAYWRIGHT_RPC_TOKEN"] = broker.token
    }

    func perform(
        action: String,
        arguments: [String: MacAgentTraceJSONValue],
        bundle: Bundle = .main
    ) async throws -> MacAgentTraceJSONValue {
        let runtime = snapshot(bundle: bundle)
        guard let endpoint = await ensureRunning(runtime: runtime),
              let url = URL(string: endpoint.rpcURL) else {
            throw NSError(
                domain: "MacPlaywrightRuntimeService",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "The managed browser is unavailable."]
            )
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(endpoint.token, forHTTPHeaderField: "X-Nola-Playwright-Token")
        request.httpBody = try JSONEncoder().encode(RPCRequest(action: action, args: arguments))
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              (200..<300).contains(httpResponse.statusCode) else {
            throw NSError(
                domain: "MacPlaywrightRuntimeService",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "The managed browser rejected the request."]
            )
        }
        return try JSONDecoder().decode(RPCResponse.self, from: data).result
    }

    private func ensureRunning(runtime: MacPlaywrightRuntimeSnapshot) async -> ControlEndpoint? {
        if process?.isRunning == true, let endpoint {
            return endpoint
        }
        guard let nodeURL = runtime.nodeURL, let bridgeURL = runtime.bridgeURL else {
            return nil
        }
        do {
            try FileManager.default.createDirectory(
                at: runtime.stateRootURL,
                withIntermediateDirectories: true
            )
        } catch {
            return nil
        }
        let launchedProcess: Process
        let controlURL: URL
        let token: String
        if let process, process.isRunning,
           let pendingControlURL,
           let pendingToken {
            launchedProcess = process
            controlURL = pendingControlURL
            token = pendingToken
        } else {
            controlURL = runtime.stateRootURL.appendingPathComponent("runtime-control.json")
            try? FileManager.default.removeItem(at: controlURL)
            token = UUID().uuidString
            let process = Process()
            process.executableURL = nodeURL
            process.arguments = [bridgeURL.path]
            var childEnvironment = ProcessInfo.processInfo.environment
            childEnvironment["NOLA_PLAYWRIGHT_STATE_ROOT"] = runtime.stateRootURL.path
            childEnvironment["NOLA_PLAYWRIGHT_CONTROL_FILE"] = controlURL.path
            childEnvironment["NOLA_PLAYWRIGHT_CONTROL_TOKEN"] = token
            childEnvironment["NOLA_PLAYWRIGHT_PARENT_PID"] = String(ProcessInfo.processInfo.processIdentifier)
            process.environment = childEnvironment
            process.standardInput = FileHandle.nullDevice
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice
            do {
                try process.run()
                MacOwnedChildProcessRegistry.shared.register(process)
            } catch {
                return nil
            }
            self.process = process
            pendingControlURL = controlURL
            pendingToken = token
            launchedProcess = process
        }

        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        while ContinuousClock.now < deadline, launchedProcess.isRunning, !Task.isCancelled {
            if let data = try? Data(contentsOf: controlURL),
               let endpoint = try? JSONDecoder().decode(ControlEndpoint.self, from: data),
               endpoint.token == token {
                self.endpoint = endpoint
                pendingControlURL = nil
                pendingToken = nil
                return endpoint
            }
            try? await Task.sleep(for: .milliseconds(20))
        }
        if self.process === launchedProcess {
            self.process = nil
            endpoint = nil
            pendingControlURL = nil
            pendingToken = nil
        }
        MacOwnedChildProcessRegistry.shared.stop(launchedProcess)
        return nil
    }

    private func stop() {
        let runningProcess = process
        process = nil
        endpoint = nil
        pendingControlURL = nil
        pendingToken = nil
        if let runningProcess {
            MacOwnedChildProcessRegistry.shared.stop(runningProcess)
        }
    }
}
