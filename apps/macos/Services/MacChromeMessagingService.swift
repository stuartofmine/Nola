// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns the Mac Chrome Native Messaging transport, extension installation, and command broker.

import AppKit
import Foundation
import Network

nonisolated struct MacChromeMessagingConfiguration: Equatable, Sendable {
    static let defaultPort: UInt16 = 5011
    static let defaultHostID = "chrome-default"
    static let defaultLocalUserID = "mac-local-user"
    static let extensionID = "abebajkkopomanodfidanihmfklgibbo"

    let serverURL: URL
    let token: String
    let hostID: String
    let localUserID: String
    let fileURL: URL

    static var defaultFileURL: URL {
        MacAppEnvironment.shared.supportRoot
            .appendingPathComponent("Chrome", isDirectory: true)
            .appendingPathComponent("native_browser_host.json")
    }

    static func load(from fileURL: URL = defaultFileURL) -> Self? {
        guard let data = try? Data(contentsOf: fileURL),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let rawServerURL = object["server_url"] as? String,
              let serverURL = URL(string: rawServerURL),
              serverURL.scheme?.lowercased() == "http",
              ["127.0.0.1", "localhost", "::1"].contains(serverURL.host?.lowercased() ?? ""),
              let token = object["token"] as? String,
              !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return nil
        }
        let hostID = nonempty(object["host_id"] as? String) ?? defaultHostID
        let localUserID = nonempty(object["local_user_id"] as? String) ?? defaultLocalUserID
        return Self(
            serverURL: serverURL,
            token: token,
            hostID: hostID,
            localUserID: localUserID,
            fileURL: fileURL
        )
    }

    static func loadOrCreate(
        fileURL: URL = defaultFileURL,
        port: UInt16 = defaultPort
    ) -> Self {
        if let existing = load(from: fileURL),
           existing.serverURL.port == Int(port) {
            return existing
        }
        let token = UUID().uuidString.replacingOccurrences(of: "-", with: "")
            + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        return Self(
            serverURL: URL(string: "http://127.0.0.1:\(port)")!,
            token: token,
            hostID: defaultHostID,
            localUserID: defaultLocalUserID,
            fileURL: fileURL
        )
    }

    static func runtimePort(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        isRunningTests: Bool = MacProcessRuntimePolicy.isRunningTests
    ) -> UInt16 {
        if let configured = environment["NOLA_NATIVE_BROWSER_PORT"].flatMap(UInt16.init) {
            return configured
        }
        return isRunningTests ? 0 : defaultPort
    }

    func persist(fileManager: FileManager = .default) throws {
        try fileManager.createDirectory(
            at: fileURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let object: [String: Any] = [
            "server_url": serverURL.absoluteString,
            "token": token,
            "host_id": hostID,
            "local_user_id": localUserID,
            "owner": "NolaMac"
        ]
        let data = try JSONSerialization.data(
            withJSONObject: object,
            options: [.prettyPrinted, .sortedKeys]
        )
        try data.write(to: fileURL, options: .atomic)
        try fileManager.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: fileURL.path
        )
    }

    func apply(to environment: inout [String: String]) {
        environment["NOLA_NATIVE_BROWSER_SERVER_URL"] = serverURL.absoluteString
        environment["NOLA_NATIVE_BROWSER_TOKEN"] = token
        environment["NOLA_NATIVE_BROWSER_LOCAL_USER_ID"] = localUserID
        environment["NOLA_NATIVE_BROWSER_HOST_ID"] = hostID
    }

    private static func nonempty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}

nonisolated struct MacChromeBridgeSnapshot: Equatable, Sendable {
    let isChromeInstalled: Bool
    let isServiceRunning: Bool
    let isNativeHostInstalled: Bool
    let isExtensionPrepared: Bool
    let isExtensionConnected: Bool
    let extensionVersion: String?
    let capabilities: [String]
    let lastSeen: Date?
    let errorMessage: String?

    var phase: MacChromeBridgePhase {
        if let errorMessage, !errorMessage.isEmpty { return .failed }
        if isExtensionConnected { return .connected }
        if !isChromeInstalled { return .chromeUnavailable }
        if !isServiceRunning { return .serviceStopped }
        if !isExtensionPrepared || !isNativeHostInstalled { return .installationRequired }
        return .waitingForChrome
    }
}

nonisolated enum MacChromeBridgePhase: String, Equatable, Sendable {
    case chromeUnavailable
    case serviceStopped
    case installationRequired
    case waitingForChrome
    case connected
    case failed
}

nonisolated struct MacChromeConnectionCheck: Equatable, Sendable {
    let succeeded: Bool
    let message: String
}

extension Notification.Name {
    nonisolated static let macChromeBridgeStatusDidChange =
        Notification.Name("NolaMac.ChromeBridgeStatusDidChange")
    nonisolated static let macChromeSetupAssistanceRequested =
        Notification.Name("NolaMac.ChromeSetupAssistanceRequested")
}

