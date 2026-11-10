// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements HealthDataSource logic in the iOS service layer.
// - Interfaces:
//   - HealthDataSource
//   - HKWorkoutActivityType
// - APIs:
//   - none
//
//
//  HealthDataSource.swift
//  Nola
//
//  Adapter for HealthKit to provide aggregated health-derived events.
//

import Foundation
import HealthKit
import Combine

private actor HealthDataSyncCoordinator {
    private struct CachedResult {
        let range: DateInterval
        let createdAt: Date
        let events: [RawEvent]
    }

    private struct PendingResult {
        let id: UUID
        let range: DateInterval
        let task: Task<[RawEvent], Error>
    }

    private var cachedResults: [CachedResult] = []
    private var pendingResults: [UUID: PendingResult] = [:]
    private var generation: UInt64 = 0
    private let cacheLifetime: TimeInterval = 45

    func events(
        for range: DateInterval,
        loader: @Sendable @escaping () async throws -> [RawEvent]
    ) async throws -> [RawEvent] {
        let now = Date()
        cachedResults.removeAll { now.timeIntervalSince($0.createdAt) >= cacheLifetime }
        if let cached = cachedResults.first(where: { matches($0.range, range) }) {
            return cached.events
        }

        if let pending = pendingResults.values.first(where: { matches($0.range, range) }) {
            return try await pending.task.value
        }

        let id = UUID()
        let requestGeneration = generation
        let task = Task { try await loader() }
        pendingResults[id] = PendingResult(id: id, range: range, task: task)
        do {
            let events = try await task.value
            pendingResults.removeValue(forKey: id)
            guard requestGeneration == generation else {
                throw CancellationError()
            }
            cachedResults.removeAll { matches($0.range, range) }
            cachedResults.append(CachedResult(range: range, createdAt: Date(), events: events))
            if cachedResults.count > 4 {
                cachedResults.removeFirst(cachedResults.count - 4)
            }
            return events
        } catch {
            pendingResults.removeValue(forKey: id)
            throw error
        }
    }

    func invalidate() {
        generation &+= 1
        cachedResults.removeAll()
        pendingResults.values.forEach { $0.task.cancel() }
        pendingResults.removeAll()
    }

    private func matches(_ lhs: DateInterval, _ rhs: DateInterval) -> Bool {
        abs(lhs.duration - rhs.duration) < 60 &&
            abs(lhs.end.timeIntervalSince(rhs.end)) < 60
    }
}

private actor HealthDataChangeNotifier {
    private var pendingNotification: Task<Void, Never>?

    func schedule() {
        pendingNotification?.cancel()
        pendingNotification = Task {
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                NotificationCenter.default.post(name: .nolaHealthDataDidChange, object: nil)
            }
        }
    }
}

/// Health data source adapter. Returns lightweight aggregated RawEvent items
/// for inclusion in the common preprocessing pipeline.
public final class HealthDataSource: DataSource, @unchecked Sendable {
    private static let syncCoordinator = HealthDataSyncCoordinator()
    private static let changeNotifier = HealthDataChangeNotifier()
    private static let authorizationRequestCompletedKey = "health.authorization.request_completed"

    public static var hasCompletedAuthorizationRequest: Bool {
        UserDefaults.standard.bool(forKey: authorizationRequestCompletedKey)
    }
    public enum ReadAuthorizationState: Equatable {
        case notDetermined
        case authorized
        case denied
        case unavailable
    }

    private let healthStore: HKHealthStore
    private let defaultReadTypes: Set<HKObjectType>
    private let logger = Logger()
    private let observerLock = NSLock()
    private var observerQueries: [HKObserverQuery] = []

    public init() {
        self.healthStore = HKHealthStore()
        self.defaultReadTypes = Self.makeDefaultReadTypes()
    }

    public func fetchEvents(in range: DateInterval) async throws -> [RawEvent] {
        let normalizedRange = Self.normalizedHealthRange(range)
        return try await Self.syncCoordinator.events(for: normalizedRange) { [self] in
            try await loadEvents(in: normalizedRange)
        }
    }

    public func invalidateSyncCache() async {
        await Self.syncCoordinator.invalidate()
    }

    private static func normalizedHealthRange(_ range: DateInterval) -> DateInterval {
        let now = Date()
        if range.end > now, range.start >= now.addingTimeInterval(-60) {
            return DateInterval(start: now.addingTimeInterval(-range.duration), end: now)
        }
        let end = min(range.end, now)
        if range.start < end {
            return DateInterval(start: range.start, end: end)
        }
        return DateInterval(start: end.addingTimeInterval(-7 * 24 * 3600), end: end)
    }

