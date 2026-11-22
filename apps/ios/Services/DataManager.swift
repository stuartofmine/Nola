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
    private var applicationRuntime: IOSApplicationRuntime?
    private var conversationStore: ConversationStore?
    private var promptTemplates: PromptTemplateService?
    
    public init(logger: Logger, localJSONStorage: LocalJSONStorage? = nil) {
        self.logger = logger
        self.localJSONStorage = localJSONStorage
    }

    func attachCanonicalServices(
        applicationRuntime: IOSApplicationRuntime,
        conversationStore: ConversationStore?,
        promptTemplates: PromptTemplateService?
    ) {
        self.applicationRuntime = applicationRuntime
        self.conversationStore = conversationStore
        self.promptTemplates = promptTemplates
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
        
        // Export canonical data sources, conversations, memories, templates, skills, and MCP servers (excluding secrets)
        let iso = ISO8601DateFormatter()
        if let runtime = applicationRuntime {
            do {
                let canonicalSources = try runtime.listDataSources()
                exportData["urlDataSources"] = canonicalSources.map { source -> [String: Any] in
                    var dict: [String: Any] = [
                        "id": source.id,
                        "name": source.name,
                        "kind": source.kind.rawValue,
                        "url": source.configuration.location,
                        "accessPolicy": source.accessPolicy,
                        "enabled": source.defaultEnabled,
                        "lifecycle": source.lifecycle.rawValue,
                        "privacy": source.privacy
                    ]
                    if let lastReadMs = source.lastSuccessfulReadAtMs {
                        dict["lastFetchDate"] = iso.string(from: Date(timeIntervalSince1970: TimeInterval(lastReadMs) / 1000))
                    }
                    if let diagnostic = source.readState.diagnostic {
                        dict["lastError"] = diagnostic
                    }
                    return dict
                }
            } catch {
                logger.log(.warn, "DataManager", "Failed to export canonical data sources", metadata: ["error": error.localizedDescription])
            }

            do {
                let conversations = try runtime.listConversations()
                exportData["conversations"] = conversations.map { conv -> [String: Any] in
                    var dict: [String: Any] = [
                        "id": conv.id,
                        "title": conv.title,
                        "createdAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(conv.createdAtMs) / 1000)),
                        "updatedAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(conv.updatedAtMs) / 1000))
                    ]
                    if let uuid = UUID(uuidString: conv.id),
                       let messages = try? conversationStore?.getMessages(conversationId: uuid, limit: 10_000) {
                        dict["messages"] = messages.map { message -> [String: Any] in
                            [
                                "id": message.id.uuidString,
                                "role": message.role.rawValue,
                                "text": message.text,
                                "createdAt": iso.string(from: message.createdAt)
                            ]
                        }
                    }
                    return dict
                }
            } catch {
                logger.log(.warn, "DataManager", "Failed to export canonical conversations", metadata: ["error": error.localizedDescription])
            }

            do {
                let memories = try runtime.listMemories().filter { $0.deletedAtMs == nil }
                exportData["memories"] = memories.map { item -> [String: Any] in
                    var dict: [String: Any] = [
                        "id": item.id,
                        "scope": item.record.scope,
                        "kind": item.record.kind,
                        "content": item.record.content,
                        "tags": item.record.tags,
                        "createdAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(item.createdAtMs) / 1000)),
                        "updatedAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(item.updatedAtMs) / 1000))
                    ]
                    if let summary = item.record.summary {
                        dict["summary"] = summary
                    }
                    return dict
                }
            } catch {
                logger.log(.warn, "DataManager", "Failed to export canonical memories", metadata: ["error": error.localizedDescription])
            }

            do {
                let skills = try runtime.listSkills().filter { $0.source != "built_in" && $0.deletedAtMs == nil }
                exportData["skills"] = skills.map { skill -> [String: Any] in
                    [
                        "id": skill.id,
                        "displayName": skill.displayName,
                        "summary": skill.summary,
                        "category": skill.category,
                        "instructionsText": skill.instructionsText,
                        "enabled": skill.enabled,
                        "createdAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(skill.createdAtMs) / 1000)),
                        "updatedAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(skill.updatedAtMs) / 1000))
                    ]
                }
            } catch {
                logger.log(.warn, "DataManager", "Failed to export canonical skills", metadata: ["error": error.localizedDescription])
            }

            do {
                let servers = try runtime.listMCPServers().filter { $0.deletedAtMs == nil }
                exportData["mcpServers"] = servers.map { server -> [String: Any] in
                    [
                        "id": server.id,
                        "displayName": server.displayName,
                        "enabled": server.enabled,
                        "createdAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(server.createdAtMs) / 1000)),
                        "updatedAt": iso.string(from: Date(timeIntervalSince1970: TimeInterval(server.updatedAtMs) / 1000))
                    ]
                }
            } catch {
                logger.log(.warn, "DataManager", "Failed to export canonical MCP servers", metadata: ["error": error.localizedDescription])
            }
        } else if let urlSourcesData = defaults.data(forKey: "urlDataSources"),
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
                    dict["lastFetchDate"] = iso.string(from: lastFetch)
                }
                if let error = source.lastError {
                    dict["lastError"] = error
                }
                return dict
            }
            exportData["urlDataSources"] = exportableSources
        }

        if let templates = promptTemplates?.templates.filter({ !$0.isBuiltIn }), !templates.isEmpty {
            exportData["promptTemplates"] = templates.map { template -> [String: Any] in
                var dict: [String: Any] = [
                    "id": template.id.uuidString,
                    "name": template.name,
                    "description": template.description,
                    "content": template.content,
                    "createdAt": iso.string(from: template.createdAt),
                    "updatedAt": iso.string(from: template.updatedAt)
                ]
                if let language = template.language {
                    dict["language"] = language
                }
                return dict
            }
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
        
        // List of keys to clear (keep hasCompletedOnboarding intact so Settings stays mounted
        // while async local-data teardown completes and surfaces its result banner).
        let keysToRemove = [
            // App settings
            "lookAheadDays",
            
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

        if let attachmentsDir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)
            .first?.appendingPathComponent("ChatAttachments", isDirectory: true),
           FileManager.default.fileExists(atPath: attachmentsDir.path) {
            try? FileManager.default.removeItem(at: attachmentsDir)
        }
        
        // Reset Core Data (if needed in future)
        // clearCoreData()
        
        logger.log(.info, "DataManager", NSLocalizedString("autogen.DataManager.f56bfab6", comment: "AUTOGEN"))
    }
    
    /// Get actual on-disk local data size across canonical SQLite storage, attachments, local storage, and downloaded models.
    public func getDataSize() -> String {
        let dbURL = applicationRuntime?.databaseURL ?? IOSApplicationRuntime.defaultDatabaseURL
        return Self.computeDataSize(databaseURL: dbURL)
    }

    public nonisolated static func computeDataSize(databaseURL dbURL: URL) -> String {
        let fm = FileManager.default
        var totalSize: Int64 = 0

        let databaseFiles = [
            dbURL,
            URL(fileURLWithPath: dbURL.path + "-wal"),
            URL(fileURLWithPath: dbURL.path + "-shm")
        ]
        for fileURL in databaseFiles {
            totalSize += Self.fileSize(at: fileURL, fileManager: fm)
        }

        var directories: [URL] = []
        if let docs = fm.urls(for: .documentDirectory, in: .userDomainMask).first {
            directories.append(docs.appendingPathComponent("ChatAttachments", isDirectory: true))
            directories.append(docs.appendingPathComponent("NolaStorage", isDirectory: true))
        }
        if let appSupport = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first {
            directories.append(appSupport.appendingPathComponent("NolaStorage", isDirectory: true))
        }
        if let caches = fm.urls(for: .cachesDirectory, in: .userDomainMask).first {
            directories.append(caches.appendingPathComponent("models", isDirectory: true))
        }

        for dir in directories {
            totalSize += Self.directorySize(at: dir, fileManager: fm)
        }

        let formatter = ByteCountFormatter()
        formatter.allowedUnits = [.useKB, .useMB, .useGB]
        formatter.countStyle = .file
        return formatter.string(fromByteCount: totalSize)
    }

    private nonisolated static func fileSize(at url: URL, fileManager: FileManager) -> Int64 {
        guard fileManager.fileExists(atPath: url.path),
              let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileAllocatedSizeKey, .fileSizeKey]),
              values.isRegularFile == true else {
            return 0
        }
        return Int64(values.fileAllocatedSize ?? values.fileSize ?? 0)
    }

    private nonisolated static func directorySize(at url: URL, fileManager: FileManager) -> Int64 {
        guard fileManager.fileExists(atPath: url.path),
              let enumerator = fileManager.enumerator(
                  at: url,
                  includingPropertiesForKeys: [.isRegularFileKey, .fileAllocatedSizeKey, .fileSizeKey],
                  options: [.skipsHiddenFiles]
              ) else {
            return 0
        }
        var total: Int64 = 0
        for case let fileURL as URL in enumerator {
            guard let values = try? fileURL.resourceValues(forKeys: [.isRegularFileKey, .fileAllocatedSizeKey, .fileSizeKey]),
                  values.isRegularFile == true else {
                continue
            }
            total += Int64(values.fileAllocatedSize ?? values.fileSize ?? 0)
        }
        return total
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
        let keychainServices = [
            "nola.llm",
            "com.nola.llm",
            "nola.cloud-model",
            "nola.web-host.token",
            "com.nola.web-host.identity",
            "nola.bing"
        ]

        for service in keychainServices {
            let query: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service
            ]
            SecItemDelete(query as CFDictionary)
        }

        ModelSelectionManager.clearSessionCredentials()
        logger.log(.debug, "DataManager", NSLocalizedString("autogen.DataManager.19498a0c", comment: "AUTOGEN"))
    }
}
