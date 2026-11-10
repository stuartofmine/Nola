// SPDX-License-Identifier: Apache-2.0
// Strongly typed HealthKit queries for the data dashboard.

import Foundation
import HealthKit

struct HealthDayValue: Identifiable, Sendable {
    let date: Date
    let steps: Double
    var id: Date { date }
}

struct HealthWorkoutSnapshot: Sendable {
    let name: String
    let startDate: Date
    let durationMinutes: Double
    let energyKilocalories: Double?
}

struct HealthDashboardSnapshot: Sendable {
    let generatedAt: Date
    let activityDate: Date
    let todaySteps: Double
    let todayDistanceKilometers: Double
    let todayActiveEnergyKilocalories: Double
    let todayExerciseMinutes: Double
    let latestHeartRate: Double?
    let latestHeartRateDate: Date?
    let restingHeartRate: Double?
    let sleepHours: Double?
    let sleepEndDate: Date?
    let latestWorkout: HealthWorkoutSnapshot?
    let dailySteps: [HealthDayValue]

    var hasReadableData: Bool {
        todaySteps > 0
            || todayDistanceKilometers > 0
            || todayActiveEnergyKilocalories > 0
            || todayExerciseMinutes > 0
            || latestHeartRate != nil
            || restingHeartRate != nil
            || sleepHours != nil
            || latestWorkout != nil
            || dailySteps.contains(where: { $0.steps > 0 })
    }

    var availableMetricCount: Int {
        (todaySteps > 0 ? 1 : 0)
            + (todayDistanceKilometers > 0 ? 1 : 0)
            + (todayActiveEnergyKilocalories > 0 ? 1 : 0)
            + (todayExerciseMinutes > 0 ? 1 : 0)
            + (latestHeartRate == nil ? 0 : 1)
            + (restingHeartRate == nil ? 0 : 1)
            + (sleepHours == nil ? 0 : 1)
            + (latestWorkout == nil ? 0 : 1)
    }
}

final class HealthDashboardRepository: @unchecked Sendable {
    private let store = HKHealthStore()
    private let calendar = Calendar.autoupdatingCurrent
    private let logger = Logger.shared

