// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Connects native macOS presentation code to the single-writer Nola Application Service.

import Foundation
import Darwin
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated enum MacApplicationServiceError: LocalizedError, Sendable {
    case executableUnavailable
    case launchFailed(String)
    case incompatibleStore(String)
    case connectionFailed(String)
    case invalidResponse
    case outcomePending(requestID: String)
    case rejected(String)
    case revisionConflict(scope: String, expected: UInt64, actual: UInt64)

    var errorDescription: String? {
        switch self {
        case .executableUnavailable:
            return String(localized: "The Nola Application Service is unavailable.")
        case .launchFailed(let detail):
            return String(
                format: String(localized: "The Nola Application Service could not start: %@"),
                detail
            )
        case .incompatibleStore(let detail):
            return detail
        case .connectionFailed(let detail):
            return String(
                format: String(localized: "The Nola Application Service connection failed: %@"),
                detail
            )
        case .invalidResponse:
            return String(localized: "The Nola Application Service returned an invalid response.")
        case .outcomePending(let requestID):
            return String(format: String(localized: "Request %@ was accepted, but its result is not confirmed. Refresh before submitting it again."), requestID)
        case .rejected(let message):
            return message
        case .revisionConflict(let scope, let expected, let actual):
            return String(
                format: String(localized: "The operation conflicted with a newer revision of %@ (%llu expected, %llu current)."),
                scope,
                expected,
                actual
            )
        }
    }
}

/// Process ownership is an application lifecycle concern, not an actor task.
/// AppKit does not wait for asynchronous termination observers while quitting,
/// so every long-lived child is registered here and stopped synchronously from
/// `applicationWillTerminate`.
nonisolated final class MacOwnedChildProcessRegistry: @unchecked Sendable {
    static let shared = MacOwnedChildProcessRegistry()

    private let lock = NSLock()
    private var processes: [Int32: Process] = [:]

    init() {}

    func register(_ process: Process) {
        let processID = process.processIdentifier
        guard processID > 0 else { return }
        lock.lock()
        processes[processID] = process
        lock.unlock()
    }

    func unregister(_ process: Process) {
        lock.lock()
        processes.removeValue(forKey: process.processIdentifier)
        lock.unlock()
    }

    func stop(_ process: Process, gracePeriod: TimeInterval = 1) {
        Self.stopProcess(process, gracePeriod: gracePeriod)
        observeExit(process)
    }

    private func observeExit(_ process: Process) {
        guard process.isRunning else {
            unregister(process)
            return
        }
        // Foundation owns waitpid. Keep ownership until it observes termination,
        // without blocking the caller or a worker in waitUntilExit/run-loop work.
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 0.1) { [self, process] in
            observeExit(process)
        }
    }

    func stopAll(gracePeriod: TimeInterval = 1) {
        lock.lock()
        let owned = Array(processes.values)
        lock.unlock()
        for process in owned {
            stop(process, gracePeriod: gracePeriod)
        }
    }

    private static func stopProcess(_ process: Process, gracePeriod: TimeInterval) {
        guard process.isRunning else { return }
        process.terminate()
        let deadline = Date().addingTimeInterval(max(0, gracePeriod))
        while process.isRunning, Date() < deadline {
            Thread.sleep(forTimeInterval: 0.02)
        }
        if process.isRunning {
            Darwin.kill(process.processIdentifier, SIGKILL)
        }
    }
}

/// Owns only cancellation, never socket lifetime. Shutdown and unregister
/// share a lock so cancellation cannot affect an already recycled descriptor.
nonisolated final class MacApplicationQueryCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false
    private var descriptor: Int32?

    func cancel() {
        lock.lock()
        defer { lock.unlock() }
        cancelled = true
        if let descriptor { _ = Darwin.shutdown(descriptor, SHUT_RDWR) }
    }

    fileprivate func register(_ descriptor: Int32) throws {
        lock.lock()
        defer { lock.unlock() }
        if cancelled { throw CancellationError() }
        self.descriptor = descriptor
    }

    fileprivate func unregister(_ descriptor: Int32) {
        lock.lock()
        defer { lock.unlock() }
        if self.descriptor == descriptor { self.descriptor = nil }
    }

    fileprivate func checkCancellation() throws {
        lock.lock()
        let cancelled = cancelled
        lock.unlock()
        if cancelled { throw CancellationError() }
    }
}

