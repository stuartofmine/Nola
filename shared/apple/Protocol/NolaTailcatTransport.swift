// SPDX-License-Identifier: Apache-2.0
// Optional ephemeral network transport. Logical Host identity and credential
// storage are never replaced by a temporary loopback port.
import Foundation
import CryptoKit
import Network
#if canImport(AppKit)
import AppKit
#endif
#if canImport(NolaTailcat)
import NolaTailcat
#endif

nonisolated enum NolaTailcatTransportError: Error, Equatable, Sendable, LocalizedError {
    case hostError(status: Int, code: String)
    case stage(String, detail: String)

    var code: String {
        switch self {
        case .hostError(_, let code): return code
        case .stage(let stage, _): return stage
        }
    }

    var errorDescription: String? {
        NolaTailcatTransport.userFacingErrorMessage(for: self)
    }
}

nonisolated enum NolaTailcatTransport {
    enum Mode: Equatable, Sendable {
        case direct
        case tailcat
        case connecting
        case failed(stage: String)
    }

    struct Configuration: Sendable, Equatable {
        let origin: URL
        let token: String
        let hostID: String
        let generation: UUID
    }

    static let modeDidChangeNotification = Notification.Name("NolaTailcatTransportModeDidChange")

    private static let lock = NSLock()
    nonisolated(unsafe) private static var configurations: [String: Configuration] = [:]
    nonisolated(unsafe) private static var transientConfigurations: [String: Configuration] = [:]
    nonisolated(unsafe) private static var modes: [String: Mode] = [:]
    nonisolated(unsafe) private static var _parked = false
    nonisolated(unsafe) private static var pathMonitor: NWPathMonitor?
    nonisolated(unsafe) private static var lastSatisfiedPathFingerprint: String?
    private static let pathMonitorQueue = DispatchQueue(label: "com.nola.tailcat.path-monitor", qos: .utility)

    static var isParked: Bool {
        lock.withLock { _parked }
    }

    static func setParked(_ value: Bool) {
        lock.withLock { _parked = value }
    }

    private static let lifecycle: Void = {
        NotificationCenter.default.addObserver(forName: .init("UIApplicationDidEnterBackgroundNotification"), object: nil, queue: nil) { _ in
            NolaTailcatTransport.setParked(true)
            Task { await NolaTailcatTransport.park() }
        }
        NotificationCenter.default.addObserver(forName: .init("UIApplicationWillEnterForegroundNotification"), object: nil, queue: nil) { _ in
            // Clear parked synchronously before kicking off async resume so
            // immediate foreground requests never race and fail with
            // URLError(.backgroundSessionWasDisconnected).
            NolaTailcatTransport.setParked(false)
            Task { await NolaTailcatTransport.resume() }
        }
        #if os(macOS)
        NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: nil) { _ in
            NolaTailcatTransport.setParked(true)
            Task { await NolaTailcatTransport.suspend() }
        }
        NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: nil) { _ in
            NolaTailcatTransport.setParked(false)
            Task { await NolaTailcatTransport.resume() }
        }
        NotificationCenter.default.addObserver(forName: NSApplication.willTerminateNotification, object: nil, queue: nil) { _ in
            NolaTailcatTransport.suspendSynchronously(timeoutSeconds: 1.5)
        }
        #else
        NotificationCenter.default.addObserver(forName: .init("NSApplicationWillTerminateNotification"), object: nil, queue: nil) { _ in
            NolaTailcatTransport.suspendSynchronously(timeoutSeconds: 1.5)
        }
        #endif
        startPathMonitor()
    }()

    private static func startPathMonitor() {
        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { path in
            guard path.status == .satisfied else { return }
            let interfaces = path.availableInterfaces.map { "\($0.name):\($0.type)" }.sorted().joined(separator: ",")
            let gateways = path.gateways.map { $0.debugDescription }.sorted().joined(separator: ",")
            let fingerprint = "\(interfaces)|\(gateways)"
            let changed = lock.withLock { () -> Bool in
                let previous = lastSatisfiedPathFingerprint
                lastSatisfiedPathFingerprint = fingerprint
                return previous != nil && previous != fingerprint
            }
            if changed {
                Task { await NolaTailcatConnections.shared.markEntriesForHealthCheck() }
            }
        }
        lock.withLock { pathMonitor = monitor }
        monitor.start(queue: pathMonitorQueue)
    }

    static func park() async {
        setParked(true)
        await NolaTailcatConnections.shared.park()
    }

    static func resume() async {
        setParked(false)
        await NolaTailcatConnections.shared.resume()
    }

    static func suspend() async {
        await NolaTailcatConnections.shared.suspend()
    }

    static func suspendSynchronously(timeoutSeconds: TimeInterval = 1.5) {
        let semaphore = DispatchSemaphore(value: 0)
        Task.detached(priority: .userInitiated) {
            await NolaTailcatConnections.shared.suspend()
            semaphore.signal()
        }
        _ = semaphore.wait(timeout: .now() + timeoutSeconds)
    }

    static var available: Bool {
        #if canImport(NolaTailcat)
        true
        #else
        false
        #endif
    }

    static func origin(_ url: URL) -> URL? {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.user == nil, parts.password == nil, parts.scheme == "http", parts.host != nil else { return nil }
        parts.path = ""; parts.query = nil; parts.fragment = nil
        return parts.url
    }

    private static func statusKey(for url: URL) -> String? {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              parts.scheme != nil, parts.host != nil else { return nil }
        parts.path = ""; parts.query = nil; parts.fragment = nil
        return parts.url?.absoluteString
    }

    static func currentTransportMode(for origin: URL) -> Mode {
        guard let key = statusKey(for: origin) else { return .direct }
        return lock.withLock { modes[key] ?? .direct }
    }

    fileprivate static func setTransportMode(_ mode: Mode, for key: String) {
        let changed = lock.withLock { () -> Bool in
            if modes[key] == mode { return false }
            modes[key] = mode
            return true
        }
        if changed {
            NotificationCenter.default.post(name: modeDidChangeNotification, object: nil, userInfo: ["origin": key])
        }
    }

    fileprivate static func clearTransportMode(for key: String) {
        let removed = lock.withLock { modes.removeValue(forKey: key) != nil }
        if removed {
            NotificationCenter.default.post(name: modeDidChangeNotification, object: nil, userInfo: ["origin": key])
        }
    }

    static func register(baseURL: URL, token: String, hostID: String?, certificatePin: String? = nil) {
        guard available, MobileHostTransportPolicy.allows(baseURL), certificatePin?.isEmpty != false,
              ["", "/", "/api", "/api/", "/api/mobile", "/api/mobile/"].contains(baseURL.path),
              let origin = origin(baseURL),
              let hostID, !hostID.isEmpty, !token.isEmpty else { return }
        _ = lifecycle
        lock.withLock {
            if let current = configurations[origin.absoluteString], current.token == token, current.hostID == hostID { return }
            configurations[origin.absoluteString] = Configuration(origin: origin, token: token, hostID: hostID, generation: UUID())
        }
    }

    static func unregister(baseURL: URL) {
        guard let key = origin(baseURL)?.absoluteString else { return }
        let previous = lock.withLock { configurations.removeValue(forKey: key) }
        if let previous {
            clearTransportMode(for: key)
            Task { await NolaTailcatConnections.shared.remove(key, generation: previous.generation) }
        }
    }

    /// Registers a temporary route used during initial pairing verification
    /// before credentials are committed to persistent storage.
    static func registerTransient(origin baseURL: URL, hostID: String, token: String, certificatePin: String? = nil,
                                  requireNativeAvailability: Bool = true) {
        guard (!requireNativeAvailability || available),
              MobileHostTransportPolicy.allows(baseURL),
              certificatePin?.isEmpty != false,
              ["", "/", "/api", "/api/", "/api/mobile", "/api/mobile/"].contains(baseURL.path),
              let origin = origin(baseURL),
              !hostID.isEmpty, !token.isEmpty else { return }
        _ = lifecycle
        lock.withLock {
            if let current = transientConfigurations[origin.absoluteString],
               current.token == token, current.hostID == hostID { return }
            transientConfigurations[origin.absoluteString] = Configuration(
                origin: origin, token: token, hostID: hostID, generation: UUID()
            )
        }
    }

    static func unregisterTransient(origin baseURL: URL) {
        guard let key = origin(baseURL)?.absoluteString else { return }
        let (previous, hasPersistent) = lock.withLock {
            (transientConfigurations.removeValue(forKey: key), configurations[key] != nil)
        }
        if let previous {
            if !hasPersistent { clearTransportMode(for: key) }
            Task { await NolaTailcatConnections.shared.remove(key, generation: previous.generation) }
        }
    }

    static func configuration(for url: URL) -> Configuration? {
        guard let key = origin(url)?.absoluteString else { return nil }
        return lock.withLock { transientConfigurations[key] ?? configurations[key] }
    }

    #if os(macOS)
    // A desktop browser can use the App-owned loopback bridge. Do not use this
    // for mobile external browsers: their owner App may be OS-suspended.
    static func browserURL(for logicalURL: URL) async throws -> URL {
        guard let configuration = configuration(for: logicalURL) else { return logicalURL }
        let endpoint = try await NolaTailcatConnections.shared.endpoint(for: configuration)
        guard var parts = URLComponents(url: logicalURL, resolvingAgainstBaseURL: false) else { throw URLError(.badURL) }
        parts.scheme = endpoint.scheme; parts.host = endpoint.host; parts.port = endpoint.port
        guard let url = parts.url else { throw URLError(.badURL) }; return url
    }
    #endif

    // Select a route using the request's frozen credential. This is not the
    // permission authority: the Host still validates proof, body and ownership.
    static func matches(_ request: URLRequest, configuration: Configuration) -> Bool {
        guard let url = request.url,
              url.path.hasPrefix("/api/mobile/") || url.path.hasPrefix("/api/"),
              let timestamp = request.value(forHTTPHeaderField: "X-Nola-Mobile-Timestamp"),
              let nonce = request.value(forHTTPHeaderField: "X-Nola-Mobile-Nonce"),
              let digest = request.value(forHTTPHeaderField: "X-Nola-Mobile-Body-SHA256"),
              let proof = request.value(forHTTPHeaderField: "X-Nola-Mobile-Auth"),
              let signature = try? NolaTailcatGrant.decode(proof) else { return false }
        let comps = URLComponents(url: url, resolvingAgainstBaseURL: false)
        let path = (comps?.percentEncodedPath ?? url.path) + (comps?.percentEncodedQuery.map { "?\($0)" } ?? "")
        let message = "v1\n\((request.httpMethod ?? "GET").uppercased())\n\(path)\n\(timestamp)\n\(nonce)\n\(digest)"
        return HMAC<SHA256>.isValidAuthenticationCode(signature, authenticating: Data(message.utf8), using: SymmetricKey(data: Data(configuration.token.utf8)))
    }

    static func sessionConfiguration() -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.default
        configuration.protocolClasses = [NolaTailcatURLProtocol.self] + (configuration.protocolClasses ?? [])
        return configuration
    }

    static let session = URLSession(configuration: sessionConfiguration())

    /// Maps Tailcat transport stages and Host error codes to clear, actionable user-facing messages.
    static func userFacingErrorMessage(for error: Error) -> String? {
        if let transportError = error as? NolaTailcatTransportError {
            return userFacingMessage(forCode: transportError.code)
        }
        let nsError = error as NSError
        if let code = nsError.userInfo["nolaTailcatCode"] as? String {
            return userFacingMessage(forCode: code)
        }
        let lowered = nsError.localizedDescription.lowercased()
        for code in knownTailcatErrorCodes where lowered.contains(code) {
            return userFacingMessage(forCode: code)
        }
        return nil
    }

    static func userFacingMessage(forCode code: String) -> String? {
        switch code.lowercased() {
        case "tailcat_not_available":
            return String(localized: "Tailcat remote tunnel is not enabled on this Nola Server. Enable Tailcat ingress and configure the helper on the host, then retry.")
        case "tailcat_start_timeout":
            return String(localized: "Tailcat tunnel timed out while starting on the host. Check the host's network connection and try again.")
        case "tailcat_start_failed", "tailcat_helper_verification_failed":
            return String(localized: "The Tailcat helper on the host failed to start or verify. Check the helper path and SHA-256 configuration on the host.")
        case "transport_host_identity_mismatch":
            return String(localized: "The host identity did not match during Tailcat verification. Remove the saved connection and pair again.")
        case "transport_capacity_reached":
            return String(localized: "The host reached its maximum number of active Tailcat sessions. Close unused sessions and try again.")
        case "dns":
            return String(localized: "Tailcat could not resolve the host or relay address (DNS). Check your network DNS settings and try again.")
        case "certificate":
            return String(localized: "Tailcat failed TLS certificate verification. Check your system clock, network proxy, or firewall settings.")
        case "timeout":
            return String(localized: "Tailcat tunnel connection timed out. The first connection can take up to 30 seconds; check network reachability and retry.")
        case "permission":
            return String(localized: "Network permission was denied for the Tailcat tunnel. Allow Local Network and VPN/network access in System Settings.")
        default:
            return nil
        }
    }

    private static let knownTailcatErrorCodes: [String] = [
        "tailcat_not_available",
        "tailcat_start_timeout",
        "tailcat_start_failed",
        "tailcat_helper_verification_failed",
        "transport_host_identity_mismatch",
        "transport_capacity_reached",
    ]

    fileprivate static func classifyNativeError(_ error: Error) -> NolaTailcatTransportError {
        let nsError = error as NSError
        let text = "\(nsError.domain) \(nsError.localizedDescription)".lowercased()
        if text.contains("dns") || text.contains("no such host") || text.contains("lookup") {
            return .stage("dns", detail: nsError.localizedDescription)
        }
        if text.contains("certificate") || text.contains("x509") || text.contains("tls") {
            return .stage("certificate", detail: nsError.localizedDescription)
        }
        if text.contains("timeout") || text.contains("timed out") || text.contains("deadline exceeded") {
            return .stage("timeout", detail: nsError.localizedDescription)
        }
        if text.contains("permission") || text.contains("operation not permitted") || text.contains("denied") {
            return .stage("permission", detail: nsError.localizedDescription)
        }
        return .stage("connect", detail: nsError.localizedDescription)
    }
}