    func fetchSnapshot(now: Date = Date()) async throws -> HealthDashboardSnapshot {
        guard HKHealthStore.isHealthDataAvailable() else {
            throw HealthDashboardError.unavailable
        }

        let today = calendar.startOfDay(for: now)
        let weekStart = calendar.date(byAdding: .day, value: -6, to: today) ?? today.addingTimeInterval(-6 * 86_400)
        let recentHeartStart = calendar.date(byAdding: .day, value: -7, to: now) ?? now.addingTimeInterval(-7 * 86_400)
        let sleepStart = calendar.date(byAdding: .hour, value: -36, to: now) ?? now.addingTimeInterval(-36 * 3_600)
        let workoutStart = calendar.date(byAdding: .day, value: -30, to: now) ?? now.addingTimeInterval(-30 * 86_400)

        async let todaySteps = cumulativeQuantity(.stepCount, unit: .count(), start: today, end: now)
        async let todayDistance = cumulativeQuantity(.distanceWalkingRunning, unit: .meterUnit(with: .kilo), start: today, end: now)
        async let todayEnergy = cumulativeQuantity(.activeEnergyBurned, unit: .kilocalorie(), start: today, end: now)
        async let todayExercise = cumulativeQuantity(.appleExerciseTime, unit: .minute(), start: today, end: now)
        async let latestHeart = try? latestQuantity(.heartRate, unit: HKUnit(from: "count/min"), start: recentHeartStart, end: now)
        async let restingHeart = try? latestQuantity(.restingHeartRate, unit: HKUnit(from: "count/min"), start: recentHeartStart, end: now)
        async let sleep = try? recentSleep(start: sleepStart, end: now)
        async let workout = try? latestWorkout(start: workoutStart, end: now)
        async let dailySteps = dailyStepValues(start: weekStart, end: now)

        let values = try await (
            todaySteps,
            todayDistance,
            todayEnergy,
            todayExercise,
            latestHeart,
            restingHeart,
            sleep,
            workout,
            dailySteps
        )

        var activityDate = today
        var activitySteps = values.0
        var activityDistance = values.1
        var activityEnergy = values.2
        var activityExercise = values.3
        let stepHistory = values.8

        if activitySteps <= 0 && activityDistance <= 0 && activityEnergy <= 0 && activityExercise <= 0 {
            for dayOffset in 1...6 {
                guard let fallbackStart = calendar.date(byAdding: .day, value: -dayOffset, to: today) else {
                    continue
                }
                let fallbackEnd = calendar.date(byAdding: .day, value: 1, to: fallbackStart)
                    ?? fallbackStart.addingTimeInterval(86_400)
                let fallbackSteps = stepHistory.first(where: {
                    calendar.isDate($0.date, inSameDayAs: fallbackStart)
                })?.steps ?? 0

                async let fallbackDistance = try? cumulativeQuantity(
                    .distanceWalkingRunning,
                    unit: .meterUnit(with: .kilo),
                    start: fallbackStart,
                    end: fallbackEnd
                )
                async let fallbackEnergy = try? cumulativeQuantity(
                    .activeEnergyBurned,
                    unit: .kilocalorie(),
                    start: fallbackStart,
                    end: fallbackEnd
                )
                async let fallbackExercise = try? cumulativeQuantity(
                    .appleExerciseTime,
                    unit: .minute(),
                    start: fallbackStart,
                    end: fallbackEnd
                )
                let fallbackValues = await (
                    fallbackDistance ?? 0,
                    fallbackEnergy ?? 0,
                    fallbackExercise ?? 0
                )

                guard fallbackSteps > 0 || fallbackValues.0 > 0 || fallbackValues.1 > 0 || fallbackValues.2 > 0 else {
                    continue
                }
                activityDate = fallbackStart
                activitySteps = fallbackSteps
                activityDistance = fallbackValues.0
                activityEnergy = fallbackValues.1
                activityExercise = fallbackValues.2
                break
            }
        }

        let snapshot = HealthDashboardSnapshot(
            generatedAt: now,
            activityDate: activityDate,
            todaySteps: activitySteps,
            todayDistanceKilometers: activityDistance,
            todayActiveEnergyKilocalories: activityEnergy,
            todayExerciseMinutes: activityExercise,
            latestHeartRate: values.4?.value,
            latestHeartRateDate: values.4?.date,
            restingHeartRate: values.5?.value,
            sleepHours: values.6?.hours,
            sleepEndDate: values.6?.endDate,
            latestWorkout: values.7,
            dailySteps: stepHistory
        )
        guard snapshot.hasReadableData else {
            throw HealthDashboardError.noReadableData
        }
        logger.log(.info, "HealthDashboardRepository", "Dashboard snapshot loaded", metadata: [
            "activityDate": activityDate.description,
            "steps": String(Int(activitySteps)),
            "distanceKm": String(format: "%.2f", activityDistance),
            "activeEnergyKcal": String(Int(activityEnergy)),
            "exerciseMinutes": String(Int(activityExercise)),
            "stepDays": String(stepHistory.count)
        ])
        recordPersonalIntelligence(snapshot)
        return snapshot
    }

