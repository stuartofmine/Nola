// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ServiceContainer logic in the iOS app.
// - Interfaces:
//   - canonicalSourceId
//   - DataSourceFetchSummary
//   - URLFetchPolicy
//   - ServiceContainer
// - APIs:
//   - none
//
//
//  ServiceContainer.swift
//  Nola
//
//  Simple DI container for app-wide services.
//

import Foundation
import Combine
@preconcurrency import UserNotifications

nonisolated enum IOSContainerCloudConfigurationFailure: String, Error {
    case profile
    case credential
    case parameters
    case providerValidation
}

@MainActor enum IOSContainerCloudConfiguration {
    static func prepare(selected: ModelInfo?,
        resolve: @MainActor (ModelInfo) throws -> IOSResolvedCloudRunProfile = ModelSelectionManager.resolvedCloudProfileForRun
    ) throws -> OpenAIProvider.Config {
        guard let selected, !selected.isLocal, !selected.isWebBackend else {
            throw IOSContainerCloudConfigurationFailure.profile
        }
        let resolved: IOSResolvedCloudRunProfile
        do { resolved = try resolve(selected) }
        catch is IOSCloudRunCredentialResolutionError { throw IOSContainerCloudConfigurationFailure.credential }
        catch is IOSCloudCredentialError { throw IOSContainerCloudConfigurationFailure.credential }
        catch { throw IOSContainerCloudConfigurationFailure.profile }
        let parameters: ModelParameters
        do { parameters = try ModelParameters.decodeProfile(resolved.parametersJSON) }
        catch { throw IOSContainerCloudConfigurationFailure.parameters }
        do {
            var configuration = try IOSConversationRunConfiguration.directCloudProviderConfig(model: resolved.model, parameters: parameters,
                reasoningSelection: resolved.reasoningSelection, reasoningTier: resolved.reasoningTier)
            configuration.reasoningSummarySupported = resolved.reasoningSummarySupported
            return configuration
        }
        catch { throw IOSContainerCloudConfigurationFailure.providerValidation }
    }
}

/// Normalize a source identifier so iOS-side UUIDs (uppercase, with hyphens)
/// compare equal to backend-stored ids (hex, no hyphens, lowercase).
@inline(__always)
fileprivate func canonicalSourceId(_ raw: String?) -> String {
    guard let raw, !raw.isEmpty else { return "" }
    return raw.replacingOccurrences(of: "-", with: "").lowercased()
}

private struct IOSPromptTemplatePersistence: PromptTemplatePersistence {
    let store: IOSApplicationRuntime?

    func load() throws -> PromptTemplateSnapshot? {
        guard let store else { throw PromptTemplateReadError.unavailable }
        guard let setting = try store.productSetting(key: "prompt.templates") else { return nil }
        return try JSONDecoder().decode(PromptTemplateSnapshot.self, from: Data(setting.valueJSON.utf8))
    }

    func save(_ snapshot: PromptTemplateSnapshot) throws {
        guard let store else { throw PromptTemplateReadError.unavailable }
        let data = try JSONEncoder().encode(snapshot)
        guard let valueJSON = String(data: data, encoding: .utf8) else { throw PromptTemplateReadError.unavailable }
        _ = try store.putProductSetting(key: "prompt.templates", valueJSON: valueJSON,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
    }
}

public struct DataSourceFetchSummary {
    public let events: [RawEvent]
    public let perSourceCounts: [String: Int]
    public let failedSources: [String: String]

    public var metadata: [String: String] {
        var meta: [String: String] = [:]
        for (key, value) in perSourceCounts {
            meta[key] = "\(value)"
        }
        if !failedSources.isEmpty {
            meta["errors"] = failedSources.map { "\($0.key):\($0.value)" }.joined(separator: ", ")
        }
        meta["totalCandidates"] = "\(events.count)"
        return meta
    }
}

public enum URLFetchPolicy: Equatable {
    case none
    case allEnabled
    case scheduledOnly
}

@MainActor
public final class ServiceContainer: ObservableObject {
    private static var isRunningUnitTests: Bool {
        ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
    }

    private static func resolveApplicationRuntime(
        _ suppliedRuntime: IOSApplicationRuntime?
    ) -> IOSApplicationRuntime {
        if let suppliedRuntime { return suppliedRuntime }
        if isRunningUnitTests {
            let databaseURL = FileManager.default.temporaryDirectory
                .appendingPathComponent("nola-ios-xctest-container-\(UUID().uuidString)", isDirectory: true)
                .appendingPathComponent("nola-product.db", isDirectory: false)
            do {
                return try IOSApplicationRuntime(databaseURL: databaseURL)
            } catch {
                fatalError("Unable to create an isolated iOS XCTest application runtime: \(error)")
            }
        }
        do {
            return try IOSApplicationRuntime.requireShared()
        } catch {
            // The product App opens the runtime before constructing services
            // and displays this error. Retain diagnostics for direct test/preview callers.
            fatalError("Unable to open the canonical iOS application runtime: \(error.localizedDescription)")
        }
    }

    @Published var llmClient: LLMClient
    @Published var logger = Logger()
    @Published var medicationService: MedicationService?
    @Published var healthDataSourceEnabled: Bool = false

    private let mainActorWatchdog = MainActorWatchdog(thresholdSeconds: 8)
    