nonisolated struct MacChromeExtensionInstaller {
    let fileManager: FileManager
    let bundle: Bundle

    init(fileManager: FileManager = .default, bundle: Bundle = .main) {
        self.fileManager = fileManager
        self.bundle = bundle
    }

    static var installationRootURL: URL {
        MacAppEnvironment.shared.supportRoot
            .appendingPathComponent("Chrome", isDirectory: true)
    }

    static var extensionDirectoryURL: URL {
        installationRootURL.appendingPathComponent("Extension", isDirectory: true)
    }

    static var launcherURL: URL {
        installationRootURL.appendingPathComponent("nola_chrome_native_host.sh")
    }

    static var nativeManifestURL: URL {
        if MacProcessRuntimePolicy.isRunningTests {
            return installationRootURL
                .appendingPathComponent("NativeMessagingHosts", isDirectory: true)
                .appendingPathComponent("com.nola.native_browser.json")
        }
        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(
                "Library/Application Support/Google/Chrome/NativeMessagingHosts",
                isDirectory: true
            )
            .appendingPathComponent("com.nola.native_browser.json")
    }

    @discardableResult
    func install(configuration: MacChromeMessagingConfiguration) throws -> Bool {
        try configuration.persist(fileManager: fileManager)
        try fileManager.createDirectory(
            at: Self.installationRootURL,
            withIntermediateDirectories: true
        )
        try installExtensionFiles()
        try installNativeHost(configuration: configuration)
        return fileManager.fileExists(
            atPath: Self.extensionDirectoryURL
                .appendingPathComponent("manifest.json").path
        )
    }

    private func installExtensionFiles() throws {
        guard let sourceURL = extensionResourceURL else {
            throw NSError(
                domain: "NolaMac.ChromeExtension",
                code: 1,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        String(localized: "The Chrome extension is missing from this Mac build.")
                ]
            )
        }
        let sourceManifest = sourceURL.appendingPathComponent("manifest.json")
        let destinationManifest = Self.extensionDirectoryURL.appendingPathComponent("manifest.json")
        let sourceData = try? Data(contentsOf: sourceManifest)
        let destinationData = try? Data(contentsOf: destinationManifest)
        guard sourceData != destinationData else { return }
        if fileManager.fileExists(atPath: Self.extensionDirectoryURL.path) {
            try fileManager.removeItem(at: Self.extensionDirectoryURL)
        }
        try fileManager.copyItem(at: sourceURL, to: Self.extensionDirectoryURL)
    }

    private func installNativeHost(configuration: MacChromeMessagingConfiguration) throws {
        guard let executableURL = bundle.executableURL else {
            throw NSError(
                domain: "NolaMac.ChromeExtension",
                code: 2,
                userInfo: [
                    NSLocalizedDescriptionKey: String(localized: "Nola executable is unavailable.")
                ]
            )
        }
        let launcher = """
        #!/bin/sh
        export NOLA_NATIVE_BROWSER_CONFIG=\(shellQuote(configuration.fileURL.path))
        exec \(shellQuote(executableURL.path)) --nola-chrome-native-host
        """
        try Data(launcher.utf8).write(to: Self.launcherURL, options: .atomic)
        try fileManager.setAttributes(
            [.posixPermissions: 0o755],
            ofItemAtPath: Self.launcherURL.path
        )

        let manifestDirectory = Self.nativeManifestURL.deletingLastPathComponent()
        try fileManager.createDirectory(
            at: manifestDirectory,
            withIntermediateDirectories: true
        )
        let manifest: [String: Any] = [
            "name": "com.nola.native_browser",
            "description": "Nola for Mac Chrome bridge",
            "path": Self.launcherURL.path,
            "type": "stdio",
            "allowed_origins": [
                "chrome-extension://\(MacChromeMessagingConfiguration.extensionID)/"
            ]
        ]
        let data = try JSONSerialization.data(
            withJSONObject: manifest,
            options: [.prettyPrinted, .sortedKeys]
        )
        try data.write(to: Self.nativeManifestURL, options: .atomic)
    }

    private var extensionResourceURL: URL? {
        if let bundled = bundle.url(
            forResource: "NolaChromeExtension",
            withExtension: "bundle"
        ) {
            return bundled
        }
        return bundle.resourceURL?
            .appendingPathComponent("NolaChromeExtension.bundle", isDirectory: true)
    }

    private func shellQuote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}

nonisolated private final class MacChromeBrokerCommand: @unchecked Sendable {
    let id: String
    let hostID: String
    let action: String
    let args: [String: Any]
    var instanceID: String?
    var manifestID: String?

    init(id: String, hostID: String, action: String, args: [String: Any]) {
        self.id = id
        self.hostID = hostID
        self.action = action
        self.args = args
    }

    var payload: [String: Any] {
        [
            "id": id,
            "action": action,
            "args": args
        ]
    }
}

