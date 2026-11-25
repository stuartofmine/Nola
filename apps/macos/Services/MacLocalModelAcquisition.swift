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

        var displayTitle: String {
            let repoName = repository.split(separator: "/").last.map(String.init) ?? repository
            let suffixes = ["-MLX-4bit", "-MLX-8bit", "-MLX-6bit", "-MLX", "-mlx", "-GGUF", "-gguf", "-4bit", "-8bit", "-bf16", "-fp16"]
            var cleaned = repoName
            for suffix in suffixes where cleaned.hasSuffix(suffix) {
                cleaned = String(cleaned.dropLast(suffix.count))
            }
            return cleaned.replacingOccurrences(of: "-", with: " ").trimmingCharacters(in: .whitespaces)
        }

        func fitBadge(deviceMemoryBytes: Int64?) -> (label: String, isComfortable: Bool) {
            if let total = deviceMemoryBytes, total > 0 {
                let ratio = Double(estimatedMemoryBytes) / Double(total)
                if ratio <= 0.65 {
                    return (String(localized: "流畅运行", table: "ModelCenter"), true)
                }
                if ratio <= 0.90 {
                    return (String(localized: "适中运行", table: "ModelCenter"), true)
                }
                return (String(localized: "内存紧张", table: "ModelCenter"), false)
            }
            return estimatedMemoryBytes <= 6_000_000_000
                ? (String(localized: "流畅运行", table: "ModelCenter"), true)
                : (String(localized: "适中运行", table: "ModelCenter"), true)
        }

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

    init(executable: URL? = Bundle.main.url(forAuxiliaryExecutable: "nola"),
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
    @Published private(set) var downloadSpeedBytesPerSecond: Double = 0
    @Published private(set) var estimatedRemainingSeconds: Double?
    @Published private(set) var lastAttemptedCandidate: MacModelAcquisitionEvent.Candidate?
    private struct RecommendationCacheKey: Hashable {
        let engine: MacLocalInferenceEngine
        let source: String
        let query: String
    }
    private struct CachedRecommendation {
        let candidates: [MacModelAcquisitionEvent.Candidate]
        let reason: String?
        let recommendationSource: String?
        let hardware: MacModelAcquisitionEvent.Hardware?
        let cachedAt: Date
    }
    private var recommendationCache: [RecommendationCacheKey: CachedRecommendation] = [:]
    private var engineCheckedAt: [MacLocalInferenceEngine: Date] = [:]
    private let recommendationCacheTTL: TimeInterval = 1800
    private let engineCacheTTL: TimeInterval = 300
    private let client: MacModelAcquisitionClient
    private var task: Task<Void, Never>?
    private var generation = UUID()

    static var defaultDownloadSource: String {
        defaultDownloadSource(timeZone: .current, locale: .current)
    }

    static func defaultDownloadSource(timeZone: TimeZone, locale: Locale) -> String {
        let chinaTimeZones: Set<String> = [
            "Asia/Shanghai", "Asia/Chongqing", "Asia/Harbin", "Asia/Urumqi",
            "Asia/Hong_Kong", "Asia/Macau", "Asia/Taipei", "PRC"
        ]
        let isUTC8 = timeZone.secondsFromGMT() == 8 * 3600
        let isChineseLocale = locale.language.languageCode?.identifier == "zh"
            || locale.region?.identifier == "CN"
            || locale.identifier.lowercased().hasPrefix("zh")
        if isUTC8 && (chinaTimeZones.contains(timeZone.identifier) || isChineseLocale) {
            return "modelscope"
        }
        if locale.region?.identifier == "CN" || locale.identifier.lowercased().hasPrefix("zh-hans") || locale.identifier.lowercased().hasPrefix("zh_hans") {
            return "modelscope"
        }
        return "huggingface"
    }

    static func parseDirectInput(_ raw: String) -> (source: String?, query: String) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return (nil, "") }
        let normalizedURLString = trimmed.contains("://") ? trimmed : (
            (trimmed.hasPrefix("huggingface.co/") || trimmed.hasPrefix("www.huggingface.co/")
             || trimmed.hasPrefix("hf-mirror.com/") || trimmed.hasPrefix("modelscope.cn/")
             || trimmed.hasPrefix("www.modelscope.cn/")) ? "https://" + trimmed : trimmed
        )
        if let url = URL(string: normalizedURLString), let host = url.host?.lowercased() {
            let parts = url.pathComponents.filter { $0 != "/" && !$0.isEmpty }
            if host == "huggingface.co" || host == "www.huggingface.co" || host == "hf-mirror.com" || host == "www.hf-mirror.com" {
                let repoParts = parts.first == "models" ? Array(parts.dropFirst()) : parts
                if repoParts.count >= 2 {
                    return ("huggingface", "\(repoParts[0])/\(repoParts[1])")
                }
            } else if host == "modelscope.cn" || host == "www.modelscope.cn" {
                let repoParts = parts.first == "models" ? Array(parts.dropFirst()) : parts
                if repoParts.count >= 2 {
                    return ("modelscope", "\(repoParts[0])/\(repoParts[1])")
                }
            }
        }
        return (nil, trimmed)
    }

    static func formatGigabytes(_ bytes: Int64) -> String {
        guard bytes > 0 else { return "0 GB" }
        let gb = Double(bytes) / 1_000_000_000.0
        if gb < 0.1 {
            let mb = Double(bytes) / 1_000_000.0
            return "\(Int(mb.rounded())) MB"
        }
        return gb.formatted(.number.precision(.fractionLength(0...1))) + " GB"
    }

    static func formatETA(_ seconds: Double?) -> String? {
        guard let seconds, seconds.isFinite, seconds >= 1 else { return nil }
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .abbreviated
        formatter.allowedUnits = seconds.rounded() >= 3600 ? [.hour, .minute] : [.minute, .second]
        formatter.zeroFormattingBehavior = .dropAll
        return formatter.string(from: seconds.rounded())
    }

    init(client: MacModelAcquisitionClient = .init()) { self.client = client }

    func checkEngine(_ engine: MacLocalInferenceEngine, install: Bool = false, force: Bool = false) {
        guard !engineOperations.contains(engine) else { return }
        let previous: MacModelAcquisitionEvent.EngineStatus?
        switch engines[engine] {
        case .checking, .installing: return
        case .detected(let status):
            previous = status
            if !install && !force,
               let checkedAt = engineCheckedAt[engine],
               Date().timeIntervalSince(checkedAt) < engineCacheTTL {
                return
            }
        case .failed(.some(let status), _):
            previous = status
        default:
            previous = nil
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
                engineCheckedAt[engine] = Date()
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

    func loadHardware(force: Bool = false) async {
        if !force && hardware != nil { return }
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
        downloadSpeedBytesPerSecond = 0
        estimatedRemainingSeconds = nil
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

    func recommend(engine: MacLocalInferenceEngine, source: String, name: String, force: Bool = false) {
        let query = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !downloading else { return }
        if !force, let request = recommendedFor, phase == .searching,
           (request.engine, request.source, request.query) == (engine, source, query) { return }
        let cacheKey = RecommendationCacheKey(engine: engine, source: source, query: query)
        if !force, let cached = recommendationCache[cacheKey],
           Date().timeIntervalSince(cached.cachedAt) < recommendationCacheTTL {
            generation = UUID()
            task?.cancel()
            task = nil
            recommendedFor = (engine, source, query)
            candidates = cached.candidates
            hardware = cached.hardware ?? hardware
            reason = cached.reason
            recommendationSource = cached.recommendationSource
            phase = .choosing
            return
        }
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
                recommendationCache[cacheKey] = CachedRecommendation(
                    candidates: items,
                    reason: response.reason,
                    recommendationSource: response.recommendationSource,
                    hardware: response.hardware,
                    cachedAt: Date()
                )
                phase = .choosing
            } catch {
                if generation == id { phase = .failed(error.localizedDescription) }
            }
        }
    }

    func confirm(_ candidate: MacModelAcquisitionEvent.Candidate) {
        guard (phase == .choosing || { if case .failed = phase { return true }; return false }()),
              candidates.contains(candidate),
              let request = recommendedFor else { return }
        generation = UUID()
        let id = generation
        received = 0; total = 0
        downloadSpeedBytesPerSecond = 0
        estimatedRemainingSeconds = nil
        lastAttemptedCandidate = candidate
        phase = .downloading(candidate.repository)
        let startedAt = Date()
        task = Task {
            do {
                let response = try await client.confirm(engine: request.engine, source: request.source, candidate: candidate) { update in
                    guard self.generation == id else { return }
                    if let done = update.downloadedBytes, let size = update.totalBytes {
                        self.received = done
                        self.total = size
                        let elapsed = Date().timeIntervalSince(startedAt)
                        if elapsed >= 0.25 && done > 0 {
                            let speed = Double(done) / elapsed
                            self.downloadSpeedBytesPerSecond = speed
                            if size > done && speed > 0 {
                                self.estimatedRemainingSeconds = Double(size - done) / speed
                            } else {
                                self.estimatedRemainingSeconds = nil
                            }
                        }
                    }
                }
                guard generation == id else { return }
                guard let installed = response.model?.id else { throw CocoaError(.fileReadCorruptFile) }
                MacLocalModelConfigurationStore.shared.invalidateReadinessCache()
                lastAttemptedCandidate = nil
                downloadSpeedBytesPerSecond = 0
                estimatedRemainingSeconds = nil
                phase = .installed(installed)
                NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
            } catch {
                if generation == id {
                    downloadSpeedBytesPerSecond = 0
                    estimatedRemainingSeconds = nil
                    phase = .failed(error.localizedDescription)
                }
            }
        }
    }

    func retryLastDownload() {
        guard let candidate = lastAttemptedCandidate else { return }
        confirm(candidate)
    }
}
