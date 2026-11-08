// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements Scheduler logic in the iOS service layer.
// - Interfaces:
//   - Scheduler
// - APIs:
//   - none
//
//
//  Scheduler.swift
//  Nola
//
//  Background task scheduling and notification management.
//

import Foundation

nonisolated struct MemoryWorkerWakeSchedulingPolicy {
    static let minimumLeadTime: TimeInterval = 60
    static let scheduledActionLeadTime: TimeInterval = 10 * 60
    static let retryDelay: TimeInterval = 60
    static let maximumRetryAttempts = 3

    static func nextRefreshDate(
        now: Date,
        daily: Date,
        scheduledAction: Date?,
        memoryWorker: Date?
    ) -> Date {
        [daily, scheduledAction, memoryWorker]
            .compactMap { $0 }
            .map { max(now.addingTimeInterval(minimumLeadTime), $0) }
            .min() ?? daily
    }

    static func nextProcessingDate(
        now: Date,
        scheduledAction: Date?,
        memoryWorker: Date?
    ) -> Date? {
        let candidates = [
            scheduledAction?.addingTimeInterval(-scheduledActionLeadTime),
            memoryWorker,
        ].compactMap { $0 }
        guard let earliest = candidates.min() else { return nil }
        return max(now.addingTimeInterval(minimumLeadTime), earliest)
    }

    static func nextMemoryWorkerAttempt(
        now: Date,
        report: IOSMemoryWorkerRunReport,
        jobBudget: Int,
        retryAttemptCount: Int
    ) -> Date? {
        if report.retryScheduled > 0 {
            // Core's maximumRetryCount counts retries after the initial claim.
            // retryAttemptCount includes the initial failed execution, so the
            // third scheduled retry is still required to reach Core's terminal
            // transition on a fourth consecutive failure.
            guard retryAttemptCount <= maximumRetryAttempts else { return nil }
            return now.addingTimeInterval(retryDelay)
        }
        if report.attempted >= jobBudget {
            return now.addingTimeInterval(retryDelay)
        }
        return nil
    }
}

nonisolated struct ScheduledURLActionAgentRequest {
    let prompt: String
    let referenceComponent: AgentCoreContextComponent

    static func configuredPrompt(userPrompt: String?, sourceDescription: String?) -> String {
        let prompt = userPrompt?.trimmingCharacters(in: .whitespacesAndNewlines)
        let description = sourceDescription?.trimmingCharacters(in: .whitespacesAndNewlines)
        return prompt?.isEmpty == false ? prompt! : (description?.isEmpty == false ? description! : "Summarize the configured source data and report the most actionable changes.")
    }

    static func make(
        actionId: String,
        sourceName: String,
        sourceURL: String,
        userPrompt: String?,
        sourceDescription: String?,
        dedupeKey: String,
        observedAt: Date,
        dateRange: ModelInput.DateRange,
        candidates: [ModelInput.Candidate]
    ) throws -> Self {
        let prompt = configuredPrompt(userPrompt: userPrompt, sourceDescription: sourceDescription)
        let reference = ModelInput(
            dateRange: dateRange,
            candidates: candidates,
            userPrefs: [
                "purpose": "scheduled_action_reference",
                "sourceName": sourceName,
                "sourceUrl": sourceURL,
            ]
        )
        let data = try JSONEncoder().encode(reference)
        guard let content = String(data: data, encoding: .utf8) else {
            throw AgentCoreRuntimeError.invalidUTF8
        }
        return Self(
            prompt: prompt,
            referenceComponent: AgentCoreContextComponent(
                kind: .document,
                id: actionId,
                content: .string(content),
                source: AgentCoreContextComponentSource(
                    platform: .ios,
                    uri: sourceURL,
                    title: sourceName,
                    mimeType: "application/json",
                    observedAt: ISO8601DateFormatter().string(from: observedAt),
                    authorization: "available"
                )
            )
        )
    }
}
import BackgroundTasks
import UserNotifications
#if os(iOS)
import UIKit
#endif
import Dispatch

/// Manages daily summary scheduling via BGAppRefreshTask.
public final class Scheduler {
    public static let taskIdentifier = "com.nola.refresh.daily"
    public static let processingTaskIdentifier = "com.nola.processing.scheduled_url"
    
    // Shared handler reference for early registration in NolaApp.init
    private static var sharedHandler: ((BGAppRefreshTask) -> Void)?
    private static var sharedProcessingHandler: ((BGProcessingTask) -> Void)?
    private static var backgroundRegistrationAttempted = false
    private static var refreshTaskRegistered = false
    private static var processingTaskRegistered = false
    private static let memoryWorkerWakeDateKey = "scheduler.memory_worker.wake_date"
    private static let memoryWorkerRetryAttemptCountKey = "scheduler.memory_worker.retry_attempt_count"
    private static let memoryWorkerJobBudget = 2
    
    private let container: ServiceContainer
    private let preprocessor: EventPreprocessor
    private let rules: RulesEngine
    private var foregroundScheduledURLTimer: DispatchSourceTimer?
    private var foregroundScheduledURLFireDate: Date?

    /// Concurrency guard for the heavy pipeline (`performSummary` /
    /// `executeScheduledURLActions`). Background-refresh, background-processing,
    /// and the foreground timer can all fire near-simultaneously; without this
    /// lock the same actions execute multiple times in parallel, double-write the
    /// run history file, and re-fire notifications past the dedup window race.
    private let runLock = NSLock()
    private var isRunning: Bool = false

    /// Acquire run lock or return false if already running. Caller must call
    /// `releaseRunLock()` when the run finishes (success or failure).
    private func tryAcquireRunLock(reason: String) -> Bool {
        runLock.lock(); defer { runLock.unlock() }
        if isRunning {
            container.logger.log(.info, "Scheduler", "Skip overlapping run", metadata: ["reason": reason])
            return false
        }
        isRunning = true
        return true
    }

    private func releaseRunLock() {
        runLock.lock(); defer { runLock.unlock() }
        isRunning = false
    }

    private func backgroundRefreshStatusName() -> String {
        #if canImport(UIKit)
        switch UIApplication.shared.backgroundRefreshStatus {
        case .available: return "available"
        case .denied: return "denied"
        case .restricted: return "restricted"
        @unknown default: return "unknown"
        }
        #else
        return "unknown"
        #endif
    }
    
    public init(
        container: ServiceContainer,
        preprocessor: EventPreprocessor = EventPreprocessor(),
        rules: RulesEngine = RulesEngine()
    ) {
        self.container = container
        self.preprocessor = preprocessor
        self.rules = rules
        
        // Set the shared handler so NolaApp.init can use it
        Self.sharedHandler = { [weak self] task in
            self?.handleBackgroundRefresh(task: task)
        }

        Self.sharedProcessingHandler = { [weak self] task in
            self?.handleBackgroundProcessing(task: task)
        }
    }
    