/// Extracted lease coordination engine so concurrent expiry rotation and
/// proactive lease renewal can be verified deterministically in unit tests.
internal actor NolaTailcatLeaseCoordinator<LeaseHandle: Sendable> {
    struct LeaseSnapshot: Sendable {
        let endpoint: URL
        let isTailcat: Bool
        let leaseID: String?
        let expiresAt: Date
        let expiresIn: TimeInterval
        let handle: LeaseHandle
    }

    private final class MutableEntryState: @unchecked Sendable {
        private let lock = NSLock()
        private var _expiresAt: Date
        private var _expiresIn: TimeInterval
        private var _renewUnsupported = false
        private var _needsHealthCheck = false
        private var _renewalTask: Task<Void, Never>?

        init(expiresAt: Date, expiresIn: TimeInterval) {
            self._expiresAt = expiresAt
            self._expiresIn = expiresIn
        }

        var expiresAt: Date { lock.withLock { _expiresAt } }
        var expiresIn: TimeInterval { lock.withLock { _expiresIn } }
        var needsHealthCheck: Bool { lock.withLock { _needsHealthCheck } }

        func markNeedsHealthCheck() { lock.withLock { _needsHealthCheck = true } }
        func clearNeedsHealthCheck() { lock.withLock { _needsHealthCheck = false } }

        func shouldTriggerProactiveRenewal(now: Date = Date()) -> Bool {
            lock.withLock {
                guard !_renewUnsupported, _renewalTask == nil else { return false }
                let remaining = _expiresAt.timeIntervalSince(now)
                guard remaining > 30 else { return false }
                let threshold = min(300.0, _expiresIn * 0.25)
                return remaining <= threshold
            }
        }

        func setRenewalTask(_ task: Task<Void, Never>?) {
            lock.withLock { _renewalTask = task }
        }

        func applyRenewal(expiresAt: Date, expiresIn: TimeInterval) {
            lock.withLock {
                _expiresAt = expiresAt
                _expiresIn = expiresIn
                _renewalTask = nil
            }
        }

        func markRenewUnsupported() {
            lock.withLock {
                _renewUnsupported = true
                _renewalTask = nil
            }
        }

        func clearRenewalTask() {
            lock.withLock { _renewalTask = nil }
        }

        func cancelRenewal() {
            let task = lock.withLock { () -> Task<Void, Never>? in
                let t = _renewalTask
                _renewalTask = nil
                return t
            }
            task?.cancel()
        }
    }

    private struct Entry {
        let id: UUID
        let configuration: NolaTailcatTransport.Configuration
        let task: Task<(LeaseSnapshot, MutableEntryState), Error>
        let isReady: @Sendable () -> Bool
        let closeHandle: @Sendable () async -> Void
    }

    private var entries: [String: Entry] = [:]

    func endpoint(
        for configuration: NolaTailcatTransport.Configuration,
        isParked: @Sendable () -> Bool,
        open: @escaping @Sendable (NolaTailcatTransport.Configuration) async throws -> (LeaseSnapshot, @Sendable () -> Bool, @Sendable () async -> Void),
        renew: @escaping @Sendable (NolaTailcatTransport.Configuration, String) async throws -> TimeInterval,
        onModeChange: @escaping @Sendable (String, NolaTailcatTransport.Mode) -> Void
    ) async throws -> URL {
        let key = configuration.origin.absoluteString
        let maxAttempts = 4

        for _ in 0..<maxAttempts {
            guard !isParked() else { throw URLError(.backgroundSessionWasDisconnected) }

            if let entry = entries[key] {
                let resolved: (LeaseSnapshot, MutableEntryState)
                do {
                    resolved = try await entry.task.value
                } catch {
                    // If another caller already rotated/replaced the entry while we awaited, retry cleanly.
                    if entries[key]?.id != entry.id { continue }
                    throw error
                }
                if entries[key]?.id != entry.id { continue }

                let (snapshot, state) = resolved
                if entry.configuration == configuration,
                   !snapshot.isTailcat || state.expiresAt.timeIntervalSinceNow > 30 {
                    if snapshot.isTailcat, let leaseID = snapshot.leaseID, state.shouldTriggerProactiveRenewal() {
                        let renewalTask = Task {
                            let requestedAt = Date()
                            do {
                                let newExpiresIn = try await renew(configuration, leaseID)
                                state.applyRenewal(
                                    expiresAt: requestedAt.addingTimeInterval(newExpiresIn),
                                    expiresIn: newExpiresIn
                                )
                            } catch let error as NolaTailcatTransportError
                                where error == .hostError(status: 400, code: "unknown_transport_operation") {
                                state.markRenewUnsupported()
                            } catch {
                                state.clearRenewalTask()
                            }
                        }
                        state.setRenewalTask(renewalTask)
                    }
                    return snapshot.endpoint
                }

                // Expired or configuration changed: remove and close old handle, then create replacement.
                entries.removeValue(forKey: key)
                state.cancelRenewal()
                await entry.closeHandle()
                continue
            }

            onModeChange(key, .connecting)
            let id = UUID()
            let box = EntryLifecycleBox()
            let task = Task<(LeaseSnapshot, MutableEntryState), Error> {
                let (snapshot, readyCheck, closeFn) = try await open(configuration)
                box.set(isReady: readyCheck, close: closeFn)
                let state = MutableEntryState(expiresAt: snapshot.expiresAt, expiresIn: snapshot.expiresIn)
                onModeChange(key, snapshot.isTailcat ? .tailcat : .direct)
                return (snapshot, state)
            }
            let entry = Entry(
                id: id,
                configuration: configuration,
                task: task,
                isReady: { box.isReady() },
                closeHandle: { await box.close() }
            )
            entries[key] = entry
            do {
                let (snapshot, _) = try await task.value
                return snapshot.endpoint
            } catch {
                if entries[key]?.id == id {
                    entries.removeValue(forKey: key)
                    if let transportError = error as? NolaTailcatTransportError {
                        onModeChange(key, .failed(stage: transportError.code))
                    } else if !Task.isCancelled {
                        onModeChange(key, .failed(stage: "connect"))
                    }
                    throw error
                }
                // Entry was superseded concurrently; retry with the new entry.
                continue
            }
        }
        throw URLError(.cancelled)
    }

    func markEntriesForHealthCheck() async {
        for entry in entries.values {
            if let (_, state) = try? await entry.task.value {
                state.markNeedsHealthCheck()
            }
        }
    }

    func invalidateIfUnhealthy(for key: String) async {
        guard let entry = entries[key] else { return }
        if let (snapshot, state) = try? await entry.task.value,
           snapshot.isTailcat, state.needsHealthCheck {
            entries.removeValue(forKey: key)
            state.cancelRenewal()
            await entry.closeHandle()
        }
    }

    func park() async {
        let pending = entries.filter { !$0.value.isReady() }
        for (key, entry) in pending {
            entries.removeValue(forKey: key)
            entry.task.cancel()
        }
        for entry in pending.values {
            await entry.closeHandle()
        }
    }

    func remove(_ key: String, generation: UUID) async {
        if let entry = entries[key], entry.configuration.generation == generation {
            entries.removeValue(forKey: key)
            entry.task.cancel()
            if let (_, state) = try? await entry.task.value {
                state.cancelRenewal()
            }
            await entry.closeHandle()
        }
    }

    func suspend() async {
        let previous = entries
        entries.removeAll()
        for entry in previous.values {
            entry.task.cancel()
            if let (_, state) = try? await entry.task.value {
                state.cancelRenewal()
            }
            await entry.closeHandle()
        }
    }

    private final class EntryLifecycleBox: @unchecked Sendable {
        private let lock = NSLock()
        private var _isReady: @Sendable () -> Bool = { false }
        private var _close: @Sendable () async -> Void = {}

        func set(isReady: @escaping @Sendable () -> Bool, close: @escaping @Sendable () async -> Void) {
            lock.withLock {
                _isReady = isReady
                _close = close
            }
        }

        func isReady() -> Bool {
            lock.withLock { _isReady }()
        }

        func close() async {
            let fn = lock.withLock { _close }
            await fn()
        }
    }
}

