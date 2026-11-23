// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns the app-scoped Nola Server process used when this Mac is shared as a Host.
// - Boundary: Conversation workers remain isolated; this service owns only the persistent control plane.

import CryptoKit
import Foundation
import Darwin
import Observation
import SystemConfiguration

nonisolated enum MacHostAdvertisedAddress {
    static func origin(_ bound: URL, addresses: [String], localHostName: String?) -> URL? {
        var parts = URLComponents(url: bound, resolvingAgainstBaseURL: false)
        let host = addresses.first { address in
            guard !address.hasPrefix("127."), let url = URL(string: "http://\(address)") else { return false }
            return MobileHostTransportPolicy.allows(url)
        } ?? localHostName.map { $0.hasSuffix(".local") ? $0 : $0 + ".local" }
        guard let host else { return nil }
        parts?.host = host
        return parts?.url
    }

    static func localAddresses() -> [String] {
        var interfaces: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&interfaces) == 0 else { return [] }
        defer { freeifaddrs(interfaces) }
        var addresses: [(String, String)] = []
        var current = interfaces
        while let item = current {
            defer { current = item.pointee.ifa_next }
            let flags = Int32(item.pointee.ifa_flags)
            guard flags & IFF_UP != 0, flags & IFF_LOOPBACK == 0,
                  let address = item.pointee.ifa_addr, address.pointee.sa_family == UInt8(AF_INET) else { continue }
            var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(address, socklen_t(address.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 {
                addresses.append((String(cString: item.pointee.ifa_name), String(cString: host)))
            }
        }
        return addresses.sorted { $0.0.hasPrefix("en") && !$1.0.hasPrefix("en") }.map(\.1)
    }
}

nonisolated struct MacManagedTailcatConfiguration: Equatable, Sendable {
    let enabled: Bool
    let helperPath: String
    let helperSHA256: String

    var normalizedHelperPath: String {
        helperPath.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var normalizedSHA256: String {
        helperSHA256.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    private static func isValidSHA256Digest(_ digest: String) -> Bool {
        digest.utf8.count == 64 && digest.utf8.allSatisfy {
            (48...57).contains($0) || (97...102).contains($0)
        }
    }

    var isValidSHA256: Bool {
        Self.isValidSHA256Digest(normalizedSHA256)
    }

    var usesCustomHelper: Bool {
        !normalizedHelperPath.isEmpty
    }

    static func bundledHelperURL(bundle: Bundle = .main) -> URL? {
        var candidates: [URL] = []
        if let executableDir = bundle.executableURL?.deletingLastPathComponent() {
            candidates.append(executableDir.appendingPathComponent("nola-tailcat"))
        }
        candidates.append(bundle.bundleURL.appendingPathComponent("Contents/MacOS/nola-tailcat"))
        candidates.append(bundle.bundleURL.appendingPathComponent("Contents/Helpers/nola-tailcat"))
        for candidate in candidates {
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }

    static func bundledHelperSHA256(bundle: Bundle = .main) -> String? {
        var candidates: [URL] = []
        if let resourceURL = bundle.resourceURL {
            candidates.append(resourceURL.appendingPathComponent("nola-tailcat.sha256"))
        }
        candidates.append(bundle.bundleURL.appendingPathComponent("Contents/Resources/nola-tailcat.sha256"))
        for candidate in candidates {
            guard let contents = try? String(contentsOf: candidate, encoding: .utf8) else { continue }
            let digest = contents.trimmingCharacters(in: .whitespacesAndNewlines)
            if isValidSHA256Digest(digest) {
                return digest
            }
        }
        return nil
    }

    func resolved(bundle: Bundle = .main) throws -> (helperPath: String, helperSHA256: String)? {
        guard enabled else { return nil }
        if usesCustomHelper {
            try validate()
            return (normalizedHelperPath, normalizedSHA256)
        }
        guard let bundled = Self.bundledHelperURL(bundle: bundle) else {
            throw MacManagedNolaServerError.launchFailed(
                String(localized: "Built-in nola-tailcat helper is unavailable in this build. Rebuild with Go installed or choose a custom helper binary.")
            )
        }
        let digest: String
        if let pinned = Self.bundledHelperSHA256(bundle: bundle) {
            digest = pinned
        } else {
            digest = try Self.computeSHA256(forFileAt: bundled)
        }
        return (bundled.path, digest)
    }

    func validate() throws {
        guard enabled else { return }
        let path = normalizedHelperPath
        guard path.hasPrefix("/"), FileManager.default.isExecutableFile(atPath: path) else {
            throw MacManagedNolaServerError.launchFailed(
                String(localized: "Select an executable Tailcat helper binary before enabling Tailcat ingress.")
            )
        }
        guard isValidSHA256 else {
            throw MacManagedNolaServerError.launchFailed(
                String(localized: "Enter a valid 64-character SHA-256 digest for the Tailcat helper binary.")
            )
        }
    }

    static func load(from defaults: UserDefaults = .standard) -> Self {
        Self(
            enabled: defaults.bool(forKey: MacManagedNolaServer.tailcatEnabledKey),
            helperPath: defaults.string(forKey: MacManagedNolaServer.tailcatHelperPathKey) ?? "",
            helperSHA256: defaults.string(forKey: MacManagedNolaServer.tailcatHelperSHA256Key) ?? ""
        )
    }

    static func computeSHA256(forFileAt url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while let chunk = try handle.read(upToCount: 65_536), !chunk.isEmpty {
            hasher.update(data: chunk)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }
}

nonisolated struct MacManagedNolaServerInfo: Equatable, Sendable {
    let origin: URL
    let clientAPI: URL
    let hostID: String
    let accessToken: String
    let credentialsURL: URL
    let tailcatIngressActive: Bool

    init(
        origin: URL,
        clientAPI: URL,
        hostID: String,
        accessToken: String,
        credentialsURL: URL,
        tailcatIngressActive: Bool = false
    ) {
        self.origin = origin
        self.clientAPI = clientAPI
        self.hostID = hostID
        self.accessToken = accessToken
        self.credentialsURL = credentialsURL
        self.tailcatIngressActive = tailcatIngressActive
    }

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

typealias MacNolaServerConnectionSetup = AppleHostConnectionSetup

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
    static let tailcatEnabledKey = "nola.mac.server.tailcat.enabled"
    static let tailcatHelperPathKey = "nola.mac.server.tailcat.helperPath"
    static let tailcatHelperSHA256Key = "nola.mac.server.tailcat.helperSHA256"

    nonisolated static func launchArguments(
        bindAddress: String,
        port: Int,
        tailcat: MacManagedTailcatConfiguration?,
        bundle: Bundle = .main
    ) throws -> [String] {
        if let tailcat, let resolved = try tailcat.resolved(bundle: bundle) {
            guard port > 0 else {
                throw MacManagedNolaServerError.launchFailed(
                    String(localized: "Tailcat ingress requires a nonzero server port.")
                )
            }
            // Keep bindAddress (e.g. 0.0.0.0) so LAN/mobile clients can reach the grant
            // endpoint while --tailcat-ingress enforces authenticated remote ingress.
            return [
                "server",
                "--host", bindAddress,
                "--port", String(port),
                "--tailcat-ingress",
                "--tailcat-helper", resolved.helperPath,
                "--tailcat-helper-sha256", resolved.helperSHA256,
                "--json",
            ]
        }
        return [
            "server",
            "--host", bindAddress,
            "--port", String(port),
            "--json",
        ]
    }

    private(set) var state: MacManagedNolaServerState = .stopped
    private var process: Process?
    private var stdoutTask: Task<Void, Never>?
    private var stderrTask: Task<Void, Never>?
    private var launchRevision = 0
    private var pendingStop: Task<Void, Never>?
    private let environment: MacAppEnvironment
    private let executableOverride: URL?
    private let bindAddress: String
    private let port: Int
    private let startupTimeout: Duration

    init(environment: MacAppEnvironment = .shared, executable: URL? = nil,
         bindAddress: String = "0.0.0.0", port: Int = 5001, startupTimeout: Duration = .seconds(12)) {
        self.environment = environment
        self.executableOverride = executable
        self.bindAddress = bindAddress
        self.port = port
        self.startupTimeout = startupTimeout
    }

    func start() async {
        guard state != .starting, process?.isRunning != true else { return }
        launchRevision &+= 1
        let revision = launchRevision
        state = .starting
        await pendingStop?.value
        guard revision == launchRevision else { return }
        pendingStop = nil

        do {
            guard let executableURL = executableOverride ?? MacAgentHostExecutableLocator.resolve() else {
                throw MacManagedNolaServerError.executableUnavailable
            }
            let tailcatConfig = MacManagedTailcatConfiguration.load()
            let bindAddress = self.bindAddress
            let port = self.port
            let arguments = try await Task.detached(priority: .userInitiated) {
                try Self.launchArguments(
                    bindAddress: bindAddress,
                    port: port,
                    tailcat: tailcatConfig
                )
            }.value
            guard revision == launchRevision else { return }
            let process = Process()
            let output = Pipe()
            let errors = Pipe()
            process.executableURL = executableURL
            process.currentDirectoryURL = environment.scratchWorkspaceURL
            process.arguments = arguments
            process.standardOutput = output
            process.standardError = errors
            process.standardInput = FileHandle.nullDevice
            var environment = self.environment.launchAgentEnvironment(
                projectURL: self.environment.scratchWorkspaceURL
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

            let deadline = Task {
                try await Task.sleep(for: startupTimeout)
                guard revision == launchRevision, state == .starting else { return }
                launchRevision &+= 1
                stopProcess()
                state = .failed(MacManagedNolaServerError.startupTimedOut.localizedDescription)
            }
            defer { deadline.cancel() }
            let line = try await Self.readStartupLine(from: output.fileHandleForReading)
            guard revision == launchRevision, self.process === process, process.isRunning else { return }
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
                guard let resolved = MacHostAdvertisedAddress.origin(handshake.origin,
                    addresses: MacHostAdvertisedAddress.localAddresses(),
                    localHostName: SCDynamicStoreCopyLocalHostName(nil) as String?) else {
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
                    credentialsURL: URL(fileURLWithPath: handshake.credentialsPath),
                    tailcatIngressActive: tailcatConfig.enabled
                )
            )
        } catch {
            guard revision == launchRevision else { return }
            stopProcess()
            state = .failed(error.localizedDescription)
        }
    }

    func stop() {
        launchRevision &+= 1
        stopProcess()
        state = .stopped
    }

    private func stopProcess() {
        stdoutTask?.cancel()
        stdoutTask = nil
        stderrTask?.cancel()
        stderrTask = nil
        if let stopping = process {
            process = nil
            // The registry retains ownership while termination is in flight.
            // A subsequent launch waits for this task to release the listen port.
            pendingStop = Task.detached(priority: .userInitiated) {
                MacOwnedChildProcessRegistry.shared.stop(stopping)
                stopping.waitUntilExit()
            }
        }
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
        launchRevision &+= 1
        state = .failed(
            String(format: String(localized: "Nola Server stopped unexpectedly with status %d."), status)
        )
    }

    nonisolated private static func readStartupLine(from handle: FileHandle) async throws -> String {
        for try await line in handle.bytes.lines where !line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return line
        }
        throw MacManagedNolaServerError.invalidHandshake
    }
}