    private static func coveredDayCount(in range: DateInterval) -> Int {
        max(1, Int(ceil(range.duration / (24 * 60 * 60))))
    }

    private func loadEvents(in range: DateInterval) async throws -> [RawEvent] {
        guard HKHealthStore.isHealthDataAvailable() else {
            logger.log(.warn, "HealthDataSource", "HealthKit not available on this device")
            return []
        }

        let enableStepGuidance = UserDefaults.standard.object(forKey: "enableStepReminders") as? Bool ?? true

        // DIRECT READ MODE: attempt to read data without pre-checking authorization
        // (Per request: replace guarded flow so app attempts reads directly.)
        // Define read types we may request
        let stepType = HKObjectType.quantityType(forIdentifier: .stepCount)
        let sleepType = HKObjectType.categoryType(forIdentifier: .sleepAnalysis)
        let heartRateType = HKObjectType.quantityType(forIdentifier: .heartRate)
        var events: [RawEvent] = []
        
        
        if let step = stepType {
            do {
                logger.log(.debug, "HealthDataSource", "Querying steps", metadata: ["start": range.start.description, "end": range.end.description])
                
                let days = Self.coveredDayCount(in: range)
                
                let totalSteps = (try await sumQuantity(step, unit: HKUnit.count(), range: range)) ?? 0
                let avgSteps = totalSteps / Double(days)
                
                logger.log(.debug, "HealthDataSource", "Steps query completed", metadata: ["total": String(Int(totalSteps)), "avg": String(Int(avgSteps)), "days": String(days)])
                
                if avgSteps > 0 {
                    let id = "health-steps-\(Int(range.start.timeIntervalSince1970))"
                    var analysis = ""
                    var notes: String? = nil
                    
                    if avgSteps < 5000 {
                        analysis = String(format: NSLocalizedString("autogen.HealthDataSource.f4335811", comment: "AUTOGEN"), Int(avgSteps))
                        notes = NSLocalizedString("autogen.HealthDataSource.5609f5e3", comment: "AUTOGEN")
                    } else if avgSteps < 7000 {
                        analysis = String(format: NSLocalizedString("autogen.HealthDataSource.f4335811", comment: "AUTOGEN"), Int(avgSteps))
                        notes = NSLocalizedString("autogen.HealthDataSource.6c1f3d50", comment: "AUTOGEN")
                    } else if avgSteps < 10000 {
                        analysis = String(format: NSLocalizedString("autogen.HealthDataSource.f4335811", comment: "AUTOGEN"), Int(avgSteps))
                        notes = NSLocalizedString("autogen.HealthDataSource.b4cd805d", comment: "AUTOGEN")
                    } else {
                        analysis = String(format: NSLocalizedString("autogen.HealthDataSource.f4335811", comment: "AUTOGEN"), Int(avgSteps))
                        notes = NSLocalizedString("autogen.HealthDataSource.7eaff2a0", comment: "AUTOGEN")
                    }
                    if !enableStepGuidance {
                        notes = nil
                    }
                    
                    let meta: [String: String] = ["avg": "\(Int(avgSteps))", "total": "\(Int(totalSteps))", "days": "\(days)", "unit": "steps", "type": "steps"]
                    events.append(RawEvent(id: id, source: .health, title: analysis, date: range.end, notes: notes, metadata: meta))
                }
            } catch let error as NSError {
                logger.log(.warn, "HealthDataSource", "Error fetching steps", metadata: ["code": String(error.code), "message": error.localizedDescription])
            }
        }

        if let hr = heartRateType {
            do {
                let avgHR = (try await averageQuantity(hr, unit: HKUnit(from: "count/min"), range: range)) ?? 0
                if avgHR > 0 {
                    let id = "health-hr-\(Int(range.start.timeIntervalSince1970))"
                    let analysis = String(format: NSLocalizedString("autogen.HealthDataSource.06fb0a82", comment: "AUTOGEN"), Int(avgHR))
                    var notes: String? = nil
                    
                    if avgHR < 60 {
                        notes = NSLocalizedString("autogen.HealthDataSource.9856f308", comment: "AUTOGEN")
                    } else if avgHR > 100 {
                        notes = NSLocalizedString("autogen.HealthDataSource.fe09efe7", comment: "AUTOGEN")
                    } else if avgHR >= 60 && avgHR <= 80 {
                        notes = NSLocalizedString("autogen.HealthDataSource.eba7d4ca", comment: "AUTOGEN")
                    }
                    
                    let meta: [String: String] = ["value": "\(Int(avgHR))", "unit": "bpm", "type": "heart_rate"]
                    events.append(RawEvent(id: id, source: .health, title: analysis, date: range.end, notes: notes, metadata: meta))
                }
            } catch {
                logger.log(.warn, "HealthDataSource", "Error fetching heart rate", metadata: ["error": error.localizedDescription])
            }
        }

        if let sleep = sleepType {
            do {
                let days = Self.coveredDayCount(in: range)
                
                let sleepSeconds = (try await sumCategoryDurations(sleep, range: range)) ?? 0
                if sleepSeconds > 0 {
                    let avgHours = (Double(sleepSeconds) / 3600.0) / Double(days)
                    let id = "health-sleep-\(Int(range.start.timeIntervalSince1970))"
                    let analysis = String(format: NSLocalizedString("autogen.HealthDataSource.7ab5bb3c", comment: "AUTOGEN"), avgHours)
                    var notes: String? = nil
                    
                    if avgHours < 6 {
                        notes = NSLocalizedString("autogen.HealthDataSource.71634d69", comment: "AUTOGEN")
                    } else if avgHours < 7 {
                        notes = NSLocalizedString("autogen.HealthDataSource.ad7e29fa", comment: "AUTOGEN")
                    } else if avgHours >= 7 && avgHours <= 9 {
                        notes = NSLocalizedString("autogen.HealthDataSource.1bdf2fda", comment: "AUTOGEN")
                    } else {
                        notes = NSLocalizedString("autogen.HealthDataSource.769d2c90", comment: "AUTOGEN")
                    }
                    
                    let meta: [String: String] = ["avg": String(format: "%.1f", avgHours), "total": String(format: "%.1f", Double(sleepSeconds)/3600.0), "days": "\(days)", "unit": "hours", "type": "sleep"]
                    events.append(RawEvent(id: id, source: .health, title: analysis, date: range.end, notes: notes, metadata: meta))
                }
            } catch {
                logger.log(.warn, "HealthDataSource", "Error fetching sleep", metadata: ["error": error.localizedDescription])
            }
        }

        if true {
            do {
                if let last = try await mostRecentWorkout(range: range), last.duration > 0 {
                    let id = "health-workout-\(Int(last.startDate.timeIntervalSince1970))"
                    let durationMin = Int(last.duration/60)
                    let analysis = String(format: NSLocalizedString("autogen.HealthDataSource.c72bfab1", comment: "AUTOGEN"), last.workoutActivityType.name, durationMin)
                    var notes: String? = nil
                    
                    let daysSinceWorkout = Calendar.current.dateComponents([.day], from: last.startDate, to: Date()).day ?? 0
                    
                    if daysSinceWorkout > 3 {
                        notes = "您已经 \(daysSinceWorkout) 天没有锻炼了。建议每周进行至少150分钟中等强度的有氧运动，保持身体活力。"
                    } else if durationMin >= 30 {
                        notes = NSLocalizedString("autogen.HealthDataSource.1afa6cc5", comment: "AUTOGEN")
                    } else {
                        notes = NSLocalizedString("autogen.HealthDataSource.0be8f3b7", comment: "AUTOGEN")
                    }
                    
                    let meta: [String: String] = ["type": last.workoutActivityType.name, "duration": "\(Int(last.duration))", "durationMin": "\(durationMin)", "daysSince": "\(daysSinceWorkout)"]
                    events.append(RawEvent(id: id, source: .health, title: analysis, date: last.startDate, notes: notes, metadata: meta))
                }
            } catch {
                logger.log(.warn, "HealthDataSource", "Error fetching workouts", metadata: ["error": error.localizedDescription])
            }
        }

        return events
    }