private actor NolaTailcatConnections {
    static let shared = NolaTailcatConnections()
    #if canImport(NolaTailcat)
    private final class Native: @unchecked Sendable {
        let session = NolaTailcatNolatailcatNewSession()!
        let configuration: NolaTailcatTransport.Configuration
        private let lock = NSLock()
        private var lease: String?
        private var connected = false
        var ready: Bool { lock.withLock { connected } }
        func markReady() { lock.withLock { connected = true } }
        init(_ configuration: NolaTailcatTransport.Configuration) { self.configuration = configuration }
        func own(_ lease: String) { lock.withLock { self.lease = lease } }
        func close() async {
            _ = try? await Task.detached { try self.session.close() }.value
            let previous = lock.withLock { let value = lease; lease = nil; return value }
            if let previous { _ = try? await NolaTailcatGrant.exchange(configuration, payload: ["operation": "close", "leaseId": previous]) }
        }
    }
    private let coordinator = NolaTailcatLeaseCoordinator<Native>()
    #endif

    func park() async {
        #if canImport(NolaTailcat)
        await coordinator.park()
        #endif
    }

    func resume() {}

    func remove(_ key: String, generation: UUID) async {
        #if canImport(NolaTailcat)
        await coordinator.remove(key, generation: generation)
        #endif
    }

    func suspend() async {
        #if canImport(NolaTailcat)
        await coordinator.suspend()
        #endif
    }

    func markEntriesForHealthCheck() async {
        #if canImport(NolaTailcat)
        await coordinator.markEntriesForHealthCheck()
        #endif
    }

    func reportForwardingFailure(for url: URL) async {
        #if canImport(NolaTailcat)
        guard let key = NolaTailcatTransport.origin(url)?.absoluteString else { return }
        await coordinator.invalidateIfUnhealthy(for: key)
        #endif
    }

    func endpoint(for configuration: NolaTailcatTransport.Configuration) async throws -> URL {
        guard !NolaTailcatTransport.isParked else { throw URLError(.backgroundSessionWasDisconnected) }
        #if canImport(NolaTailcat)
        return try await coordinator.endpoint(
            for: configuration,
            isParked: { NolaTailcatTransport.isParked },
            open: { configuration in
                let native = Native(configuration)
                do {
                    let inspect = configuration.origin.appendingPathComponent("api/web/session")
                    var check = URLRequest(url: inspect); check.timeoutInterval = 8
                    let (statusData, statusResponse) = try await URLSession.shared.data(for: check)
                    guard (statusResponse as? HTTPURLResponse)?.statusCode == 200,
                          let status = try JSONSerialization.jsonObject(with: statusData) as? [String: Any],
                          status["hostId"] as? String == configuration.hostID else { throw URLError(.badServerResponse) }
                    guard let advertised = status["tailcatAvailable"] as? Bool else { throw URLError(.badServerResponse) }
                    if !advertised {
                        let snapshot = NolaTailcatLeaseCoordinator<Native>.LeaseSnapshot(
                            endpoint: configuration.origin,
                            isTailcat: false,
                            leaseID: nil,
                            expiresAt: .distantFuture,
                            expiresIn: 0,
                            handle: native
                        )
                        return (snapshot, { true }, { await native.close() })
                    }
                    let requestedAt = Date()
                    let grant = try await NolaTailcatGrant.exchange(configuration, payload: [
                        "operation": "connect", "expectedHostId": configuration.hostID,
                        "clientPublicKey": native.session.publicKey()
                    ])
                    guard grant.hostId == configuration.hostID,
                          let address = grant.address,
                          let lease = grant.leaseId, !lease.isEmpty,
                          let seconds = grant.expiresIn, (31...1200).contains(seconds) else { throw URLError(.badServerResponse) }
                    native.own(lease)
                    let local: URL
                    do {
                        local = try await Task.detached {
                            var failure: NSError?
                            let value = native.session.connect(address, expectedHostID: configuration.hostID, relay: "", timeoutMillis: 30000, error: &failure)
                            if let failure { throw failure }
                            guard let url = URL(string: value), url.host == "127.0.0.1", url.scheme == "http" else { throw URLError(.badServerResponse) }
                            return url
                        }.value
                    } catch {
                        throw NolaTailcatTransport.classifyNativeError(error)
                    }
                    try Task.checkCancellation()
                    native.markReady()
                    let snapshot = NolaTailcatLeaseCoordinator<Native>.LeaseSnapshot(
                        endpoint: local,
                        isTailcat: true,
                        leaseID: lease,
                        expiresAt: requestedAt.addingTimeInterval(seconds),
                        expiresIn: seconds,
                        handle: native
                    )
                    return (snapshot, { native.ready }, { await native.close() })
                } catch {
                    await native.close()
                    throw error
                }
            },
            renew: { configuration, leaseID in
                let receipt = try await NolaTailcatGrant.exchange(configuration, payload: [
                    "operation": "renew",
                    "leaseId": leaseID,
                ])
                guard let seconds = receipt.expiresIn, (31...1200).contains(seconds) else {
                    throw URLError(.badServerResponse)
                }
                return seconds
            },
            onModeChange: { key, mode in
                NolaTailcatTransport.setTransportMode(mode, for: key)
            }
        )
        #else
        return configuration.origin
        #endif
    }
}