    // Chat management
    @Published var chatManager: ChatManager!
    // Deep-link target from notification taps (dedupeKey or eventId)
    @Published public var pendingNotificationDedupeKey: String? = nil
    // In-memory cache of recently loaded ProcessedEvent objects for quick matching
    @Published public var recentProcessedEvents: [ProcessedEvent] = []
    // Advanced services controlled by local feature gates
    @Published var urlFetcher: URLFetcherService?
    @Published var promptTemplates: PromptTemplateService?
    
    // Model downloader for local AI models
    let modelDownloader: ModelDownloader
    
    // Data management
    let applicationRuntime: IOSApplicationRuntime
    let dataManager: DataManager
    let localJSONStorage: LocalJSONStorage?
    let summaryStorage: SummaryStorageService
    let chatMemoryProfileStore: ChatMemoryProfileStore

    // MLX Local Model Configuration
    @Published var useLocalModel: Bool = false
    @Published var localModelStatus: LocalModelStatus? = nil
    
    enum LocalModelStatus {
        case checking
        case available
        case notAvailable
        case downloadRequired
    }
    
    // Data sources & pipeline
    var dataSources: [DataSource]
    public internal(set) var healthDataSource: HealthDataSource? = nil
    let preprocessor: EventPreprocessor
    let rulesEngine: RulesEngine
    let notificationService: NotificationService
    // Unified services (new)
    public var unifiedStorage: UnifiedStorageService?
    public var dataTransformer: DataTransformationService?
    public let unifiedNotificationService: UnifiedNotificationService
    public var candidateQueue: CandidateQueue?
    // Notification persistence
    public var notificationStore: NotificationStore?
    public var notificationManager: NotificationManager?
    // Conversation & Context
    public var conversationStore: ConversationStore?
    public var contextManager: ContextManager?
    lazy var scheduler: Scheduler = {
        Scheduler(
            container: self,
            preprocessor: preprocessor,
            rules: rulesEngine
        )
    }()

    public func fetchEventsFromDataSources(
        pastWindow: TimeInterval = 7 * 24 * 3600,
        futureWindow: TimeInterval = 7 * 24 * 3600,
        urlFetchPolicy: URLFetchPolicy = .allEnabled
    ) async -> DataSourceFetchSummary {
        let now = Date()
        let urlRange = DateInterval(start: now.addingTimeInterval(-pastWindow), end: now.addingTimeInterval(futureWindow))
        let snapshot = dataSources
        let urlFetcherSnapshot = (urlFetchPolicy == .none) ? nil : urlFetcher
        var allEvents: [RawEvent] = []
        var perSourceCounts: [String: Int] = [:]
        var failedSources: [String: String] = [:]

        // Each read settles through Application before its events enter this summary.
        // Aggregation must not write source configuration or replay stale UI snapshots.
        if let urlFetcherSnapshot {
            let urlSourcesSnapshot: [URLDataSource] = {
                switch urlFetchPolicy {
                case .none:
                    return []
                case .allEnabled:
                    return urlFetcherSnapshot.sources.filter { $0.enabled }
                case .scheduledOnly:
                    return urlFetcherSnapshot.sources.filter { urlFetcherSnapshot.shouldFetchNow($0) }
                }
            }()

            if !urlSourcesSnapshot.isEmpty {
                await withTaskGroup(of: (URLDataSource, [RawEvent]?, String?).self) { group in
                    for src in urlSourcesSnapshot {
                        group.addTask {
                            do {
                                let evs = try await urlFetcherSnapshot.fetchEvents(from: src)
                                return (src, evs, nil)
                            } catch {
                                return (src, nil, error.localizedDescription)
                            }
                        }
                    }

                    for await (src, evs, err) in group {
                        if let evs {
                            let filtered = evs.filter { ev in
                                guard let d = ev.date else { return true }
                                return urlRange.contains(d)
                            }
                            allEvents.append(contentsOf: filtered)
                            perSourceCounts["URL:\(src.name)"] = filtered.count
                        } else {
                            perSourceCounts["URL:\(src.name)"] = 0
                            if let err {
                                failedSources["URL:\(src.name)"] = err
                            }
                        }
                    }
                }

            }
        }

        await withTaskGroup(of: (String, [RawEvent], String?).self) { group in

            for ds in snapshot {
                group.addTask {
                    let name = String(describing: type(of: ds))
                    let dsRange = Self.dateRange(for: ds, anchor: now, pastWindow: pastWindow, futureWindow: futureWindow)
                    do {
                        let events = try await ds.fetchEvents(in: dsRange)
                        return (name, events, nil)
                    } catch let dataSourceError as DataSourceError {
                        return (name, [], dataSourceError.errorDescription ?? dataSourceError.localizedDescription)
                    } catch {
                        return (name, [], error.localizedDescription)
                    }
                }
            }

            for await (name, events, error) in group {
                allEvents.append(contentsOf: events)
                perSourceCounts[name] = events.count
                if let error = error {
                    failedSources[name] = error
                }
            }
        }

        let summary = DataSourceFetchSummary(events: allEvents, perSourceCounts: perSourceCounts, failedSources: failedSources)
        logger.log(.info, "ServiceContainer", "Fetched data sources for summary", metadata: summary.metadata)
        return summary
    }

    nonisolated private static func dateRange(for source: DataSource, anchor: Date, pastWindow: TimeInterval, futureWindow: TimeInterval) -> DateInterval {
        if source is HealthDataSource {
            return DateInterval(start: anchor.addingTimeInterval(-pastWindow), end: anchor)
        }

        if source is ContactsDataSource || source is CalendarsDataSource || source is RemindersDataSource {
            return DateInterval(start: anchor, end: anchor.addingTimeInterval(futureWindow))
        }

        return DateInterval(start: anchor.addingTimeInterval(-pastWindow), end: anchor.addingTimeInterval(futureWindow))
    }

