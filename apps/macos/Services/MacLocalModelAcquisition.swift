// SPDX-License-Identifier: Apache-2.0
import Foundation
import Combine

nonisolated struct MacModelAcquisitionEvent: Decodable, Sendable {
    struct EngineStatus: Decodable, Equatable, Sendable {
        let engine: String
        let ready: Bool
        let canInstall: Bool
        let guideUrl: String?
        let reason: String
    }
    let engineStatus: EngineStatus?
    struct Hardware: Decodable, Sendable {
        let cpu: String?
        let cores: Int?
        let memoryBytes: Int64?
        let gpu: String?
        let gpuMemoryBytes: Int64?
        let unifiedMemory: Bool
        let freeDiskBytes: Int64?
    }
    struct Candidate: Decodable, Sendable, Equatable {
        let repository: String
        let revision: String
        let downloadBytes: Int64
        let estimatedMemoryBytes: Int64
        let contextTokens: Int
        let nativeContextKnown: Bool?
        let nativeContextTokens: Int?
        let contextLimitReason: String?
        let variant: String?
        let device: String?
        let plan: [String: MacCanonicalJSONValue]

        var estimates128K: Bool { nativeContextKnown == true && contextTokens >= 131_072 }

        var contextDescription: String {
            let estimate = contextTokens.formatted()
            if estimates128K {
                return String(format: String(localized: "Estimated %@ tokens · meets the 128K target", table: "ModelCenter"), estimate)
            }
            if nativeContextKnown != true || contextLimitReason == "metadata_unknown" {
                return String(format: String(localized: "Planned %@ tokens · 128K support unverified", table: "ModelCenter"), estimate)
            }
            if contextLimitReason == "native_limit", let limit = nativeContextTokens, limit < 131_072 {
                return String(format: String(localized: "Estimated %@ tokens · native limit %@ (below 128K)", table: "ModelCenter"), estimate, limit.formatted())
            }
            return String(format: String(localized: "Estimated %@ tokens on this Mac · below 128K", table: "ModelCenter"), estimate)
        }
    }
    let hardware: Hardware?
    let reason: String?
    let recommendationSource: String?
    struct Installed: Decodable, Sendable { let id: String }
    let candidates: [Candidate]?
    let model: Installed?
    let downloadedBytes: Int64?
    let totalBytes: Int64?
    let removed: Bool?
}