    private func recordPersonalIntelligence(_ snapshot: HealthDashboardSnapshot) {
        let dayFormatter = DateFormatter()
        dayFormatter.calendar = calendar
        dayFormatter.locale = Locale(identifier: "en_US_POSIX")
        dayFormatter.dateFormat = "yyyy-MM-dd"
        let day = dayFormatter.string(from: snapshot.activityDate)
        let observedAt = ISO8601DateFormatter().string(from: snapshot.generatedAt)
        let eventID = UUID().uuidString
        let steps = Int(snapshot.todaySteps.rounded())
        let distance = (snapshot.todayDistanceKilometers * 100).rounded() / 100
        let energy = Int(snapshot.todayActiveEnergyKilocalories.rounded())
        let exercise = Int(snapshot.todayExerciseMinutes.rounded())
        let summary = "\(day) 健康 / health: \(steps) 步 / steps, \(distance) km, \(energy) kcal, \(exercise) min"
        let dedupeKey = "health:\(day):\(steps):\(distance):\(energy):\(exercise):\(Int(snapshot.latestHeartRate ?? 0)):" +
            String(format: "%.1f", snapshot.sleepHours ?? 0)
        let event = IntelligenceEvent(
            id: eventID,
            source: "healthkit",
            kind: "health.snapshot.updated",
            occurredAt: observedAt,
            observedAt: observedAt,
            subject: day,
            summary: summary,
            payload: [
                "steps": .number(Double(steps)),
                "distance_km": .number(distance),
                "active_energy_kcal": .number(Double(energy)),
                "exercise_minutes": .number(Double(exercise))
            ],
            importance: 0.64,
            confidence: 1,
            tags: ["health", "健康", "activity", "活动", "steps", "步数"],
            dedupeKey: dedupeKey
        )
        var candidates = [
            IntelligenceMemoryCandidate(
                semanticKey: "health.activity.\(day)",
                kind: .episodic,
                statement: summary,
                confidence: 1,
                sourceEventIDs: [eventID],
                observedAt: observedAt,
                tags: ["health", "健康", "activity", "活动", "steps", "步数"]
            )
        ]
        if let heartRate = snapshot.latestHeartRate {
            candidates.append(
                IntelligenceMemoryCandidate(
                    semanticKey: "health.heart_rate.latest",
                    kind: .episodic,
                    statement: "最近心率 / latest heart rate: \(Int(heartRate.rounded())) bpm",
                    confidence: 1,
                    sourceEventIDs: [eventID],
                    observedAt: observedAt,
                    tags: ["health", "健康", "heart", "心率"]
                )
            )
        }
        if let sleepHours = snapshot.sleepHours {
            candidates.append(
                IntelligenceMemoryCandidate(
                    semanticKey: "health.sleep.latest",
                    kind: .episodic,
                    statement: String(format: "最近睡眠 / latest sleep: %.1f hours", sleepHours),
                    confidence: 1,
                    sourceEventIDs: [eventID],
                    observedAt: observedAt,
                    tags: ["health", "健康", "sleep", "睡眠"]
                )
            )
        }
        if let workout = snapshot.latestWorkout {
            candidates.append(
                IntelligenceMemoryCandidate(
                    semanticKey: "health.workout.latest",
                    kind: .episodic,
                    statement: "最近训练 / latest workout: \(workout.name), \(Int(workout.durationMinutes.rounded())) min",
                    confidence: 1,
                    sourceEventIDs: [eventID],
                    observedAt: observedAt,
                    tags: ["health", "健康", "workout", "训练"]
                )
            )
        }
        _ = PersonalIntelligenceCore.ingest(
            IntelligenceIngestRequest(events: [event], memoryCandidates: candidates)
        )
    }

    private func cumulativeQuantity(
        _ identifier: HKQuantityTypeIdentifier,
        unit: HKUnit,
        start: Date,
        end: Date
    ) async throws -> Double {
        guard let type = HKObjectType.quantityType(forIdentifier: identifier) else { return 0 }
        let statisticsValue: Double = try await withCheckedThrowingContinuation { continuation in
            let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: .strictStartDate)
            let query = HKStatisticsQuery(quantityType: type, quantitySamplePredicate: predicate, options: .cumulativeSum) { _, result, error in
                if let error { continuation.resume(throwing: error); return }
                continuation.resume(returning: result?.sumQuantity()?.doubleValue(for: unit) ?? 0)
            }
            store.execute(query)
        }
        guard statisticsValue <= 0 else { return statisticsValue }

