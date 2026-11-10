// SPDX-License-Identifier: MIT
// Supervises the out-of-process MLX runtime bundled with NolaMac.

import Darwin
import Foundation

extension Notification.Name {
    nonisolated static let nolaMacLocalModelRuntimeDidChange = Notification.Name(
        "nola.mac.localModelRuntimeDidChange"
    )
}

nonisolated enum MacLocalModelRuntimePhase: String, Sendable {
    case idle
    case startingService
    case loadingWeights
    case ready
    case failed
}

nonisolated struct MacLocalModelRuntimeStatus: Sendable, Equatable {
    let phase: MacLocalModelRuntimePhase
    let modelID: String?
    let displayName: String?
    let errorDescription: String?
    let startedAt: Date?
    let phaseStartedAt: Date?

    static let idle = MacLocalModelRuntimeStatus(
        phase: .idle,
        modelID: nil,
        displayName: nil,
        errorDescription: nil,
        startedAt: nil,
        phaseStartedAt: nil
    )

    var isStarting: Bool { phase == .startingService || phase == .loadingWeights }

    func elapsedSeconds(at date: Date = .now) -> Int {
        max(0, Int(date.timeIntervalSince(startedAt ?? date)))
    }
}

nonisolated enum MacLocalModelChatStartupError: LocalizedError {
    case unavailable(String)

    var errorDescription: String? {
        switch self {
        case .unavailable(let detail): return detail
        }
    }
}