// Reuses Nola's encrypted mobile envelope and proof contract. No bearer token,
// grant address, temporary key or response is logged or persisted.
private nonisolated struct NolaTailcatGrantReceipt: Decodable, Sendable {
    let hostId: String?
    let address: String?
    let leaseId: String?
    let expiresIn: Double?
    let closed: Bool?
    let closing: Bool?
    let error: String?
}

private nonisolated enum NolaTailcatGrant {
    static func base64(_ value: Data) -> String {
        value.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
    static func decode(_ value: String) throws -> Data {
        var value = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        value += String(repeating: "=", count: (4 - value.count % 4) % 4)
        guard let data = Data(base64Encoded: value) else { throw URLError(.cannotDecodeContentData) }; return data
    }
    static func exchange(_ configuration: NolaTailcatTransport.Configuration, payload: [String: String]) async throws -> NolaTailcatGrantReceipt {
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data(configuration.token.utf8))))
        let aad = Data("nola-mobile-bridge-v1".utf8)
        let sealed = try AES.GCM.seal(JSONSerialization.data(withJSONObject: payload), using: key, authenticating: aad)
        let body = try JSONSerialization.data(withJSONObject: ["v": 1, "alg": "AES-256-GCM", "nonce": base64(Data(sealed.nonce)), "ciphertext": base64(sealed.ciphertext), "tag": base64(sealed.tag)])
        var request = URLRequest(url: configuration.origin.appendingPathComponent("api/mobile/transport/tailcat"))
        let op = payload["operation"]
        request.httpMethod = "POST"
        request.httpBody = body
        request.timeoutInterval = op == "close" ? 6 : (op == "renew" ? 12 : 40)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("aes-gcm-v1", forHTTPHeaderField: "X-Nola-Mobile-Crypto")
        let timestamp = String(UInt64(Date().timeIntervalSince1970)), nonce = UUID().uuidString.lowercased()
        let digest = SHA256.hash(data: body).map { String(format: "%02x", $0) }.joined()
        let comps = request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
        let path = (comps?.percentEncodedPath ?? request.url!.path) + (comps?.percentEncodedQuery.map { "?\($0)" } ?? "")
        let message = "v1\nPOST\n\(path)\n\(timestamp)\n\(nonce)\n\(digest)"
        request.setValue(timestamp, forHTTPHeaderField: "X-Nola-Mobile-Timestamp")
        request.setValue(nonce, forHTTPHeaderField: "X-Nola-Mobile-Nonce")
        request.setValue(digest, forHTTPHeaderField: "X-Nola-Mobile-Body-SHA256")
        request.setValue(base64(Data(HMAC<SHA256>.authenticationCode(for: Data(message.utf8), using: SymmetricKey(data: Data(configuration.token.utf8))))), forHTTPHeaderField: "X-Nola-Mobile-Auth")
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, data.count <= 65536 else {
            throw URLError(.badServerResponse)
        }
        let plain: Data
        if let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           envelope["v"] as? Int == 1, envelope["alg"] as? String == "AES-256-GCM",
           let nonce = envelope["nonce"] as? String, let ciphertext = envelope["ciphertext"] as? String,
           let tag = envelope["tag"] as? String {
            let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: decode(nonce)), ciphertext: decode(ciphertext), tag: decode(tag))
            plain = try AES.GCM.open(box, using: key, authenticating: aad)
        } else if http.statusCode != 200 {
            if let plainObject = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let errorCode = plainObject["error"] as? String, !errorCode.isEmpty {
                throw NolaTailcatTransportError.hostError(status: http.statusCode, code: errorCode)
            }
            throw URLError(.badServerResponse)
        } else {
            throw URLError(.badServerResponse)
        }
        let receipt = try JSONDecoder().decode(NolaTailcatGrantReceipt.self, from: plain)
        if http.statusCode != 200 {
            if let errorCode = receipt.error, !errorCode.isEmpty {
                throw NolaTailcatTransportError.hostError(status: http.statusCode, code: errorCode)
            }
            throw URLError(.badServerResponse)
        }
        return receipt
    }
}