        // Some HealthKit stores return an empty statistics result while the raw
        // samples are readable (notably immediately after authorization or an
        // iPhone/Watch sync). Falling back to samples keeps the dashboard in
        // agreement with HealthDataSource instead of presenting a false zero.
        return try await rawQuantitySum(type: type, unit: unit, start: start, end: end)
    }

    private func latestQuantity(
        _ identifier: HKQuantityTypeIdentifier,
        unit: HKUnit,
        start: Date,
        end: Date
    ) async throws -> (value: Double, date: Date)? {
        guard let type = HKObjectType.quantityType(forIdentifier: identifier) else { return nil }
        return try await withCheckedThrowingContinuation { continuation in
            let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: .strictStartDate)
            let sort = NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: false)
            let query = HKSampleQuery(sampleType: type, predicate: predicate, limit: 1, sortDescriptors: [sort]) { _, samples, error in
                if let error { continuation.resume(throwing: error); return }
                guard let sample = samples?.first as? HKQuantitySample else {
                    continuation.resume(returning: nil)
                    return
                }
                continuation.resume(returning: (sample.quantity.doubleValue(for: unit), sample.endDate))
            }
            store.execute(query)
        }
    }

    private func rawQuantitySum(
        type: HKQuantityType,
        unit: HKUnit,
        start: Date,
        end: Date
    ) async throws -> Double {
        let samples: [HKQuantitySample] = try await withCheckedThrowingContinuation { continuation in
            let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: [])
            let query = HKSampleQuery(
                sampleType: type,
                predicate: predicate,
                limit: HKObjectQueryNoLimit,
                sortDescriptors: nil
            ) { _, samples, error in
                if let error { continuation.resume(throwing: error); return }
                continuation.resume(returning: samples as? [HKQuantitySample] ?? [])
            }
            store.execute(query)
        }
        return samples.reduce(0) { partial, sample in
            partial + sample.quantity.doubleValue(for: unit)
        }
    }

    private func rawDailyStepValues(
        type: HKQuantityType,
        start: Date,
        end: Date
    ) async throws -> [HealthDayValue] {
        let samples: [HKQuantitySample] = try await withCheckedThrowingContinuation { continuation in
            let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: [])
            let query = HKSampleQuery(
                sampleType: type,
                predicate: predicate,
                limit: HKObjectQueryNoLimit,
                sortDescriptors: nil
            ) { _, samples, error in
                if let error { continuation.resume(throwing: error); return }
                continuation.resume(returning: samples as? [HKQuantitySample] ?? [])
            }
            store.execute(query)
        }

        var totals: [Date: Double] = [:]
        for sample in samples {
            let day = calendar.startOfDay(for: sample.startDate)
            totals[day, default: 0] += sample.quantity.doubleValue(for: .count())
        }

        var values: [HealthDayValue] = []
        var day = calendar.startOfDay(for: start)
        let finalDay = calendar.startOfDay(for: end)
        while day <= finalDay {
            values.append(HealthDayValue(date: day, steps: totals[day, default: 0]))
            guard let next = calendar.date(byAdding: .day, value: 1, to: day) else { break }
            day = next
        }
        return values
    }

    private func dailyStepValues(start: Date, end: Date) async throws -> [HealthDayValue] {
        guard let type = HKObjectType.quantityType(forIdentifier: .stepCount) else { return [] }
        let collectionValues: [HealthDayValue] = try await withCheckedThrowingContinuation { continuation in
            let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: .strictStartDate)
            var interval = DateComponents()
            interval.day = 1
            let query = HKStatisticsCollectionQuery(
                quantityType: type,
                quantitySamplePredicate: predicate,
                options: .cumulativeSum,
                anchorDate: calendar.startOfDay(for: start),
                intervalComponents: interval
            )
            query.initialResultsHandler = { _, collection, error in
                if let error { continuation.resume(throwing: error); return }
                var values: [HealthDayValue] = []
                collection?.enumerateStatistics(from: start, to: end) { statistics, _ in
                    values.append(HealthDayValue(
                        date: statistics.startDate,
                        steps: statistics.sumQuantity()?.doubleValue(for: .count()) ?? 0
                    ))
                }
                continuation.resume(returning: values)
            }
            store.execute(query)
        }
        guard collectionValues.allSatisfy({ $0.steps <= 0 }) else {
            return collectionValues
        }

        let rawValues = try await rawDailyStepValues(type: type, start: start, end: end)
        return rawValues.contains(where: { $0.steps > 0 }) ? rawValues : collectionValues
    }

    private func recentSleep(start: Date, end: Date) async throws -> (hours: Double, endDate: Date)? {
        guard let type = HKObjectType.categoryType(forIdentifier: .sleepAnalysis) else { return nil }
        let samples: [HKCategorySample] = try await withCheckedThrowingContinuation { continuation in
            let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: [])
            let sort = NSSortDescriptor(key: HKSampleSortIdentifierStartDate, ascending: true)
            let query = HKSampleQuery(sampleType: type, predicate: predicate, limit: HKObjectQueryNoLimit, sortDescriptors: [sort]) { _, samples, error in
                if let error { continuation.resume(throwing: error); return }
                continuation.resume(returning: samples as? [HKCategorySample] ?? [])
            }
            store.execute(query)
        }

        let asleepValues: Set<Int> = [
            HKCategoryValueSleepAnalysis.asleepUnspecified.rawValue,
            HKCategoryValueSleepAnalysis.asleepCore.rawValue,
            HKCategoryValueSleepAnalysis.asleepDeep.rawValue,
            HKCategoryValueSleepAnalysis.asleepREM.rawValue
        ]
        let intervals = samples
            .filter { asleepValues.contains($0.value) }
            .map { DateInterval(start: max($0.startDate, start), end: min($0.endDate, end)) }
            .filter { $0.duration > 0 }
            .sorted { $0.start < $1.start }
        guard !intervals.isEmpty else { return nil }

        var sessions: [[DateInterval]] = []
        for interval in intervals {
            if let previous = sessions.last?.last, interval.start.timeIntervalSince(previous.end) <= 2 * 3_600 {
                sessions[sessions.count - 1].append(interval)
            } else {
                sessions.append([interval])
            }
        }
        guard let latestSession = sessions.last else { return nil }
        var merged: [DateInterval] = []
        for interval in latestSession {
            if let last = merged.last, interval.start <= last.end {
                merged[merged.count - 1] = DateInterval(start: last.start, end: max(last.end, interval.end))
            } else {
                merged.append(interval)
            }
        }
        let seconds = merged.reduce(0) { $0 + $1.duration }
        return (seconds / 3_600, merged.last?.end ?? end)
    }

    private func latestWorkout(start: Date, end: Date) async throws -> HealthWorkoutSnapshot? {
        let workout: HKWorkout? = try await withCheckedThrowingContinuation { continuation in
            let type = HKObjectType.workoutType()
            let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: .strictStartDate)
            let sort = NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: false)
            let query = HKSampleQuery(sampleType: type, predicate: predicate, limit: 1, sortDescriptors: [sort]) { _, samples, error in
                if let error { continuation.resume(throwing: error); return }
                continuation.resume(returning: samples?.first as? HKWorkout)
            }
            store.execute(query)
        }
        guard let workout else { return nil }
        let energy = try await workoutEnergyKilocalories(
            start: workout.startDate,
            end: workout.endDate
        )
        return HealthWorkoutSnapshot(
            name: Self.workoutName(workout.workoutActivityType),
            startDate: workout.startDate,
            durationMinutes: workout.duration / 60,
            energyKilocalories: energy
        )
    }

    private func workoutEnergyKilocalories(start: Date, end: Date) async throws -> Double? {
        guard let energyType = HKQuantityType.quantityType(forIdentifier: .activeEnergyBurned) else {
            return nil
        }
        return try await withCheckedThrowingContinuation { continuation in
            let predicate = HKQuery.predicateForSamples(
                withStart: start,
                end: end,
                options: .strictStartDate
            )
            let query = HKStatisticsQuery(
                quantityType: energyType,
                quantitySamplePredicate: predicate,
                options: .cumulativeSum
            ) { _, statistics, error in
                if let error {
                    continuation.resume(throwing: error)
                    return
                }
                continuation.resume(
                    returning: statistics?.sumQuantity()?.doubleValue(for: .kilocalorie())
                )
            }
            store.execute(query)
        }
    }

    private static func workoutName(_ type: HKWorkoutActivityType) -> String {
        switch type {
        case .walking: return NSLocalizedString("autogen.HealthDataSource.191f5b40", comment: "Walking")
        case .running: return NSLocalizedString("autogen.HealthDataSource.8eb2fcd6", comment: "Running")
        case .cycling: return NSLocalizedString("autogen.HealthDataSource.596c5a92", comment: "Cycling")
        case .swimming: return NSLocalizedString("autogen.HealthDataSource.b66f447b", comment: "Swimming")
        case .yoga: return NSLocalizedString("autogen.HealthDataSource.9e5ab613", comment: "Yoga")
        default: return NSLocalizedString("autogen.HealthDataSource.a3350811", comment: "Workout")
        }
    }
}

private enum HealthDashboardError: LocalizedError {
    case unavailable
    case noReadableData

    var errorDescription: String? {
        switch self {
        case .unavailable:
            return NSLocalizedString(
                "health.dashboard.error.unavailable",
                value: "Health data is unavailable on this device.",
                comment: ""
            )
        case .noReadableData:
            return NSLocalizedString(
                "health.dashboard.no_readable_data",
                value: "No readable Health data is available yet.",
                comment: ""
            )
        }
    }
}