nonisolated final class MacMLXOpenAIService: @unchecked Sendable {
    struct Endpoint: Sendable, Equatable {
        let baseURL: String
        let apiKey: String
        let modelID: String
        let contextWindowTokens: Int
        let profileRevision: Int64?

        init(baseURL: String, apiKey: String, modelID: String, contextWindowTokens: Int,
             profileRevision: Int64? = nil) {
            self.baseURL = baseURL
            self.apiKey = apiKey
            self.modelID = modelID
            self.contextWindowTokens = contextWindowTokens
            self.profileRevision = profileRevision
        }

        func nativeRoute() throws -> [String: Any] {
            guard let revision = profileRevision, revision > 0 else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            return ["adapter": "mlx", "adapter_contract_revision": 1, "model_id": modelID,
                    "profile_ref": ["profile_id": modelID, "admitted_revision": revision]]
        }
    }

    private struct RuntimeConfiguration: Encodable {
        let parentPID: Int32
        let token: String
        let modelID: String
        let displayName: String
        let installedDirectory: String?
        let estimatedRuntimeBytes: Int64
        let contextWindowTokens: Int
        let maxOutputTokens: Int
        let temperature: Double
        let topP: Double
        let topK: Int
        let repetitionPenalty: Double
        let presencePenalty: Double
        let frequencyPenalty: Double
    }

    private struct RuntimeEvent: Decodable {
        let phase: String
        let port: UInt16?
        let error: String?
    }

    static let shared = MacMLXOpenAIService()

    private let lock = NSLock()
    private var process: Process?
    private var endpointValue: Endpoint?
    private var activeApplicationPort: MacApplicationPort?
    private var runtimeStatus = MacLocalModelRuntimeStatus.idle
    private var startupTask: (id: UUID, modelID: String, task: Task<Endpoint, Error>)?
    private let runtimePersistenceQueue = DispatchQueue(label: "ai.nola.mlx-runtime-store")

    private init() {}

    var runningModelID: String? {
        lock.withLock { endpointValue?.modelID }
    }

    var status: MacLocalModelRuntimeStatus {
        lock.withLock { runtimeStatus }
    }

    func statusUpdates() -> AsyncStream<MacLocalModelRuntimeStatus> {
        AsyncStream { continuation in
            continuation.yield(status)
            let observer = NotificationCenter.default.addObserver(
                forName: .nolaMacLocalModelRuntimeDidChange,
                object: nil,
                queue: .main
            ) { [weak self] _ in
                guard let self else { return }
                continuation.yield(self.status)
            }
            continuation.onTermination = { _ in
                NotificationCenter.default.removeObserver(observer)
            }
        }
    }

    func isRunning(_ profile: MacLocalModelProfile) -> Bool {
        lock.withLock {
            endpointValue?.modelID == profile.id && process?.isRunning == true
        }
    }

    func start(_ profile: MacLocalModelProfile, expectedRevision: Int64? = nil,
               applicationPort: MacApplicationPort = .shared) async throws -> Endpoint {
        if let endpoint = try? endpoint(for: profile) {
            try validate(endpoint, applicationPort: applicationPort, expectedRevision: expectedRevision)
            return endpoint
        }
        let startup: (id: UUID, task: Task<Endpoint, Error>) = try lock.withLock {
            if let current = startupTask {
                guard current.modelID == profile.id else {
                    throw MacMLXSupervisorError.anotherModelStarting
                }
                return (current.id, current.task)
            }
            let id = UUID()
            let task = Task { [self] in try await startOnce(profile, expectedRevision: expectedRevision, applicationPort: applicationPort) }
            startupTask = (id, profile.id, task)
            return (id, task)
        }
        do {
            let endpoint = try await startup.task.value
            try validate(endpoint, applicationPort: applicationPort, expectedRevision: expectedRevision)
            clearStartupTask(id: startup.id)
            return endpoint
        } catch {
            clearStartupTask(id: startup.id)
            throw error
        }
    }

    func endpoint(for profile: MacLocalModelProfile) throws -> Endpoint {
        let running: Endpoint? = lock.withLock {
            guard endpointValue?.modelID == profile.id, process?.isRunning == true else { return nil }
            return endpointValue
        }
        guard let running else { throw MacMLXSupervisorError.modelNotStarted }
        return running
    }

    func stop() {
        let stopped: (Process?, Task<Endpoint, Error>?, String?, Date?) = lock.withLock {
            let value = (process, startupTask?.task, runtimeStatus.modelID, runtimeStatus.startedAt)
            process = nil
            endpointValue = nil
            startupTask = nil
            return value
        }
        stopped.1?.cancel()
        if let process = stopped.0 {
            process.terminationHandler = nil
            terminate(process)
        }
        if let modelID = stopped.2 {
            persistRuntime(
                phase: "stopped",
                modelID: modelID,
                endpoint: nil,
                processID: stopped.0.map { Int64($0.processIdentifier) },
                error: nil,
                startedAt: stopped.3,
                stoppedAt: .now
            )
        }
        setStatus(.idle)
    }

    func validate(_ endpoint: Endpoint, applicationPort: MacApplicationPort,
                  expectedRevision: Int64? = nil) throws {
        guard let revision = endpoint.profileRevision,
              expectedRevision == nil || expectedRevision == revision,
              let record = try applicationPort.modelProfile(endpoint.modelID),
              record.enabled, record.deletedAtMs == nil, record.provider == "local",
              record.runtime == "mlx", record.modelName == endpoint.modelID,
              record.revision == revision,
              lock.withLock({ endpointValue == endpoint && process?.isRunning == true
                  && activeApplicationPort?.databaseURL == applicationPort.databaseURL }) else {
            throw MacLocalAgentResumeError.credentialUnavailable
        }
    }

    private func startOnce(_ profile: MacLocalModelProfile, expectedRevision: Int64?,
                           applicationPort: MacApplicationPort) async throws -> Endpoint {
        let startedAt = Date()
        let (parameters, installedDirectory, profileRevision) = try await MainActor.run {
            let store = MacLocalModelConfigurationStore(applicationPort: applicationPort)
            // Existing canonical profiles are authority. Starting an execution
            // channel must not rewrite their revision, selection or parameters.
            if try applicationPort.modelProfile(profile.id) == nil {
                guard expectedRevision == nil else { throw MacLocalAgentResumeError.credentialUnavailable }
                try store.persist(profile: profile, selected: store.selectedModelID == profile.id)
            }
            guard let record = try applicationPort.modelProfile(profile.id),
                  record.enabled, record.deletedAtMs == nil, record.runtime == "mlx",
                  record.provider == "local", record.modelName == profile.id,
                  expectedRevision == nil || record.revision == expectedRevision,
                  let parameterData = record.parametersJSON.data(using: .utf8),
                  let sourceData = record.sourceJSON.data(using: .utf8),
                  let source = try JSONSerialization.jsonObject(with: sourceData) as? [String: Any],
                  source["modelID"] as? String == profile.id,
                  source["format"] as? String == profile.format,
                  source["installedDirectory"] as? String == profile.installedDirectory?.path else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            let parameters = try JSONDecoder().decode(ModelParameters.self, from: parameterData)
            let directory = profile.installedDirectory ?? ModelDownloader.ModelType(rawValue: profile.id)
                .map(store.modelDirectory(for:))
            return (parameters, directory, record.revision)
        }
        stopRunningProcessForReplacement()
        lock.withLock { activeApplicationPort = applicationPort }
        let contextWindowTokens = min(
            max(MacLocalModelProfile.minimumContextTokens, parameters.maxContextLength),
            profile.nativeContextTokens
        )
        let token = UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let configuration = RuntimeConfiguration(
            parentPID: ProcessInfo.processInfo.processIdentifier,
            token: token,
            modelID: profile.id,
            displayName: profile.displayName,
            installedDirectory: installedDirectory?.path,
            estimatedRuntimeBytes: profile.estimatedRuntimeBytesAt64K,
            contextWindowTokens: contextWindowTokens,
            maxOutputTokens: min(max(64, parameters.maxOutputTokens), 32_768),
            temperature: min(max(parameters.temperature, 0), 2),
            topP: min(max(parameters.topP, 0), 1),
            topK: max(1, LocalSamplingSettings(parameters: parameters).topK),
            repetitionPenalty: min(max(LocalSamplingSettings(parameters: parameters).repetitionPenalty, 0.8), 1.4),
            presencePenalty: min(max(parameters.presencePenalty, -2), 2),
            frequencyPenalty: min(max(parameters.frequencyPenalty, -2), 2)
        )

        setStatus(.init(
            phase: .startingService,
            modelID: profile.id,
            displayName: profile.displayName,
            errorDescription: nil,
            startedAt: startedAt,
            phaseStartedAt: startedAt
        ))

        let child = Process()
        child.executableURL = try helperExecutableURL()
        let standardInput = Pipe()
        let standardOutput = Pipe()
        child.standardInput = standardInput
        child.standardOutput = standardOutput
        child.standardError = FileHandle.nullDevice
        child.terminationHandler = { [weak self, weak child] process in
            guard let self, let child else { return }
            self.handleUnexpectedTermination(child, status: process.terminationStatus)
        }
        do {
            try child.run()
            lock.withLock { process = child }
            try standardInput.fileHandleForWriting.write(contentsOf: JSONEncoder().encode(configuration))
            try standardInput.fileHandleForWriting.close()

            for try await line in standardOutput.fileHandleForReading.bytes.lines {
                try Task.checkCancellation()
                guard let eventData = line.data(using: .utf8),
                      let event = try? JSONDecoder().decode(RuntimeEvent.self, from: eventData) else {
                    continue
                }
                switch event.phase {
                case "loading_weights":
                    setStatus(.init(
                        phase: .loadingWeights,
                        modelID: profile.id,
                        displayName: profile.displayName,
                        errorDescription: nil,
                        startedAt: startedAt,
                        phaseStartedAt: .now
                    ), processID: Int64(child.processIdentifier))
                case "ready":
                    guard let port = event.port else { throw MacMLXSupervisorError.invalidRuntimeEvent }
                    let endpoint = Endpoint(
                        baseURL: "http://127.0.0.1:\(port)/v1",
                        apiKey: token,
                        modelID: profile.id,
                        contextWindowTokens: contextWindowTokens,
                        profileRevision: profileRevision
                    )
                    lock.withLock { endpointValue = endpoint }
                    setStatus(.init(
                        phase: .ready,
                        modelID: profile.id,
                        displayName: profile.displayName,
                        errorDescription: nil,
                        startedAt: startedAt,
                        phaseStartedAt: .now
                    ), endpoint: endpoint, processID: Int64(child.processIdentifier))
                    return endpoint
                case "failed":
                    throw MacLocalModelChatStartupError.unavailable(
                        event.error ?? String(localized: "Unable to start the local model service.")
                    )
                default:
                    continue
                }
            }
            throw MacMLXSupervisorError.runtimeExited
        } catch {
            lock.withLock {
                if process === child { process = nil }
                endpointValue = nil
            }
            terminate(child)
            setStatus(.init(
                phase: .failed,
                modelID: profile.id,
                displayName: profile.displayName,
                errorDescription: error.localizedDescription,
                startedAt: startedAt,
                phaseStartedAt: .now
            ), processID: Int64(child.processIdentifier))
            throw error
        }
    }

    /// Stops a previously ready runtime without cancelling or clearing the
    /// startup task that is currently replacing it. Calling the public
    /// `stop()` here would let a concurrent `start()` create a second helper.
    private func stopRunningProcessForReplacement() {
        let stopped: (Process?, String?, Date?) = lock.withLock {
            let value = (process, runtimeStatus.modelID, runtimeStatus.startedAt)
            process = nil
            endpointValue = nil
            return value
        }
        if let process = stopped.0 {
            process.terminationHandler = nil
            terminate(process)
        }
        if let modelID = stopped.1 {
            persistRuntime(
                phase: "stopped",
                modelID: modelID,
                endpoint: nil,
                processID: stopped.0.map { Int64($0.processIdentifier) },
                error: nil,
                startedAt: stopped.2,
                stoppedAt: .now
            )
        }
        setStatus(.idle)
    }

    private func helperExecutableURL() throws -> URL {
        guard let executable = Bundle.main.executableURL else { throw MacMLXSupervisorError.helperMissing }
        let helper = executable.deletingLastPathComponent().appendingPathComponent("nola-mlx-runtime")
        guard FileManager.default.isExecutableFile(atPath: helper.path) else {
            throw MacMLXSupervisorError.helperMissing
        }
        return helper
    }

    private func clearStartupTask(id: UUID) {
        lock.withLock {
            guard startupTask?.id == id else { return }
            startupTask = nil
        }
    }

    private func handleUnexpectedTermination(_ child: Process, status: Int32) {
        let prior: MacLocalModelRuntimeStatus? = lock.withLock {
            guard process === child else { return nil }
            process = nil
            endpointValue = nil
            return runtimeStatus
        }
        guard let prior, prior.phase != .idle else { return }
        let detail = String(
            format: String(localized: "The local model service exited with status %d.", table: "ModelCenter"),
            status
        )
        setStatus(.init(
            phase: .failed,
            modelID: prior.modelID,
            displayName: prior.displayName,
            errorDescription: detail,
            startedAt: prior.startedAt,
            phaseStartedAt: .now
        ), processID: Int64(child.processIdentifier))
    }

    private func terminate(_ child: Process) {
        guard child.isRunning else { return }
        child.terminate()
        let pid = child.processIdentifier
        DispatchQueue.global(qos: .utility).async {
            let deadline = Date().addingTimeInterval(2)
            while child.isRunning, Date() < deadline { Thread.sleep(forTimeInterval: 0.02) }
            if child.isRunning { Darwin.kill(pid, SIGKILL) }
        }
    }

    private func setStatus(
        _ status: MacLocalModelRuntimeStatus,
        endpoint: Endpoint? = nil,
        processID: Int64? = nil
    ) {
        lock.withLock { runtimeStatus = status }
        if let modelID = status.modelID {
            switch status.phase {
            case .startingService, .loadingWeights:
                persistRuntime(phase: "starting", modelID: modelID, endpoint: nil, processID: processID, error: nil, startedAt: status.startedAt, stoppedAt: nil)
            case .ready:
                persistRuntime(phase: "ready", modelID: modelID, endpoint: endpoint, processID: processID, error: nil, startedAt: status.startedAt, stoppedAt: nil)
            case .failed:
                persistRuntime(phase: "failed", modelID: modelID, endpoint: nil, processID: processID, error: status.errorDescription, startedAt: status.startedAt, stoppedAt: .now)
            case .idle:
                break
            }
        }
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .nolaMacLocalModelRuntimeDidChange, object: nil)
            NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        }
    }

    private func persistRuntime(
        phase: String,
        modelID: String,
        endpoint: Endpoint?,
        processID: Int64?,
        error: String?,
        startedAt: Date?,
        stoppedAt: Date?
    ) {
        let endpointJSON = endpoint.flatMap { endpoint -> String? in
            let value: [String: Any] = [
                "base_url": endpoint.baseURL,
                "model_id": endpoint.modelID,
                "context_window_tokens": endpoint.contextWindowTokens,
            ]
            guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) else { return nil }
            return String(data: data, encoding: .utf8)
        }
        let hostID = "mac:\(MacCoreResourceAccess.localPrincipal())"
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        guard let applicationPort = lock.withLock({ activeApplicationPort }) else { return }
        runtimePersistenceQueue.async {
            do {
                try applicationPort.reportModelRuntime(MacCanonicalModelRuntime(
                    hostID: hostID,
                    profileID: modelID,
                    phase: phase,
                    endpointJSON: endpointJSON,
                    processID: processID,
                    error: error,
                    startedAtMs: startedAt.map { Int64($0.timeIntervalSince1970 * 1_000) },
                    stoppedAtMs: stoppedAt.map { Int64($0.timeIntervalSince1970 * 1_000) },
                    updatedAtMs: now
                ))
            } catch {
                Logger.shared.log(.error, "MacMLXOpenAIService", "Failed to persist model runtime state", metadata: [
                    "error": error.localizedDescription,
                    "model": modelID,
                ])
            }
        }
    }
}

nonisolated private enum MacMLXSupervisorError: LocalizedError {
    case helperMissing
    case modelNotStarted
    case anotherModelStarting
    case invalidRuntimeEvent
    case runtimeExited

    var errorDescription: String? {
        switch self {
        case .helperMissing:
            return String(localized: "The local model runtime is missing. Rebuild or reinstall Nola.", table: "ModelCenter")
        case .modelNotStarted:
            return String(localized: "Start the selected local model before using it in chat.", table: "ModelCenter")
        case .anotherModelStarting:
            return String(localized: "Another local model is running. Stop it before starting the selected model.", table: "ModelCenter")
        case .invalidRuntimeEvent:
            return String(localized: "The local model runtime returned an invalid status.", table: "ModelCenter")
        case .runtimeExited:
            return String(localized: "The local model runtime stopped before it was ready.", table: "ModelCenter")
        }
    }
}