    // Config
    @Published var baseURLString: String
    @Published var apiKey: String
    @Published var model: String
    @Published var configSource: ConfigSource = .`default`
    @Published var testStatus: TestStatus = .idle

    enum ConfigSource: String { case keychain, `default` }
    enum TestStatus: Equatable {
        case idle
        case testing
        case ok(modelCount: Int)
        case okChat // chat/completions fallback succeeded
        case unauthorized // 401
        case rateLimited // 429
        case network(String)
        case server(Int)
        case invalidResponse
        case echoTesting
        case echoOk
        case echoMismatch
    }

    // Track last applied provider config to avoid redundant reconfiguration
    private var lastConfigSignature: String? = nil
    private var lastCloudConfiguration: OpenAIProvider.Config?
    /// The workspace's explicit selection owns provider refreshes. The persisted
    /// default only seeds configuration before any conversation selects a model.
    private(set) var configurationModel: ModelInfo?
    private var isReconfiguring: Bool = false

    init(
        applicationRuntime suppliedApplicationRuntime: IOSApplicationRuntime? = nil,
        chatMemoryProfileStore: ChatMemoryProfileStore? = nil
    ) {
        let isRunningUnitTests = Self.isRunningUnitTests
        let applicationRuntime = Self.resolveApplicationRuntime(suppliedApplicationRuntime)
        self.applicationRuntime = applicationRuntime
        if !isRunningUnitTests {
            mainActorWatchdog.setStage("init:start")
            mainActorWatchdog.start(logger: Logger.shared)
        }

        // Initialize logger first
        let logger = Logger()
        
        // Initialize model downloader
        self.modelDownloader = ModelDownloader(logger: logger)
        
        // Initialize data sources (health data source is optional and controlled by user)
        self.dataSources = [
            ContactsDataSource(),
            CalendarsDataSource(),
            RemindersDataSource()
        ]
        self.healthDataSource = nil
        self.preprocessor = EventPreprocessor()
        self.rulesEngine = RulesEngine()
        
        // Initialize services
        let notificationService = NotificationService(logger: logger)
        self.notificationService = notificationService
        // Initialize unified services
        self.dataTransformer = DefaultDataTransformationService()
        self.unifiedNotificationService = DefaultUnifiedNotificationService(center: UNUserNotificationCenter.current())
        do {
            self.candidateQueue = try CandidateQueue()
        } catch {
            logger.log(.error, "ServiceContainer", "Failed to initialize CandidateQueue", metadata: ["error": error.localizedDescription])
            self.candidateQueue = nil
        }
        do {
            self.unifiedStorage = try FileBasedUnifiedStorage()
        } catch {
            self.unifiedStorage = nil
            logger.log(.error, "ServiceContainer", "Failed to initialize FileBasedUnifiedStorage", metadata: ["error": error.localizedDescription])
        }

        // Initialize LocalJSONStorage (file-backed storage used by DataManager export/cleanup)
        let localStorage: LocalJSONStorage?
        do {
            localStorage = try LocalJSONStorage()
        } catch {
            localStorage = nil
            logger.log(.error, "ServiceContainer", "Failed to initialize LocalJSONStorage", metadata: ["error": error.localizedDescription])
        }
        self.localJSONStorage = localStorage

        // Initialize DataManager after LocalJSONStorage so exports/cleanup can include file-backed data.
        self.dataManager = DataManager(logger: logger, localJSONStorage: localStorage)

        // Initialize notification persistence store (file-backed preferred)
        do {
            let f = try FileBasedNotificationStore()
            self.notificationStore = f
        } catch {
            logger.log(.warn, "ServiceContainer", "File-based notification store unavailable, falling back to in-memory", metadata: ["error": error.localizedDescription])
            self.notificationStore = InMemoryNotificationStore()
        }
        // Initialize notification manager backed by the store
        if let store = self.notificationStore {
            self.notificationManager = NotificationManager(store: store)
        } else {
            self.notificationManager = NotificationManager()
        }
        
        // Initialize SummaryStorageService
        self.summaryStorage = SummaryStorageService()
        self.chatMemoryProfileStore = chatMemoryProfileStore
            ?? CanonicalChatMemoryProfileStore(applicationRuntime: applicationRuntime)

        // Initialize ConversationStore & ContextManager
        do {
            let convStore = try LocalConversationStore(applicationRuntime: applicationRuntime)
            self.conversationStore = convStore
            self.contextManager = DefaultContextManager(store: convStore, profileStore: self.chatMemoryProfileStore)
            self.contextManager?.localJSONStorage = self.localJSONStorage
        } catch {
            self.conversationStore = nil
            self.contextManager = nil
            logger.log(.error, "ServiceContainer", "Failed to initialize ConversationStore/ContextManager", metadata: ["error": error.localizedDescription])
        }
        
        // Initialize advanced services for the open-source build.
        self.urlFetcher = isRunningUnitTests ? nil : URLFetcherService(
            logger: logger,
            maxURLs: ClientConfig.shared.effectiveURLLimit(),
            maxHistoryEntries: 90,
            userDefaults: UserDefaults.standard,
            applicationRuntime: applicationRuntime
        )
        self.promptTemplates = isRunningUnitTests ? nil : PromptTemplateService(
            logger: logger,
            persistence: IOSPromptTemplatePersistence(store: applicationRuntime)
        )
        self.medicationService = isRunningUnitTests ? nil : MedicationService(logger: logger)
        
        // Keep startup lightweight: do not initialize MLX until local mode is explicitly selected.
        let defaultProvider: LLMClient = PlaceholderLocalProvider.shared
        let modelService = ModelService(provider: defaultProvider)
        self.llmClient = modelService

        // Wire LLM client into context manager so it can perform summarization (defer until after init finishes)
        // Will be set in the scheduled main-queue block below to avoid using 'self' before initialization completes.
        
        self.baseURLString = "https://api-inference.modelscope.cn/v1"
        self.apiKey = ""
        self.model = "deepseek-ai/DeepSeek-V3.2"
        if !isRunningUnitTests {
            loadInitialConfig()
            reconfigureIfPossible()
        }

        // Ensure Scheduler is initialized during app launch (including background launches).
        // This sets Scheduler.sharedHandler so BGTaskScheduler callbacks can execute reliably
        // even if no SwiftUI view has appeared yet.
        if !isRunningUnitTests {
            _ = self.scheduler
        }

        // Initialize ChatManager after all properties are set. A real UI E2E reset must
        // start from an empty in-memory projection as well as an empty store; otherwise
        // ChatManager can restore the previous run before NolaApp removes its records.
        let resetsRealChatFixture = ProcessInfo.processInfo.arguments.contains("-ui-testing-real-chat-host")
            && ProcessInfo.processInfo.environment["NOLA_REAL_IOS_UI_RESET"] == "1"
        self.chatManager = ChatManager(
            container: self,
            restorePersistedState: !isRunningUnitTests && !resetsRealChatFixture
        )
        if isRunningUnitTests {
            mainActorWatchdog.setStage("init:done:unit-tests")
            return
        }
        if !resetsRealChatFixture {
            Task { [weak self] in
                await self?.scheduler.recoverBoundAutomationExecutions()
            }
        }
        
        // Setup notification categories
        Task {
            await notificationService.setupNotificationCategories()
        }

        // Debug: log current notification authorization settings on startup to help diagnose missing banners
        // Disabled in simulator as it can hang indefinitely
        do {
            let isSimulatorCheck = ProcessInfo.processInfo.environment["SIMULATOR_DEVICE_NAME"] != nil
            if !isSimulatorCheck {
                Task { @MainActor in
                    self.mainActorWatchdog.setStage("startup:notificationSettings")
                    let settings = await UNUserNotificationCenter.current().notificationSettings()
                    var meta: [String: String] = [:]
                    meta["authorizationStatus"] = "\(settings.authorizationStatus.rawValue)"
                    meta["alertSetting"] = settings.alertSetting == .enabled ? "enabled" : "disabled"
                    meta["badgeSetting"] = settings.badgeSetting == .enabled ? "enabled" : "disabled"
                    meta["soundSetting"] = settings.soundSetting == .enabled ? "enabled" : "disabled"
                    self.logger.log(.info, "ServiceContainer", "Notification settings", metadata: meta)
                    self.mainActorWatchdog.setStage("startup:notificationSettings:done")
                }
            } else {
                logger.log(.info, "ServiceContainer", "Skipping notification settings check in simulator")
                mainActorWatchdog.setStage("startup:notificationSettings:skipped-simulator")
            }
        }
        
        // Observe health data source toggle
        observeHealthDataSourceToggle()
        
        // Check local model availability on startup
        Task {
            let isSimulator = ProcessInfo.processInfo.environment["SIMULATOR_DEVICE_NAME"] != nil
            if isSimulator {
                await MainActor.run {
                    self.mainActorWatchdog.setStage("startup:checkLocalModelAvailability:skipped-simulator")
                    self.logger.log(.info, "ServiceContainer", "Skipping local model check in simulator")
                }
            } else if !self.useLocalModel {
                await MainActor.run {
                    self.mainActorWatchdog.setStage("startup:checkLocalModelAvailability:skipped-not-enabled")
                    self.logger.log(.debug, "ServiceContainer", "Skipping local model check at startup: local model not enabled")
                }
            } else {
                await MainActor.run { self.mainActorWatchdog.setStage("startup:checkLocalModelAvailability") }
                await checkLocalModelAvailability()
                await MainActor.run { self.mainActorWatchdog.setStage("startup:checkLocalModelAvailability:done") }
            }
        }
        
        // Auto-enable Health data source if HealthKit is already authorized
        Task { @MainActor in
            let isSimulator = ProcessInfo.processInfo.environment["SIMULATOR_DEVICE_NAME"] != nil
            if isSimulator {
                self.mainActorWatchdog.setStage("startup:checkAndEnableHealthKit:skipped-simulator")
                self.logger.log(.info, "ServiceContainer", "Skipping HealthKit check in simulator")
            } else {
                self.mainActorWatchdog.setStage("startup:checkAndEnableHealthKit")
                await checkAndEnableHealthKitIfAuthorized()
                self.mainActorWatchdog.setStage("startup:checkAndEnableHealthKit:done")
            }
        }
        // Schedule replay of queued candidates after init completes and inject llmClient into contextManager.
        DispatchQueue.main.async { [weak self] in
            // Preload last processed events into memory so the Today UI can render immediately
            // without doing heavy file IO on the main thread.
            if let store = self?.unifiedStorage as? FileBasedUnifiedStorage {
                let apply: @MainActor ([ProcessedEvent]) -> Void = { [weak self] events in
                    self?.recentProcessedEvents = events
                }
                Task.detached(priority: .utility) { [store, apply] in
                    let fetched = (try? await store.fetchProcessedEventsAsync(limit: nil)) ?? []
                    await apply(fetched)
                }
            }

            if let dm = self?.contextManager as? DefaultContextManager, let client = self?.llmClient {
                dm.llmClient = client
                // Also inject URL fetcher and data sources if available so context manager can include them
                dm.urlFetcher = self?.urlFetcher
                dm.dataSources = self?.dataSources
                dm.localJSONStorage = self?.localJSONStorage
            }
            Task { @MainActor in
                self?.mainActorWatchdog.setStage("startup:drainAndReplayCandidates")
                await self?.drainAndReplayCandidates()
                self?.mainActorWatchdog.setStage("startup:drainAndReplayCandidates:done")
            }
        }

        mainActorWatchdog.setStage("init:done")

        // Listen for external candidate resultset notifications (best-effort diagnostic/fallback)
        // External integrators may post a Notification named "com.nola.external.candidates"
        // with userInfo keys: "requestToken" (String) and "candidates" (Data / JSON array of SummaryItem)
        NotificationCenter.default.addObserver(forName: NSNotification.Name("com.nola.external.candidates"), object: nil, queue: OperationQueue.main) { [weak self] note in
            MainActor.assumeIsolated {
                self?.handleExternalCandidateNotification(note)
            }
        }

        // Listen for user tapping notifications (from AppNotificationDelegate)
        NotificationCenter.default.addObserver(forName: NSNotification.Name("com.nola.notification.response"), object: nil, queue: OperationQueue.main) { [weak self] note in
            MainActor.assumeIsolated {
            guard let self = self else { return }
            // Extract dedupeKey/eventId from posted userInfo if present
            if let info = note.userInfo {
                if let dedupe = info["dedupeKey"] as? String {
                    self.pendingNotificationDedupeKey = dedupe
                } else if let eventId = info["eventId"] as? String {
                    self.pendingNotificationDedupeKey = eventId
                } else if let id = info["id"] as? String {
                    // fallback to notification identifier
                    self.pendingNotificationDedupeKey = id
                }
                self.markTappedNotificationAsRead(info: info, dedupeFromResult: nil)
            }
            // Optionally, log or trigger analytics
            var metaMap: [String: String] = [:]
            if let raw = note.userInfo {
                // Convert AnyHashable keys and Any values into [String: String]
                for (k, v) in raw {
                    metaMap[String(describing: k)] = String(describing: v)
                }
            }
            self.logger.log(.info, "ServiceContainer", "Notification tap received", metadata: metaMap)
            }
        }

        // Listen for internal notification open requests (e.g., NotificationCenterView context menu)
        NotificationCenter.default.addObserver(forName: NSNotification.Name("com.nola.notification.open"), object: nil, queue: OperationQueue.main) { [weak self] note in
            MainActor.assumeIsolated {
                guard let self else { return }
                if let info = note.userInfo {
                    if let dedupe = info["dedupeKey"] as? String {
                        self.pendingNotificationDedupeKey = dedupe
                    } else if let id = info["id"] as? String {
                        self.pendingNotificationDedupeKey = id
                    }
                }
            }
        }

    }