/// Shared pooled forwarding URLSession so loopback HTTP keep-alive connections
/// are reused instead of allocating a fresh URLSession and DialTCPPort per request.
private final class NolaTailcatForwardingPool: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    static let shared = NolaTailcatForwardingPool()

    private let lock = NSLock()
    private var handlers: [Int: NolaTailcatURLProtocol] = [:]
    private lazy var session: URLSession = {
        let config = URLSessionConfiguration.default
        config.httpMaximumConnectionsPerHost = 12
        return URLSession(configuration: config, delegate: self, delegateQueue: nil)
    }()

    func createTask(with request: URLRequest, for handler: NolaTailcatURLProtocol) -> URLSessionDataTask {
        let task = session.dataTask(with: request)
        lock.withLock { handlers[task.taskIdentifier] = handler }
        return task
    }

    func cancel(task: URLSessionDataTask) {
        lock.withLock { _ = handlers.removeValue(forKey: task.taskIdentifier) }
        task.cancel()
    }

    private func handler(for task: URLSessionTask) -> NolaTailcatURLProtocol? {
        lock.withLock { handlers[task.taskIdentifier] }
    }

    private func removeHandler(for task: URLSessionTask) -> NolaTailcatURLProtocol? {
        lock.withLock { handlers.removeValue(forKey: task.taskIdentifier) }
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let handler = handler(for: dataTask) else {
            completionHandler(.cancel)
            return
        }
        handler.handleResponse(response, completionHandler: completionHandler)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        handler(for: dataTask)?.handleData(data)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        removeHandler(for: task)?.handleCompletion(error: error)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        // Never forward signed mobile credentials to a different origin.
        completionHandler(nil)
    }
}