nonisolated final class MacChromeMessagingService: @unchecked Sendable {
    static let shared = MacChromeMessagingService()

    private let condition = NSCondition()
    private let lifecycleCondition = NSCondition()
    private let startupLock = NSLock()
    private let listenerQueue = DispatchQueue(
        label: "com.nola.mac.chrome.listener",
        qos: .userInitiated
    )
    private let workerQueue = DispatchQueue(
        label: "com.nola.mac.chrome.worker",
        qos: .utility,
        attributes: .concurrent
    )
    private let fileManager: FileManager
    private let installer: MacChromeExtensionInstaller?
    private var listener: NWListener?
    private var activeConfiguration: MacChromeMessagingConfiguration?
    private var startupError: String?
    private var listenerReady = false
    private var pendingCommands: [MacChromeBrokerCommand] = []
    private var claimedCommands: [String: MacChromeBrokerCommand] = [:]
    private var commandResults: [String: [String: Any]] = [:]
    private var lastHostSeen: Date?
    private var hostCapabilities: [String] = []
    private var extensionVersion: String?

    typealias ManagedBrowserAction = @Sendable (
        String, [String: MacAgentTraceJSONValue]
    ) async throws -> MacAgentTraceJSONValue

    private let managedBrowserAction: ManagedBrowserAction
    private let setupAssistance: (@Sendable () -> Void)?
    private let setupAssistanceCondition = NSCondition()
    private var lastSetupAssistanceAt: Date?

    init(
        fileManager: FileManager = .default,
        installer: MacChromeExtensionInstaller? = MacChromeExtensionInstaller(),
        managedBrowserAction: @escaping ManagedBrowserAction = { action, arguments in
            try await MacPlaywrightRuntimeService.shared.perform(action: action, arguments: arguments)
        },
        setupAssistance: (@Sendable () -> Void)? = nil
    ) {
        self.fileManager = fileManager
        self.installer = installer
        self.managedBrowserAction = managedBrowserAction
        self.setupAssistance = setupAssistance
    }

    @discardableResult
    func ensureStarted(
        configuration requestedConfiguration: MacChromeMessagingConfiguration =
            .loadOrCreate()
    ) throws -> MacChromeMessagingConfiguration {
        // App warm-up and the first chat can arrive on different queues.
        // Own the complete start transaction, including readiness and install.
        startupLock.lock()
        defer { startupLock.unlock() }
        lifecycleCondition.lock()
        if let activeConfiguration, listener != nil, listenerReady {
            lifecycleCondition.unlock()
            return activeConfiguration
        }
        listener?.cancel()
        listener = nil
        activeConfiguration = nil
        listenerReady = false
        startupError = nil
        lifecycleCondition.unlock()

        let parameters = NWParameters.tcp
        parameters.allowLocalEndpointReuse = true
        parameters.requiredLocalEndpoint = .hostPort(
            host: "127.0.0.1",
            port: NWEndpoint.Port(rawValue: UInt16(requestedConfiguration.serverURL.port ?? 0))!
        )
        let newListener = try NWListener(using: parameters)
        newListener.newConnectionHandler = { [weak self] connection in
            self?.accept(connection)
        }
        newListener.stateUpdateHandler = { [weak self, weak newListener] state in
            guard let self else { return }
            self.lifecycleCondition.lock()
            guard let newListener, self.listener === newListener else {
                self.lifecycleCondition.unlock()
                return
            }
            switch state {
            case .ready:
                self.listenerReady = true
                if let boundPort = newListener.port?.rawValue,
                   requestedConfiguration.serverURL.port != Int(boundPort) {
                    self.activeConfiguration = MacChromeMessagingConfiguration(
                        serverURL: URL(string: "http://127.0.0.1:\(boundPort)")!,
                        token: requestedConfiguration.token,
                        hostID: requestedConfiguration.hostID,
                        localUserID: requestedConfiguration.localUserID,
                        fileURL: requestedConfiguration.fileURL
                    )
                }
            case .failed(let error):
                self.listenerReady = false
                self.startupError = error.localizedDescription
            default:
                break
            }
            self.lifecycleCondition.broadcast()
            self.lifecycleCondition.unlock()
        }

        lifecycleCondition.lock()
        listener = newListener
        activeConfiguration = requestedConfiguration
        lifecycleCondition.unlock()
        newListener.start(queue: listenerQueue)

        lifecycleCondition.lock()
        let deadline = Date().addingTimeInterval(3)
        while !listenerReady, startupError == nil, lifecycleCondition.wait(until: deadline) {
            continue
        }
        let error = startupError
        let resolvedConfiguration = activeConfiguration ?? requestedConfiguration
        let ready = listenerReady
        lifecycleCondition.unlock()
        if let error {
            stop()
            throw NSError(
                domain: "NolaMac.ChromeBridge",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: error]
            )
        }
        guard ready else {
            stop()
            throw NSError(
                domain: "NolaMac.ChromeBridge",
                code: 2,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        String(localized: "The Chrome connection service did not start in time.")
                ]
            )
        }

        try resolvedConfiguration.persist(fileManager: fileManager)
        _ = try installer?.install(configuration: resolvedConfiguration)
        postStatusChanged()
        return resolvedConfiguration
    }

    func stop() {
        lifecycleCondition.lock()
        listener?.cancel()
        listener = nil
        listenerReady = false
        activeConfiguration = nil
        lifecycleCondition.broadcast()
        lifecycleCondition.unlock()

        condition.lock()
        for command in pendingCommands {
            commandResults[command.id] = [
                "ok": false,
                "error": String(localized: "Nola for Mac stopped the Chrome connection.")
            ]
        }
        pendingCommands.removeAll()
        claimedCommands.removeAll()
        condition.broadcast()
        condition.unlock()
        postStatusChanged()
    }

    var configuration: MacChromeMessagingConfiguration? {
        lifecycleCondition.lock()
        defer { lifecycleCondition.unlock() }
        return activeConfiguration
    }

    func snapshot(now: Date = .now) -> MacChromeBridgeSnapshot {
        lifecycleCondition.lock()
        let running = listener != nil && listenerReady
        let error = startupError
        lifecycleCondition.unlock()

        condition.lock()
        let lastSeen = lastHostSeen
        let connected = lastSeen.map { now.timeIntervalSince($0) < 45 } ?? false
        let capabilities = hostCapabilities
        let version = extensionVersion
        condition.unlock()

        return MacChromeBridgeSnapshot(
            isChromeInstalled: NSWorkspace.shared.urlForApplication(
                withBundleIdentifier: "com.google.Chrome"
            ) != nil,
            isServiceRunning: running,
            isNativeHostInstalled: fileManager.fileExists(
                atPath: MacChromeExtensionInstaller.nativeManifestURL.path
            ),
            isExtensionPrepared: fileManager.fileExists(
                atPath: MacChromeExtensionInstaller.extensionDirectoryURL
                    .appendingPathComponent("manifest.json").path
            ),
            isExtensionConnected: connected,
            extensionVersion: version,
            capabilities: capabilities,
            lastSeen: lastSeen,
            errorMessage: error
        )
    }

    func reinstall() throws {
        let configuration = try ensureStarted()
        _ = try installer?.install(configuration: configuration)
        postStatusChanged()
    }

    @discardableResult
    func beginExtensionInstallation() throws -> String {
        try reinstall()
        let path = MacChromeExtensionInstaller.extensionDirectoryURL.path
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(path, forType: .string)
        guard openChromeExtensions() else {
            throw NSError(
                domain: "MacChromeMessagingService",
                code: 3,
                userInfo: [NSLocalizedDescriptionKey: String(localized: "Google Chrome is not installed.")]
            )
        }
        return path
    }

    func checkConnection(timeout: TimeInterval = 8) -> MacChromeConnectionCheck {
        let current = snapshot()
        guard current.phase == .connected else {
            return MacChromeConnectionCheck(
                succeeded: false,
                message: String(localized: "Chrome is not connected. Open Chrome and enable the Nola extension.")
            )
        }
        let response = submitCommand([
            "action": "status",
            "args": [:],
            "timeout_sec": min(max(timeout, 1), 20),
        ])
        guard response.status == 200,
              response.object["ok"] as? Bool == true,
              let result = response.object["result"] as? [String: Any],
              result["ok"] as? Bool == true else {
            let result = response.object["result"] as? [String: Any]
            let message = result?["error"] as? String
                ?? response.object["error"] as? String
                ?? String(localized: "Chrome connection test failed.")
            return MacChromeConnectionCheck(succeeded: false, message: message)
        }
        return MacChromeConnectionCheck(
            succeeded: true,
            message: String(localized: "Chrome is connected and ready for browser actions.")
        )
    }

    func revealExtension() {
        NSWorkspace.shared.activateFileViewerSelecting([
            MacChromeExtensionInstaller.extensionDirectoryURL
        ])
    }

    @discardableResult
    func openChromeExtensions() -> Bool {
        guard let url = URL(string: "chrome://extensions") else { return false }
        if let chrome = NSWorkspace.shared.urlForApplication(
            withBundleIdentifier: "com.google.Chrome"
        ) {
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            NSWorkspace.shared.open(
                [url],
                withApplicationAt: chrome,
                configuration: configuration
            )
        } else {
            NSWorkspace.shared.open(url)
        }
        return true
    }

    private func accept(_ connection: NWConnection) {
        connection.start(queue: listenerQueue)
        receive(on: connection, accumulated: Data())
    }

    private func receive(on connection: NWConnection, accumulated: Data) {
        connection.receive(
            minimumIncompleteLength: 1,
            maximumLength: 64 * 1024
        ) { [weak self] data, _, isComplete, error in
            guard let self else {
                connection.cancel()
                return
            }
            var buffer = accumulated
            if let data { buffer.append(data) }
            if buffer.count > 2 * 1024 * 1024 {
                self.send(
                    status: 413,
                    object: ["error": String(localized: "Request is too large.")],
                    on: connection
                )
                return
            }
            if let request = Self.parseRequest(buffer) {
                if request.path == "/api/native-browser/managed/rpc" {
                    self.handleManagedBrowserRequest(request, on: connection)
                    return
                }
                self.workerQueue.async {
                    let response = self.route(request)
                    self.send(status: response.status, object: response.object, on: connection)
                }
                return
            }
            if isComplete || error != nil {
                self.send(
                    status: 400,
                    object: ["error": String(localized: "Malformed local Chrome bridge request.")],
                    on: connection
                )
                return
            }
            self.receive(on: connection, accumulated: buffer)
        }
    }

    nonisolated private struct HTTPRequest {
        let method: String
        let path: String
        let headers: [String: String]
        let body: [String: Any]
    }

    nonisolated private struct HTTPResponse {
        let status: Int
        let object: [String: Any]
    }

    private static func parseRequest(_ data: Data) -> HTTPRequest? {
        let separator = Data("\r\n\r\n".utf8)
        guard let headerRange = data.range(of: separator),
              let headerText = String(
                data: data[..<headerRange.lowerBound],
                encoding: .utf8
              ) else {
            return nil
        }
        let lines = headerText.components(separatedBy: "\r\n")
        guard let requestLine = lines.first else { return nil }
        let components = requestLine.split(separator: " ", omittingEmptySubsequences: true)
        guard components.count >= 2 else { return nil }
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            guard let separatorIndex = line.firstIndex(of: ":") else { continue }
            let key = line[..<separatorIndex]
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
            let value = line[line.index(after: separatorIndex)...]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            headers[key] = value
        }
        let contentLength = Int(headers["content-length"] ?? "0") ?? 0
        let bodyStart = headerRange.upperBound
        guard data.count >= bodyStart + contentLength else { return nil }
        let bodyData = Data(data[bodyStart..<(bodyStart + contentLength)])
        let body: [String: Any]
        if bodyData.isEmpty {
            body = [:]
        } else {
            guard let object = try? JSONSerialization.jsonObject(with: bodyData),
                  let dictionary = object as? [String: Any] else {
                return nil
            }
            body = dictionary
        }
        let rawPath = String(components[1])
        return HTTPRequest(
            method: String(components[0]).uppercased(),
            path: rawPath.split(separator: "?", maxSplits: 1).first.map(String.init) ?? rawPath,
            headers: headers,
            body: body
        )
    }

    private func route(_ request: HTTPRequest) -> HTTPResponse {
        if request.method == "GET", request.path == "/api/health" {
            return HTTPResponse(
                status: 200,
                object: ["ok": true, "service": "NolaMacChromeMessaging"]
            )
        }
        guard request.path.hasPrefix("/api/native-browser/"),
              authorized(request.headers) else {
            return HTTPResponse(status: 401, object: ["error": String(localized: "Unauthorized.")])
        }
        switch (request.method, request.path) {
        case ("GET", "/api/native-browser/status"):
            let snapshot = snapshot()
            return HTTPResponse(
                status: 200,
                object: [
                    "connected": snapshot.isExtensionConnected,
                    "host_id": configuration?.hostID ?? "",
                    "capabilities": snapshot.capabilities,
                    "extension_version": snapshot.extensionVersion ?? ""
                ]
            )
        case ("POST", "/api/native-browser/host/poll"):
            return poll(request.body)
        case ("POST", "/api/native-browser/host/result"):
            return acceptResult(request.body)
        case ("POST", "/api/native-browser/command"):
            return submitCommand(request.body)
        default:
            return HTTPResponse(status: 404, object: ["error": String(localized: "Not found.")])
        }
    }

    private struct ManagedBrowserRequest: Decodable {
        let action: String
        let args: [String: MacAgentTraceJSONValue]
    }

    private func handleManagedBrowserRequest(_ request: HTTPRequest, on connection: NWConnection) {
        // Match the existing Host Playwright RPC contract; authenticate before
        // invoking the lazy runtime, including for malformed/unknown actions.
        guard let expected = configuration?.token,
              let supplied = request.headers["x-nola-playwright-token"],
              supplied.utf8.elementsEqual(expected.utf8) else {
            send(status: 401, object: ["error": "Unauthorized."], on: connection)
            return
        }
        guard request.method == "POST",
              let data = try? JSONSerialization.data(withJSONObject: request.body),
              let command = try? JSONDecoder().decode(ManagedBrowserRequest.self, from: data),
              !command.action.isEmpty else {
            send(status: 400, object: ["error": "Malformed managed browser request."], on: connection)
            return
        }
        Task {
            do {
                let result = try await managedBrowserAction(command.action, command.args)
                let data = try JSONEncoder().encode(result)
                let object = try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
                send(status: 200, object: ["result": object], on: connection)
            } catch {
                send(status: 503, object: ["error": error.localizedDescription], on: connection)
            }
        }
    }

    private func authorized(_ headers: [String: String]) -> Bool {
        guard let expected = configuration?.token,
              let supplied = headers["x-nola-native-browser-token"] else {
            return false
        }
        return supplied.utf8.elementsEqual(expected.utf8)
    }

    private func poll(_ body: [String: Any]) -> HTTPResponse {
        guard let hostID = body["host_id"] as? String,
              !hostID.isEmpty,
              let instanceID = body["instance_id"] as? String,
              !instanceID.isEmpty,
              let manifestID = body["manifest_id"] as? String,
              !manifestID.isEmpty else {
            return HTTPResponse(
                status: 400,
                object: ["error": String(localized: "Host identity is required.")]
            )
        }
        let capabilities = (body["capabilities"] as? [String] ?? [])
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
            .sorted()
        let version = (body["extension_version"] as? String)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let wait = min(max((body["wait_sec"] as? NSNumber)?.doubleValue ?? 25, 0), 30)
        let deadline = Date().addingTimeInterval(wait)

        condition.lock()
        lastHostSeen = .now
        hostCapabilities = capabilities
        if let version, !version.isEmpty {
            extensionVersion = version
        }
        condition.broadcast()
        condition.unlock()
        postStatusChanged()
        condition.lock()
        while true {
            if let index = pendingCommands.firstIndex(where: { $0.hostID == hostID }) {
                let command = pendingCommands.remove(at: index)
                command.instanceID = instanceID
                command.manifestID = manifestID
                claimedCommands[command.id] = command
                let payload = command.payload
                condition.unlock()
                return HTTPResponse(
                    status: 200,
                    object: ["type": "command", "command": payload]
                )
            }
            guard condition.wait(until: deadline) else {
                condition.unlock()
                return HTTPResponse(
                    status: 200,
                    object: ["type": "noop", "retry_ms": 350]
                )
            }
        }
    }

    private func acceptResult(_ body: [String: Any]) -> HTTPResponse {
        guard let commandID = body["command_id"] as? String,
              let hostID = body["host_id"] as? String,
              let instanceID = body["instance_id"] as? String,
              let manifestID = body["manifest_id"] as? String,
              let result = body["result"] as? [String: Any] else {
            return HTTPResponse(
                status: 400,
                object: ["error": String(localized: "Invalid command result.")]
            )
        }
        condition.lock()
        guard let command = claimedCommands[commandID],
              command.hostID == hostID,
              command.instanceID == instanceID,
              command.manifestID == manifestID else {
            condition.unlock()
            return HTTPResponse(
                status: 409,
                object: [
                    "accepted": false,
                    "error": String(localized: "Command identity mismatch.")
                ]
            )
        }
        claimedCommands.removeValue(forKey: commandID)
        commandResults[commandID] = result
        lastHostSeen = .now
        condition.broadcast()
        condition.unlock()
        return HTTPResponse(status: 200, object: ["accepted": true])
    }

    private func submitCommand(_ body: [String: Any]) -> HTTPResponse {
        guard let action = body["action"] as? String,
              !action.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return HTTPResponse(status: 400, object: ["error": "action is required"])
        }
        let hostID = (body["host_id"] as? String)
            ?? configuration?.hostID
            ?? MacChromeMessagingConfiguration.defaultHostID
        let args = body["args"] as? [String: Any] ?? [:]
        let timeout = min(
            max((body["timeout_sec"] as? NSNumber)?.doubleValue ?? 45, 1),
            120
        )
        let command = MacChromeBrokerCommand(
            id: "mac-\(UUID().uuidString.lowercased())",
            hostID: hostID,
            action: action.lowercased(),
            args: args
        )

        condition.lock()
        let connected = lastHostSeen.map { Date().timeIntervalSince($0) < 45 } ?? false
        guard connected else {
            condition.unlock()
            requestSetupAssistanceIfNeeded()
            let message = String(
                localized:
                    "Chrome is not connected. Open Chrome and enable the Nola extension."
            )
            return HTTPResponse(
                status: 503,
                object: [
                    "ok": false,
                    "code": "chrome_extension_not_connected",
                    "error": message,
                    "recovery_action": "open_browser_setup"
                ]
            )
        }
        pendingCommands.append(command)
        condition.broadcast()
        let deadline = Date().addingTimeInterval(timeout)
        while commandResults[command.id] == nil, condition.wait(until: deadline) {
            continue
        }
        if let result = commandResults.removeValue(forKey: command.id) {
            condition.unlock()
            return HTTPResponse(
                status: 200,
                object: [
                    "ok": (result["ok"] as? Bool) == true,
                    "action": action,
                    "result": result
                ]
            )
        }
        pendingCommands.removeAll { $0.id == command.id }
        claimedCommands.removeValue(forKey: command.id)
        condition.unlock()
        let message = String(localized: "Chrome did not finish the command in time.")
        return HTTPResponse(
            status: 200,
            object: [
                "ok": false,
                "action": action,
                "result": ["ok": false, "error": message]
            ]
        )
    }

    private func send(status: Int, object: [String: Any], on connection: NWConnection) {
        let body = (try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]))
            ?? Data(
                "{\"error\":\"\(String(localized: "Serialization failed."))\"}".utf8
            )
        let reason: String
        switch status {
        case 200: reason = "OK"
        case 400: reason = "Bad Request"
        case 401: reason = "Unauthorized"
        case 404: reason = "Not Found"
        case 409: reason = "Conflict"
        case 413: reason = "Content Too Large"
        case 503: reason = "Service Unavailable"
        default: reason = "Error"
        }
        let header = "HTTP/1.1 \(status) \(reason)\r\n"
            + "Content-Type: application/json; charset=utf-8\r\n"
            + "Content-Length: \(body.count)\r\n"
            + "Connection: close\r\n"
            + "Cache-Control: no-store\r\n\r\n"
        var response = Data(header.utf8)
        response.append(body)
        connection.send(content: response, completion: .contentProcessed { _ in
            connection.cancel()
        })
    }

    private func postStatusChanged() {
        if Thread.isMainThread {
            NotificationCenter.default.post(
                name: .macChromeBridgeStatusDidChange,
                object: nil
            )
        } else {
            DispatchQueue.main.async {
                NotificationCenter.default.post(
                    name: .macChromeBridgeStatusDidChange,
                    object: nil
                )
            }
        }
    }

    private func requestSetupAssistanceIfNeeded(now: Date = .now) {
        setupAssistanceCondition.lock()
        let shouldRequest = lastSetupAssistanceAt.map {
            now.timeIntervalSince($0) >= 15
        } ?? true
        if shouldRequest {
            lastSetupAssistanceAt = now
        }
        setupAssistanceCondition.unlock()
        guard shouldRequest else { return }

        if let setupAssistance {
            setupAssistance()
            return
        }
        DispatchQueue.main.async { [weak self] in
            _ = self?.openChromeExtensions()
            NotificationCenter.default.post(
                name: .macChromeSetupAssistanceRequested,
                object: nil,
                userInfo: ["recovery_action": "open_browser_setup"]
            )
        }
    }
}