    @MainActor
    private func markTappedNotificationAsRead(info: [AnyHashable: Any], dedupeFromResult: String?) {
        var keys: [String] = []
        if let id = info["id"] as? String {
            keys.append(id)
        }
        if let dedupe = info["dedupeKey"] as? String {
            keys.append(dedupe)
        }
        if let dedupeFromResult {
            keys.append(dedupeFromResult)
        }
        if let eventId = info["eventId"] as? String {
            keys.append(eventId)
        }

        var marked = 0
        for key in Set(keys.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }) {
            do {
                marked += try notificationManager?.markMatchingAsRead(key: key) ?? 0
            } catch {
                logger.log(.debug, "ServiceContainer", "Failed to mark tapped notification as read", metadata: [
                    "key": key,
                    "error": error.localizedDescription
                ])
            }
        }
        if marked > 0 {
            logger.log(.info, "ServiceContainer", "Marked tapped notification as read", metadata: ["count": String(marked)])
        }
    }

    @MainActor
    private func handleExternalCandidateNotification(_ note: Notification) {
        if let token = note.userInfo?["requestToken"] as? String {
            logger.log(.info, "ServiceContainer", "Received external candidate resultset notification", metadata: ["requestToken": token])
        } else {
            logger.log(.info, "ServiceContainer", "Received external candidate notification (no token)")
        }

        // Try to parse candidates payload if present
        if let raw = note.userInfo?["candidates"] {
            do {
                var items: [SummaryItem] = []
                if let data = raw as? Data {
                    items = try JSONDecoder().decode([SummaryItem].self, from: data)
                } else if let arr = raw as? [[String: Any]] {
                    let d = try JSONSerialization.data(withJSONObject: arr, options: [])
                    items = try JSONDecoder().decode([SummaryItem].self, from: d)
                }

                logger.log(.debug, "ServiceContainer", "External candidates parsed", metadata: ["count": "\(items.count)"])

                // Convert to ProcessedEvent via transformer if available, otherwise enqueue raw summaries
                if let transformer = self.dataTransformer {
                    // create a ModelOutput wrapper and invoke transformer
                    let modelOutput = ModelOutput(items: items, explain: nil)
                    let events = transformer.transform(modelOutput: modelOutput, sourceId: "external_integration", sourceType: .manual)
                    if let queue = self.candidateQueue {
                        queue.enqueue(events)
                        logger.log(.info, "ServiceContainer", "Enqueued external candidate events for replay", metadata: ["count": "\(events.count)"])
                    } else {
                        logger.log(.warn, "ServiceContainer", "No candidateQueue available; dropping external candidates", metadata: ["count": "\(events.count)"])
                    }
                } else if let queue = self.candidateQueue {
                    // If no transformer available, persist as ProcessedEvent placeholders
                    let events = items.map { si in
                        ProcessedEvent(id: UUID().uuidString, source: .reminder, sourceType: .manual, sourceId: "external_integration", title: si.text, date: nil, start: nil, due: nil, priority: si.priority, tags: nil, dedupeKey: si.dedupeKey, provenance: nil, metadata: [:], notes: nil, hash: si.text.hashValue)
                    }
                    queue.enqueue(events)
                    logger.log(.info, "ServiceContainer", "Enqueued external candidate summaries for replay", metadata: ["count": "\(events.count)"])
                } else {
                    logger.log(.warn, "ServiceContainer", "No transformer or queue available for external candidates; dropping")
                }
            } catch {
                logger.log(.error, "ServiceContainer", "Failed to parse external candidates", metadata: ["error": error.localizedDescription])
            }
        } else {
            logger.log(.debug, "ServiceContainer", "External candidate notification had no payload")
        }
    }
    