    /// Register background task handler (called from NolaApp.init).
    public static func registerBackgroundTasksEarly() {
        guard !backgroundRegistrationAttempted else { return }
        backgroundRegistrationAttempted = true
        let successRefresh = BGTaskScheduler.shared.register(forTaskWithIdentifier: taskIdentifier, using: nil) { task in
            if let handler = sharedHandler {
                guard let refreshTask = task as? BGAppRefreshTask else {
                    Logger().log(.error, "Scheduler", "Registered refresh task returned unexpected BGTask type", metadata: [
                        "taskId": task.identifier,
                        "actualTaskType": String(describing: type(of: task)),
                        "expectedTaskType": "BGAppRefreshTask"
                    ])
                    task.setTaskCompleted(success: false)
                    return
                }
                handler(refreshTask)
            } else {
                Logger().log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.abcf95e0", comment: "AUTOGEN"), metadata: [
                    "taskId": task.identifier,
                    "hint": "Ensure ServiceContainer initializes scheduler during app launch"
                ])
                task.setTaskCompleted(success: false)
            }
        }

        let successProcessing = BGTaskScheduler.shared.register(forTaskWithIdentifier: processingTaskIdentifier, using: nil) { task in
            if let handler = sharedProcessingHandler {
                guard let processingTask = task as? BGProcessingTask else {
                    Logger().log(.error, "Scheduler", "Registered processing task returned unexpected BGTask type", metadata: [
                        "taskId": task.identifier,
                        "actualTaskType": String(describing: type(of: task)),
                        "expectedTaskType": "BGProcessingTask"
                    ])
                    task.setTaskCompleted(success: false)
                    return
                }
                handler(processingTask)
            } else {
                Logger().log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.e74c7118", comment: "AUTOGEN"), metadata: [
                    "taskId": task.identifier,
                    "hint": "Ensure ServiceContainer initializes scheduler during app launch"
                ])
                task.setTaskCompleted(success: false)
            }
        }

        refreshTaskRegistered = successRefresh
        processingTaskRegistered = successProcessing

        if successRefresh {
            Logger().log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.0f36e286", comment: "AUTOGEN"), metadata: ["taskId": taskIdentifier])
        } else {
            Logger().log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.bc2069f7", comment: "AUTOGEN"), metadata: ["taskId": taskIdentifier])
        }

        if successProcessing {
            Logger().log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.f9a58c29", comment: "AUTOGEN"), metadata: ["taskId": processingTaskIdentifier])
        } else {
            Logger().log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.d7afabe9", comment: "AUTOGEN"), metadata: ["taskId": processingTaskIdentifier])
        }
    }
    
    /// Schedule next background refresh (target time window).
    public func scheduleNextRefresh(targetHour: Int = 8) {
        // Cancel any existing pending requests first
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.taskIdentifier)
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.processingTaskIdentifier)
        
        let request = BGAppRefreshTaskRequest(identifier: Self.taskIdentifier)

        let now = Date()
        let nextDaily = nextDailySummaryDate(targetHour: targetHour, from: now)
        let nextScheduled = nextScheduledURLActionDate(from: now)
        let nextMemoryWorker = pendingMemoryWorkerWakeDate()

        let nextRun = MemoryWorkerWakeSchedulingPolicy.nextRefreshDate(
            now: now,
            daily: nextDaily,
            scheduledAction: nextScheduled,
            memoryWorker: nextMemoryWorker
        )

        request.earliestBeginDate = nextRun
        let iso = ISO8601DateFormatter()
        let untilSec = Int(max(0, nextRun.timeIntervalSince(now)).rounded())

        let bgRefreshStatusRaw: String = {
            #if canImport(UIKit)
            return String(UIApplication.shared.backgroundRefreshStatus.rawValue)
            #else
            return "unknown"
            #endif
        }()

        let bgRefreshStatusName = backgroundRefreshStatusName()

        let shouldAttemptBGTaskScheduling: Bool = {
            #if canImport(UIKit)
            return UIApplication.shared.backgroundRefreshStatus == .available
            #else
            return false
            #endif
        }()

        let lowPowerMode: String = {
            #if canImport(UIKit)
            return ProcessInfo.processInfo.isLowPowerModeEnabled ? "true" : "false"
            #else
            return "unknown"
            #endif
        }()

        let appState: String = {
            #if canImport(UIKit)
            switch UIApplication.shared.applicationState {
            case .active: return "active"
            case .inactive: return "inactive"
            case .background: return "background"
            @unknown default: return "unknown"
            }
            #else
            return "unknown"
            #endif
        }()

        #if canImport(UIKit)
        if UIApplication.shared.backgroundRefreshStatus != .available {
            container.logger.log(.warn, "Scheduler", "Background App Refresh is disabled/restricted; BGTask execution is not expected", metadata: [
                "backgroundRefreshStatus": bgRefreshStatusRaw,
                "backgroundRefreshStatusName": bgRefreshStatusName,
                "lowPowerMode": lowPowerMode,
                "appState": appState,
                "hint": "Check Settings > General > Background App Refresh (global + per-app), and Low Power Mode"
            ])
        }
        #endif
        container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.dad3a85e", comment: "AUTOGEN"), metadata: [
            "now": iso.string(from: now),
            "nextRun": iso.string(from: nextRun),
            "nextRunHuman": nextRun.formatted(date: .abbreviated, time: .shortened),
            "untilNextRunSec": String(untilSec),
            "nextDaily": iso.string(from: nextDaily),
            "nextScheduledAction": nextScheduled.map { iso.string(from: $0) } ?? "none",
            "nextMemoryWorker": nextMemoryWorker.map { iso.string(from: $0) } ?? "none",
            "hasURLFetcher": "\(container.urlFetcher != nil)",
            "targetHour": "\(targetHour)",
            "backgroundRefreshStatus": bgRefreshStatusRaw,
            "backgroundRefreshStatusName": bgRefreshStatusName,
            "lowPowerMode": lowPowerMode,
            "appState": appState
        ])
        
        // Submitting an unregistered identifier raises an Objective-C exception,
        // not a Swift Error. Registration may be skipped or fail at launch.
        if shouldAttemptBGTaskScheduling && Self.refreshTaskRegistered {
            do {
                try BGTaskScheduler.shared.submit(request)
                container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.dc971e08", comment: "AUTOGEN"), metadata: [
                    "target": "\(targetHour):00",
                    "earliestBegin": request.earliestBeginDate?.formatted(date: .abbreviated, time: .shortened) ?? "none",
                    "earliestBeginISO": request.earliestBeginDate.map { iso.string(from: $0) } ?? "none"
                ])

                // Debug: inspect pending task requests (best-effort)
                BGTaskScheduler.shared.getPendingTaskRequests { requests in
                    let pendingRefresh = requests.filter { $0.identifier == Self.taskIdentifier }
                    let pendingProcessing = requests.filter { $0.identifier == Self.processingTaskIdentifier }
                    let nextRefresh = pendingRefresh.first?.earliestBeginDate
                    let nextProcessing = pendingProcessing.first?.earliestBeginDate
                    let metadata = [
                        "totalPending": String(requests.count),
                        "pendingRefresh": String(pendingRefresh.count),
                        "nextRefreshEarliestISO": nextRefresh.map { iso.string(from: $0) } ?? "none",
                        "pendingProcessing": String(pendingProcessing.count),
                        "nextProcessingEarliestISO": nextProcessing.map { iso.string(from: $0) } ?? "none"
                    ]
                    Task { @MainActor in
                        self.container.logger.log(.debug, "Scheduler", "BGTask pending requests", metadata: metadata)
                    }
                }
            } catch let error as NSError {
            let errorCode = BGTaskScheduler.Error.Code(rawValue: error.code)
            var errorDesc = error.localizedDescription
            
            switch errorCode {
            case .unavailable:
                errorDesc = NSLocalizedString("autogen.Scheduler.84908629", comment: "AUTOGEN")
            case .tooManyPendingTaskRequests:
                errorDesc = NSLocalizedString("autogen.Scheduler.8e8892f4", comment: "AUTOGEN")
            case .notPermitted:
                errorDesc = NSLocalizedString("autogen.Scheduler.a1191dd3", comment: "AUTOGEN")
            default:
                break
            }
            
            container.logger.log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.f88048d7", comment: "AUTOGEN"), metadata: [
                "error": errorDesc,
                "code": "\(error.code)",
                "taskId": Self.taskIdentifier,
                "earliestBeginISO": request.earliestBeginDate.map { iso.string(from: $0) } ?? "none"
            ])
            } catch {
                container.logger.log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.f88048d7", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            }
        } else {
            container.logger.log(.info, "Scheduler", "Skipped background refresh submission", metadata: [
                "registered": String(Self.refreshTaskRegistered),
                "target": "\(targetHour):00",
                "earliestBeginISO": request.earliestBeginDate.map { iso.string(from: $0) } ?? "none",
                "backgroundRefreshStatus": bgRefreshStatusRaw,
                "backgroundRefreshStatusName": bgRefreshStatusName
            ])
        }

        // Schedule processing for the earliest URL action or durable Memory Worker wake.
        if shouldAttemptBGTaskScheduling,
           Self.processingTaskRegistered,
           let processingDate = MemoryWorkerWakeSchedulingPolicy.nextProcessingDate(
               now: now,
               scheduledAction: nextScheduled,
               memoryWorker: nextMemoryWorker
           ) {
            let processingRequest = BGProcessingTaskRequest(identifier: Self.processingTaskIdentifier)
            processingRequest.requiresNetworkConnectivity = true
            processingRequest.requiresExternalPower = false

            processingRequest.earliestBeginDate = processingDate

            let untilProcSec = Int(max(0, processingDate.timeIntervalSince(now)).rounded())
            container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.67c471a2", comment: "AUTOGEN"), metadata: [
                "now": iso.string(from: now),
                "nextScheduledAction": nextScheduled.map { iso.string(from: $0) } ?? "none",
                "nextMemoryWorker": nextMemoryWorker.map { iso.string(from: $0) } ?? "none",
                "earliestBeginISO": iso.string(from: processingDate),
                "untilEarliestSec": String(untilProcSec),
                "requiresNetwork": "true"
            ])

            do {
                try BGTaskScheduler.shared.submit(processingRequest)
                container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.61d32f61", comment: "AUTOGEN"), metadata: [
                    "earliestBegin": processingRequest.earliestBeginDate?.formatted(date: .abbreviated, time: .shortened) ?? "none",
                    "earliestBeginISO": processingRequest.earliestBeginDate.map { iso.string(from: $0) } ?? "none"
                ])
            } catch let error as NSError {
                let errorCode = BGTaskScheduler.Error.Code(rawValue: error.code)
                var errorDesc = error.localizedDescription
                switch errorCode {
                case .unavailable:
                    errorDesc = NSLocalizedString("autogen.Scheduler.84908629", comment: "AUTOGEN")
                case .tooManyPendingTaskRequests:
                    errorDesc = NSLocalizedString("autogen.Scheduler.8e8892f4", comment: "AUTOGEN")
                case .notPermitted:
                    errorDesc = NSLocalizedString("autogen.Scheduler.a1191dd3", comment: "AUTOGEN")
                default:
                    break
                }
                container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.141780c1", comment: "AUTOGEN"), metadata: [
                    "error": errorDesc,
                    "code": "\(error.code)",
                    "taskId": Self.processingTaskIdentifier,
                    "earliestBeginISO": processingRequest.earliestBeginDate.map { iso.string(from: $0) } ?? "none"
                ])
            } catch {
                container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.141780c1", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            }
        }

        // Foreground best-effort fallback: if the app is active around the due time,
        // run scheduled URL actions automatically without relying on BGTask.
        scheduleForegroundScheduledURLTimer(nextScheduled: nextScheduled)
    }

    private func scheduleForegroundScheduledURLTimer(nextScheduled: Date?) {
        // Only meaningful when URLFetcher exists and we have a scheduled time.
        guard container.urlFetcher != nil, let nextScheduled else {
            foregroundScheduledURLTimer?.cancel()
            foregroundScheduledURLTimer = nil
            foregroundScheduledURLFireDate = nil
            return
        }

        // Avoid thrashing: if timer already targets this same time (within 1s), keep it.
        if let existing = foregroundScheduledURLFireDate, abs(existing.timeIntervalSince(nextScheduled)) < 1 {
            return
        }

        foregroundScheduledURLTimer?.cancel()

        foregroundScheduledURLFireDate = nextScheduled
        let iso = ISO8601DateFormatter()
        container.logger.log(.debug, "Scheduler", "Scheduled foreground timer for scheduled URL actions", metadata: [
            "fireAtISO": iso.string(from: nextScheduled),
            "fireAtHuman": nextScheduled.formatted(date: .abbreviated, time: .shortened)
        ])

        let timer = DispatchSource.makeTimerSource(queue: .main)
        let deadline = DispatchTime.now() + max(0, nextScheduled.timeIntervalSinceNow)
        timer.schedule(deadline: deadline, leeway: .seconds(1))
        timer.setEventHandler { [weak self] in
            guard let self else { return }

            let state: String = {
                #if canImport(UIKit)
                switch UIApplication.shared.applicationState {
                case .active: return "active"
                case .inactive: return "inactive"
                case .background: return "background"
                @unknown default: return "unknown"
                }
                #else
                return "unknown"
                #endif
            }()

            self.container.logger.log(.info, "Scheduler", "Foreground timer fired for scheduled URL actions", metadata: [
                "nowISO": iso.string(from: Date()),
                "appState": state
            ])

            Task { [weak self] in
                guard let self else { return }
                guard self.tryAcquireRunLock(reason: "ForegroundTimer") else { return }
                defer { self.releaseRunLock() }
                let pastWindow: TimeInterval = 7 * 24 * 3600
                let futureWindow: TimeInterval = 7 * 24 * 3600
                _ = await self.executeScheduledURLActions(
                    isBackground: false,
                    pastWindow: pastWindow,
                    futureWindow: futureWindow,
                    sourcesOverride: nil,
                    respectQuietHours: true,
                    dedupeKeyMode: .daily
                )
                self.scheduleNextRefresh()
            }
        }
        foregroundScheduledURLTimer = timer
        timer.resume()
    }

    // MARK: - Next-run calculation

    private func nextDailySummaryDate(targetHour: Int, from now: Date) -> Date {
        var components = Calendar.current.dateComponents([.year, .month, .day], from: now)
        components.hour = targetHour
        components.minute = 0
        components.second = 0
        if let todayTarget = Calendar.current.date(from: components), todayTarget > now {
            return todayTarget
        }
        let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: now) ?? now.addingTimeInterval(24 * 3600)
        var tomorrowComponents = Calendar.current.dateComponents([.year, .month, .day], from: tomorrow)
        tomorrowComponents.hour = targetHour
        tomorrowComponents.minute = 0
        tomorrowComponents.second = 0
        return Calendar.current.date(from: tomorrowComponents) ?? tomorrow
    }

    private func nextScheduledURLActionDate(from now: Date) -> Date? {
        guard let fetcher = container.urlFetcher else { return nil }
        let scheduledSources = fetcher.getConfiguredScheduledSources()
        guard !scheduledSources.isEmpty else { return nil }

        let iso = ISO8601DateFormatter()
        let tz = TimeZone.current
        container.logger.log(.debug, "Scheduler", "Computing scheduled URL candidates", metadata: [
            "now": iso.string(from: now),
            "timeZone": tz.identifier,
            "tzOffsetSec": String(tz.secondsFromGMT(for: now)),
            "sources": String(scheduledSources.count)
        ])

        var best: Date? = nil
        for source in scheduledSources {
            if source.scheduleFrequency == "once",
               let targetDate = source.scheduledDate {
                let toleranceSec: TimeInterval = 5 * 60
                if let lastFetch = source.lastFetchDate,
                   lastFetch >= targetDate.addingTimeInterval(-toleranceSec) {
                    continue
                }
                let candidate = max(targetDate, now)
                if best == nil || candidate < best! {
                    best = candidate
                }
                continue
            }

            guard let freq = source.scheduleFrequency, let timeISO = source.scheduledTimeISO else {
                container.logger.log(.warn, "Scheduler", "Scheduled source missing frequency/time", metadata: [
                    "sourceName": source.name,
                    "sourceId": source.id.uuidString,
                    "freq": source.scheduleFrequency ?? "nil",
                    "timeISO": source.scheduledTimeISO ?? "nil"
                ])
                continue
            }

            guard let hm = ScheduledTimeParser.parseHourMinute(timeISO) else {
                container.logger.log(.warn, "Scheduler", "Failed to parse scheduledTimeISO", metadata: [
                    "sourceName": source.name,
                    "sourceId": source.id.uuidString,
                    "freq": freq,
                    "timeISO": timeISO
                ])
                continue
            }

            guard let candidate = try? canonicalOccurrenceDates(source, after: now,
                through: now.addingTimeInterval(8*86400), limit: 1).first else { continue }
            let candidateToday=candidate
            let rolled=freq


            container.logger.log(.debug, "Scheduler", "Scheduled URL candidate", metadata: [
                "sourceName": source.name,
                "sourceId": source.id.uuidString,
                "freq": freq,
                "timeISO": timeISO,
                "candidateToday": iso.string(from: candidateToday),
                "candidateFinal": iso.string(from: candidate),
                "rolled": rolled,
                "bestBefore": best.map { iso.string(from: $0) } ?? "none"
            ])

            if best == nil || candidate < best! {
                best = candidate
                container.logger.log(.debug, "Scheduler", "Scheduled URL best updated", metadata: [
                    "best": iso.string(from: candidate),
                    "sourceId": source.id.uuidString,
                    "sourceName": source.name
                ])
            }
        }

        if let best {
            container.logger.log(.debug, "Scheduler", "Next scheduled URL action computed", metadata: [
                "now": iso.string(from: now),
                "best": iso.string(from: best),
                "sources": String(scheduledSources.count)
            ])
        } else {
            container.logger.log(.debug, "Scheduler", "No eligible scheduled URL action time", metadata: [
                "sources": String(scheduledSources.count)
            ])
        }
        return best
    }
    
    /// Manual trigger: generate summary and notify.
    public func generateSummaryNow() async throws {
        container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.af60885d", comment: "AUTOGEN"))
        try await performSummary(isBackground: false)
    }

    /// Manual trigger: run scheduled URL actions immediately (without running the daily summary).
    ///
    /// - Note: This runs *only* scheduled URL actions (fetch + best-effort report/notify + run record).
    ///         It does not fetch other data sources and does not generate the daily summary.
    /// - Returns: `true` if there are configured scheduled URL actions to run.
    public func runScheduledURLActionsNow() async -> Bool {
        guard let fetcher = container.urlFetcher else { return false }

        let sources = fetcher.sources.filter { $0.enabled && $0.isScheduledAutomation }

        guard !sources.isEmpty else {
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.0df7a9f3", comment: "AUTOGEN"))
            return false
        }

        container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.700374cd", comment: "AUTOGEN"), metadata: ["sources": "\(sources.count)"])

        let pastWindow: TimeInterval = 7 * 24 * 3600
        let futureWindow: TimeInterval = 7 * 24 * 3600
        _ = await executeScheduledURLActions(
            isBackground: false,
            pastWindow: pastWindow,
            futureWindow: futureWindow,
            sourcesOverride: sources,
            respectQuietHours: false,
            dedupeKeyMode: .manualPerRun
        )

        return true
    }

    /// Manual trigger: run a single scheduled URL action immediately.
    ///
    /// - Returns: `true` if the action exists and is configured as a scheduled action.
    public func runScheduledURLActionNow(sourceId: UUID) async -> Bool {
        guard let fetcher = container.urlFetcher else { return false }

        guard let source = fetcher.sources.first(where: { $0.id == sourceId }) else {
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.c17e8c5b", comment: "AUTOGEN"), metadata: ["sourceId": sourceId.uuidString])
            return false
        }

        guard source.enabled else {
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.f7092ea4", comment: "AUTOGEN"), metadata: ["source": source.name, "sourceId": sourceId.uuidString])
            return false
        }

        guard source.isScheduledAutomation else {
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.04018530", comment: "AUTOGEN"), metadata: ["source": source.name, "sourceId": sourceId.uuidString])
            return false
        }

        container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.dfdee7e7", comment: "AUTOGEN"), metadata: ["source": source.name, "sourceId": sourceId.uuidString])

        let pastWindow: TimeInterval = 7 * 24 * 3600
        let futureWindow: TimeInterval = 7 * 24 * 3600
        _ = await executeScheduledURLActions(
            isBackground: false,
            pastWindow: pastWindow,
            futureWindow: futureWindow,
            sourcesOverride: [source],
            respectQuietHours: false,
            dedupeKeyMode: .manualPerRun
        )

        return true
    }
    
    // MARK: - Private

    nonisolated private final class BackgroundTaskCompletionState: @unchecked Sendable {
        private let lock = NSLock()
        private var isExpired = false
        private var didComplete = false

        var expired: Bool {
            lock.withLock { isExpired }
        }

        func expire() -> Bool {
            lock.withLock {
                isExpired = true
                guard !didComplete else { return false }
                didComplete = true
                return true
            }
        }

        func completeIfActive() -> Bool {
            lock.withLock {
                guard !didComplete, !isExpired else { return false }
                didComplete = true
                return true
            }
        }
    }

    /// Persist a near-term wake independently of daily summaries and URL actions.
    /// BGTask requests are best-effort, while this deadline survives app relaunches
    /// and is re-submitted whenever the scheduler recalculates its next run.
    public func scheduleMemoryWorkerWake(after delay: TimeInterval = 60) {
        let defaults = UserDefaults.standard
        let requested = Date().addingTimeInterval(max(delay, MemoryWorkerWakeSchedulingPolicy.minimumLeadTime))
        let existing = pendingMemoryWorkerWakeDate()
        defaults.set(min(existing ?? requested, requested).timeIntervalSince1970, forKey: Self.memoryWorkerWakeDateKey)
        defaults.set(0, forKey: Self.memoryWorkerRetryAttemptCountKey)
        scheduleNextRefresh()
    }

    private func pendingMemoryWorkerWakeDate() -> Date? {
        let timestamp = UserDefaults.standard.double(forKey: Self.memoryWorkerWakeDateKey)
        guard timestamp > 0 else { return nil }
        return Date(timeIntervalSince1970: timestamp)
    }

    private func recordMemoryWorkerAttempt(_ report: IOSMemoryWorkerRunReport) {
        guard pendingMemoryWorkerWakeDate() != nil else { return }
        let defaults = UserDefaults.standard
        let retryAttempts = report.retryScheduled > 0
            ? defaults.integer(forKey: Self.memoryWorkerRetryAttemptCountKey) + 1
            : 0
        if let next = MemoryWorkerWakeSchedulingPolicy.nextMemoryWorkerAttempt(
            now: Date(),
            report: report,
            jobBudget: Self.memoryWorkerJobBudget,
            retryAttemptCount: retryAttempts
        ) {
            defaults.set(next.timeIntervalSince1970, forKey: Self.memoryWorkerWakeDateKey)
            defaults.set(retryAttempts, forKey: Self.memoryWorkerRetryAttemptCountKey)
        } else {
            defaults.removeObject(forKey: Self.memoryWorkerWakeDateKey)
            defaults.removeObject(forKey: Self.memoryWorkerRetryAttemptCountKey)
        }
    }
    
    private func handleBackgroundRefresh(task: BGAppRefreshTask) {
        container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.3f737a0e", comment: "AUTOGEN"), metadata: [
            "taskId": task.identifier,
            "time": Date().formatted(date: .omitted, time: .complete)
        ])

        // Capture notification authorization snapshot for diagnosis (best-effort)
        Task {
            let settings = await UNUserNotificationCenter.current().notificationSettings()
            self.container.logger.log(.debug, "Scheduler", "Notification settings snapshot", metadata: [
                "authStatus": "\(settings.authorizationStatus.rawValue)",
                "alertSetting": "\(settings.alertSetting.rawValue)",
                "soundSetting": "\(settings.soundSetting.rawValue)",
                "badgeSetting": "\(settings.badgeSetting.rawValue)"
            ])
        }
        
        let completionState = BackgroundTaskCompletionState()
        
        task.expirationHandler = {
            let shouldComplete = completionState.expire()
            self.container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.6709783d", comment: "AUTOGEN"))
            if shouldComplete {
                task.setTaskCompleted(success: false)
            }
            // Re-schedule even if task expired to ensure next opportunity
            self.scheduleNextRefresh()
        }
        
        Task {
            var success = false
            defer {
                if completionState.completeIfActive() {
                    task.setTaskCompleted(success: success)
                    scheduleNextRefresh()
                }
            }
            guard tryAcquireRunLock(reason: "BGAppRefresh") else {
                success = true // not a failure, just a skip
                return
            }
            defer { releaseRunLock() }
            do {
                // Check if task already expired
                guard !completionState.expired else {
                    container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.176d4835", comment: "AUTOGEN"))
                    return
                }

                self.container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.d5152143", comment: "AUTOGEN"), metadata: [
                    "quietHours": "\(self.rules.isQuietHours())",
                    "hasURLFetcher": "\(self.container.urlFetcher != nil)"
                ])
                
                try await performSummary(isBackground: true)
                await processPendingMemoryWorkers(reason: "BGAppRefresh")
                success = true
                container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.c9e93c9b", comment: "AUTOGEN"))
            } catch {
                container.logger.log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.3a8e3c6a", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            }
            
        }
    }

    private func handleBackgroundProcessing(task: BGProcessingTask) {
        container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.ed22106a", comment: "AUTOGEN"), metadata: [
            "taskId": task.identifier,
            "time": Date().formatted(date: .omitted, time: .complete)
        ])

        let completionState = BackgroundTaskCompletionState()
        task.expirationHandler = {
            let shouldComplete = completionState.expire()
            self.container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.7e3b2c88", comment: "AUTOGEN"))
            if shouldComplete {
                task.setTaskCompleted(success: false)
            }
            self.scheduleNextRefresh()
        }

        Task {
            var success = false
            defer {
                if completionState.completeIfActive() {
                    task.setTaskCompleted(success: success)
                    scheduleNextRefresh()
                }
            }
            guard tryAcquireRunLock(reason: "BGProcessing") else {
                success = true
                return
            }
            defer { releaseRunLock() }
            guard !completionState.expired else {
                container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.aae7761e", comment: "AUTOGEN"))
                return
            }

            // Memory jobs have their own Application-owned readiness and
            // lease policy. Process them before the scheduled-URL gate below;
            // otherwise a future URL deadline can make a cold/background wake
            // return without ever claiming an already-ready memory job.
            await processPendingMemoryWorkers(reason: "BGProcessing")

            // Only execute scheduled URL actions when we are at/after the scheduled time (with tolerance).
            let now = Date()
            if let next = self.nextScheduledURLActionDate(from: now) {
                let toleranceSec: TimeInterval = 60
                if now < next.addingTimeInterval(-toleranceSec) {
                    container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.7704a080", comment: "AUTOGEN"), metadata: [
                        "now": ISO8601DateFormatter().string(from: now),
                        "nextScheduledAction": ISO8601DateFormatter().string(from: next)
                    ])
                    success = true
                    return
                }
            }

            self.container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.1c5c5747", comment: "AUTOGEN"), metadata: [
                "hasURLFetcher": "\(self.container.urlFetcher != nil)",
                "quietHours": "\(self.rules.isQuietHours())"
            ])

            let pastWindow: TimeInterval = 7 * 24 * 3600
            let futureWindow: TimeInterval = 7 * 24 * 3600
            _ = await executeScheduledURLActions(isBackground: true, pastWindow: pastWindow, futureWindow: futureWindow)
            success = true
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.1c099943", comment: "AUTOGEN"))

        }
    }
    
    private func performSummary(isBackground: Bool = false) async throws {
        let pastWindow: TimeInterval = 7 * 24 * 3600
        let futureWindow: TimeInterval = 7 * 24 * 3600

        container.logger.log(.debug, "Scheduler", "performSummary start", metadata: [
            "isBackground": "\(isBackground)",
            "pastWindowSec": String(Int(pastWindow)),
            "futureWindowSec": String(Int(futureWindow)),
            "quietHours": "\(rules.isQuietHours())"
        ])

        // 1) Execute scheduled URL actions first (fetch always; report/notify best-effort)
        let scheduledURLEvents = await executeScheduledURLActions(isBackground: isBackground, pastWindow: pastWindow, futureWindow: futureWindow)

        // 2) Fetch events from non-URL sources (avoid double fetching scheduled URL sources)
        let fetchResult = await container.fetchEventsFromDataSources(
            pastWindow: pastWindow,
            futureWindow: futureWindow,
            urlFetchPolicy: .none
        )

        let allEvents = scheduledURLEvents + fetchResult.events

        container.logger.log(.debug, "Scheduler", "Fetched events for summary", metadata: [
            "scheduledURL": String(scheduledURLEvents.count),
            "otherSources": String(fetchResult.events.count),
            "total": String(allEvents.count)
        ])
        
        guard !allEvents.isEmpty else {
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.a43d1f16", comment: "AUTOGEN"))
            return
        }
        
        // Preprocess
        let processed = preprocessor.process(allEvents)
        container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.d01983b8", comment: "AUTOGEN"), metadata: ["processed": "\(processed.count)"])
        
        // Apply rules
        let candidates = rules.filter(processed)
        container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.d157c0c2", comment: "AUTOGEN"), metadata: ["candidates": "\(candidates.count)"])
        let shouldAggregateCalendarNotifications = UserDefaults.standard.object(forKey: "aggregateCalendarNotifications") as? Bool ?? true
        let calendarCandidateKeys = Set(candidates.filter { $0.event.sourceType == .calendar }.map { $0.dedupeKey })
        
        // Generate summary with LLM when notifications can be delivered.
        guard !rules.isQuietHours() else {
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.a15945a4", comment: "AUTOGEN"), metadata: [
                "isBackground": "\(isBackground)",
                "candidates": "\(candidates.count)"
            ])
            return
        }

        // Build a ModelInput from candidates and ask the LLM for a short summary/priority list.
        let iso = ISO8601DateFormatter()
        let startStr = iso.string(from: Date().addingTimeInterval(-pastWindow))
        let endStr = iso.string(from: Date().addingTimeInterval(futureWindow))
        let dateRange = ModelInput.DateRange(start: startStr, end: endStr)
        let modelCandidates: [ModelInput.Candidate] = candidates.prefix(20).map { nc in
            return ModelInput.Candidate(
                type: nc.event.sourceType.rawValue,
                title: nc.event.title,
                name: nil,
                date: nc.event.due,
                start: nc.event.start,
                due: nc.event.due,
                relation: nil,
                notesTag: nil,
                durationMin: nil
            )
        }

        let modelInput = ModelInput(dateRange: dateRange, candidates: modelCandidates, userPrefs: [:])

        do {
            let timeout: TimeInterval = isBackground ? 15 : 30
            let llmStart = Date()
            let client = container.llmClient
            let invocation = (client as? ModelService)?.summaryInvocation(ownerID: "local-account", scenario: .eventSummary)
                ?? LLMSummaryInvocation(ownerID: "local-account", provider: client, scenario: .eventSummary)
            let llmOutput = try await invocation.generate(input: modelInput, timeout: timeout)
            try Task.checkCancellation()
            let llmDuration = Date().timeIntervalSince(llmStart)
            
            container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.67db48b7", comment: "AUTOGEN"), metadata: ["durationMs": String(format: "%.2f", llmDuration * 1000), "provider": container.llmClient is LocalProvider ? "local" : "cloud"])
            
            let today = DateFormatter.dayFormatter.string(from: Date())
            let dailySummary = DailySummary(
                date: today,
                items: llmOutput.items,
                explain: llmOutput.explain,
                generatedAt: Date(),
                lastRefresh: Date()
            )
            do {
                try container.summaryStorage.saveSummary(dailySummary)
                container.logger.log(.info, "Scheduler", "Saved daily summary", metadata: ["items": "\(llmOutput.items.count)"])
            } catch {
                container.logger.log(.error, "Scheduler", "Failed to save summary", metadata: ["error": error.localizedDescription])
            }

            // Transform LLM output using dataTransformer
            let processed: [ProcessedEvent]
            if let transformer = container.dataTransformer {
                processed = transformer.transform(modelOutput: llmOutput, sourceId: "scheduler", sourceType: .manual)
            } else {
                // Fallback: create ProcessedEvent directly from LLM items
                processed = llmOutput.items.map { item in
                    ProcessedEvent(
                        id: UUID().uuidString,
                        source: .reminder,
                        sourceType: .manual,
                        sourceId: "scheduler",
                        title: item.text,
                        date: nil,
                        start: nil,
                        due: nil,
                        priority: item.priority,
                        tags: nil,
                        dedupeKey: item.dedupeKey,
                        provenance: ["llmText": .string(item.text), "explain": .string(llmOutput.explain ?? "")],
                        metadata: [:],
                        notes: nil,
                        hash: item.text.hashValue
                    )
                }
            }

            // Persist and schedule notifications via unified services if available
            container.logger.log(.debug, "Scheduler", "Notification pipeline status", metadata: [
                "hasUnifiedStorage": "\(container.unifiedStorage != nil)"
            ])

            let aggregatedCalendarEvent: ProcessedEvent? = {
                guard shouldAggregateCalendarNotifications else { return nil }
                let calendarEvents = processed.filter { calendarCandidateKeys.contains($0.dedupeKey) }
                guard calendarEvents.count > 1 else { return nil }
                return makeAggregatedCalendarNotificationEvent(from: calendarEvents)
            }()
            let aggregatedCalendarKeys = Set(processed.filter { calendarCandidateKeys.contains($0.dedupeKey) }.map { $0.dedupeKey })
            var hasScheduledAggregatedCalendar = false

            func scheduleProcessedEvent(_ event: ProcessedEvent) async throws {
                let date: Date? = {
                    if let s = event.start, let d = ISO8601DateFormatter().date(from: s) { return d }
                    if let dStr = event.due, let d = ISO8601DateFormatter().date(from: dStr) { return d }
                    return nil
                }()
                let text = notificationText(for: event)
                let candidate = NotificationCandidate(event: event, dedupeKey: event.dedupeKey, text: text, actionType: .openCalendar)
                if let nm = container.notificationManager {
                    do { _ = try nm.post(candidate: candidate) } catch { container.logger.log(.warn, "Scheduler", "Failed to persist notification candidate before scheduling", metadata: ["error": error.localizedDescription]) }
                }
                let ok = await container.unifiedNotificationService.scheduleNotification(for: event, at: date)
                if !ok, let queue = container.candidateQueue {
                    queue.enqueue([event])
                    container.logger.log(.info, "Scheduler", "Queued notification after scheduling failure", metadata: ["dedupeKey": event.dedupeKey])
                } else {
                    container.logger.log(.debug, "Scheduler", "Scheduled notification", metadata: ["dedupeKey": event.dedupeKey])
                }
            }

            for event in processed {
                do {
                    try container.unifiedStorage?.saveProcessedEvent(event)
                } catch {
                    container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.7cb0753d", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
                }

                if shouldAggregateCalendarNotifications, aggregatedCalendarKeys.contains(event.dedupeKey) {
                    if !hasScheduledAggregatedCalendar, let aggregatedCalendarEvent {
                        do {
                            try container.unifiedStorage?.saveProcessedEvent(aggregatedCalendarEvent)
                        } catch {
                            container.logger.log(.warn, "Scheduler", "Failed to save aggregated calendar event", metadata: ["error": error.localizedDescription])
                        }
                        try await scheduleProcessedEvent(aggregatedCalendarEvent)
                        rules.markSent(aggregatedCalendarEvent.dedupeKey)
                        hasScheduledAggregatedCalendar = true

                        for dedupeKey in aggregatedCalendarKeys {
                            rules.markSent(dedupeKey)
                        }
                    }
                    continue
                }

                try await scheduleProcessedEvent(event)
                rules.markSent(event.dedupeKey)
            }
            // Use Rust scheduler to compute schedule and optionally log
            if let processedData = try? JSONEncoder().encode(processed), let scheduleRes = RustInterop.schedulerSchedule(processedData, targetHour: 8) {
                for sd in scheduleRes {
                    container.logger.log(.debug, "Scheduler", "Rust scheduler computed time", metadata: ["event": sd.event.id, "scheduledTime": sd.scheduledTime ?? "none"]) 
                }
            }
                // If there are no processed items but the LLM provided an explain text, send a summary notification
                if processed.isEmpty, let explain = llmOutput.explain, !explain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    container.logger.log(.info, "Scheduler", "LLM returned explain-only summary; creating summary notification")
                    let day = DateFormatter.dayFormatter.string(from: Date())
                    let dedupeKey = "summary:\(day):\(abs(explain.hashValue))"

                    let summaryEvent = ProcessedEvent(
                        id: UUID().uuidString,
                        source: .reminder,
                        sourceType: .manual,
                        sourceId: "scheduler_summary",
                        title: String(explain.prefix(240)),
                        date: nil,
                        start: nil,
                        due: nil,
                        priority: 0.5,
                        tags: nil,
                        dedupeKey: dedupeKey,
                        provenance: ["explain": .string(explain)],
                        metadata: ["generatedBy": "scheduler"],
                        notes: nil,
                        hash: explain.hashValue
                    )

                    let ok = await container.unifiedNotificationService.scheduleNotification(for: summaryEvent, at: nil)
                    if ok {
                        container.logger.log(.debug, "Scheduler", "Scheduled summary", metadata: ["dedupeKey": dedupeKey])
                    } else if let queue = container.candidateQueue {
                        queue.enqueue([summaryEvent])
                        container.logger.log(.info, "Scheduler", "Enqueued summary event for replay", metadata: ["dedupeKey": dedupeKey])
                    } else {
                        container.logger.log(.warn, "Scheduler", "Summary notification could not be scheduled", metadata: ["dedupeKey": dedupeKey])
                    }
                }
        } catch let pending as LLMRemoteAuxiliaryScopeBlocked {
            container.logger.log(.warn, "Scheduler", "Prior auxiliary source still owns the worker slot; no new model request", metadata: [
                "sourceHost": pending.sourceHostID, "operation": pending.operationID, "attempt": pending.attemptID
            ])
        } catch let pending as LLMRemoteAuxiliaryOutcomeUnknown {
            container.logger.log(.warn, "Scheduler", "Auxiliary source outcome remains pending; no automatic model retry or failure fallback", metadata: [
                "sourceHost": pending.sourceHostID, "operation": pending.operationID, "attempt": pending.attemptID
            ])
        } catch {
            container.logger.log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.30b349b6", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            guard !Task.isCancelled, !(error is CancellationError) else { return }
            // Preserve a bounded notification attempt when summarization fails.
            let fallbackCandidates = aggregateCalendarFallbackCandidates(candidates: Array(candidates.prefix(5)), enabled: shouldAggregateCalendarNotifications)
            for candidate in fallbackCandidates {
                _ = await container.unifiedNotificationService.scheduleNotification(for: candidate.event, at: nil)
                rules.markSent(candidate.dedupeKey)
            }
        }
    }

    private func processPendingMemoryWorkers(reason: String) async {
        do {
            let report = try await container.chatManager.processPendingMemoryWorkerReport(
                maximumJobCount: Self.memoryWorkerJobBudget
            )
            recordMemoryWorkerAttempt(report)
            container.logger.log(
                .debug,
                "Scheduler",
                "Processed deferred Memory Workers",
                metadata: [
                    "reason": reason,
                    "attempted": "\(report.attempted)",
                    "completed": "\(report.completed)",
                    "retryScheduled": "\(report.retryScheduled)",
                ]
            )
        } catch {
            recordMemoryWorkerAttempt(IOSMemoryWorkerRunReport(retryScheduled: 1))
            container.logger.log(
                .warn,
                "Scheduler",
                "Deferred Memory Worker execution failed",
                metadata: ["reason": reason, "error": error.localizedDescription]
            )
        }
    }

    /// Recovery enumerates durable occurrences, not today's due source list.
    /// Unbound work is not re-submitted here; exact Run work remains owned by Application.
    func recoverBoundAutomationExecutions() async {
        guard let store = IOSApplicationRuntime.shared, let chat = container.chatManager else { return }
        var after: String?
        do {
            while true {
                let page = try store.listPendingAutomationExecutions(afterOccurrenceID: after, limit: 128)
                guard !page.isEmpty else { break }
                for occurrenceID in page {
                    do {
                        let observed = try store.observeAutomationExecution(occurrenceID: occurrenceID)
                        guard let binding = observed.binding else { continue }
                        if observed.terminal == nil {
                            // Open user decisions are durable waits, not executable work.
                            // A later answer can make the same exact Run schedulable.
                            if observed.runStatus == "waiting_input" || observed.runStatus == "waiting_permission" { continue }
                            _ = try await chat.resumeScheduledAction(execution: observed.identity, binding: binding)
                        }
                        _ = try store.reconcileAutomationExecution(occurrenceID: occurrenceID,
                            reconciledAtMs: epochMilliseconds(Date()))
                    } catch {
                        container.logger.log(.warn, "Scheduler", "Exact automation recovery remains pending",
                            metadata: ["occurrenceId": occurrenceID, "error": error.localizedDescription])
                    }
                }
                after = page.last
            }
        } catch {
            container.logger.log(.warn, "Scheduler", "Could not enumerate pending automation executions",
                metadata: ["error": error.localizedDescription])
        }
    }

    /// Execute URL scheduled actions (URLDataSource with schedule) and produce a single report notification per source.
    ///
    /// Behavior:
    /// - Always fetch scheduled URL sources when due (so summary can include the events).
    /// - Generate report + notify + history best-effort (skips on quiet hours / failures).
    private func canonicalOccurrenceDates(_ source:URLDataSource,after:Date,through:Date,limit:Int) throws -> [Date] {
        guard let runtime=IOSApplicationRuntime.shared else {throw IOSApplicationRuntimeError.invalidResponse}
        let weekday=source.scheduledWeekday ?? Calendar.current.component(.weekday,from:Date())
        let time=source.scheduledTimeISO.flatMap(ScheduledTimeParser.parseHourMinute).map {String(format:"%02d:%02d",$0.hour,$0.minute)}
        let result=try runtime.executeAgentCommand(.object(["type":.string("query_automation_occurrences"),"input":.object([
            "automation_id":.string(source.id.uuidString),"timezone":.string(TimeZone.current.identifier),
            "frequency":.string(source.scheduleFrequency ?? "daily"),"time":time.map(AgentCoreJSONValue.string) ?? .null,
            "weekday":.signedInteger(Int64((weekday+5)%7+1)),"once_at_ms":source.scheduledDate.map { .signedInteger(epochMilliseconds($0)) } ?? .null,
            "after_ms":.signedInteger(epochMilliseconds(after)),"through_ms":.signedInteger(epochMilliseconds(through)),"limit":.signedInteger(Int64(limit))])]))
        struct Page:Decodable {struct Item:Decodable {let scheduled_for_ms:Int64};let occurrences:[Item]}
        return try JSONDecoder().decode(Page.self,from:JSONEncoder().encode(result)).occurrences.map {Date(timeIntervalSince1970:Double($0.scheduled_for_ms)/1000)}
    }

    private func executeScheduledURLActions(
        isBackground: Bool,
        pastWindow: TimeInterval,
        futureWindow: TimeInterval
        ,
        sourcesOverride: [URLDataSource]? = nil,
        respectQuietHours: Bool = true,
        dedupeKeyMode: ScheduledActionDedupeKeyMode = .daily
    ) async -> [RawEvent] {
        await recoverBoundAutomationExecutions()
        guard let fetcher = container.urlFetcher else { return [] }

        let scheduledSources = sourcesOverride ?? fetcher.getScheduledSources()
        guard !scheduledSources.isEmpty else { return [] }

        // Even if quiet hours, we still fetch so later summary (if run) has data.
        // Manual runs can override quiet-hour behavior.
        let isQuiet = respectQuietHours ? rules.isQuietHours() : false
        if isQuiet {
            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.410abcf5", comment: "AUTOGEN"), metadata: ["sources": "\(scheduledSources.count)"])
        }

        var allEvents: [RawEvent] = []
        let now = Date()

        func scheduledAnchor(for source: URLDataSource, now: Date) throws -> Date {
            if source.scheduleFrequency == "once", let target=source.scheduledDate {return target}
            guard let anchor=try canonicalOccurrenceDates(source,after:now.addingTimeInterval(-8*86400),
                through:now.addingTimeInterval(5*60),limit:16).last else {throw IOSApplicationRuntimeError.invalidResponse}
            return anchor
        }

        for source in scheduledSources {
            let runStartedAt = Date()
            let actionId = source.id.uuidString
            guard let anchor = try? ((dedupeKeyMode == .daily) ? scheduledAnchor(for: source, now: now) : runStartedAt) else {continue}
            let anchorDay = DateFormatter.dayFormatter.string(from: anchor)
            let urlRange = DateInterval(start: anchor.addingTimeInterval(-pastWindow), end: anchor.addingTimeInterval(futureWindow))
            let dedupeKey: String = {
                switch dedupeKeyMode {
                case .daily:
                    return "scheduled_action:url:\(actionId):\(anchorDay)"
                case .manualPerRun:
                    let iso = ISO8601DateFormatter()
                    let ts = iso.string(from: runStartedAt)
                    return "scheduled_action:url:\(actionId):manual:\(ts)"
                }
            }()

            guard let automationStore = IOSApplicationRuntime.shared else {
                container.logger.log(.error, "Scheduler", "Canonical automation Store is unavailable", metadata: ["actionId": actionId])
                continue
            }
            let proposedRunID = UUID().uuidString
            let leaseToken = UUID().uuidString
            let workerID = "ios:\(ProcessInfo.processInfo.processIdentifier)"
            let runStartedAtMs = epochMilliseconds(runStartedAt)
            let enqueuedRun: IOSCanonicalAutomationRun
            let executionSpec = IOSAutomationExecutionSpec(
                prompt: ScheduledURLActionAgentRequest.configuredPrompt(userPrompt: source.userPrompt, sourceDescription: source.sourceDescription),
                executionScope: .deviceLocal(hostID: "ios-device"), permissionProfile: "cautious",
                modelSelection: "use_current_default_at_admission", contextRefs: [.init(kind: "web_source", id: actionId)], skillRefs: [])
            let definition: IOSCanonicalAutomationDefinition
            let executionIdentity: IOSAutomationExecutionIdentity
            do {
                definition = try automationStore.upsertAutomationDefinition(.init(id: actionId, name: source.name,
                    scheduleFormat: "ios.scheduled_url_report", enabled: true, revision: 0, executionSpec: .agent(executionSpec),
                    metadataJSON: "{}", createdAtMs: runStartedAtMs, updatedAtMs: runStartedAtMs, deletedAtMs: nil))
                enqueuedRun = try automationStore.enqueueAutomationRun(IOSCanonicalAutomationRun(
                    id: proposedRunID,
                    automationID: actionId,
                    definitionRevision: definition.revision,
                    occurrenceKey: dedupeKey,
                    trigger: isBackground ? "background" : (dedupeKeyMode == .manualPerRun ? "manual" : "foreground"),
                    status: "queued",
                    attempt: 0,
                    workerID: nil,
                    leaseToken: nil,
                    leaseUntilMs: nil,
                    scheduledForMs: epochMilliseconds(anchor),
                    startedAtMs: nil,
                    finishedAtMs: nil,
                    nextRetryAtMs: nil,
                    resultJSON: nil,
                    errorJSON: nil,
                    createdAtMs: runStartedAtMs,
                    updatedAtMs: runStartedAtMs
                ))
                let observed = try automationStore.observeAutomationExecution(occurrenceID: enqueuedRun.id)
                executionIdentity = observed.identity
                if let binding = observed.binding {
                    if observed.terminal == nil {
                        guard let chatManager = container.chatManager else { throw AgentCoreRuntimeError.core("ChatManager is unavailable") }
                        _ = try await chatManager.resumeScheduledAction(execution: observed.identity, binding: binding)
                    }
                    _ = try automationStore.reconcileAutomationExecution(occurrenceID: enqueuedRun.id, reconciledAtMs: epochMilliseconds(Date()))
                    // A bound occurrence always uses its frozen exact Run, never Begin.
                    continue
                }
                let claimed = try automationStore.claimAutomationRun(
                    id: enqueuedRun.id,
                    workerID: workerID,
                    leaseToken: leaseToken,
                    nowMs: runStartedAtMs,
                    leaseUntilMs: runStartedAtMs + 5 * 60 * 1_000
                )
                guard claimed else {
                    container.logger.log(.debug, "Scheduler", "Automation occurrence is already owned, waiting, or completed", metadata: [
                        "actionId": actionId, "runId": enqueuedRun.id, "occurrenceKey": dedupeKey
                    ])
                    continue
                }
            } catch {
                container.logger.log(.error, "Scheduler", "Failed to enqueue canonical automation run", metadata: [
                    "actionId": actionId, "error": error.localizedDescription
                ])
                continue
            }
            let executionLease = IOSAutomationExecutionLease(occurrenceID: enqueuedRun.id, definitionRevision: enqueuedRun.definitionRevision, workerID: workerID, leaseToken: leaseToken)
            let leaseHeartbeat = Task { @MainActor [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: 30_000_000_000)
                    guard !Task.isCancelled else { break }
                    guard let self else { break }
                    let nowMs = self.epochMilliseconds(Date())
                    do {
                        guard try automationStore.heartbeatAutomationRun(
                            id: enqueuedRun.id, leaseToken: leaseToken, nowMs: nowMs,
                            leaseUntilMs: nowMs + 5 * 60 * 1_000
                        ) else {
                            self.container.logger.log(.warn, "Scheduler", "Automation occurrence lease is no longer owned",
                                metadata: ["occurrenceId": enqueuedRun.id])
                            break
                        }
                    } catch {
                        self.container.logger.log(.warn, "Scheduler", "Automation occurrence heartbeat failed; result requires canonical reconciliation",
                            metadata: ["occurrenceId": enqueuedRun.id, "error": error.localizedDescription])
                        break
                    }
                }
            }

            func persistRun(
                status: LocalJSONStorage.ScheduledActionRunRecord.Status,
                reason: String? = nil,
                failureReason: String? = nil,
                resultEventId: String? = nil,
                notificationSent: Bool? = nil,
                notificationError: String? = nil,
                modelUsed: String? = nil,
                tokensUsed: Int? = nil,
                processingDurationSec: Double? = nil,
                details: String? = nil,
                errorMessage: String? = nil,
                eventsCount: Int? = nil,
                resultJournalId: String? = nil,
                nextEligibleAt: Date? = nil,
                shouldRetry: Bool? = nil,
                retryCount: Int? = nil
            ) {
                leaseHeartbeat.cancel()
                let finishedAt = Date()
                do {
                    let observed = try automationStore.observeAutomationExecution(occurrenceID: enqueuedRun.id)
                    if observed.binding == nil {
                        _ = try automationStore.observeAutomationWorker(lease: executionLease,
                            observation: .launchFailed(detail: errorMessage ?? reason ?? "Scheduled input was not admitted"),
                            observedAtMs: epochMilliseconds(finishedAt))
                    } else if let errorMessage {
                        _ = try automationStore.observeAutomationWorker(lease: executionLease,
                            observation: .disconnected(detail: errorMessage), observedAtMs: epochMilliseconds(finishedAt))
                    }
                    // Scheduling diagnostics never write an alternate result/status.
                    _ = try automationStore.reconcileAutomationExecution(occurrenceID: enqueuedRun.id,
                        reconciledAtMs: epochMilliseconds(finishedAt))
                    NotificationCenter.default.post(name: .nolaScheduledActionRunsUpdated, object: nil)
                } catch {
                    container.logger.log(.warn, "Scheduler", "Automation observation/reconciliation failed",
                        metadata: ["error": error.localizedDescription, "actionId": actionId])
                }
            }

            // Claim eligibility above is the only retry/backoff gate. JSON history
            // remains a UI projection and must not decide whether work executes.
            let computedRetryCount: Int? = (try? automationStore.loadAutomationRun(id: enqueuedRun.id))
                .flatMap { $0 }
                .map { max(0, Int($0.attempt) - 1) }

            let fetched: [RawEvent]
            do {
                fetched = try await fetcher.fetchEvents(from: source)
            } catch {
                container.logger.log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.9f973728", comment: "AUTOGEN"), metadata: ["source": source.name, "error": error.localizedDescription])
                // Smart retry logic: network/timeout errors are retryable
                let shouldRetry = isRetryableError(error)
                let nextEligible = shouldRetry ? Date().addingTimeInterval(3600) : nil  // retry in 1h if transient
                persistRun(status: .failed, reason: "fetch_failed", details: nil, errorMessage: error.localizedDescription, eventsCount: 0, nextEligibleAt: nextEligible, shouldRetry: shouldRetry, retryCount: shouldRetry ? (computedRetryCount ?? 1) : computedRetryCount)
                continue
            }

                let filtered = fetched.filter { ev in
                    guard let d = ev.date else { return true }
                    return urlRange.contains(d)
                }
                allEvents.append(contentsOf: filtered)

                // Best-effort report generation (skip when quiet hours)
                if isQuiet {
                    persistRun(status: .skipped, reason: "quiet_hours", details: "events=\(filtered.count)", eventsCount: filtered.count, retryCount: computedRetryCount)
                    continue
                }

                // Build model input from fetched URL events
                let preprocessed = preprocessor.process(filtered)
                guard !preprocessed.isEmpty else {
                    container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.1118cd82", comment: "AUTOGEN"), metadata: ["source": source.name])
                    persistRun(status: .skipped, reason: "no_events", details: "events=0", eventsCount: 0, retryCount: computedRetryCount)
                    continue
                }

                let iso = ISO8601DateFormatter()
                let startStr = iso.string(from: anchor.addingTimeInterval(-pastWindow))
                let endStr = iso.string(from: anchor.addingTimeInterval(futureWindow))
                let dateRange = ModelInput.DateRange(start: startStr, end: endStr)
                let candidates: [ModelInput.Candidate] = preprocessed.prefix(20).map { ev in
                    let contentSnippet: String? = {
                        guard ev.source == .url else { return nil }
                        guard let n = ev.notes?.trimmingCharacters(in: .whitespacesAndNewlines), !n.isEmpty else { return nil }
                        return String(n.prefix(2500))
                    }()

                    let titleWithContent: String = {
                        guard let contentSnippet else { return ev.title }
                        return ev.title + "\n\nCONTENT:\n" + contentSnippet
                    }()

                    return ModelInput.Candidate(
                        type: ev.source.rawValue,
                        title: titleWithContent,
                        name: nil,
                        date: ev.date.map { iso.string(from: $0) },
                        start: ev.start,
                        due: ev.due,
                        relation: ev.metadata["relation"],
                        notesTag: ev.tags?.first,
                        durationMin: ev.metadata["duration"].flatMap { Int(Double($0) ?? 0.0 / 60.0) }
                    )
                }

                let agentRequest: ScheduledURLActionAgentRequest
                do {
                    agentRequest = try ScheduledURLActionAgentRequest.make(
                        actionId: actionId,
                        sourceName: source.name,
                        sourceURL: source.url,
                        userPrompt: source.userPrompt,
                        sourceDescription: source.sourceDescription,
                        dedupeKey: dedupeKey,
                        observedAt: Date(),
                        dateRange: dateRange,
                        candidates: candidates
                    )
                } catch {
                    persistRun(
                        status: .failed,
                        reason: "reference_encoding_failed",
                        details: "events=\(filtered.count)",
                        errorMessage: error.localizedDescription,
                        eventsCount: filtered.count,
                        shouldRetry: false,
                        retryCount: computedRetryCount
                    )
                    continue
                }

                let didOverrideLocalModel = applyPreferredLocalModelIfNeeded(for: source)
                let effectiveModelId = effectiveModelIdentifier(for: source)
                defer {
                    if didOverrideLocalModel {
                        container.reconfigureIfPossible()
                    }
                }

                let llmStart = Date()
                do {
                    guard let chatManager = container.chatManager else {
                        throw AgentCoreRuntimeError.core("ChatManager is unavailable")
                    }
                    let output = try await chatManager.processScheduledAction(
                        agentRequest.prompt,
                        execution: executionIdentity,
                        lease: executionLease,
                        spec: executionSpec,
                        referenceComponents: [agentRequest.referenceComponent]
                    )
                    guard let executionReceipt = try automationStore.reconcileAutomationExecution(
                        occurrenceID: enqueuedRun.id, reconciledAtMs: epochMilliseconds(Date())
                    ) else { throw AgentCoreRuntimeError.core("Scheduled Run has no committed result; preserve its binding for recovery") }
                    let duration = Date().timeIntervalSince(llmStart)

                    // Save result as Journal entry for later review (not just notification)
                    var journalId: String? = nil
                    if let storage = container.localJSONStorage {
                        let journalTitle = "\(source.name) · \(anchorDay)"
                        let journalText = output
                        let id = UUID().uuidString
                        let journal = LocalJSONStorage.JournalRecord(
                            id: id,
                            title: journalTitle,
                            content: journalText,
                            mood: nil,
                            tags: ["scheduled_action", "url_report", source.name],
                            createdAt: runStartedAt,
                            updatedAt: runStartedAt
                        )
                        do {
                            try storage.saveJournal(journal)
                            journalId = journal.id
                            container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.43565123", comment: "AUTOGEN"), metadata: ["journalId": journal.id, "source": source.name])
                        } catch {
                            container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.7db6318f", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
                        }
                    }

                    // Build a single report text (user-visible final result)
                    let trimmedOutput = output.trimmingCharacters(in: .whitespacesAndNewlines)
                    let reportText = trimmedOutput.isEmpty
                        ? NSLocalizedString("autogen.Scheduler.04a28c8c", comment: "AUTOGEN")
                        : trimmedOutput

                    // Avoid duplicate sends within throttle window
                    if rules.wasRecentlySent(dedupeKey) {
                        container.logger.log(.debug, "Scheduler", NSLocalizedString("autogen.Scheduler.f7c89d47", comment: "AUTOGEN"), metadata: ["dedupeKey": dedupeKey, "source": source.name])
                        persistRun(status: .skipped, reason: "deduped", details: "events=\(filtered.count)", retryCount: computedRetryCount)
                        continue
                    }

                    // Create a synthetic report event so NotificationDetailView can display it
                    let priority: Double = 0.5
                    let reportEvent = ProcessedEvent(
                        id: executionReceipt.notificationID,
                        source: .url,
                        sourceType: .url,
                        sourceId: source.id.uuidString,
                        title: "\(source.name)",
                        date: nil,
                        start: nil,
                        due: nil,
                        priority: priority,
                        tags: nil,
                        dedupeKey: dedupeKey,
                        provenance: ["explain": .string(reportText)],
                        metadata: [
                            "generatedBy": "scheduler_scheduled_action",
                            "url": source.url,
                            "sourceName": source.name,
                            "sourceId": source.id.uuidString
                        ],
                        notes: nil,
                        hash: reportText.hashValue
                    )

                do {
                    try container.unifiedStorage?.saveProcessedEvent(reportEvent)
                } catch {
                    container.logger.log(.warn, "Scheduler", NSLocalizedString("autogen.Scheduler.b6c248fa", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription, "dedupeKey": dedupeKey])
                }

                    let rawResponse: String? = output
                    let tokensUsed: Int? = Telemetry.estimateTokens(from: output)

                // Append history entry
                    let entry = URLDataSource.HistoryEntry(
                        dedupeKey: dedupeKey,
                        title: String(reportText.prefix(240)),
                        details: "events=\(filtered.count)",
                        eventId: reportEvent.id,
                        priority: priority,
                        tokensUsed: tokensUsed,
                        processingDuration: duration,
                        modelUsed: effectiveModelId,
                        rawResponse: rawResponse
                    )
                    fetcher.appendHistoryEntry(forURL: source.url, entry: entry)

                // Send one notification
                let candidate = NotificationCandidate(event: reportEvent, dedupeKey: dedupeKey, text: reportText, actionType: .openCalendar)
                    let delivered = await container.unifiedNotificationService.scheduleNotification(for: candidate.event, at: nil)
                    if delivered {
                        _ = try automationStore.ackAutomationNotification(occurrenceID: enqueuedRun.id, notificationID: executionReceipt.notificationID, deliveredAtMs: epochMilliseconds(Date()))
                        rules.markSent(dedupeKey)
                    }
                        container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.9046455a", comment: "AUTOGEN"), metadata: ["source": source.name, "dedupeKey": dedupeKey, "durationMs": String(format: "%.2f", duration * 1000)])
                        persistRun(
                            status: .success,
                            resultEventId: reportEvent.id,
                            notificationSent: delivered,
                            modelUsed: effectiveModelId,
                            tokensUsed: tokensUsed,
                            processingDurationSec: duration,
                            details: "events=\(filtered.count)",
                            eventsCount: filtered.count,
                            resultJournalId: journalId,
                            retryCount: computedRetryCount
                        )
                } catch {
                    let duration = Date().timeIntervalSince(llmStart)
                    container.logger.log(.error, "Scheduler", NSLocalizedString("autogen.Scheduler.1876ea54", comment: "AUTOGEN"), metadata: [
                        "source": source.name,
                        "dedupeKey": dedupeKey,
                        "durationMs": String(format: "%.2f", duration * 1000),
                        "error": error.localizedDescription
                    ])
                    let shouldRetry = isRetryableError(error)
                    let nextEligible = shouldRetry ? Date().addingTimeInterval(3600) : nil
                    persistRun(
                        status: .failed,
                        reason: "llm_failed",
                        details: "events=\(filtered.count)",
                        errorMessage: error.localizedDescription,
                        eventsCount: filtered.count,
                        nextEligibleAt: nextEligible,
                        shouldRetry: shouldRetry,
                        retryCount: shouldRetry ? (computedRetryCount ?? 1) : computedRetryCount
                    )
                    continue
                }
        }

        return allEvents
    }

    private enum ScheduledActionDedupeKeyMode {
        case daily
        case manualPerRun
    }

    nonisolated private func epochMilliseconds(_ date: Date) -> Int64 {
        Int64((date.timeIntervalSince1970 * 1_000).rounded())
    }

    private func makeAggregatedCalendarNotificationEvent(from events: [ProcessedEvent]) -> ProcessedEvent {
        let sortedEvents = events.sorted { lhs, rhs in
            let l = lhs.date ?? .distantFuture
            let r = rhs.date ?? .distantFuture
            return l < r
        }
        let count = sortedEvents.count
        let title = String(format: NSLocalizedString("notification.calendar.aggregate.title_format", comment: "Calendar aggregate notification title"), count)
        let preview = sortedEvents.map { $0.title }.prefix(2).joined(separator: "、")
        let bodyTemplate = NSLocalizedString("notification.calendar.aggregate.body_format", comment: "Calendar aggregate notification body")
        let body = preview.isEmpty ? bodyTemplate : String(format: bodyTemplate, preview)
        let earliestDate = sortedEvents.compactMap { $0.date }.min()
        let dedupeSeed = sortedEvents.map { $0.dedupeKey }.sorted().joined(separator: "|")
        let dedupeKey = DedupeKeyGenerator.generate(source: "calendar_aggregate", title: dedupeSeed, date: earliestDate ?? Date(), sourceId: "scheduler")
        let maxPriority = sortedEvents.map(\.priority).max() ?? 0.7

        return ProcessedEvent(
            id: UUID().uuidString,
            source: .calendar,
            sourceType: .calendar,
            sourceId: "scheduler_calendar_aggregate",
            title: title,
            date: earliestDate,
            start: nil,
            due: nil,
            priority: maxPriority,
            tags: nil,
            dedupeKey: dedupeKey,
            provenance: [
                "summary": .string(body),
                "aggregatedCount": .number(Double(count))
            ],
            metadata: ["aggregated": "true"],
            notes: nil,
            hash: title.hashValue
        )
    }

    private func aggregateCalendarFallbackCandidates(candidates: [NotificationCandidate], enabled: Bool) -> [NotificationCandidate] {
        guard enabled else { return candidates }
        let calendar = candidates.filter { $0.event.sourceType == .calendar }
        guard calendar.count > 1 else { return candidates }

        let nonCalendar = candidates.filter { $0.event.sourceType != .calendar }
        let aggregatedEvent = makeAggregatedCalendarNotificationEvent(from: calendar.map { $0.event })
        let text = notificationText(for: aggregatedEvent)
        let aggregated = NotificationCandidate(event: aggregatedEvent, dedupeKey: aggregatedEvent.dedupeKey, text: text, actionType: .openCalendar)
        return [aggregated] + nonCalendar
    }

    private func notificationText(for event: ProcessedEvent) -> String {
        guard let summary = event.provenance?["summary"] else { return event.title }
        switch summary {
        case .string(let value): return value
        case .number(let value): return String(value)
        case .bool(let value): return value ? "true" : "false"
        case .null, .array, .object: return event.title
        }
    }

    private func effectiveModelIdentifier(for source: URLDataSource) -> String? {
        let preferred = source.preferredModel?.trimmingCharacters(in: .whitespacesAndNewlines)
        if let preferred, !preferred.isEmpty {
            return preferred
        }
        return container.chatManager?.getCurrentModel()?.id
    }

    private func applyPreferredLocalModelIfNeeded(for source: URLDataSource) -> Bool {
        guard let preferredRaw = source.preferredModel?.trimmingCharacters(in: .whitespacesAndNewlines), !preferredRaw.isEmpty else { return false }
        guard let modelService = container.llmClient as? ModelService else { return false }

        let desiredModel: LMModel? = {
            if let mType = ModelDownloader.ModelType(rawValue: preferredRaw),
               let mlxName = mType.mlxModelName,
               let lm = MLXService.availableModels.first(where: { $0.name == mlxName }) {
                return lm
            }

            if let mTypeByMLX = ModelDownloader.ModelType.allCases.first(where: { $0.mlxModelName == preferredRaw }),
               let mlxName = mTypeByMLX.mlxModelName,
               let lm = MLXService.availableModels.first(where: { $0.name == mlxName }) {
                return lm
            }

            return MLXService.availableModels.first(where: { $0.id == preferredRaw || $0.name == preferredRaw })
        }()

        guard let desiredModel else { return false }
        modelService.useLocalProvider(model: desiredModel)
        return true
    }

    /// Determine if an error is retryable (network/timeout) vs permanent (auth/parse)
    private func isRetryableError(_ error: Error) -> Bool {
        if let urlError = error as? URLError {
            switch urlError.code {
            case .timedOut, .cannotFindHost, .cannotConnectToHost, .networkConnectionLost, .notConnectedToInternet:
                return true
            case .badServerResponse, .badURL, .unsupportedURL:
                return false
            default:
                return false
            }
        }
        if let nolaError = error as? NolaError {
            switch nolaError {
            case .llm(.network):
                return true
            case .llm(.rateLimited):
                return true  // 429 should retry later
            default:
                return false
            }
        }
        return false  // unknown errors: don't retry by default
    }

    /// Execute scheduled analysis for non-URL data sources (contacts/calendar/reminders snapshot).
    /// Future extension: periodic digest of all data source changes, save as Journal.
    public func executeScheduledDataSourceAnalysis() async -> Bool {
        container.logger.log(.info, "Scheduler", NSLocalizedString("autogen.Scheduler.7b6411d9", comment: "AUTOGEN"))
        // Placeholder: fetch from all enabled non-URL sources, compare with last snapshot,
        // generate a summary, save as Journal, optionally notify.
        return false
    }
    
    private func sendNotification(_ candidate: NotificationCandidate) async throws {
        // Persist to centralized NotificationManager if available
        if let nm = container.notificationManager {
            do {
                _ = try nm.post(candidate: candidate)
            } catch {
                container.logger.log(.warn, "Scheduler", "Failed to persist notification candidate", metadata: ["error": error.localizedDescription])
            }
        }
        _ = await container.unifiedNotificationService.scheduleNotification(for: candidate.event, at: nil)
    }
}