    // Per Apple docs: Apps should NOT check authorization status.
    // HealthKit automatically returns only authorized data when reading.

    /// Fetch raw samples (fallback when statistics queries fail)
    private func fetchRawSamples(for type: HKSampleType, range: DateInterval, limit: Int) async throws -> [HKSample] {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<[HKSample], Error>) in
            let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: [])
            let query = HKSampleQuery(sampleType: type, predicate: predicate, limit: limit, sortDescriptors: nil) { _, samples, error in
                if let e = error { cont.resume(throwing: e); return }
                cont.resume(returning: samples ?? [])
            }
            healthStore.execute(query)
        }
    }

    private static func makeDefaultReadTypes() -> Set<HKObjectType> {
        var types = Set<HKObjectType>()
        if let stepType = HKObjectType.quantityType(forIdentifier: .stepCount) { types.insert(stepType) }
        if let distanceType = HKObjectType.quantityType(forIdentifier: .distanceWalkingRunning) { types.insert(distanceType) }
        if let activeEnergyType = HKObjectType.quantityType(forIdentifier: .activeEnergyBurned) { types.insert(activeEnergyType) }
        if let exerciseType = HKObjectType.quantityType(forIdentifier: .appleExerciseTime) { types.insert(exerciseType) }
        if let sleepType = HKObjectType.categoryType(forIdentifier: .sleepAnalysis) { types.insert(sleepType) }
        if let heartRateType = HKObjectType.quantityType(forIdentifier: .heartRate) { types.insert(heartRateType) }
        if let restingHeartRateType = HKObjectType.quantityType(forIdentifier: .restingHeartRate) { types.insert(restingHeartRateType) }
        types.insert(HKObjectType.workoutType())
        return types
    }

    // MARK: - Helpers

    private func isHealthKitReadAuthorizationError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == HKErrorDomain else { return false }
        guard let code = HKError.Code(rawValue: nsError.code) else { return false }
        switch code {
        case .errorAuthorizationDenied, .errorAuthorizationNotDetermined:
            return true
        default:
            return false
        }
    }

    private func isHealthKitNoDataError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == HKErrorDomain else { return false }
        guard let code = HKError.Code(rawValue: nsError.code) else { return false }
        return code == .errorNoData
    }

    private func canReadAnySampleType(_ types: Set<HKObjectType>) async -> Bool {
        // Probe a short range; for read access, an empty result is still success.
        let now = Date()
        let range = DateInterval(start: now.addingTimeInterval(-24 * 60 * 60), end: now)

        for type in types {
            guard let sampleType = type as? HKSampleType else { continue }
            do {
                _ = try await fetchRawSamples(for: sampleType, range: range, limit: 1)
                return true
            } catch {
                if isHealthKitReadAuthorizationError(error) {
                    continue
                }
                logger.log(.warn, "HealthDataSource", "Read probe failed (non-auth error)", metadata: [
                    "type": String(describing: type),
                    "error": error.localizedDescription
                ])
            }
        }

        return false
    }

    /// Request authorization for the given read types. Public so callers (UI/ServiceContainer) can prompt.
    public func requestAuthorization(read types: Set<HKObjectType>) async throws -> Bool {
        logger.log(.info, "HealthDataSource", "Authorization request started", metadata: ["typeCount": String(types.count)])
        
        // First, check if HKHealthStore is available
        let isAvailable = HKHealthStore.isHealthDataAvailable()
        logger.log(.debug, "HealthDataSource", "HealthKit availability check", metadata: ["available": isAvailable ? "true" : "false"])
        if !isAvailable {
            logger.log(.error, "HealthDataSource", "HealthKit not available on this device")
            return false
        }
        
        // NOTE: `authorizationStatus(for:)` is the *share/write* authorization state.
        // HealthKit does not provide a reliable per-type API for *read* authorization status.
        // We keep this log only as a diagnostic hint.
        logger.log(.debug, "HealthDataSource", "Share (write) status snapshot before request")
        for type in types {
            let status = healthStore.authorizationStatus(for: type)
            logger.log(.debug, "HealthDataSource", "Type share status", metadata: [
                "type": String(describing: type).split(separator: ".").last.map(String.init) ?? "unknown",
                "status": String(status.rawValue)
            ])
        }
        
        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.main.async { [weak self] in
                guard let self = self else {
                    continuation.resume(throwing: NSError(domain: "HealthDataSource", code: -1, userInfo: [NSLocalizedDescriptionKey: L10n.string("health_data_source.error.deallocated")]))
                    return
                }
                
                self.logger.log(.debug, "HealthDataSource", "Calling requestAuthorization on main thread")
                
                // Use the standard HealthKit authorization API
                self.healthStore.requestAuthorization(toShare: [], read: types) { success, error in
                    self.logger.log(.debug, "HealthDataSource", "Authorization callback received", metadata: ["success": success ? "true" : "false"])
                    
                    if let error = error {
                        self.logger.log(.error, "HealthDataSource", "Authorization error", metadata: ["message": error.localizedDescription, "code": String((error as NSError).code)])
                        continuation.resume(throwing: error)
                        return
                    }

                    guard success else {
                        self.logger.log(.info, "HealthDataSource", "Authorization request failed", metadata: ["success": "false"])
                        continuation.resume(returning: false)
                        return
                    }

                    // HealthKit does not reveal per-type read authorization.
                    // Completion means the authorization sheet was handled;
                    // direct metric reads determine which data is available.
                    self.logger.log(.info, "HealthDataSource", "Authorization request completed", metadata: [
                        "success": "true"
                    ])
                    UserDefaults.standard.set(true, forKey: Self.authorizationRequestCompletedKey)
                    continuation.resume(returning: true)
                }
            }
        }
    }
    
    /// Convenience: request authorization for the default set of Health types used by this adapter.
    public func requestAuthorization() async throws -> Bool {
        return try await requestAuthorization(read: defaultReadTypes)
    }

    public func currentReadAuthorizationState() async -> ReadAuthorizationState {
        guard HKHealthStore.isHealthDataAvailable() else {
            return .unavailable
        }

        // HealthKit intentionally does not expose read authorization. Once the
        // system sheet has completed, getRequestStatusForAuthorization may still
        // return shouldRequest when a newly supported metric is added. Treating
        // that as "not determined" regresses already configured users and blocks
        // the dashboard even though existing metrics remain readable.
        if Self.hasCompletedAuthorizationRequest {
            return .authorized
        }

        return await withCheckedContinuation { continuation in
            healthStore.getRequestStatusForAuthorization(toShare: [], read: defaultReadTypes) { status, error in
                guard error == nil else {
                    continuation.resume(returning: .notDetermined)
                    return
                }
                switch status {
                case .unnecessary:
                    UserDefaults.standard.set(true, forKey: Self.authorizationRequestCompletedKey)
                    continuation.resume(returning: .authorized)
                case .shouldRequest, .unknown:
                    continuation.resume(returning: .notDetermined)
                @unknown default:
                    continuation.resume(returning: .notDetermined)
                }
            }
        }
    }

    /// Return a summary of current authorization statuses for the default read types.
    /// Key: readable type description, Value: current `HKAuthorizationStatus`.
    public func authorizationSummary() -> [String: HKAuthorizationStatus] {
        var result: [String: HKAuthorizationStatus] = [:]
        for type in defaultReadTypes {
            // NOTE: This is share/write status, not read status.
            let status = healthStore.authorizationStatus(for: type)
            let name = String(describing: type)
            result[name] = status
        }
        return result
    }

    /// Run diagnostics: for each default read type, report authorization and sample counts (last `range`).
    /// Useful to debug why authorized apps still see no data.
    public func runDiagnostics(range: DateInterval = DateInterval(start: Date().addingTimeInterval(-7*24*3600), end: Date())) async -> [String: String] {
        var report: [String: String] = [:]

        let list = Self.defaultReadTypeList()
        for (name, type) in list {
            // This status is share/write status. Still useful as a hint.
            let status = healthStore.authorizationStatus(for: type)
            var entry = "shareStatus=\(status.rawValue)"

            // Only sample HKSampleType
            guard let sampleType = type as? HKSampleType else {
                entry += ", not a sample type"
                report[name] = entry
                continue
            }

            do {
                let count = try await samplesCount(for: sampleType, range: range)
                entry += ", samples=\(count)"

                if count > 0 {
                    // Optionally fetch one sample detail
                    if let detail = try await oneSampleDetail(for: sampleType, range: range) {
                        entry += ", detail=\(detail)"
                    }
                }
            } catch {
                entry += ", error=\((error as NSError).localizedDescription)"
            }

            report[name] = entry
        }

        return report
    }

    // Helper: return a stable list of default types with readable names
    private static func defaultReadTypeList() -> [(String, HKObjectType)] {
        var out: [(String, HKObjectType)] = []
        if let step = HKObjectType.quantityType(forIdentifier: .stepCount) { out.append(("stepCount", step)) }
        if let distance = HKObjectType.quantityType(forIdentifier: .distanceWalkingRunning) { out.append(("distanceWalkingRunning", distance)) }
        if let energy = HKObjectType.quantityType(forIdentifier: .activeEnergyBurned) { out.append(("activeEnergyBurned", energy)) }
        if let exercise = HKObjectType.quantityType(forIdentifier: .appleExerciseTime) { out.append(("appleExerciseTime", exercise)) }
        if let sleep = HKObjectType.categoryType(forIdentifier: .sleepAnalysis) { out.append(("sleepAnalysis", sleep)) }
        if let hr = HKObjectType.quantityType(forIdentifier: .heartRate) { out.append(("heartRate", hr)) }
        if let restingHR = HKObjectType.quantityType(forIdentifier: .restingHeartRate) { out.append(("restingHeartRate", restingHR)) }
        out.append(("workout", HKObjectType.workoutType()))
        return out
    }

    private func samplesCount(for type: HKSampleType, range: DateInterval) async throws -> Int {
        try await withCheckedThrowingContinuation { cont in
            let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: .strictStartDate)
            let query = HKSampleQuery(sampleType: type, predicate: predicate, limit: HKObjectQueryNoLimit, sortDescriptors: nil) { _, samples, error in
                if let e = error { cont.resume(throwing: e); return }
                cont.resume(returning: samples?.count ?? 0)
            }
            healthStore.execute(query)
        }
    }

    private func oneSampleDetail(for type: HKSampleType, range: DateInterval) async throws -> String? {
        try await withCheckedThrowingContinuation { cont in
            let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: .strictStartDate)
            let sort = NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: false)
            let query = HKSampleQuery(sampleType: type, predicate: predicate, limit: 1, sortDescriptors: [sort]) { _, samples, error in
                if let e = error { cont.resume(throwing: e); return }
                guard let s = samples?.first else { cont.resume(returning: nil); return }

                if let q = s as? HKQuantitySample {
                    // Try to pick a reasonable unit
                    let unit: HKUnit
                    if type == HKObjectType.quantityType(forIdentifier: .stepCount) {
                        unit = HKUnit.count()
                    } else if type == HKObjectType.quantityType(forIdentifier: .heartRate)
                                || type == HKObjectType.quantityType(forIdentifier: .restingHeartRate) {
                        unit = HKUnit(from: "count/min")
                    } else if type == HKObjectType.quantityType(forIdentifier: .distanceWalkingRunning) {
                        unit = HKUnit.meter()
                    } else if type == HKObjectType.quantityType(forIdentifier: .activeEnergyBurned) {
                        unit = HKUnit.kilocalorie()
                    } else if type == HKObjectType.quantityType(forIdentifier: .appleExerciseTime) {
                        unit = HKUnit.minute()
                    } else {
                        unit = HKUnit.count()
                    }
                    let v = q.quantity.doubleValue(for: unit)
                    cont.resume(returning: "quantity=\(v) \(unit.unitString)")
                    return
                }

                if let c = s as? HKCategorySample {
                    cont.resume(returning: "category value=\(c.value), start=\(c.startDate), end=\(c.endDate)")
                    return
                }

                if let w = s as? HKWorkout {
                    cont.resume(returning: "workout type=\(w.workoutActivityType.name), duration=\(w.duration)")
                    return
                }

                cont.resume(returning: "sample type unknown")
            }
            healthStore.execute(query)
        }
    }

    /// Attempt to read raw samples directly without performing an explicit authorization
    /// request beforehand. This is a best-effort debug API: if the app lacks read
    /// permission for a type, HealthKit will simply return no samples.
    ///
    /// Note: Using this to bypass the documented authorization flow is not
    /// recommended for production; apps must still obtain user consent before
    /// relying on Health data. This API exists to surface what (if any) samples
    /// are visible to the running process for debugging.
    public func fetchRawSamplesDirect(range: DateInterval = DateInterval(start: Date().addingTimeInterval(-7*24*3600), end: Date()), limit: Int = 10) async -> [String: [String]] {
        var out: [String: [String]] = [:]

        let list = Self.defaultReadTypeList()
        for (name, type) in list {
            var entries: [String] = []

            // Execute a sample query directly (no pre-check)
            if let sampleType = type as? HKSampleType {
                do {
                    let samples = try await withCheckedThrowingContinuation { (cont: CheckedContinuation<[HKSample], Error>) in
                        let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: .strictStartDate)
                        let sort = NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: false)
                        let query = HKSampleQuery(sampleType: sampleType, predicate: predicate, limit: limit, sortDescriptors: [sort]) { _, samples, error in
                            if let e = error { cont.resume(throwing: e); return }
                            cont.resume(returning: samples ?? [])
                        }
                        healthStore.execute(query)
                    }

                    for s in samples {
                        if let q = s as? HKQuantitySample {
                            let unit: HKUnit
                            if type == HKObjectType.quantityType(forIdentifier: .stepCount) {
                                unit = HKUnit.count()
                            } else if type == HKObjectType.quantityType(forIdentifier: .heartRate)
                                        || type == HKObjectType.quantityType(forIdentifier: .restingHeartRate) {
                                unit = HKUnit(from: "count/min")
                            } else if type == HKObjectType.quantityType(forIdentifier: .distanceWalkingRunning) {
                                unit = HKUnit.meter()
                            } else if type == HKObjectType.quantityType(forIdentifier: .activeEnergyBurned) {
                                unit = HKUnit.kilocalorie()
                            } else if type == HKObjectType.quantityType(forIdentifier: .appleExerciseTime) {
                                unit = HKUnit.minute()
                            } else {
                                unit = HKUnit.count()
                            }
                            let v = q.quantity.doubleValue(for: unit)
                            entries.append("quantity=\(v) \(unit.unitString) @\(q.startDate)")
                        } else if let c = s as? HKCategorySample {
                            entries.append("category=\(c.value) start=\(c.startDate) end=\(c.endDate)")
                        } else if let w = s as? HKWorkout {
                            entries.append("workout=\(w.workoutActivityType.name) duration=\(w.duration) @\(w.startDate)")
                        } else {
                            entries.append("sample unknown @\(s.startDate)")
                        }
                    }
                } catch {
                    entries.append("error=\((error as NSError).localizedDescription)")
                }
            } else {
                entries.append("not a sample type")
            }

            out[name] = entries
        }

        return out
    }

    private func sumQuantity(_ type: HKQuantityType, unit: HKUnit, range: DateInterval) async throws -> Double? {
        try await withCheckedThrowingContinuation { cont in
            let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: .strictStartDate)
            let query = HKStatisticsQuery(quantityType: type, quantitySamplePredicate: predicate, options: .cumulativeSum) { _, result, error in
                if let e = error {
                    if self.isHealthKitNoDataError(e) {
                        cont.resume(returning: nil)
                        return
                    }
                    cont.resume(throwing: e)
                    return
                }
                let sum = result?.sumQuantity()?.doubleValue(for: unit)
                cont.resume(returning: sum)
            }
            healthStore.execute(query)
        }
    }

    private func averageQuantity(_ type: HKQuantityType, unit: HKUnit, range: DateInterval) async throws -> Double? {
        try await withCheckedThrowingContinuation { cont in
            let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: .strictStartDate)
            let query = HKStatisticsQuery(quantityType: type, quantitySamplePredicate: predicate, options: .discreteAverage) { _, result, error in
                if let e = error {
                    if self.isHealthKitNoDataError(e) {
                        cont.resume(returning: nil)
                        return
                    }
                    cont.resume(throwing: e)
                    return
                }
                let avg = result?.averageQuantity()?.doubleValue(for: unit)
                cont.resume(returning: avg)
            }
            healthStore.execute(query)
        }
    }

    private func sumCategoryDurations(_ type: HKCategoryType, range: DateInterval) async throws -> TimeInterval? {
        try await withCheckedThrowingContinuation { cont in
            let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: [])
            let query = HKSampleQuery(sampleType: type, predicate: predicate, limit: HKObjectQueryNoLimit, sortDescriptors: nil) { _, samples, error in
                if let e = error { cont.resume(throwing: e); return }
                guard let samples = samples as? [HKCategorySample], !samples.isEmpty else { cont.resume(returning: 0); return }
                let asleepValues: Set<Int> = {
                    if #available(iOS 16.0, *) {
                        return [
                            HKCategoryValueSleepAnalysis.asleepUnspecified.rawValue,
                            HKCategoryValueSleepAnalysis.asleepCore.rawValue,
                            HKCategoryValueSleepAnalysis.asleepDeep.rawValue,
                            HKCategoryValueSleepAnalysis.asleepREM.rawValue
                        ]
                    }
                    // The legacy `.asleep` value is numerically identical to
                    // `.asleepUnspecified`; use the stable wire value on older OSes.
                    return [1]
                }()
                let intervals = samples
                    .filter { asleepValues.contains($0.value) }
                    .map {
                        DateInterval(
                            start: max($0.startDate, range.start),
                            end: min($0.endDate, range.end)
                        )
                    }
                    .filter { $0.duration > 0 }
                    .sorted { $0.start < $1.start }

                var total: TimeInterval = 0
                var merged: DateInterval?
                for interval in intervals {
                    guard let current = merged else {
                        merged = interval
                        continue
                    }
                    if interval.start <= current.end {
                        merged = DateInterval(start: current.start, end: max(current.end, interval.end))
                    } else {
                        total += current.duration
                        merged = interval
                    }
                }
                total += merged?.duration ?? 0
                cont.resume(returning: total)
            }
            healthStore.execute(query)
        }
    }

    private func mostRecentWorkout(range: DateInterval) async throws -> HKWorkout? {
        try await withCheckedThrowingContinuation { cont in
            let predicate = HKQuery.predicateForSamples(withStart: range.start, end: range.end, options: .strictStartDate)
            let sort = NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: false)
            let query = HKSampleQuery(sampleType: HKObjectType.workoutType(), predicate: predicate, limit: 1, sortDescriptors: [sort]) { _, samples, error in
                if let e = error { cont.resume(throwing: e); return }
                let wk = samples?.first as? HKWorkout
                cont.resume(returning: wk)
            }
            healthStore.execute(query)
        }
    }

    public func startObserverQueries() {
        observerLock.lock()
        let alreadyStarted = !observerQueries.isEmpty
        observerLock.unlock()
        guard !alreadyStarted else { return }
        try? registerObserverQueries(for: defaultReadTypes)
    }

    public func registerObserverQueries(for types: Set<HKObjectType>) throws {
        Logger.shared.log(.debug, "HealthDataSource", "Setting up observer queries", metadata: ["types": String(types.count)])
        
        for type in types {
            // NOTE: `authorizationStatus(for:)` reflects *share/write* authorization, not read.
            // HealthKit does not offer a reliable per-type API for read authorization status,
            // so we register observers and rely on callback errors to indicate lack of read access.
            let shareStatus = healthStore.authorizationStatus(for: type)
            Logger.shared.log(.debug, "HealthDataSource", "Observer authorization snapshot", metadata: ["type": String(describing: type), "shareStatus": String(shareStatus.rawValue)])
            
            guard let sampleType = type as? HKSampleType else {
                Logger.shared.log(.debug, "HealthDataSource", "Skipping observer (not HKSampleType)", metadata: ["type": String(describing: type)])
                continue
            }
            
            let query = HKObserverQuery(sampleType: sampleType, predicate: nil) { _, completionHandler, error in
                if let error = error {
                    Logger.shared.log(.warn, "HealthDataSource", "Observer query error", metadata: ["error": error.localizedDescription])
                    completionHandler()
                    return
                }

                Logger.shared.log(.debug, "HealthDataSource", "Observer detected changes", metadata: ["type": String(describing: type)])
                Task {
                    await Self.syncCoordinator.invalidate()
                    await Self.changeNotifier.schedule()
                    completionHandler()
                }
            }

            observerLock.lock()
            observerQueries.append(query)
            observerLock.unlock()
            healthStore.execute(query)
            Logger.shared.log(.debug, "HealthDataSource", "Registered observer", metadata: ["type": String(describing: type)])
        }
    }
    
    public func stopObserverQueries() {
        observerLock.lock()
        let queries = observerQueries
        observerQueries.removeAll()
        observerLock.unlock()
        queries.forEach(healthStore.stop)
        Logger.shared.log(.debug, "HealthDataSource", "Observer queries stopped")
    }
}

extension Notification.Name {
    static let nolaHealthDataDidChange = Notification.Name("com.gaojc.Nola.healthDataDidChange")
}

// Small helper to map HKWorkoutActivityType to a readable name
private extension HKWorkoutActivityType {
    var name: String {
        switch self {
        case .running: return NSLocalizedString("autogen.HealthDataSource.8eb2fcd6", comment: "AUTOGEN")
        case .walking: return NSLocalizedString("autogen.HealthDataSource.191f5b40", comment: "AUTOGEN")
        case .cycling: return NSLocalizedString("autogen.HealthDataSource.596c5a92", comment: "AUTOGEN")
        case .yoga: return NSLocalizedString("autogen.HealthDataSource.9e5ab613", comment: "AUTOGEN")
        case .swimming: return NSLocalizedString("autogen.HealthDataSource.b66f447b", comment: "AUTOGEN")
        default: return NSLocalizedString("autogen.HealthDataSource.a3350811", comment: "AUTOGEN")
        }
    }
}