nonisolated final class MacApplicationServiceClient: @unchecked Sendable {
    static let protocolVersion = 1
    static let applicationABIVersion: UInt32 = 1
    #if !canImport(NolaCore)
    #error("NolaMac requires NolaCore.framework for its Application Service protocol identity")
    #endif
    /// Client contracts do not adapt to whichever framework happens to link.
    /// Keep this pure check testable without opening a store or starting a writer.
    static func validateLinkedContract(abiVersion: UInt32, serviceProtocol: UInt32) throws {
        guard abiVersion == applicationABIVersion, serviceProtocol == UInt32(protocolVersion) else {
            throw MacApplicationServiceError.rejected(
                "Application ABI/protocol mismatch: expected \(applicationABIVersion)/\(protocolVersion), received \(abiVersion)/\(serviceProtocol)")
        }
    }

    private static func requireLinkedContract() throws {
        try validateLinkedContract(abiVersion: nola_core_application_abi_version(),
            serviceProtocol: nola_core_application_service_protocol_version())
    }
    private static let maximumFrameBytes = 16 * 1_024 * 1_024
    // Commands and responses share the same newline-delimited transport
    // envelope. Keep this aligned with nola-application's frame contract so
    // bounded transcript-import batches are not rejected by the client before
    // Core can validate them.
    private static let maximumCommandBytes = maximumFrameBytes

    let databaseURL: URL
    let socketURL: URL
    let clientID: String
    private let ownsSupervisorReference: Bool
    private let expertHostJournal: ApplePendingCommandJournal

    init(databaseURL: URL) throws {
        try Self.requireLinkedContract()
        self.databaseURL = databaseURL
        expertHostJournal = Self.makeExpertHostJournal(databaseURL)
        socketURL = Self.serviceSocketURL(for: databaseURL)
        clientID = Self.serviceClientID(for: databaseURL)
        ownsSupervisorReference = true
        try FileManager.default.createDirectory(
            at: databaseURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try MacApplicationServiceSupervisor.shared.ensureRunning(
            databaseURL: databaseURL,
            socketURL: socketURL
        )
        MacApplicationServiceSupervisor.shared.retainClient(socketURL: socketURL)
    }

    deinit {
        if ownsSupervisorReference {
            MacApplicationServiceSupervisor.shared.releaseClient(socketURL: socketURL)
        }
    }

    private init(databaseURL: URL, socketURL: URL, retainsSupervisor: Bool = false) {
        self.databaseURL = databaseURL
        expertHostJournal = Self.makeExpertHostJournal(databaseURL)
        self.socketURL = socketURL
        clientID = Self.serviceClientID(for: databaseURL)
        ownsSupervisorReference = retainsSupervisor
        if retainsSupervisor {
            MacApplicationServiceSupervisor.shared.retainClient(socketURL: socketURL)
        }
    }

    /// Creating the application dependency graph must not perform fallible IO.
    /// Startup and individual commands surface connection errors to their caller.
    static func deferred(databaseURL: URL) -> MacApplicationServiceClient {
        MacApplicationServiceClient(
            databaseURL: databaseURL,
            socketURL: serviceSocketURL(for: databaseURL),
            retainsSupervisor: true
        )
    }

    func prepare() throws {
        try Self.requireLinkedContract()
        try MacApplicationServiceSupervisor.shared.ensureRunning(
            databaseURL: databaseURL,
            socketURL: socketURL,
            verifyExisting: true
        )
    }

    func archiveIncompatibleStoreAndPrepare() throws -> URL? {
        try Self.requireLinkedContract()
        return try MacApplicationServiceSupervisor.shared.archiveIncompatibleStoreAndStart(
            databaseURL: databaseURL,
            socketURL: socketURL
        )
    }

    fileprivate static func unstarted(
        databaseURL: URL,
        socketURL: URL
    ) -> MacApplicationServiceClient {
        MacApplicationServiceClient(databaseURL: databaseURL, socketURL: socketURL)
    }

    func execute(
        commandData: Data,
        requestID: String = UUID().uuidString.lowercased(),
        queryCancellation: MacApplicationQueryCancellation? = nil,
        expectedRevision: UInt64? = nil,
        onCommittedRevision: ((UInt64?) -> Void)? = nil
    ) throws -> Data {
        try Self.requireLinkedContract()
        // Establish a live writer before sending this stable operation
        // identity. A transport failure is recovered by querying its durable
        // receipt; the write command itself is never replayed implicitly.
        try queryCancellation?.checkCancellation()
        try MacApplicationServiceSupervisor.shared.ensureRunning(
            databaseURL: databaseURL,
            socketURL: socketURL
        )
        guard var command = try JSONSerialization.jsonObject(
            with: commandData,
            options: [.fragmentsAllowed]
        ) as? [String: Any],
              let commandType = command.removeValue(forKey: "type") as? String,
              !commandType.isEmpty else {
            throw MacApplicationServiceError.invalidResponse
        }
        let isProjectionQuery: Bool
        switch commandType {
        case "projection_cursor_wait", "projection_event_suffix", "projection_snapshot",
             "command_receipt", "health", "resolve_model_reasoning", "resolve_remote_model_profile", "query_automation_occurrences", "resolve_chat_model", "canonical_run_terminal", "inspect_run_recovery", "read_permission_configuration", "inspect_unknown_effect_recovery",
             "load_conversation_projection_window_v1", "load_conversation_projection_window_update_v1",
             "read_conversation_text_range",
             "list_activity", "load_activity_detail", "load_today_facts", "load_prepared_edited_resend", "load_conversation_context_usage_v1", "load_conversation_permission_projection_v1":
            isProjectionQuery = true
        default:
            isProjectionQuery = false
        }
        guard queryCancellation == nil || isProjectionQuery else {
            throw MacApplicationServiceError.invalidResponse
        }
        var request: [String: Any] = [
            "protocol_version": Self.protocolVersion,
            "request_id": requestID,
            "client_id": clientID,
            "command": [
                "type": commandType,
                "arguments": command,
            ],
        ]
        if let expectedRevision {
            request["expected_revision"] = expectedRevision
        }
        var response: [String: Any]
        var attemptedReceiptRecovery = false
        do {
            response = try send(request, queryCancellation: queryCancellation)
        } catch {
            try queryCancellation?.checkCancellation()
            // Queries have no durable receipt. Their caller can reload the
            // current projection; receipt recovery is reserved for mutations.
            if isProjectionQuery { throw error }
            let originalError = error
            try? MacApplicationServiceSupervisor.shared.ensureRunning(
                databaseURL: databaseURL,
                socketURL: socketURL,
                verifyExisting: true
            )
            let recovered: [String: Any]?
            do {
                attemptedReceiptRecovery = true
                recovered = try recoverReceipt(requestID: requestID)
            } catch {
                throw originalError
            }
            guard let recovered else { throw originalError }
            response = recovered
        }
        guard response["request_id"] as? String == requestID,
              response["client_id"] as? String == clientID,
              response["protocol_version"] as? Int == Self.protocolVersion else {
            throw MacApplicationServiceError.invalidResponse
        }
        if response["status"] as? String == "accepted" {
            guard !isProjectionQuery else { throw MacApplicationServiceError.invalidResponse }
            guard !attemptedReceiptRecovery else {
                throw MacApplicationServiceError.outcomePending(requestID: requestID)
            }
            guard let recovered = try recoverReceipt(requestID: requestID) else {
                throw MacApplicationServiceError.outcomePending(requestID: requestID)
            }
            response = recovered
            guard response["request_id"] as? String == requestID,
                  response["client_id"] as? String == clientID,
                  response["protocol_version"] as? Int == Self.protocolVersion else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
        switch response["status"] as? String {
        case "committed":
            onCommittedRevision?((response["committed_revision"] as? NSNumber)?.uint64Value)
            let result = response["result"] ?? NSNull()
            return try JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed])
        case "rejected":
            let error = response["error"] as? [String: Any]
            throw MacApplicationServiceError.rejected(
                error?["message"] as? String
                    ?? String(localized: "The Nola Application Service rejected the operation.")
            )
        case "conflict":
            guard let conflict = response["conflict"] as? [String: Any],
                  let scope = conflict["scope_id"] as? String,
                  let expected = (conflict["expected_revision"] as? NSNumber)?.uint64Value,
                  let actual = (conflict["actual_revision"] as? NSNumber)?.uint64Value else {
                throw MacApplicationServiceError.invalidResponse
            }
            throw MacApplicationServiceError.revisionConflict(
                scope: scope,
                expected: expected,
                actual: actual
            )
        case "accepted":
            throw MacApplicationServiceError.outcomePending(requestID: requestID)
        default:
            throw MacApplicationServiceError.invalidResponse
        }
    }

    private static func makeExpertHostJournal(_ database: URL) -> ApplePendingCommandJournal {
        ApplePendingCommandJournal(databaseURL: database, namespace: .expertHost,
            validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
    }

    /// Native Host-only typed channel. Transport IDs are distinct from business
    /// intent IDs; uncertain requests retain their exact original envelope.
    func expertHostOperation<Result: Decodable>(_ operation: AppleExpertHostOperation<MacCanonicalJSONValue>) throws -> Result {
        try Self.requireLinkedContract()
        try MacApplicationServiceSupervisor.shared.ensureRunning(databaseURL: databaseURL, socketURL: socketURL)
        guard let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(operation)) as? [String: Any] else {
            throw MacApplicationServiceError.invalidResponse
        }
        func decode(_ response: [String: Any], requestID: String) throws -> Result {
            guard response["request_id"] as? String == requestID,
                  response["client_id"] as? String == clientID,
                  response["protocol_version"] as? Int == Self.protocolVersion else {
                throw MacApplicationServiceError.outcomePending(requestID: requestID)
            }
            guard response["status"] as? String == "committed" else {
                throw MacApplicationServiceError.invalidResponse
            }
            let data = try JSONSerialization.data(withJSONObject: response["result"] ?? NSNull(), options: [.fragmentsAllowed])
            if !operation.isReadOnly { try Self.validateExpertMutationResult(data, operation: operation) }
            return try JSONDecoder().decode(Result.self, from: data)
        }
        func request(_ id: String) -> [String: Any] {
            ["protocol_version": Self.protocolVersion, "request_id": id, "client_id": clientID,
             "command": ["type": "host_expert_template_operation", "arguments": ["operation": encoded]]]
        }
        if operation.isReadOnly {
            let id = UUID().uuidString.lowercased()
            let response = try send(request(id))
            if response["status"] as? String == "rejected" {
                throw MacApplicationServiceError.rejected((response["error"] as? [String: Any])?["message"] as? String ?? "Expert Host read rejected")
            }
            return try decode(response, requestID: id)
        }
        let scope = try expertHostJournal.beginTransaction()
        defer { scope.close() }
        var pending = try scope.load()
        let stable = try AppleExpertHostIntentScope.stableIdentity(encoded)
        var existing: (String, [String: Any])?
        for key in pending.keys.sorted() {
            guard let saved = pending[key], saved["client_id"] as? String == clientID,
                  saved["protocol_version"] as? Int == Self.protocolVersion else {
                throw MacApplicationServiceError.outcomePending(requestID: key)
            }
            let original = try AppleExpertHostIntentScope.operation(in: saved)
            if try AppleExpertHostIntentScope.stableIdentity(original) == stable {
                existing = (key, saved)
            } else if AppleExpertHostIntentScope.intersects(original, encoded) {
                throw MacApplicationServiceError.outcomePending(requestID: key)
            }
        }
        let hadPending = existing != nil
        let envelope: [String: Any]
        let id: String
        if let existing { id = existing.0; envelope = existing.1 }
        else {
            id = UUID().uuidString.lowercased(); envelope = request(id)
            pending[id] = envelope
            try scope.save(pending)
        }
        let response: [String: Any]
        do {
            if hadPending, let receipt = try recoverReceipt(requestID: id) { response = receipt }
            else { response = try send(envelope) }
        } catch { throw MacApplicationServiceError.outcomePending(requestID: id) }
        guard response["request_id"] as? String == id,
              response["client_id"] as? String == clientID,
              response["protocol_version"] as? Int == Self.protocolVersion else {
            throw MacApplicationServiceError.outcomePending(requestID: id)
        }
        switch response["status"] as? String {
        case "committed":
            let value: Result
            do { value = try decode(response, requestID: id) }
            catch { throw MacApplicationServiceError.outcomePending(requestID: id) }
            pending[id] = nil; try scope.save(pending)
            return value
        case "rejected":
            let failure = response["error"] as? [String: Any]
            if Self.expertOutcomeNeedsReopen(failure?["code"] as? String ?? "") {
                throw MacApplicationServiceError.outcomePending(requestID: id)
            }
            pending[id] = nil; try scope.save(pending)
            throw MacApplicationServiceError.rejected(failure?["message"] as? String ?? "Expert Host operation rejected")
        default:
            throw MacApplicationServiceError.outcomePending(requestID: id)
        }
    }

    func resumePendingExpertAdmission<Result: Decodable>(_ input: AppleAdmitExpertConversationMessage) throws -> Result? {
        let scope = try expertHostJournal.beginTransaction()
        let pending: [String: [String: Any]]
        do { pending = try scope.load(); scope.close() }
        catch { scope.close(); throw error }
        let authority = AppleExpertTemplateAuthority.local()
        let replay = AppleExpertHostOperation<MacCanonicalJSONValue>.admit(input: input,
            context: .init(actor: authority.actor, request_host_id: authority.request_host_id, selections: [], builtins: []))
        guard let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(replay)) as? [String: Any] else {
            throw MacApplicationServiceError.invalidResponse
        }
        let stable = try AppleExpertHostIntentScope.stableIdentity(encoded)
        var found = false
        for key in pending.keys.sorted() {
            guard let saved = pending[key], saved["client_id"] as? String == clientID,
                  saved["protocol_version"] as? Int == Self.protocolVersion else {
                throw MacApplicationServiceError.outcomePending(requestID: key)
            }
            let original = try AppleExpertHostIntentScope.operation(in: saved)
            if try AppleExpertHostIntentScope.stableIdentity(original) == stable { found = true }
        }
        guard found else { return nil }
        // expertHostOperation will replay the saved exact envelope and transport
        // ID. The empty execution context above is only a stable-intent probe.
        return try expertHostOperation(replay)
    }

    private static func expertOutcomeNeedsReopen(_ code: String) -> Bool {
        ["command_transaction_commit_outcome_unknown", "runtime_reconciliation_commit_outcome_unknown",
         "command_transaction_commit_failed", "runtime_transaction_commit_failed", "runtime_adoption_failed",
         "runtime_reopen_required", "reopen_required", "command_transaction_rollback_failed",
         "command_savepoint_rollback_failed", "command_savepoint_release_failed"].contains(code)
    }

    private static func validateExpertMutationResult(_ data: Data,
        operation: AppleExpertHostOperation<MacCanonicalJSONValue>) throws {
        switch operation {
        case .admit(let input, _):
            let value = try JSONDecoder().decode(AppleExpertMessageManifest<MacCanonicalJSONValue>.self, from: data)
            guard value.version == 1, value.input == input,
                  value.entries.map(\.participant) == input.selected_participants else {
                throw MacApplicationServiceError.invalidResponse
            }
        case .prepareResend(let input, _):
            let value = try JSONDecoder().decode(ApplePreparedExpertResend<MacCanonicalJSONValue>.self, from: data)
            guard value.input == input else { throw MacApplicationServiceError.invalidResponse }
        default: throw MacApplicationServiceError.invalidResponse
        }
    }

    private func recoverReceipt(requestID: String) throws -> [String: Any]? {
        let deadline = Date().addingTimeInterval(2)
        var retryDelay: TimeInterval = 0.025
        var acceptedReceipt: [String: Any]?
        repeat {
            let lookupRequestID = UUID().uuidString.lowercased()
            let response = try send([
                "protocol_version": Self.protocolVersion,
                "request_id": lookupRequestID,
                "client_id": clientID,
                "command": [
                    "type": "command_receipt",
                    "arguments": ["request_id": requestID],
                ],
            ], deadline: DispatchTime.now().uptimeNanoseconds + UInt64(max(0, deadline.timeIntervalSinceNow) * 1_000_000_000))
            guard response["request_id"] as? String == lookupRequestID,
                  response["client_id"] as? String == clientID,
                  response["protocol_version"] as? Int == Self.protocolVersion,
                  response["status"] as? String == "committed" else {
                throw MacApplicationServiceError.invalidResponse
            }
            if response["result"] is NSNull {
                Thread.sleep(forTimeInterval: retryDelay)
                retryDelay = min(retryDelay * 2, 0.25)
                continue
            }
            guard let receipt = response["result"] as? [String: Any],
                  receipt["request_id"] as? String == requestID,
                  receipt["client_id"] as? String == clientID,
                  receipt["protocol_version"] as? Int == Self.protocolVersion else {
                throw MacApplicationServiceError.invalidResponse
            }
            if receipt["status"] as? String == "accepted" {
                acceptedReceipt = receipt
                Thread.sleep(forTimeInterval: retryDelay)
                retryDelay = min(retryDelay * 2, 0.25)
                continue
            }
            return receipt
        } while Date() < deadline
        return acceptedReceipt
    }

    func healthCheck() throws {
        try Self.requireLinkedContract()
        let requestID = UUID().uuidString.lowercased()
        let response = try send([
            "protocol_version": Self.protocolVersion,
            "request_id": requestID,
            "client_id": clientID,
            "command": [
                "type": "health",
                "arguments": [:],
            ],
        ], deadline: DispatchTime.now().uptimeNanoseconds + 250_000_000)
        guard response["request_id"] as? String == requestID,
              response["client_id"] as? String == clientID,
              response["protocol_version"] as? Int == Self.protocolVersion,
              response["status"] as? String == "committed" else {
            throw MacApplicationServiceError.invalidResponse
        }
    }

    func send(
        _ request: [String: Any],
        queryCancellation: MacApplicationQueryCancellation? = nil,
        deadline requestedDeadline: UInt64? = nil
    ) throws -> [String: Any] {
        #if DEBUG
        let diagnoseMainThread = Thread.isMainThread
            && ProcessInfo.processInfo.environment["NOLA_DIAGNOSE_MAIN_SERVICE"] == "1"
        let diagnosticStart = SuspendingClock.now
        defer {
            if diagnoseMainThread {
                let elapsed = diagnosticStart.duration(to: .now)
                if elapsed > .milliseconds(5) {
                    // Never include request bodies, credentials or resource paths.
                    print("main_service_timing elapsed=\(elapsed) stack=\(Thread.callStackSymbols.prefix(22).joined(separator: " | "))")
                }
            }
        }
        #endif
        // One monotonic deadline covers connect, write and read. Receiving a
        // partial frame must not reset the budget or keep a client thread alive.
        let deadline: UInt64? = requestedDeadline ?? (DispatchTime.now().uptimeNanoseconds + 30_000_000_000)
        try queryCancellation?.checkCancellation()
        let payload = try JSONSerialization.data(withJSONObject: request, options: [.sortedKeys])
        guard payload.count < Self.maximumCommandBytes else {
            throw MacApplicationServiceError.connectionFailed("request exceeds the command limit")
        }
        guard payload.count < Self.maximumFrameBytes else {
            throw MacApplicationServiceError.connectionFailed("request exceeds the frame limit")
        }
        var framed = payload
        framed.append(0x0A)

        let descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else {
            throw MacApplicationServiceError.connectionFailed(Self.posixMessage())
        }
        defer {
            queryCancellation?.unregister(descriptor)
            Darwin.close(descriptor)
        }
        var noSignal: Int32 = 1
        _ = withUnsafePointer(to: &noSignal) {
            Darwin.setsockopt(
                descriptor,
                SOL_SOCKET,
                SO_NOSIGPIPE,
                $0,
                socklen_t(MemoryLayout<Int32>.size)
            )
        }
        if deadline != nil {
            let flags = Darwin.fcntl(descriptor, F_GETFL)
            guard flags >= 0, Darwin.fcntl(descriptor, F_SETFL, flags | O_NONBLOCK) == 0 else {
                throw MacApplicationServiceError.connectionFailed(Self.posixMessage())
            }
        }
        try queryCancellation?.register(descriptor)
        try Self.connect(descriptor, path: socketURL.path, deadline: deadline)
        try queryCancellation?.checkCancellation()
        try framed.withUnsafeBytes { rawBuffer in
            guard let base = rawBuffer.baseAddress else { return }
            var sent = 0
            while sent < rawBuffer.count {
                try queryCancellation?.checkCancellation()
                try Self.waitForSocket(descriptor, events: Int16(POLLOUT), deadline: deadline)
                let count = Darwin.write(descriptor, base.advanced(by: sent), rawBuffer.count - sent)
                try queryCancellation?.checkCancellation()
                if count < 0, errno == EINTR || (deadline != nil && (errno == EAGAIN || errno == EWOULDBLOCK)) {
                    continue
                }
                guard count > 0 else {
                    throw MacApplicationServiceError.connectionFailed(Self.posixMessage())
                }
                sent += count
            }
        }

        var response = Data()
        var buffer = [UInt8](repeating: 0, count: 64 * 1_024)
        var reachedFrameEnd = false
        while response.count <= Self.maximumFrameBytes {
            try queryCancellation?.checkCancellation()
            try Self.waitForSocket(descriptor, events: Int16(POLLIN), deadline: deadline)
            let count = buffer.withUnsafeMutableBytes { rawBuffer in
                Darwin.read(descriptor, rawBuffer.baseAddress, rawBuffer.count)
            }
            try queryCancellation?.checkCancellation()
            if count == 0 { break }
            guard count > 0 else {
                if errno == EINTR || (deadline != nil && (errno == EAGAIN || errno == EWOULDBLOCK)) { continue }
                throw MacApplicationServiceError.connectionFailed(Self.posixMessage())
            }
            let chunk = buffer[0..<count]
            if let newline = chunk.firstIndex(of: 0x0A) {
                response.append(contentsOf: chunk[..<newline])
                reachedFrameEnd = true
                break
            }
            response.append(contentsOf: chunk)
        }
        guard reachedFrameEnd, !response.isEmpty,
              response.count <= Self.maximumFrameBytes,
              let object = try JSONSerialization.jsonObject(with: response) as? [String: Any] else {
            throw MacApplicationServiceError.invalidResponse
        }
        return object
    }

    private static func waitForSocket(_ descriptor: Int32, events: Int16, deadline: UInt64?) throws {
        guard let deadline else { return }
        while true {
            let now = DispatchTime.now().uptimeNanoseconds
            guard now < deadline else {
                throw MacApplicationServiceError.connectionFailed("service request timed out")
            }
            let remainingMS = min((deadline - now + 999_999) / 1_000_000, UInt64(Int32.max))
            var descriptorState = pollfd(fd: descriptor, events: events, revents: 0)
            let result = Darwin.poll(&descriptorState, 1, Int32(remainingMS))
            if result > 0 { return }
            if result == 0 { continue }
            if errno == EINTR { continue }
            throw MacApplicationServiceError.connectionFailed(posixMessage())
        }
    }

    private static func connect(_ descriptor: Int32, path: String, deadline: UInt64? = nil) throws {
        let pathBytes = Array(path.utf8)
        var address = sockaddr_un()
        guard pathBytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw MacApplicationServiceError.connectionFailed("service socket path is too long")
        }
        address.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &address.sun_path) { destination in
            destination.initializeMemory(as: UInt8.self, repeating: 0)
            destination.copyBytes(from: pathBytes)
        }
        let length = socklen_t(MemoryLayout<sa_family_t>.size + pathBytes.count + 1)
        let result = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(descriptor, $0, length)
            }
        }
        if result != 0, deadline != nil, errno == EINPROGRESS || errno == EAGAIN || errno == EWOULDBLOCK {
            try waitForSocket(descriptor, events: Int16(POLLOUT), deadline: deadline)
            var socketError: Int32 = 0
            var size = socklen_t(MemoryLayout<Int32>.size)
            guard Darwin.getsockopt(descriptor, SOL_SOCKET, SO_ERROR, &socketError, &size) == 0 else {
                throw MacApplicationServiceError.connectionFailed(posixMessage())
            }
            guard socketError == 0 else {
                throw MacApplicationServiceError.connectionFailed(String(cString: strerror(socketError)))
            }
            return
        }
        guard result == 0 else {
            throw MacApplicationServiceError.connectionFailed(posixMessage())
        }
    }

    static func serviceSocketURL(for databaseURL: URL) -> URL {
        let hash = databaseIdentityHash(for: databaseURL)
        // This is a protocol identity shared with nola-service-client. Both the
        // native app and Agent Host must address the same single writer for a
        // database; a platform-specific socket name starts competing services
        // and produces revision conflicts during transcript reconciliation.
        let hexadecimal = String(format: "%016llx", hash)
        return URL(fileURLWithPath: "/tmp", isDirectory: true).appendingPathComponent(
            "nola-application-v\(protocolVersion)-\(hexadecimal).sock",
            isDirectory: false
        )
    }

    static func serviceClientID(for databaseURL: URL) -> String {
        String(format: "nola-local-%016llx", databaseIdentityHash(for: databaseURL))
    }

    private static func databaseIdentityHash(for databaseURL: URL) -> UInt64 {
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in databaseURL.standardizedFileURL.path.utf8 {
            hash ^= UInt64(byte)
            hash &*= 1_099_511_628_211
        }
        return hash
    }

    private static func posixMessage() -> String {
        String(cString: strerror(errno))
    }
}