internal final class NolaTailcatURLProtocol: URLProtocol, @unchecked Sendable {
    private let stateLock = NSLock()
    private var stopped = false
    private var preparing: Task<Void, Never>?
    private var loading: URLSessionDataTask?

    override class func canInit(with request: URLRequest) -> Bool {
        guard let url = request.url, let configuration = NolaTailcatTransport.configuration(for: url) else { return false }
        return NolaTailcatTransport.matches(request, configuration: configuration)
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let task = Task {
            do {
                guard let original = request.url, let configuration = NolaTailcatTransport.configuration(for: original) else { throw URLError(.badURL) }
                guard NolaTailcatTransport.matches(request, configuration: configuration) else { throw URLError(.cancelled) }
                let endpoint = try await NolaTailcatConnections.shared.endpoint(for: configuration)
                var parts = URLComponents(url: original, resolvingAgainstBaseURL: false)!
                parts.scheme = endpoint.scheme; parts.host = endpoint.host; parts.port = endpoint.port
                guard let url = parts.url else { throw URLError(.badURL) }
                var forwarded = request; forwarded.url = url
                try Task.checkCancellation()
                let dataTask = NolaTailcatForwardingPool.shared.createTask(with: forwarded, for: self)
                let shouldResume = stateLock.withLock { () -> Bool in
                    if stopped { return false }
                    loading = dataTask
                    return true
                }
                if shouldResume {
                    dataTask.resume()
                } else {
                    NolaTailcatForwardingPool.shared.cancel(task: dataTask)
                }
            } catch {
                deliverFailure(error)
            }
        }
        stateLock.withLock {
            if stopped {
                task.cancel()
            } else {
                preparing = task
            }
        }
    }

