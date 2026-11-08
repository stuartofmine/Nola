// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines a small cross-device aggregate Health snapshot transported through iCloud KVS.

import Foundation
import Security

struct HealthSummarySyncDay: Codable, Hashable, Sendable, Identifiable {
    let date: Date
    let steps: Double
    var id: Date { date }
}

struct HealthSummarySyncWorkout: Codable, Hashable, Sendable {
    let name: String
    let startDate: Date
    let durationMinutes: Double
    let energyKilocalories: Double?
}

struct HealthSummarySyncPayload: Codable, Hashable, Sendable {
    let schemaVersion: Int
    let sourceDevice: String
    let generatedAt: Date
    let activityDate: Date
    let steps: Double
    let distanceKilometers: Double
    let activeEnergyKilocalories: Double
    let exerciseMinutes: Double
    let latestHeartRate: Double?
    let latestHeartRateDate: Date?
    let restingHeartRate: Double?
    let sleepHours: Double?
    let sleepEndDate: Date?
    let latestWorkout: HealthSummarySyncWorkout?
    let dailySteps: [HealthSummarySyncDay]
}

enum HealthSummarySync {
    static let cloudKey = "nola.health.summary.v1"
    static let localCacheKey = "nola.health.summary.local-cache.v1"

    static var cloudSyncAvailable: Bool {
#if os(macOS)
        guard let task = SecTaskCreateFromSelf(nil),
              let value = SecTaskCopyValueForEntitlement(
                task,
                "com.apple.developer.ubiquity-kvstore-identifier" as CFString,
                nil
              ) as? String else {
            return false
        }
        return !value.isEmpty
#else
        return true
#endif
    }

    static func publish(_ payload: HealthSummarySyncPayload) {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        guard let data = try? encoder.encode(payload) else { return }
        UserDefaults.standard.set(data, forKey: localCacheKey)
        guard cloudSyncAvailable else { return }
        let store = NSUbiquitousKeyValueStore.default
        store.set(data, forKey: cloudKey)
        store.synchronize()
    }

    static func latest() -> HealthSummarySyncPayload? {
        let cloud = cloudSyncAvailable
            ? NSUbiquitousKeyValueStore.default.data(forKey: cloudKey)
            : nil
        let local = UserDefaults.standard.data(forKey: localCacheKey)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let candidates = [cloud, local].compactMap { data -> (Data, HealthSummarySyncPayload)? in
            guard let data,
                  let payload = try? decoder.decode(HealthSummarySyncPayload.self, from: data),
                  payload.schemaVersion == 1 else {
                return nil
            }
            return (data, payload)
        }
        guard let newest = candidates.max(by: { $0.1.generatedAt < $1.1.generatedAt }) else {
            return nil
        }
        UserDefaults.standard.set(newest.0, forKey: localCacheKey)
        return newest.1
    }

    @discardableResult
    static func synchronize() -> Bool {
        guard cloudSyncAvailable else { return false }
        return NSUbiquitousKeyValueStore.default.synchronize()
    }
}
