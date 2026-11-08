// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements RulesEngine logic in the iOS service layer.
// - Interfaces:
//   - ActionType
//   - NotificationCandidate
//   - RulesEngine
// - APIs:
//   - none
//
// RulesEngine.swift (single clean copy)
import Foundation

public enum ActionType: String, Codable {
    case openCalendar
    case openReminder
    case openMessages
    case openHealth
    case dismiss
}

public struct NotificationCandidate: Codable {
    public let event: ProcessedEvent
    public let dedupeKey: String
    public let text: String
    public let actionType: ActionType
}

public final class RulesEngine {
    public struct Config {
        public var priorityThreshold: Double
        public var quietHoursStart: Int
        public var quietHoursEnd: Int
        public var throttleWindowHours: Int
        public var birthdayLeadDays: Int
        public var reminderLeadDays: Int
        public static let `default` = Config(priorityThreshold: 0.6, quietHoursStart: 22, quietHoursEnd: 7, throttleWindowHours: 24, birthdayLeadDays: 7, reminderLeadDays: 1)
    }

    private let config: Config
    private var recentNotifications: [String: Date] = [:]
    private let recentLock = NSLock()
    public init(config: Config = .default) {
        // Allow runtime overrides from UserDefaults for lead day settings
        var cfg = config
        let ud = UserDefaults.standard
        let b = ud.integer(forKey: "birthdayLeadDays")
        if b != 0 { cfg.birthdayLeadDays = b }
        let r = ud.integer(forKey: "reminderLeadDays")
        if r != 0 { cfg.reminderLeadDays = r }
        self.config = cfg
    }

    public func filter(_ events: [ProcessedEvent]) -> [NotificationCandidate] {
        // Delegate to Rust rules engine exclusively
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        guard let data = try? encoder.encode(events) else { return [] }
        if let candidates = RustInterop.rulesFilter(data) {
            return candidates
        }

        var fallback: [NotificationCandidate] = []
        for e in events {
            let dedupe = DedupeKeyGenerator.generate(source: e.source.rawValue, title: e.title, date: e.date, sourceId: e.sourceId)
            let candidate = NotificationCandidate(event: e, dedupeKey: dedupe, text: e.title, actionType: .openCalendar)
            fallback.append(candidate)
        }
        return fallback
    }

    public func isQuietHours() -> Bool {
        return RustInterop.rulesIsQuietHours()
    }

    /// Best-effort, in-process dedupe check.
    ///
    /// Note: Rust rules engine may also apply its own throttling/deduplication.
    /// This method is intentionally lightweight and does not persist across app launches.
    public func wasRecentlySent(_ dedupeKey: String, now: Date = Date()) -> Bool {
        let windowSeconds = TimeInterval(max(0, config.throttleWindowHours)) * 3600
        guard windowSeconds > 0 else { return false }

        recentLock.lock(); defer { recentLock.unlock() }

        // Opportunistic pruning.
        let cutoff = now.addingTimeInterval(-windowSeconds)
        recentNotifications = recentNotifications.filter { $0.value >= cutoff }

        if let lastSent = recentNotifications[dedupeKey] {
            return now.timeIntervalSince(lastSent) < windowSeconds
        }
        return false
    }

    public func markSent(_ dedupeKey: String) {
        recentLock.lock()
        recentNotifications[dedupeKey] = Date()
        recentLock.unlock()
        RustInterop.rulesMarkSent(dedupeKey)
    }

    public func clearHistory() {
        RustInterop.rulesClearHistory()
    }

    private func buildDedupeKey(_ e: ProcessedEvent) -> String {
        // Prefer Rust-generated key when available, otherwise fall back to local generator
        if let s = RustInterop.generateDedupeKey(source: e.source.rawValue, title: e.title, dateISO: e.start ?? e.due, sourceId: e.sourceId) {
            return s
        }
        return DedupeKeyGenerator.generate(source: e.source.rawValue, title: e.title, date: e.date, sourceId: e.sourceId)
    }

    private func buildNotificationText(_ e: ProcessedEvent) -> String { return e.title }
    private func suggestAction(_ e: ProcessedEvent) -> ActionType { return .openCalendar }
}