    private func observeHealthDataSourceToggle() {
        $healthDataSourceEnabled
            .sink { [weak self] enabled in
                guard let self = self else { return }
                Task { @MainActor in
                    self.setHealthDataSourceEnabled(enabled)
                }
            }
            .store(in: &cancellables)
    }
    
    private var cancellables = Set<AnyCancellable>()

    /// Drain any queued candidates and attempt to replay using unified notification service when available.
    @MainActor
    public func drainAndReplayCandidates() async {
        guard let queue = self.candidateQueue else {
            logger.log(.debug, "ServiceContainer", "No candidate queue present; nothing to replay")
            return
        }

        let batches = queue.drainAll()
        guard !batches.isEmpty else {
            logger.log(.debug, "ServiceContainer", "Candidate queue empty; nothing to replay")
            return
        }

        logger.log(.info, "ServiceContainer", "Replaying queued candidate batches", metadata: ["batches": "\(batches.count)"])

        for (reqId, events) in batches {
            for event in events {
                let date: Date? = {
                        if let s = event.start, let d = ISO8601DateFormatter().date(from: s) { return d }
                        if let dStr = event.due, let d = ISO8601DateFormatter().date(from: dStr) { return d }
                        return nil
                    }()
                    let ok = await self.unifiedNotificationService.scheduleNotification(for: event, at: date)
                    if ok {
                        logger.log(.info, "ServiceContainer", "Replayed candidate", metadata: ["reqId": reqId, "dedupeKey": event.dedupeKey])
                    } else {
                        logger.log(.warn, "ServiceContainer", "Notification was deduplicated or could not be scheduled during replay", metadata: ["reqId": reqId, "dedupeKey": event.dedupeKey])
                    }
                self.rulesEngine.markSent(event.dedupeKey)
            }
        }
    }