nonisolated private final class MacChromeHTTPResultBox: @unchecked Sendable {
    var data: Data?
    var response: URLResponse?
    var error: Error?
}

nonisolated enum MacChromeNativeHostProcess {
    static func run() -> Int32 {
        guard let configuration = loadConfiguration() else {
            log("Native host configuration is unavailable.")
            return 2
        }
        let process = Runner(configuration: configuration)
        return process.run()
    }

    private static func loadConfiguration() -> MacChromeMessagingConfiguration? {
        if let rawPath = ProcessInfo.processInfo.environment["NOLA_NATIVE_BROWSER_CONFIG"],
           !rawPath.isEmpty {
            return MacChromeMessagingConfiguration.load(
                from: URL(fileURLWithPath: rawPath)
            )
        }
        return MacChromeMessagingConfiguration.load()
    }

    private static func log(_ message: String) {
        let fileManager = FileManager.default
        let directory = fileManager.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/Nola", isDirectory: true)
        try? fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("chrome-native-host.log")
        let line = "[\(ISO8601DateFormatter().string(from: .now))] \(message)\n"
        if !fileManager.fileExists(atPath: url.path) {
            try? Data(line.utf8).write(to: url, options: .atomic)
        } else if let handle = try? FileHandle(forWritingTo: url) {
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: Data(line.utf8))
            try? handle.close()
        }
    }

    nonisolated private final class Runner: @unchecked Sendable {
        private let configuration: MacChromeMessagingConfiguration
        private let stateLock = NSLock()
        private let writeLock = NSLock()
        private var running = true
        private var capabilities: [String] = []
        private var extensionVersion = ""
        private let instanceID: String
        private let manifestID: String

        init(configuration: MacChromeMessagingConfiguration) {
            self.configuration = configuration
            let instanceID = UUID().uuidString.replacingOccurrences(of: "-", with: "")
            self.instanceID = instanceID
            self.manifestID = "native-browser:\(configuration.hostID):\(instanceID)"
        }

        func run() -> Int32 {
            MacChromeNativeHostProcess.log(
                "Starting Mac-owned Chrome Native Messaging host."
            )
            let pollThread = Thread { [weak self] in self?.pollLoop() }
            pollThread.name = "Nola Chrome Native Host Poll"
            pollThread.start()
            while let message = readMessage() {
                let type = (message["type"] as? String)?.lowercased() ?? ""
                switch type {
                case "hello":
                    let values = message["capabilities"] as? [String] ?? []
                    stateLock.lock()
                    capabilities = values
                    extensionVersion = message["extension_version"] as? String ?? ""
                    stateLock.unlock()
                    writeMessage(["type": "host_ready"])
                case "result":
                    guard let commandID = message["command_id"] as? String else { continue }
                    let result = message["result"] as? [String: Any] ?? [
                        "ok": false,
                        "error": String(localized: "Chrome returned an invalid result.")
                    ]
                    _ = post(
                        path: "/api/native-browser/host/result",
                        payload: [
                            "host_id": configuration.hostID,
                            "instance_id": instanceID,
                            "manifest_id": manifestID,
                            "command_id": commandID,
                            "result": result
                        ],
                        timeout: 12
                    )
                default:
                    continue
                }
            }
            stateLock.lock()
            running = false
            stateLock.unlock()
            MacChromeNativeHostProcess.log("Chrome Native Messaging stream closed.")
            return 0
        }

        private func pollLoop() {
            var backoff = 0.4
            while isRunning {
                stateLock.lock()
                let activeCapabilities = capabilities
                let activeVersion = extensionVersion
                stateLock.unlock()
                guard !activeCapabilities.isEmpty else {
                    Thread.sleep(forTimeInterval: 0.1)
                    continue
                }
                let now = UInt64(Date().timeIntervalSince1970 * 1000)
                let response = post(
                    path: "/api/native-browser/host/poll",
                    payload: [
                        "host_id": configuration.hostID,
                        "instance_id": instanceID,
                        "manifest_id": manifestID,
                        "manifest_version": 1,
                        "issued_at_ms": now,
                        "expires_at_ms": now + 90_000,
                        "capabilities": activeCapabilities,
                        "extension_version": activeVersion,
                        "wait_sec": 25
                    ],
                    timeout: 35
                )
                if let command = response?["command"] as? [String: Any],
                   response?["type"] as? String == "command" {
                    writeMessage(["type": "command", "command": command])
                    backoff = 0.4
                } else if response != nil {
                    backoff = 0.4
                    Thread.sleep(forTimeInterval: 0.25)
                } else {
                    Thread.sleep(forTimeInterval: backoff)
                    backoff = min(backoff * 1.8, 8)
                }
            }
        }

        private var isRunning: Bool {
            stateLock.lock()
            defer { stateLock.unlock() }
            return running
        }

        private func post(
            path: String,
            payload: [String: Any],
            timeout: TimeInterval
        ) -> [String: Any]? {
            guard let url = URL(string: path, relativeTo: configuration.serverURL)?.absoluteURL,
                  let body = try? JSONSerialization.data(withJSONObject: payload) else {
                return nil
            }
            var request = URLRequest(url: url)
            request.httpMethod = "POST"
            request.httpBody = body
            request.timeoutInterval = timeout
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.setValue(
                configuration.token,
                forHTTPHeaderField: "X-Nola-Native-Browser-Token"
            )
            let box = MacChromeHTTPResultBox()
            let semaphore = DispatchSemaphore(value: 0)
            let sessionConfiguration = URLSessionConfiguration.ephemeral
            sessionConfiguration.connectionProxyDictionary = [:]
            let session = URLSession(configuration: sessionConfiguration)
            session.dataTask(with: request) { data, response, error in
                box.data = data
                box.response = response
                box.error = error
                semaphore.signal()
            }.resume()
            guard semaphore.wait(timeout: .now() + timeout + 1) == .success,
                  box.error == nil,
                  let http = box.response as? HTTPURLResponse,
                  (200..<300).contains(http.statusCode),
                  let data = box.data,
                  let object = try? JSONSerialization.jsonObject(with: data)
                    as? [String: Any] else {
                session.invalidateAndCancel()
                return nil
            }
            session.finishTasksAndInvalidate()
            return object
        }

        private func readMessage() -> [String: Any]? {
            guard let lengthData = readExactly(4) else { return nil }
            let length = lengthData.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))
            }
            guard length > 0, length <= 4 * 1024 * 1024,
                  let body = readExactly(Int(length)),
                  let object = try? JSONSerialization.jsonObject(with: body)
                    as? [String: Any] else {
                return nil
            }
            return object
        }

        private func readExactly(_ count: Int) -> Data? {
            var result = Data()
            while result.count < count {
                guard let chunk = try? FileHandle.standardInput.read(
                    upToCount: count - result.count
                ), !chunk.isEmpty else {
                    return nil
                }
                result.append(chunk)
            }
            return result
        }

        private func writeMessage(_ object: [String: Any]) {
            guard let body = try? JSONSerialization.data(withJSONObject: object),
                  body.count <= 4 * 1024 * 1024 else {
                return
            }
            var length = UInt32(body.count).littleEndian
            let prefix = Data(bytes: &length, count: MemoryLayout<UInt32>.size)
            writeLock.lock()
            defer { writeLock.unlock() }
            try? FileHandle.standardOutput.write(contentsOf: prefix)
            try? FileHandle.standardOutput.write(contentsOf: body)
        }
    }
}
