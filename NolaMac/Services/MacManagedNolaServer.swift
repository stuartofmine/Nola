// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns the app-scoped Nola Server process used when this Mac is shared as a Host.
// - Boundary: Conversation workers remain isolated; this service owns only the persistent control plane.

import Foundation
import Observation

nonisolated struct MacManagedNolaServerInfo: Equatable, Sendable {
    let origin: URL
    let clientAPI: URL
    let hostID: String
    let accessToken: String
    let credentialsURL: URL

    var connectionSetupText: String? {
        let payload: [String: Any] = [
            "type": "nola.server.connection",
            "protocol_version": 1,
            "origin": origin.absoluteString,
            "client_api": clientAPI.absoluteString,
            "host_id": hostID,
            "access_token": accessToken,
        ]
        guard JSONSerialization.isValidJSONObject(payload),
              let data = try? JSONSerialization.data(
                withJSONObject: payload,
                options: [.prettyPrinted, .sortedKeys]
              ) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

nonisolated struct MacNolaServerConnectionSetup: Sendable {
    let origin: URL
    let hostID: String
    let accessToken: String

    static func decode(_ text: String) -> MacNolaServerConnectionSetup? {
        guard let data = text.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = object["type"] as? String,
              ["nola.server.connection", "server.started"].contains(type),
              (object["protocol_version"] as? NSNumber)?.intValue == 1,
              let rawOrigin = object["origin"] as? String,
              let origin = URL(string: rawOrigin),
              ["http", "https"].contains(origin.scheme?.lowercased() ?? ""),
              origin.host != nil,
              let hostID = object["host_id"] as? String,
              !hostID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let accessToken = object["access_token"] as? String,
              !accessToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return nil
        }
        return MacNolaServerConnectionSetup(
            origin: origin,
            hostID: hostID.trimmingCharacters(in: .whitespacesAndNewlines),
            accessToken: accessToken.trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }
}

nonisolated enum MacManagedNolaServerState: Equatable, Sendable {
    case stopped
    case starting
    case ready(MacManagedNolaServerInfo)
    case failed(String)

    var isRunning: Bool {
        if case .ready = self { return true }
        return false
    }
}

private struct MacManagedNolaServerHandshake: Decodable {
    let type: String
    let protocolVersion: Int
    let origin: URL
    let clientAPI: URL
    let hostID: String
    let accessToken: String
    let credentialsPath: String

    enum CodingKeys: String, CodingKey {
        case type, origin
        case protocolVersion = "protocol_version"
        case clientAPI = "client_api"
        case hostID = "host_id"
        case accessToken = "access_token"
        case credentialsPath = "credentials_path"
    }
}

enum MacManagedNolaServerError: LocalizedError {
    case executableUnavailable
    case startupTimedOut
    case invalidHandshake
    case launchFailed(String)

    var errorDescription: String? {
        switch self {
        case .executableUnavailable:
            return String(localized: "The Nola Server executable is unavailable. Rebuild the Mac target.")
        case .startupTimedOut:
            return String(localized: "Nola Server did not become ready in time.")
        case .invalidHandshake:
            return String(localized: "Nola Server returned an invalid startup handshake.")
        case .launchFailed(let detail):
            return String(format: String(localized: "Nola Server could not start: %@"), detail)
        }
    }
}

private final class MacManagedNolaServerReference: @unchecked Sendable {
    weak var value: MacManagedNolaServer?

    init(_ value: MacManagedNolaServer) {
        self.value = value
    }
}

@MainActor
@Observable
final class MacManagedNolaServer {
    static let shared = MacManagedNolaServer()
    static let enabledKey = "nola.mac.server.enabled"
    static let permissionProfileKey = "nola.mac.server.permissionProfile"

    private(set) var state: MacManagedNolaServerState = .stopped
    private var process: Process?
    private var stdoutTask: Task<Void, Never>?
    private var stderrTask: Task<Void, Never>?
    private var requestedStop = false

    func start() async {
        guard process?.isRunning != true else { return }
        state = .starting
        requestedStop = false

        do {
            guard let executableURL = MacAgentHostExecutableLocator.resolve() else {
                throw MacManagedNolaServerError.executableUnavailable
            }
            let process = Process()
            let output = Pipe()
            let errors = Pipe()
            process.executableURL = executableURL
            process.currentDirectoryURL = MacAppEnvironment.shared.scratchWorkspaceURL
            process.arguments = [
                "server",
                "--host", "0.0.0.0",
                "--port", "5001",
                "--json",
            ]
            process.standardOutput = output
            process.standardError = errors
            process.standardInput = FileHandle.nullDevice
            var environment = MacAppEnvironment.shared.launchAgentEnvironment(
                projectURL: MacAppEnvironment.shared.scratchWorkspaceURL
            )
            environment["NOLA_CLIENT_SURFACE"] = "mac_server"
            let storedProfile = UserDefaults.standard.object(forKey: Self.permissionProfileKey)
            guard storedProfile == nil || storedProfile is String else {
                throw MacManagedNolaServerError.launchFailed("Invalid stored permission profile.")
            }
            environment["NOLA_PERMISSION_PROFILE"] = try MacPermissionMode.configured(storedProfile as? String).rawValue
            process.environment = environment
            let serverReference = MacManagedNolaServerReference(self)
            process.terminationHandler = { terminated in
                let processIdentifier = terminated.processIdentifier
                let terminationStatus = terminated.terminationStatus
                Task { @MainActor in
                    serverReference.value?.processDidTerminate(
                        processIdentifier: processIdentifier,
                        status: terminationStatus
                    )
                }
            }

            try process.run()
            MacOwnedChildProcessRegistry.shared.register(process)
            self.process = process
            stderrTask = Task.detached {
                do {
                    for try await line in errors.fileHandleForReading.bytes.lines {
                        guard !Task.isCancelled else { return }
                        await Logger.shared.log(.debug, "MacNolaServer", line)
                    }
                } catch {
                    guard !Task.isCancelled else { return }
                    await Logger.shared.log(
                        .debug,
                        "MacNolaServer",
                        "Server stderr stream closed: \(error.localizedDescription)"
                    )
                }
            }

            let line = try await Self.readStartupLine(
                from: output.fileHandleForReading,
                timeout: .seconds(12)
            )
            let handshake = try JSONDecoder().decode(
                MacManagedNolaServerHandshake.self,
                from: Data(line.utf8)
            )
            guard handshake.type == "server.started",
                  handshake.protocolVersion == 1,
                  ["0.0.0.0", "::", "127.0.0.1", "::1"].contains(handshake.origin.host ?? ""),
                  handshake.clientAPI.path.hasPrefix("/api/mobile"),
                  !handshake.hostID.isEmpty,
                  !handshake.accessToken.isEmpty else {
                throw MacManagedNolaServerError.invalidHandshake
            }
            stdoutTask = Task.detached {
                do {
                    for try await line in output.fileHandleForReading.bytes.lines {
                        guard !Task.isCancelled else { return }
                        await Logger.shared.log(.debug, "MacNolaServer", line)
                    }
                } catch {
                    guard !Task.isCancelled else { return }
                    await Logger.shared.log(
                        .debug,
                        "MacNolaServer",
                        "Server stdout stream closed: \(error.localizedDescription)"
                    )
                }
            }
            let advertisedOrigin: URL
            if handshake.origin.host == "0.0.0.0" || handshake.origin.host == "::" {
                var components = URLComponents(url: handshake.origin, resolvingAgainstBaseURL: false)
                components?.host = ProcessInfo.processInfo.hostName
                guard let resolved = components?.url else {
                    throw MacManagedNolaServerError.invalidHandshake
                }
                advertisedOrigin = resolved
            } else {
                advertisedOrigin = handshake.origin
            }
            let advertisedClientAPI = advertisedOrigin
                .appendingPathComponent("api/mobile", isDirectory: false)
            state = .ready(
                MacManagedNolaServerInfo(
                    origin: advertisedOrigin,
                    clientAPI: advertisedClientAPI,
                    hostID: handshake.hostID,
                    accessToken: handshake.accessToken,
                    credentialsURL: URL(fileURLWithPath: handshake.credentialsPath)
                )
            )
        } catch {
            stopProcess()
            state = .failed(error.localizedDescription)
        }
    }

    func stop() {
        requestedStop = true
        stopProcess()
        state = .stopped
    }

    private func stopProcess() {
        stdoutTask?.cancel()
        stdoutTask = nil
        stderrTask?.cancel()
        stderrTask = nil
        if let process {
            MacOwnedChildProcessRegistry.shared.stop(process)
        }
        process = nil
    }

    private func processDidTerminate(processIdentifier: Int32, status: Int32) {
        guard process?.processIdentifier == processIdentifier else { return }
        if let process {
            MacOwnedChildProcessRegistry.shared.unregister(process)
        }
        process = nil
        stdoutTask?.cancel()
        stdoutTask = nil
        stderrTask?.cancel()
        stderrTask = nil
        if requestedStop {
            state = .stopped
        } else {
            state = .failed(
                String(
                    format: String(localized: "Nola Server stopped unexpectedly with status %d."),
                    status
                )
            )
        }
    }

    nonisolated private static func readStartupLine(
        from handle: FileHandle,
        timeout: Duration
    ) async throws -> String {
        try await withThrowingTaskGroup(of: String.self) { group in
            group.addTask {
                for try await line in handle.bytes.lines {
                    if !line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        return line
                    }
                }
                throw MacManagedNolaServerError.invalidHandshake
            }
            group.addTask {
                try await Task.sleep(for: timeout)
                throw MacManagedNolaServerError.startupTimedOut
            }
            guard let result = try await group.next() else {
                throw MacManagedNolaServerError.invalidHandshake
            }
            group.cancelAll()
            return result
        }
    }
}