    func saveCredentials() {
        _ = KeychainService.save(service: "nola.llm", account: "baseURL", value: Data(baseURLString.utf8))
        _ = KeychainService.save(service: "nola.llm", account: "apiKey", value: Data(apiKey.utf8))
        configSource = .keychain
        reconfigureIfPossible()
        logger.log(.info, "config", "Saved credentials; reconfiguring & testing")
        Task { await testConnection() }
    }

    func reconfigureIfPossible(selectedModel: ModelInfo? = nil) {
        // Prevent concurrent reconfiguration
        if isReconfiguring {
            logger.log(.debug, "config", "reconfigure skipped: already running")
            return
        }

        if let selectedModel { configurationModel = selectedModel }
        let selected = configurationModel ?? (try? ModelSelectionManager.lastSelectedModel(using: applicationRuntime))
        if !useLocalModel && selected?.isWebBackend != true {
            isReconfiguring = true
            defer { isReconfiguring = false }
            do {
                // Resolve the current profile and credential even when a previously
                // prepared provider appears unchanged. Revocation precedes deduplication.
                let config = try IOSContainerCloudConfiguration.prepare(selected: selected) {
                    try ModelSelectionManager.resolvedCloudProfileForRun($0, using: self.applicationRuntime)
                }
                guard lastCloudConfiguration != config else { return }
                if let service = llmClient as? ModelService {
                    try service.useOpenAIProvider(config: config)
                } else {
                    llmClient = ModelService(provider: OpenAIProvider(config: config))
                }
                lastCloudConfiguration = config
                lastConfigSignature = nil
            } catch {
                if let service = llmClient as? ModelService {
                    service.usePlaceholderProvider()
                } else {
                    llmClient = ModelService(provider: PlaceholderLocalProvider.shared)
                }
                lastCloudConfiguration = nil
                lastConfigSignature = nil
                let stage = (error as? IOSContainerCloudConfigurationFailure)?.rawValue ?? "unknown"
                logger.log(.warn, "config", "Cloud configuration rejected at \(stage); provider cleared")
            }
            return
        }
        lastCloudConfiguration = nil

        // Compute a signature of desired config to avoid redundant work
        // Include selected model id when in local mode so switching between
        // different local models isn't incorrectly deduped.
        let desiredSignature: String
        if useLocalModel {
            let selectedId = selected?.id ?? "(unknown)"
            desiredSignature = "local:true|selected:\(selectedId)"
        } else if let selected, selected.isWebBackend {
            desiredSignature = "web:\(baseURLString)|tokenHash:\(apiKey.hashValue)|model:\(model)|selected:\(selected.id)|pin:\(selected.tlsCertificateSHA256 ?? "")"
        } else {
            desiredSignature = "remote:\(baseURLString)|apiKeyHash:\(apiKey.hashValue)|model:\(model)|protocol:\(selected?.wireProtocol?.rawValue ?? "chat_completions")"
        }

        if desiredSignature == lastConfigSignature {
            logger.log(.debug, "config", "reconfigure skipped: desired config matches last applied")
            return
        }

        isReconfiguring = true
        defer { isReconfiguring = false }

        // If user has enabled local model, use LocalProvider
        if useLocalModel {
            // Use the same explicit workspace selection as cloud configuration.
            var desiredLMModel: LMModel? = nil
            if let sel = selected, sel.isLocal {
                // Try mapping via downloader enum first
                if let mtype = ModelDownloader.ModelType(rawValue: sel.id), let mlxName = mtype.mlxModelName {
                    desiredLMModel = MLXService.availableModels.first { $0.name == mlxName }
                }
                // Fallback: try matching id/name directly
                if desiredLMModel == nil {
                    desiredLMModel = MLXService.availableModels.first { $0.id == sel.id || $0.name == sel.id }
                }
            }

            if let svc = llmClient as? ModelService {
                svc.useLocalProvider(model: desiredLMModel)
            } else {
                // Avoid instantiating LocalProvider directly on Simulator
                if ProcessInfo.processInfo.environment["SIMULATOR_DEVICE_NAME"] != nil {
                    llmClient = ModelService(provider: PlaceholderLocalProvider.shared)
                } else {
                    llmClient = ModelService(provider: LocalProvider(model: desiredLMModel))
                }
            }
            logger.log(.info, "config", "Using LocalProvider (local mode)", metadata: ["selectedModel": selected?.id ?? "(none)"])
            lastConfigSignature = desiredSignature
            return
        }

        if let selected, selected.isWebBackend {
            guard let url = URL(string: baseURLString), !apiKey.isEmpty else {
                if let svc = llmClient as? ModelService {
                    svc.usePlaceholderProvider()
                } else {
                    llmClient = ModelService(provider: PlaceholderLocalProvider.shared)
                }
                logger.log(.warn, "config", "Tearing down WebAgentProvider; missing URL or token", metadata: [
                    "baseURLLength": "\(baseURLString.count)",
                    "tokenLength": "\(apiKey.count)"
                ])
                lastConfigSignature = desiredSignature
                return
            }

            let config = WebAgentProvider.Config(
                baseURL: url,
                token: apiKey,
                model: model,
                pinnedCertificateSHA256: selected.tlsCertificateSHA256,
                expectedHostId: ModelSelectionManager.pairedWebHostIdentity(for: url)
            )
            if let svc = llmClient as? ModelService {
                svc.useWebAgentProvider(config: config)
            } else {
                llmClient = ModelService(provider: WebAgentProvider(config: config))
            }
            logger.log(.info, "config", "Configured WebAgentProvider", metadata: ["baseURL": url.absoluteString, "model": model])
            lastConfigSignature = desiredSignature
            return
        }

    }
    