    override func stopLoading() {
        let (prep, load) = stateLock.withLock { () -> (Task<Void, Never>?, URLSessionDataTask?) in
            stopped = true
            let p = preparing; preparing = nil
            let l = loading; loading = nil
            return (p, l)
        }
        prep?.cancel()
        if let load {
            NolaTailcatForwardingPool.shared.cancel(task: load)
        }
    }

    /// Exposed for unit testing to verify the URLProtocol stop-guard contract.
    internal func deliverFailure(_ error: Error) {
        let activeClient = stateLock.withLock { stopped ? nil : client }
        activeClient?.urlProtocol(self, didFailWithError: error)
    }

    fileprivate func handleResponse(_ response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        let activeClient = stateLock.withLock { stopped ? nil : client }
        guard let activeClient else {
            completionHandler(.cancel)
            return
        }
        if let http = response as? HTTPURLResponse, let original = request.url,
           let remapped = HTTPURLResponse(url: original, statusCode: http.statusCode, httpVersion: nil, headerFields: http.allHeaderFields as? [String: String]) {
            activeClient.urlProtocol(self, didReceive: remapped, cacheStoragePolicy: .notAllowed)
        } else {
            activeClient.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        }
        completionHandler(.allow)
    }

    fileprivate func handleData(_ data: Data) {
        let activeClient = stateLock.withLock { stopped ? nil : client }
        activeClient?.urlProtocol(self, didLoad: data)
    }

    fileprivate func handleCompletion(error: Error?) {
        let activeClient = stateLock.withLock { () -> URLProtocolClient? in
            loading = nil
            return stopped ? nil : client
        }
        if let error {
            if let url = request.url, (error as? URLError)?.code != .cancelled {
                Task { await NolaTailcatConnections.shared.reportForwardingFailure(for: url) }
            }
            activeClient?.urlProtocol(self, didFailWithError: error)
        } else {
            activeClient?.urlProtocolDidFinishLoading(self)
        }
    }
}
