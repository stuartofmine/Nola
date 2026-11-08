// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements SummaryStorageService logic in the iOS service layer.
// - Interfaces:
//   - DailySummary
//   - SummaryStorageService
//   - DateFormatter
// - APIs:
//   - none
//
//
//  SummaryStorageService.swift
//  Nola
//
//  Service for persisting and loading daily summaries
//

import Foundation

public struct DailySummary: Codable {
    public let date: String // YYYY-MM-DD format
    public let items: [SummaryItem]
    public let explain: String?
    public let generatedAt: Date
    public let lastRefresh: Date
    
    public init(date: String, items: [SummaryItem], explain: String?, generatedAt: Date, lastRefresh: Date) {
        self.date = date
        self.items = items
        self.explain = explain
        self.generatedAt = generatedAt
        self.lastRefresh = lastRefresh
    }
    
    public static var empty: DailySummary {
        let today = DateFormatter.dayFormatter.string(from: Date())
        return DailySummary(
            date: today,
            items: [],
            explain: Optional<String>.none,
            generatedAt: Date(),
            lastRefresh: Date()
        )
    }
}

public final class SummaryStorageService {
    private let storage: UserDefaults
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    
    private let summariesKey = "cached_daily_summaries"
    private let lastSummaryDateKey = "last_summary_date"
    
    public init(storage: UserDefaults = .standard) {
        self.storage = storage
        self.encoder = JSONEncoder()
        self.encoder.dateEncodingStrategy = .iso8601
        self.decoder = JSONDecoder()
        self.decoder.dateDecodingStrategy = .iso8601
    }
    
    /// Save summary for a specific date
    public func saveSummary(_ summary: DailySummary) throws {
        var summaries = loadAllSummaries()
        summaries[summary.date] = summary
        
        // Keep only last 30 days to save space
        guard let cutoffDate = Calendar.current.date(byAdding: .day, value: -30, to: Date()) else {
            Logger.shared.log(.error, "SummaryStorageService", "Failed to calculate summary retention cutoff date")
            return
        }
        let cutoffString = DateFormatter.dayFormatter.string(from: cutoffDate)
        summaries = summaries.filter { $0.key >= cutoffString }
        
        let data = try encoder.encode(summaries)
        storage.set(data, forKey: summariesKey)
        storage.set(summary.date, forKey: lastSummaryDateKey)
    }
    
    /// Load summary for a specific date
    public func loadSummary(for date: Date) -> DailySummary? {
        let dateString = DateFormatter.dayFormatter.string(from: date)
        return loadSummary(for: dateString)
    }
    
    /// Load summary for a specific date string
    public func loadSummary(for dateString: String) -> DailySummary? {
        let summaries = loadAllSummaries()
        return summaries[dateString]
    }
    
    /// Load the most recent summary
    public func loadMostRecentSummary() -> DailySummary? {
        if let lastDate = storage.string(forKey: lastSummaryDateKey) {
            return loadSummary(for: lastDate)
        }
        return nil
    }
    
    /// Load today's summary or return empty if not exists
    public func loadTodaySummary() -> DailySummary {
        let today = DateFormatter.dayFormatter.string(from: Date())
        return loadSummary(for: today) ?? DailySummary.empty
    }
    
    /// Check if summary exists for today
    public func hasTodaySummary() -> Bool {
        let today = DateFormatter.dayFormatter.string(from: Date())
        let summaries = loadAllSummaries()
        return summaries[today] != nil
    }
    
    /// Get all summaries
    private func loadAllSummaries() -> [String: DailySummary] {
        guard let data = storage.data(forKey: summariesKey) else {
            return [:]
        }
        
        do {
            return try decoder.decode([String: DailySummary].self, from: data)
        } catch {
            Logger.shared.log(.warn, "SummaryStorageService", "Failed to decode summaries", metadata: [
                "error": String(describing: error)
            ])
            return [:]
        }
    }
    
    /// Clear all cached summaries
    public func clearAllSummaries() {
        storage.removeObject(forKey: summariesKey)
        storage.removeObject(forKey: lastSummaryDateKey)
    }
    
    /// Get summary count
    public func getSummaryCount() -> Int {
        return loadAllSummaries().count
    }
}

// MARK: - Date Formatter Extension

extension DateFormatter {
    static let dayFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        formatter.timeZone = Calendar.current.timeZone
        return formatter
    }()
}