    // MARK: - Local Model Management
    
    @MainActor
    func checkLocalModelAvailability() async {
        localModelStatus = .checking
        logger.log(.info, "local-model", "Checking local model availability")

        let downloaded = modelDownloader.computeDownloadedModels()

        // Apply snapshot on main
        modelDownloader.applyDownloadedModels(downloaded)
        let hasDownloadedModels = !downloaded.isEmpty

        if hasDownloadedModels {
            localModelStatus = .available
            logger.log(.info, "local-model", "Local model is available", metadata: [
                "count": "\(downloaded.count)",
                "models": "\(downloaded.map { $0.rawValue }.joined(separator: ", "))"
            ])
        } else {
            localModelStatus = .downloadRequired
            logger.log(.warn, "local-model", "No local models downloaded - download required")
        }
    }

    func resetCredentials() {
        _ = KeychainService.delete(service: "nola.llm", account: "baseURL")
        _ = KeychainService.delete(service: "nola.llm", account: "apiKey")
        loadInitialConfig()
        reconfigureIfPossible()
        testStatus = .idle
    }

    private func loadInitialConfig() {
        // 1) Prefer Keychain values first.
        if let dataURL = KeychainService.read(service: "nola.llm", account: "baseURL"),
           let savedURL = String(data: dataURL, encoding: .utf8),
           let dataKey = KeychainService.read(service: "nola.llm", account: "apiKey"),
           let savedKey = String(data: dataKey, encoding: .utf8) {
            self.baseURLString = savedURL
            self.apiKey = savedKey
            self.configSource = .keychain
            return
        }
        // An unconfigured installation starts with the product default.
        self.baseURLString = "https://api-inference.modelscope.cn/v1"
        self.apiKey = ""
        self.configSource = .default
    }

