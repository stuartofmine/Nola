// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataManager logic in the iOS service layer.
// - Interfaces:
//   - DataManager
// - APIs:
//   - none
//
//
//  DataManager.swift
//  Nola
//
//  Data export and cleanup management.
//

import Foundation

/// Data management service for export and cleanup
public final class DataManager {
    private let logger: Logger
    private let localJSONStorage: LocalJSONStorage?
    
    public init(logger: Logger, localJSONStorage: LocalJSONStorage? = nil) {
        self.logger = logger
        self.localJSONStorage = localJSONStorage
    }
    
    // MARK: - Export Data
    
    /// Export all user data to JSON format
    public func exportData() -> Data? {
        logger.log(.info, "DataManager", NSLocalizedString("autogen.DataManager.b2556e9c", comment: "AUTOGEN"))
        
        var exportData: [String: Any] = [:]
        exportData["exportDate"] = ISO8601DateFormatter().string(from: Date())
        exportData["version"] = "1"
        
        // Export UserDefaults data (excluding sensitive keys)
        let defaults = UserDefaults.standard
        var settings: [String: Any] = [:]
        
        // App settings (only safe, serializable types)
        settings["lookAheadDays"] = defaults.integer(forKey: "lookAheadDays")
        settings["hasCompletedOnboarding"] = defaults.bool(forKey: "hasCompletedOnboarding")
        
        // New notification and reminder settings
        settings["quietHoursStart"] = defaults.integer(forKey: "quietHoursStart")
        settings["quietHoursEnd"] = defaults.integer(forKey: "quietHoursEnd")
        settings["birthdayLeadDays"] = defaults.integer(forKey: "birthdayLeadDays")
        settings["reminderLeadDays"] = defaults.integer(forKey: "reminderLeadDays")
        settings["notificationFrequency"] = defaults.string(forKey: "notificationFrequency") ?? "daily"
        settings["enableNotifications"] = defaults.bool(forKey: "enableNotifications")
        settings["enableStepReminders"] = defaults.object(forKey: "enableStepReminders") as? Bool ?? true
        settings["aggregateCalendarNotifications"] = defaults.object(forKey: "aggregateCalendarNotifications") as? Bool ?? true
        
        // Logger level (enum as int)
        settings["loggerMinLevel"] = defaults.integer(forKey: "logger.minLevel")
        
        exportData["settings"] = settings
        
        // Export URL data sources (if available)
        if let urlSourcesData = defaults.data(forKey: "urlDataSources"),
           let urlSources = try? {
               let d = JSONDecoder()
               d.dateDecodingStrategy = .iso8601
               return try d.decode([URLDataSource].self, from: urlSourcesData)
           }() {
            let exportableSources = urlSources.map { source -> [String: Any] in
                var dict: [String: Any] = [
                    "id": source.id.uuidString,
                    "name": source.name,
                    "url": source.url,
                    "format": source.format.rawValue,
                    "enabled": source.enabled
                ]
                if let lastFetch = source.lastFetchDate {
                    dict["lastFetchDate"] = ISO8601DateFormatter().string(from: lastFetch)
                }
                if let error = source.lastError {
                    dict["lastError"] = error
                }
                return dict
            }
            exportData["urlDataSources"] = exportableSources
        }

        // Export structured scheduled-action run history (file-backed local storage)
        if let storage = localJSONStorage {
            do {
                let runs = try storage.getScheduledActionRuns(limit: 500)
                let iso = ISO8601DateFormatter()
                let exportableRuns: [[String: Any]] = runs.map { r in
                    var dict: [String: Any] = [
                        "id": r.id,
                        "createdAt": iso.string(from: r.createdAt),
                        "startedAt": iso.string(from: r.startedAt),
                        "finishedAt": iso.string(from: r.finishedAt),
                        "isBackground": r.isBackground,
                        "actionType": r.actionType,
                        "actionId": r.actionId,
                        "status": r.status.rawValue
                    ]
                    if let v = r.actionName { dict["actionName"] = v }
                    if let v = r.actionUrl { dict["actionUrl"] = v }
                    if let v = r.dedupeKey { dict["dedupeKey"] = v }
                    if let v = r.reason { dict["reason"] = v }
                    if let v = r.resultEventId { dict["resultEventId"] = v }
                    if let v = r.notificationSent { dict["notificationSent"] = v }
                    if let v = r.notificationError { dict["notificationError"] = v }
                    if let v = r.modelUsed { dict["modelUsed"] = v }
                    if let v = r.tokensUsed { dict["tokensUsed"] = v }
                    if let v = r.processingDurationSec { dict["processingDurationSec"] = v }
                    if let v = r.details { dict["details"] = v }
                    if let v = r.errorMessage { dict["errorMessage"] = v }
                    return dict
                }
                exportData["scheduledActionRuns"] = exportableRuns
            } catch {
                logger.log(.warn, "DataManager", NSLocalizedString("autogen.DataManager.4c447732", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            }
        }
        
        // Export notification settings
        var notificationSettings: [String: Any] = [:]
        if let lastScheduled = defaults.object(forKey: "lastScheduledTime") as? Date {
            notificationSettings["lastScheduledTime"] = ISO8601DateFormatter().string(from: lastScheduled)
        }
        exportData["notificationSettings"] = notificationSettings
        
        // Export health data aggregation settings (note: raw HealthKit samples are not exported per privacy policy)
        // Only export user-configured health data settings and aggregation preferences
        var healthSettings: [String: Any] = [:]
        healthSettings["healthDataEnabled"] = defaults.bool(forKey: "healthDataEnabled")
        healthSettings["healthDataTypes"] = defaults.stringArray(forKey: "healthDataTypes") ?? []
        exportData["healthSettings"] = healthSettings
        
        // Note: Aggregated health metrics (steps, sleep, etc.) that were used for summaries
        // should be handled by the application's data retention policy and not exported
        // unless explicitly approved by the user. Reference: HealthKit Privacy Guidelines.
        
        do {
            // Try to serialize to catch any remaining issues
            let jsonData = try JSONSerialization.data(withJSONObject: exportData, options: [.prettyPrinted, .sortedKeys])
            logger.log(.info, "DataManager", NSLocalizedString("autogen.DataManager.76d44f70", comment: "AUTOGEN"), metadata: ["size": "\(jsonData.count) bytes"])
            return jsonData
        } catch let error as NSError {
            logger.log(.error, "DataManager", NSLocalizedString("autogen.DataManager.c2dd674e", comment: "AUTOGEN"), metadata: [
                "error": error.localizedDescription,
                "domain": error.domain,
                "code": "\(error.code)"
            ])
            
            // Try to identify problematic keys
            for (key, value) in exportData {
                if JSONSerialization.isValidJSONObject([key: value]) {
                    logger.log(.debug, "DataManager", NSLocalizedString("autogen.DataManager.138108a7", comment: "AUTOGEN"), metadata: ["key": key])
                } else {
                    logger.log(.error, "DataManager", NSLocalizedString("autogen.DataManager.8018989a", comment: "AUTOGEN"), metadata: [
                        "key": key,
                        "type": "\(type(of: value))"
                    ])
                }
            }
            
            return nil
        }
    }
    
    // MARK: - Clear Data
    
    /// Clear all user data and reset to factory settings
    public func clearAllData() {
        logger.log(.warn, "DataManager", NSLocalizedString("autogen.DataManager.640e954b", comment: "AUTOGEN"))
        
        let defaults = UserDefaults.standard
        
        // List of keys to clear
        let keysToRemove = [
            // App settings
            "lookAheadDays",
            "hasCompletedOnboarding",
            
            // Advanced features
            "urlDataSources",
            
            // Notification settings
            "lastScheduledTime",
            "enableStepReminders",
            "aggregateCalendarNotifications",
            
            // Logger
            "logger.minLevel",
            "logger.entries"
        ]
        
        for key in keysToRemove {
            defaults.removeObject(forKey: key)
        }
        
        // Clear Keychain (API credentials)
        clearKeychain()

        // Clear file-backed local storage (journals/reminders/tasks/scheduled-action runs)
        do {
            try localJSONStorage?.clearAll()
        } catch {
            logger.log(.warn, "DataManager", NSLocalizedString("autogen.DataManager.9e60996c", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
        }
        
        // Reset Core Data (if needed in future)
        // clearCoreData()
        
        logger.log(.info, "DataManager", NSLocalizedString("autogen.DataManager.f56bfab6", comment: "AUTOGEN"))
    }
    
    /// Get data size estimation
    public func getDataSize() -> String {
        var totalSize = 0
        
        let defaults = UserDefaults.standard
        
        // Estimate size of individual data items (avoid serializing entire dictionary)
        let keys = [
            "urlDataSources",
            "lookAheadDays",
            "hasCompletedOnboarding",
            "logger.minLevel",
            "lastScheduledTime"
        ]
        
        for key in keys {
            if let data = defaults.data(forKey: key) {
                totalSize += data.count
            } else if let obj = defaults.object(forKey: key) {
                // For non-data objects, estimate size as string representation
                let stringRep = String(describing: obj)
                totalSize += stringRep.utf8.count
            }
        }
        
        // Format size
        let formatter = ByteCountFormatter()
        formatter.allowedUnits = [.useKB, .useMB]
        formatter.countStyle = .file
        return formatter.string(fromByteCount: Int64(totalSize))
    }
    
    // MARK: - HealthKit Export (Optional)
    
    /// Export HealthKit aggregated data if user has enabled health data sharing
    /// This is separate from the main export for privacy compliance
    public func exportHealthKitData() -> Data? {
        let defaults = UserDefaults.standard
        
        // Check if user has explicitly enabled health data export
        let healthExportEnabled = defaults.bool(forKey: "healthDataExportEnabled")
        guard healthExportEnabled else {
            logger.log(.debug, "DataManager", "HealthKit export not enabled by user")
            return nil
        }
        
        logger.log(.info, "DataManager", NSLocalizedString("autogen.DataManager.2707fc27", comment: "AUTOGEN"))
        
        var healthData: [String: Any] = [:]
        healthData["exportDate"] = ISO8601DateFormatter().string(from: Date())
        healthData["exportType"] = "HealthKit Aggregated Data"
        
        // Export health data aggregation preferences
        var aggregationSettings: [String: Any] = [:]
        aggregationSettings["enabledTypes"] = defaults.stringArray(forKey: "healthDataTypes") ?? []
        aggregationSettings["aggregationFrequency"] = defaults.string(forKey: "healthAggregationFrequency") ?? "daily"
        
        // Store last export timestamp
        let lastHealthExport = defaults.object(forKey: "lastHealthKitExportDate") as? Date
        if let lastDate = lastHealthExport {
            aggregationSettings["lastExportDate"] = ISO8601DateFormatter().string(from: lastDate)
        }
        
        healthData["aggregationSettings"] = aggregationSettings
        
        // Export any cached health metrics (summary level only, not raw samples)
        var healthMetrics: [String: Any] = [:]
        
        // These are examples of what could be stored and exported
        // In production, these would come from your health data cache
        if let todaySteps = defaults.integer(forKey: "cachedStepsToday") as Int?, todaySteps > 0 {
            healthMetrics["todaySteps"] = todaySteps
        }
        if let sleepHours = defaults.double(forKey: "cachedSleepHours") as Double?, sleepHours > 0 {
            healthMetrics["lastNightSleepHours"] = String(format: "%.1f", sleepHours)
        }
        if let avgHeartRate = defaults.integer(forKey: "cachedAvgHeartRate") as Int?, avgHeartRate > 0 {
            healthMetrics["averageHeartRate"] = avgHeartRate
        }
        
        if !healthMetrics.isEmpty {
            healthData["cachedMetrics"] = healthMetrics
            logger.log(.debug, "DataManager", NSLocalizedString("autogen.DataManager.72b82bb2", comment: "AUTOGEN"), metadata: ["count": String(healthMetrics.count)])
        }
        
        // Privacy note in export
        healthData["privacyNote"] = "This HealthKit data export contains only aggregated metrics. Individual raw samples are never exported per HealthKit privacy guidelines."
        
        do {
            let jsonData = try JSONSerialization.data(withJSONObject: healthData, options: [.prettyPrinted, .sortedKeys])
            
            // Update last export timestamp
            defaults.set(Date(), forKey: "lastHealthKitExportDate")
            
            logger.log(.info, "DataManager", NSLocalizedString("autogen.DataManager.7e09322c", comment: "AUTOGEN"), metadata: ["size": "\(jsonData.count) bytes"])
            return jsonData
        } catch {
            logger.log(.error, "DataManager", NSLocalizedString("autogen.DataManager.f97c62e7", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            return nil
        }
    }
    
    // MARK: - Private Helpers
    
    private func clearKeychain() {
        let keychainService = "com.nola.llm"
        
        // Clear base URL
        let baseURLQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: "baseURL"
        ]
        SecItemDelete(baseURLQuery as CFDictionary)
        
        // Clear API key
        let apiKeyQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: "apiKey"
        ]
        SecItemDelete(apiKeyQuery as CFDictionary)
        
        // Clear model
        let modelQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: "model"
        ]
        SecItemDelete(modelQuery as CFDictionary)
        
        logger.log(.debug, "DataManager", NSLocalizedString("autogen.DataManager.19498a0c", comment: "AUTOGEN"))
    }
}