nonisolated struct MacModelAcquisitionClient: Sendable {
    let executable: URL?
    let root: URL
    let environment: [String: String]
    let engineEnvironment: [String: String]

    init(executable: URL? = Bundle.main.url(forAuxiliaryExecutable: "nola-agent-host"),
         root: URL = MacLocalModelProfile.modelsRoot,
         environment: [String: String]? = nil) {
        self.executable = executable
        self.root = root
        let isolated = environment ?? MacAppEnvironment.shared.launchAgentEnvironment(projectURL: MacAppEnvironment.shared.scratchWorkspaceURL)
        self.environment = isolated
        if let environment {
            engineEnvironment = environment
        } else {
            var user = isolated
            user["HOME"] = FileManager.default.homeDirectoryForCurrentUser.path
            for key in ["NOLA_LLAMA_SERVER", "NOLA_VLLM_BIN", "TMPDIR", "LANG", "HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY"] {
                user[key] = ProcessInfo.processInfo.environment[key]
            }
            for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
                user[key] = ProcessInfo.processInfo.environment[key]
            }
            engineEnvironment = user
        }
    }

    func engineStatus(_ engine: MacLocalInferenceEngine, install: Bool = false) async throws -> MacModelAcquisitionEvent.EngineStatus {
        let result = try await run(arguments: [install ? "setup-engine" : "engine-status", engine.rawValue], environment: engineEnvironment)
        guard let status = result.engineStatus, status.engine == engine.rawValue else { throw CocoaError(.fileReadCorruptFile) }
        return status
    }

    func hardware() async throws -> MacModelAcquisitionEvent {
        try await run(arguments: ["hardware"])
    }

    func recommend(engine: MacLocalInferenceEngine, source: String, query: String) async throws -> MacModelAcquisitionEvent {
        try await run(arguments: ["recommend", engine.rawValue, source] + (query.isEmpty ? [] : [query]))
    }

    func confirm(engine: MacLocalInferenceEngine, source: String, candidate: MacModelAcquisitionEvent.Candidate,
                 event: @escaping @MainActor @Sendable (MacModelAcquisitionEvent) -> Void = { _ in }) async throws -> MacModelAcquisitionEvent {
        let plan = try JSONEncoder().encode(candidate.plan)
        return try await run(arguments: ["confirm", engine.rawValue, source, String(decoding: plan, as: UTF8.self)], event: event)
    }

    func remove(_ id: String) async throws {
        let result = try await run(arguments: ["remove", id], event: { _ in })
        guard result.removed == true else { throw CocoaError(.fileReadCorruptFile) }
    }

    private func run(arguments: [String], environment: [String: String]? = nil,
                     event: @escaping @MainActor @Sendable (MacModelAcquisitionEvent) -> Void = { _ in }) async throws -> MacModelAcquisitionEvent {
        guard let executable else { throw CocoaError(.executableNotLoadable) }
        let process = Process()
        process.executableURL = executable
        process.arguments = ["model", "acquire"] + arguments
        var env = environment ?? self.environment
        env["NOLA_MODELS_ROOT"] = root.path
        process.environment = env
        let session = MacLocalAgentProcessSession(process: process, inputPipe: Pipe(), outputPipe: Pipe(), errorPipe: Pipe())
        try Task.checkCancellation()
        try session.launch()
        try? session.input.close()
        let diagnostics = Task.detached {
            var output = Data()
            while let chunk = try? session.error.read(upToCount: 16_384), !chunk.isEmpty {
                output.append(chunk)
                if output.count > 8192 { output.removeFirst(output.count - 8192) }
            }
            return String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        defer { session.releaseResources() }
        return try await withTaskCancellationHandler {
            var final: MacModelAcquisitionEvent?
            do {
                for try await line in session.output.bytes.lines where !line.isEmpty {
                    let update = try JSONDecoder().decode(MacModelAcquisitionEvent.self, from: Data(line.utf8))
                    if update.model != nil || update.candidates != nil || update.hardware != nil || update.removed != nil || update.engineStatus != nil { final = update }
                    await event(update)
                }
            } catch {
                await session.shutdownAndDrain()
                throw error
            }
            let exit = await session.waitForExit()
            let detail = await diagnostics.value
            try Task.checkCancellation()
            if exit.status != 0 || exit.signaled {
                throw MacLocalAgentRuntimeError.processFailed(exit.status, detail)
            }
            guard let final else { throw CocoaError(.fileReadCorruptFile) }
            return final
        } onCancel: {
            Task { await session.shutdownAndDrain() }
        }
    }
}

@MainActor
final class MacLocalModelAcquisition: ObservableObject {
    enum EngineState: Equatable {
        case checking, installing
        case detected(MacModelAcquisitionEvent.EngineStatus)
        case failed(MacModelAcquisitionEvent.EngineStatus?, String)

        var ready: Bool {
            if case .detected(let status) = self { return status.ready }
            return false
        }
    }
    @Published private(set) var engines: [MacLocalInferenceEngine: EngineState] = [:]
    @Published private(set) var engineOperations: Set<MacLocalInferenceEngine> = []
    static let shared = MacLocalModelAcquisition()
    enum Phase: Equatable { case idle, searching, choosing, downloading(String), installed(String), failed(String) }
    @Published private(set) var phase: Phase = .idle
    @Published private(set) var candidates: [MacModelAcquisitionEvent.Candidate] = []
    @Published private(set) var hardware: MacModelAcquisitionEvent.Hardware?
    @Published private(set) var hardwareError: String?
    @Published private(set) var reason: String?
    @Published private(set) var recommendationSource: String?
    @Published var query = ""
    private var recommendedFor: (engine: MacLocalInferenceEngine, source: String, query: String)?
    @Published private(set) var received: Int64 = 0
    @Published private(set) var total: Int64 = 0
    private let client: MacModelAcquisitionClient
    private var task: Task<Void, Never>?
    private var generation = UUID()