    // MARK: - Connection test (/v1/models)
    @MainActor
    func testConnection() async {
        guard let url = endpointUnderV1("models") else { testStatus = .invalidResponse; logger.log(.error, "test", "Invalid base URL", metadata: ["baseURL": baseURLString]); return }
        guard !apiKey.isEmpty else {
            testStatus = .unauthorized
            logger.log(.warn, "test", "API key missing when testing")
            return
        }
        testStatus = .testing
        logger.log(.info, "test", "GET /v1/models")
        var req = URLRequest(url: url)
        req.httpMethod = "GET"
        req.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        req.timeoutInterval = 10
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            guard let http = resp as? HTTPURLResponse else { testStatus = .invalidResponse; return }
            switch http.statusCode {
            case 200:
                if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                    // OpenAI-compatible providers commonly return { data: [...] }.
                    if let arr = json["data"] as? [Any] { testStatus = .ok(modelCount: arr.count); logger.log(.info, "test", "Models OK", metadata: ["count": String(arr.count)]) }
                    else { testStatus = .ok(modelCount: 0) }
                } else { testStatus = .invalidResponse; logger.log(.warn, "test", "Invalid JSON in /models response") }
                await testEcho()
            case 401: testStatus = .unauthorized; logger.log(.error, "test", "Unauthorized (401)")
            case 404, 405:
                // Some providers do not implement /models; probe with chat/completions.
                logger.log(.debug, "test", "Models not available (\(http.statusCode)); trying chat/completions")
                await testChatFallback()
            case 429: testStatus = .rateLimited; logger.log(.warn, "test", "Rate limited (429)")
            default: testStatus = .server(http.statusCode); logger.log(.error, "test", "Server status", metadata: ["code": String(http.statusCode)])
            }
        } catch {
            testStatus = .network(error.localizedDescription)
            logger.log(.error, "test", "Network error", metadata: ["error": error.localizedDescription])
        }
    }

    // Try a minimal chat/completions call as fallback
    @MainActor
    private func testChatFallback() async {
    guard let url = endpointUnderV1("chat/completions") else { testStatus = .invalidResponse; logger.log(.error, "test", "Invalid chat endpoint"); return }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        req.timeoutInterval = 10
    logger.log(.info, "test", "POST /v1/chat/completions (ping)")
        let body: [String: Any] = [
            "model": model,
            "messages": [["role": "user", "content": "ping"]],
            "max_tokens": 1,
            "temperature": 0.7
        ]
        req.httpBody = try? JSONSerialization.data(withJSONObject: body)
        do {
            let (_, resp) = try await URLSession.shared.data(for: req)
            guard let http = resp as? HTTPURLResponse else { testStatus = .invalidResponse; return }
            switch http.statusCode {
            case 200: testStatus = .okChat; logger.log(.info, "test", "Chat OK"); await testEcho()
            case 401: testStatus = .unauthorized; logger.log(.error, "test", "Unauthorized (401)")
            case 429: testStatus = .rateLimited; logger.log(.warn, "test", "Rate limited (429)")
            default: testStatus = .server(http.statusCode); logger.log(.error, "test", "Server status", metadata: ["code": String(http.statusCode)])
            }
        } catch {
            testStatus = .network(error.localizedDescription)
            logger.log(.error, "test", "Network error", metadata: ["error": error.localizedDescription])
        }
    }

    // Echo verification: send a unique token and verify it appears in the reply.
    @MainActor
    private func testEcho() async {
        guard testStatus.isSuccessfulBase else { return }
        testStatus = .echoTesting
        let token = "echo:" + UUID().uuidString.prefix(8)
    guard let url = endpointUnderV1("chat/completions") else { testStatus = .invalidResponse; logger.log(.error, "echo", "Invalid chat endpoint"); return }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        req.timeoutInterval = 12
    logger.log(.info, "echo", "POST /v1/chat/completions (echo)", metadata: ["token": String(token)])
        let body: [String: Any] = [
            "model": model,
            "messages": [["role": "system", "content": "Echo diagnostic"],
                         ["role": "user", "content": PromptTemplates.echoUser(token: String(token))]],
            "max_tokens": 8,
            "temperature": 0.7
        ]
        req.httpBody = try? JSONSerialization.data(withJSONObject: body)
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            guard let http = resp as? HTTPURLResponse else { testStatus = .invalidResponse; return }
            guard http.statusCode == 200 else { testStatus = .echoMismatch; logger.log(.warn, "echo", "Non-200 on echo", metadata: ["code": String(http.statusCode)]); return }
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let choices = json["choices"] as? [[String: Any]],
               let first = choices.first,
               let message = first["message"] as? [String: Any],
               let content = message["content"] as? String {
                if content.contains(token) { testStatus = .echoOk; logger.log(.info, "echo", "Echo match") } else { testStatus = .echoMismatch; logger.log(.warn, "echo", "Echo mismatch") }
            } else {
                testStatus = .echoMismatch
                logger.log(.warn, "echo", "Malformed echo response JSON")
            }
        } catch {
            logger.log(.error, "echo", "Network error", metadata: ["error": error.localizedDescription])
            testStatus = .echoMismatch
        }
    }

    // Build an endpoint ensuring there is exactly one /v1 and proper subpath joining
    private func endpointUnderV1(_ subpath: String) -> URL? {
        guard var base = URL(string: baseURLString) else { return nil }
        // If base already ends with /v1, keep; otherwise append v1
        if base.lastPathComponent != "v1" {
            base.appendPathComponent("v1")
        }
        let clean = subpath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return base.appendingPathComponent(clean)
    }
}

private extension ServiceContainer.TestStatus {
    var isSuccessfulBase: Bool {
        switch self {
        case .ok, .okChat: return true
        default: return false
        }
    }
}