/// Drain stderr for the entire child lifetime, not only after startup fails.
/// Otherwise a full pipe can stall a healthy service, and a synchronous read
/// during recovery can wait indefinitely for a still-open writer.
nonisolated private final class MacApplicationServiceStderr: @unchecked Sendable {
    let pipe = Pipe()
    private let lock = NSLock()
    private var tail = Data()
    private let limit = 16 * 1_024

    init() {
        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else {
                handle.readabilityHandler = nil
                return
            }
            self?.append(data)
        }
    }

    deinit {
        pipe.fileHandleForReading.readabilityHandler = nil
    }

    private func append(_ data: Data) {
        lock.lock()
        defer { lock.unlock() }
        if data.count >= limit {
            tail = Data(data.suffix(limit))
        } else {
            let overflow = tail.count + data.count - limit
            if overflow > 0 { tail.removeFirst(overflow) }
            tail.append(data)
        }
    }

    func diagnostic() -> String {
        lock.lock()
        let data = tail
        lock.unlock()
        return String(decoding: data, as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

nonisolated final class MacApplicationServiceSupervisor: @unchecked Sendable {
    static let shared = MacApplicationServiceSupervisor()

    static func serviceArguments(
        databaseURL: URL,
        socketURL: URL,
        parentPID: Int32 = ProcessInfo.processInfo.processIdentifier,
        archiveIncompatibleStoreAt archiveURL: URL? = nil
    ) -> [String] {
        var arguments = [
            "--database", databaseURL.path,
            "--socket", socketURL.path,
            "--parent-pid", String(parentPID),
        ]
        if let archiveURL {
            arguments.append(contentsOf: ["--archive-incompatible-store", archiveURL.path])
        }
        return arguments
    }

    private let lock = NSLock()
    private var ownedProcesses: [String: Process] = [:]
    private var stderrBuffers: [String: MacApplicationServiceStderr] = [:]
    private var clientReferences: [String: Int] = [:]
    private final class WeakLifecycleGate {
        weak var value: NSRecursiveLock?
        init(_ value: NSRecursiveLock) { self.value = value }
    }
    private var lifecycleGates: [String: WeakLifecycleGate] = [:]
    private var activeStopOperations = 0

    var isStoppingServices: Bool {
        lock.lock()
        defer { lock.unlock() }
        return activeStopOperations > 0
    }

    private func lifecycleGate(for socketURL: URL) -> NSRecursiveLock {
        lock.lock()
        defer { lock.unlock() }
        if let existing = lifecycleGates[socketURL.path]?.value { return existing }
        lifecycleGates = lifecycleGates.filter { $0.value.value != nil }
        let gate = NSRecursiveLock()
        lifecycleGates[socketURL.path] = WeakLifecycleGate(gate)
        return gate
    }

    private init() {}

    func ensureRunning(
        databaseURL: URL,
        socketURL: URL,
        verifyExisting: Bool = false
    ) throws {
        _ = try ensureRunning(
            databaseURL: databaseURL,
            socketURL: socketURL,
            verifyExisting: verifyExisting,
            archiveIncompatibleStoreAt: nil
        )
    }

    func archiveIncompatibleStoreAndStart(
        databaseURL: URL,
        socketURL: URL
    ) throws -> URL? {
        let archiveRoot = databaseURL.deletingLastPathComponent()
            .appendingPathComponent("Recovered Stores", isDirectory: true)
        let archiveURL = archiveRoot.appendingPathComponent(
            "incompatible-\(Int(Date().timeIntervalSince1970))-\(UUID().uuidString.lowercased())",
            isDirectory: true
        )
        let archived = try ensureRunning(
            databaseURL: databaseURL,
            socketURL: socketURL,
            verifyExisting: true,
            archiveIncompatibleStoreAt: archiveURL
        )
        return archived && FileManager.default.fileExists(atPath: archiveURL.path)
            ? archiveURL
            : nil
    }

    private func ensureRunning(
        databaseURL: URL,
        socketURL: URL,
        verifyExisting: Bool,
        archiveIncompatibleStoreAt archiveURL: URL?
    ) throws -> Bool {
        let gate = lifecycleGate(for: socketURL)
        gate.lock()
        defer { gate.unlock() }
        lock.lock()
        guard activeStopOperations == 0 else {
            lock.unlock()
            throw CancellationError()
        }
        let existing = ownedProcesses[socketURL.path]
        lock.unlock()

        if !verifyExisting, let existing, existing.isRunning {
            return false
        }
        let client = MacApplicationServiceClient.unstarted(
            databaseURL: databaseURL,
            socketURL: socketURL
        )
        if (try? client.healthCheck()) != nil { return false }

        if let existing, existing.isRunning {
            try waitUntilHealthy(client: client, process: existing)
            return false
        }
        guard let executableURL = Self.executableURL() else {
            throw MacApplicationServiceError.executableUnavailable
        }
        try FileManager.default.createDirectory(
            at: databaseURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let process = Process()
        process.executableURL = executableURL
        process.arguments = Self.serviceArguments(
            databaseURL: databaseURL,
            socketURL: socketURL,
            archiveIncompatibleStoreAt: archiveURL
        )
        process.environment = MacAppEnvironment.shared.launchAgentEnvironment(
            projectURL: MacAppEnvironment.shared.scratchWorkspaceURL
        )
        #if DEBUG
        if ProcessInfo.processInfo.environment["NOLA_SERVICE_QUERY_TIMING"] == "1" {
            process.environment?["NOLA_SERVICE_QUERY_TIMING"] = "1"
        }
        #endif
        let stderr = MacApplicationServiceStderr()
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = stderr.pipe
        do {
            try process.run()
        } catch {
            throw MacApplicationServiceError.launchFailed(error.localizedDescription)
        }
        MacOwnedChildProcessRegistry.shared.register(process)
        lock.lock()
        ownedProcesses[socketURL.path] = process
        stderrBuffers[socketURL.path] = stderr
        lock.unlock()
        do {
            try waitUntilHealthy(client: client, process: process)
        } catch {
            // Only this launch belongs to this failure. Never leave a failed
            // child registered, and never wait for EOF from inherited pipes.
            lock.lock()
            ownedProcesses.removeValue(forKey: socketURL.path)
            stderrBuffers.removeValue(forKey: socketURL.path)
            lock.unlock()
            Self.stop(process)
            let detail = stderr.diagnostic()
            if !detail.isEmpty {
                if let classified = Self.classifiedStartupFailure(from: detail) {
                    throw classified
                }
                throw MacApplicationServiceError.launchFailed(detail)
            }
            throw error
        }
        return archiveURL != nil
    }

    static func classifiedStartupFailure(from diagnostic: String) -> MacApplicationServiceError? {
        for line in diagnostic.split(whereSeparator: { $0.isNewline }).reversed() {
            guard let data = String(line).data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  object["type"] as? String == "nola.service.startup_failure",
                  (object["protocol_version"] as? NSNumber)?.intValue == 1,
                  let code = object["code"] as? String,
                  let detail = object["detail"] as? String else { continue }
            switch code {
            case "incompatible_canonical_store":
                return .incompatibleStore(detail)
            case "service_startup_failed", "invalid_service_arguments":
                return .launchFailed(detail)
            default:
                return nil
            }
        }
        return nil
    }

    func retainClient(socketURL: URL) {
        let gate = lifecycleGate(for: socketURL)
        gate.lock()
        defer { gate.unlock() }
        lock.lock()
        clientReferences[socketURL.path, default: 0] += 1
        lock.unlock()
    }

    func releaseClient(socketURL: URL) {
        let gate = lifecycleGate(for: socketURL)
        gate.lock()
        defer { gate.unlock() }
        let process: Process?
        let stderr: MacApplicationServiceStderr?
        lock.lock()
        let key = socketURL.path
        guard let referenceCount = clientReferences[key] else {
            lock.unlock()
            return
        }
        let remaining = referenceCount - 1
        if remaining == 0 {
            clientReferences.removeValue(forKey: key)
            process = ownedProcesses.removeValue(forKey: key)
            stderr = stderrBuffers.removeValue(forKey: key)
        } else {
            clientReferences[key] = remaining
            process = nil
            stderr = nil
        }
        lock.unlock()
        if let process {
            withExtendedLifetime(stderr) { Self.stop(process) }
        }
    }

    func ownsProcessForTesting(socketURL: URL) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return ownedProcesses[socketURL.path]?.isRunning == true
    }

    func ownedProcessIdentifier(socketURL: URL) -> Int32? {
        lock.lock()
        defer { lock.unlock() }
        guard let process = ownedProcesses[socketURL.path], process.isRunning else { return nil }
        return process.processIdentifier
    }

    #if DEBUG
    func queryTimingsForTesting(databaseURL: URL) -> String {
        let socket = MacApplicationServiceClient.serviceSocketURL(for: databaseURL)
        lock.lock()
        let buffer = stderrBuffers[socket.path]
        lock.unlock()
        return (buffer?.diagnostic() ?? "").split(separator: "\n")
            .filter { $0.hasPrefix("nola_query_timing command=") }.joined(separator: "\n")
    }
    #endif

    func stopOwnedProcesses() {
        lock.lock()
        activeStopOperations += 1
        let keys = Set(ownedProcesses.keys).union(lifecycleGates.keys).sorted()
        let gates = keys.map { key -> NSRecursiveLock in
            if let gate = lifecycleGates[key]?.value { return gate }
            let gate = NSRecursiveLock()
            lifecycleGates[key] = WeakLifecycleGate(gate)
            return gate
        }
        lock.unlock()
        // Startup/release holds one gate only. Acquire the stop snapshot in a
        // stable order without holding the shared metadata lock during waits.
        for gate in gates { gate.lock() }
        defer {
            lock.lock()
            activeStopOperations -= 1
            lock.unlock()
            for gate in gates.reversed() { gate.unlock() }
        }
        lock.lock()
        let processes = Array(ownedProcesses.values)
        let buffers = Array(stderrBuffers.values)
        ownedProcesses.removeAll()
        stderrBuffers.removeAll()
        // Clients can outlive an explicit stop and later restart their service.
        // Keep their references so the last release still reclaims that writer.
        lock.unlock()
        withExtendedLifetime(buffers) {
            for process in processes {
                Self.stop(process)
            }
        }
    }

    private static func stop(_ process: Process) {
        MacOwnedChildProcessRegistry.shared.stop(process, gracePeriod: 2)
    }

    private func waitUntilHealthy(
        client: MacApplicationServiceClient,
        process: Process
    ) throws {
        let deadline = Date().addingTimeInterval(5)
        repeat {
            if (try? client.healthCheck()) != nil { return }
            if !process.isRunning {
                throw MacApplicationServiceError.launchFailed(
                    "service exited with status \(process.terminationStatus)"
                )
            }
            Thread.sleep(forTimeInterval: 0.02)
        } while Date() < deadline
        // Readiness failure is not authority to kill an existing writer.
        // The new-launch caller alone owns cleanup of its failed child.
        throw MacApplicationServiceError.launchFailed("startup timed out")
    }

    private static func executableURL() -> URL? {
        if let configured = ProcessInfo.processInfo.environment["NOLA_APPLICATION_SERVICE_EXECUTABLE"],
           FileManager.default.isExecutableFile(atPath: configured) {
            return URL(fileURLWithPath: configured)
        }
        if let bundled = Bundle.main.url(forAuxiliaryExecutable: "nola-service") {
            return bundled
        }
        let adjacent = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS/nola-service", isDirectory: false)
        return FileManager.default.isExecutableFile(atPath: adjacent.path) ? adjacent : nil
    }
}