    init(client: MacModelAcquisitionClient = .init()) { self.client = client }

    func checkEngine(_ engine: MacLocalInferenceEngine, install: Bool = false) {
        guard !engineOperations.contains(engine) else { return }
        let previous: MacModelAcquisitionEvent.EngineStatus?
        switch engines[engine] {
        case .checking, .installing: return
        case .detected(let status), .failed(.some(let status), _): previous = status
        default: previous = nil
        }
        guard !install || (previous?.canInstall == true && previous?.ready == false) else { return }
        engineOperations.insert(engine)
        if install { engines[engine] = .installing }
        else if previous == nil { engines[engine] = .checking }
        Task {
            defer { engineOperations.remove(engine) }
            do {
                let status = try await client.engineStatus(engine, install: install)
                engines[engine] = .detected(status)
            } catch {
                engines[engine] = .failed(previous, error.localizedDescription)
            }
        }
    }

    var busy: Bool {
        switch phase { case .searching, .downloading: true; default: false }
    }
    var downloading: Bool {
        if case .downloading = phase { return true }
        return false
    }

    func loadHardware() async {
        do {
            let snapshot = try await client.hardware()
            guard let device = snapshot.hardware else { throw CocoaError(.fileReadCorruptFile) }
            hardware = device
            hardwareError = nil
        } catch {
            guard !Task.isCancelled else { return }
            hardwareError = error.localizedDescription
        }
    }

    func reset() {
        generation = UUID()
        task?.cancel()
        task = nil
        phase = .idle
        received = 0; total = 0
        candidates = []
        reason = nil
        recommendationSource = nil
        recommendedFor = nil
    }

    func updateInputs(engine: MacLocalInferenceEngine, source: String) {
        guard !downloading else { return }
        let name = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if let request = recommendedFor, (request.engine, request.source, request.query) == (engine, source, name) { return }
        reset()
    }

    func recommendOnOpen(engine: MacLocalInferenceEngine, source: String) {
        guard hardware != nil else { return }
        switch phase {
        case .idle, .installed: break
        default: return
        }
        recommend(engine: engine, source: source, name: query)
    }

    func recommend(engine: MacLocalInferenceEngine, source: String, name: String) {
        let query = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !downloading else { return }
        if let request = recommendedFor, phase == .searching,
           (request.engine, request.source, request.query) == (engine, source, query) { return }
        reset()
        phase = .searching
        recommendedFor = (engine, source, query)
        let id = generation
        task = Task {
            do {
                let response = try await client.recommend(engine: engine, source: source, query: query)
                guard generation == id else { return }
                guard let items = response.candidates else { throw CocoaError(.fileReadCorruptFile) }
                candidates = items
                hardware = response.hardware ?? hardware
                reason = response.reason
                recommendationSource = response.recommendationSource
                phase = .choosing
            } catch {
                if generation == id { phase = .failed(error.localizedDescription) }
            }
        }
    }

    func confirm(_ candidate: MacModelAcquisitionEvent.Candidate) {
        guard phase == .choosing, candidates.contains(candidate), let request = recommendedFor else { return }
        generation = UUID()
        let id = generation
        received = 0; total = 0
        phase = .downloading(candidate.repository)
        task = Task {
            do {
                let response = try await client.confirm(engine: request.engine, source: request.source, candidate: candidate) { update in
                    guard self.generation == id else { return }
                    if let done = update.downloadedBytes, let size = update.totalBytes {
                        self.received = done
                        self.total = size
                    }
                }
                guard generation == id else { return }
                guard let installed = response.model?.id else { throw CocoaError(.fileReadCorruptFile) }
                MacLocalModelConfigurationStore.shared.invalidateReadinessCache()
                phase = .installed(installed)
                NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            } catch {
                if generation == id { phase = .failed(error.localizedDescription) }
            }
        }
    }
}
