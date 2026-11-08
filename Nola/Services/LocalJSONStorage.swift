// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LocalJSONStorage logic in the iOS service layer.
// - Interfaces:
//   - LocalJSONStorage
// - APIs:
//   - none
//
//
//  LocalJSONStorage.swift
//  Nola
//
//  Simplified document-oriented local JSON storage with atomic writes.

import Foundation

/// Simple document storage with atomic writes for reminders, journals, and metadata.
public final class LocalJSONStorage: @unchecked Sendable {
    // Destruction performs no storage operation or actor-bound cleanup.
    // Explicit isolation avoids the older-runtime synthesized deinit crash
    // documented in swiftlang/swift#88036; normal method isolation is unchanged.
    nonisolated deinit {}

    private let storageDir: URL
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private let queue: DispatchQueue
    private let applicationRuntime: IOSApplicationRuntime
    
    // File paths
    private var remindersPath: URL { storageDir.appendingPathComponent("reminders.json") }
    private var journalsPath: URL { storageDir.appendingPathComponent("journals.json") }
    private var metadataPath: URL { storageDir.appendingPathComponent("metadata.json") }
    private var scheduledActionRunsPath: URL { storageDir.appendingPathComponent("scheduled_action_runs.json") }
    
    public convenience init() throws {
        try self.init(applicationRuntime: IOSApplicationRuntime.shared)
    }

    init(applicationRuntime: IOSApplicationRuntime?) throws {
        self.queue = DispatchQueue(label: "com.nola.storage", attributes: .initiallyInactive)
        self.queue.activate()
        
        let fm = FileManager.default
        let appSupport = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        self.storageDir = appSupport.appendingPathComponent("NolaStorage")
        
        try fm.createDirectory(at: storageDir, withIntermediateDirectories: true, attributes: nil)
        
        self.encoder = JSONEncoder()
        self.encoder.dateEncodingStrategy = .iso8601
        self.decoder = JSONDecoder()
        self.decoder.dateDecodingStrategy = .iso8601
        guard let applicationRuntime else { throw IOSApplicationRuntimeError.applicationUnavailable }
        self.applicationRuntime = applicationRuntime
    }
    
    // MARK: - Reminder Management
    
    public struct ReminderRecord: Codable, Identifiable {
        public let id: String
        public var title: String
        public var description: String?
        public var dueDate: Date
        public var priority: Int
        public var completed: Bool
        public let createdAt: Date
        public var updatedAt: Date
    }
    
    public func saveReminder(_ reminder: ReminderRecord) throws {
        var result: Result<Void, Error>?
        queue.sync {
            do {
                var reminders = (try loadReminders()) ?? [:]
                reminders[reminder.id] = reminder
                let data = try encoder.encode(reminders)
                try atomicWrite(data, to: remindersPath)
                result = .success(())
            } catch {
                result = .failure(error)
            }
        }
        if let result = result, case .failure(let error) = result {
            throw error
        }
    }
    
    public func getReminder(id: String) throws -> ReminderRecord? {
        return try queue.sync {
            let reminders = try loadReminders() ?? [:]
            return reminders[id]
        }
    }
    
    public func getAllReminders() throws -> [ReminderRecord] {
        return try queue.sync {
            let reminders = try loadReminders() ?? [:]
            return Array(reminders.values)
        }
    }
    
    private func loadReminders() throws -> [String: ReminderRecord]? {
        guard FileManager.default.fileExists(atPath: remindersPath.path) else { return nil }
        let data = try Data(contentsOf: remindersPath)
        return try decoder.decode([String: ReminderRecord].self, from: data)
    }
    
    // MARK: - Journal Management
    
    public struct JournalRecord: Codable, Identifiable {
        public let id: String
        public var title: String
        public var content: String
        public var mood: String?
        public var tags: [String]
        public let createdAt: Date
        public var updatedAt: Date
    }
    
    public func saveJournal(_ journal: JournalRecord) throws {
        var result: Result<Void, Error>?
        queue.sync {
            do {
                var journals = (try loadJournals()) ?? [:]
                journals[journal.id] = journal
                let data = try encoder.encode(journals)
                try atomicWrite(data, to: journalsPath)
                result = .success(())
            } catch {
                result = .failure(error)
            }
        }
        if let result = result, case .failure(let error) = result {
            throw error
        }
    }
    
    public func getJournal(id: String) throws -> JournalRecord? {
        return try queue.sync {
            let journals = try loadJournals() ?? [:]
            return journals[id]
        }
    }
    
    public func getAllJournals() throws -> [JournalRecord] {
        return try queue.sync {
            let journals = try loadJournals() ?? [:]
            return Array(journals.values)
        }
    }
    
    private func loadJournals() throws -> [String: JournalRecord]? {
        guard FileManager.default.fileExists(atPath: journalsPath.path) else { return nil }
        let data = try Data(contentsOf: journalsPath)
        return try decoder.decode([String: JournalRecord].self, from: data)
    }
    
    // MARK: - Metadata
    
    public struct StorageMetadata: Codable {
        public var lastSyncDate: Date?
        public var totalReminders: Int = 0
        public var totalJournals: Int = 0
        public var storageVersion: String = "1.0"
    }
    
    public func saveMetadata(_ metadata: StorageMetadata) throws {
        var result: Result<Void, Error>?
        queue.sync {
            do {
                let data = try encoder.encode(metadata)
                try atomicWrite(data, to: metadataPath)
                result = .success(())
            } catch {
                result = .failure(error)
            }
        }
        if let result = result, case .failure(let error) = result {
            throw error
        }
    }
    
    public func getMetadata() throws -> StorageMetadata {
        return try queue.sync {
            guard FileManager.default.fileExists(atPath: metadataPath.path) else {
                return StorageMetadata()
            }
            let data = try Data(contentsOf: metadataPath)
            return try decoder.decode(StorageMetadata.self, from: data)
        }
    }
    
    // MARK: - Utilities
    
    private func atomicWrite(_ data: Data, to url: URL) throws {
        let tmp = url.appendingPathExtension("tmp")
        try data.write(to: tmp, options: [.atomic, .completeFileProtection])
        _ = try? FileManager.default.replaceItemAt(url, withItemAt: tmp)
    }
    
    public func clearAll() throws {
        queue.sync {
            let fm = FileManager.default
            try? fm.removeItem(at: remindersPath)
            try? fm.removeItem(at: journalsPath)
            try? fm.removeItem(at: metadataPath)
            try? fm.removeItem(at: scheduledActionRunsPath)
        }
    }

    // MARK: - Scheduled Action Runs

    /// Structured, persistent execution record for scheduled actions (e.g. scheduled URL reports).
    nonisolated public struct ScheduledActionRunRecord: Codable, Identifiable {
        nonisolated public enum Status: String, Codable {
            case success
            case partial
            case skipped
            case failed
            case answered
            case actionCompleted = "action_completed"
            case delegationAccepted = "delegation_accepted"
            case incomplete, blocked, cancelled
        }

        public let id: String
        public let createdAt: Date
        public let startedAt: Date
        public let finishedAt: Date

        public let isBackground: Bool
        public let actionType: String
        public let actionId: String
        public let actionName: String?
        public let actionUrl: String?

        public let dedupeKey: String?
        public let status: Status
        public var runResult: IOSRunResult? = nil
        public var resultDigest: String? = nil
        public let reason: String?
        public let failureReason: String?

        public let resultEventId: String?
        public let notificationSent: Bool?
        public let notificationError: String?

        public let modelUsed: String?
        public let tokensUsed: Int?
        public let processingDurationSec: Double?

        public let details: String?
        public let errorMessage: String?

        // Enhanced fields for intelligent scheduling and richer diagnostics
        public let eventsCount: Int?
        public let resultJournalId: String?
        public let nextEligibleAt: Date?
        public let shouldRetry: Bool?
        public let retryCount: Int?

        // Traceable provenance fields
        /// The backend-issued result UUID (from /v1/results/{id}), for direct traceability.
        public let serverResultId: String?
        /// How the run was triggered: "manual", "push", "background".
        public let trigger: String?
        /// Where the run executed: "server" or "local".
        public let executionPath: String?
        /// Raw bytes of the fetched URL content.
        public let fetchedContentBytes: Int?

        public init(
            id: String = UUID().uuidString,
            createdAt: Date = Date(),
            startedAt: Date,
            finishedAt: Date,
            isBackground: Bool,
            actionType: String,
            actionId: String,
            actionName: String? = nil,
            actionUrl: String? = nil,
            dedupeKey: String? = nil,
            status: Status,
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
            retryCount: Int? = nil,
            serverResultId: String? = nil,
            trigger: String? = nil,
            executionPath: String? = nil,
            fetchedContentBytes: Int? = nil
        ) {
            self.id = id
            self.createdAt = createdAt
            self.startedAt = startedAt
            self.finishedAt = finishedAt
            self.isBackground = isBackground
            self.actionType = actionType
            self.actionId = actionId
            self.actionName = actionName
            self.actionUrl = actionUrl
            self.dedupeKey = dedupeKey
            self.status = status
            self.reason = reason
            self.failureReason = failureReason
            self.resultEventId = resultEventId
            self.notificationSent = notificationSent
            self.notificationError = notificationError
            self.modelUsed = modelUsed
            self.tokensUsed = tokensUsed
            self.processingDurationSec = processingDurationSec
            self.details = details
            self.errorMessage = errorMessage
            self.eventsCount = eventsCount
            self.resultJournalId = resultJournalId
            self.nextEligibleAt = nextEligibleAt
            self.shouldRetry = shouldRetry
            self.retryCount = retryCount
            self.serverResultId = serverResultId
            self.trigger = trigger
            self.executionPath = executionPath
            self.fetchedContentBytes = fetchedContentBytes
        }
    }

    /// Fetch run records (optionally filtered), newest-first.
    public func getScheduledActionRuns(limit: Int? = nil, actionId: String? = nil) throws -> [ScheduledActionRunRecord] {
        return try queue.sync {
            let requestedLimit = max(1, limit ?? 500)
            let all: [ScheduledActionRunRecord] = try applicationRuntime.listAutomationRuns(
                automationID: actionId?.isEmpty == false ? actionId : nil,
                limit: requestedLimit
            ).compactMap { run -> ScheduledActionRunRecord? in
                let observation = try applicationRuntime.observeAutomationExecution(occurrenceID: run.id)
                guard let receipt = observation.receipt else { return nil }
                let status: ScheduledActionRunRecord.Status
                switch receipt.terminal.result.value.kind {
                case .answered: status = .answered
                case .actionCompleted: status = .actionCompleted
                case .delegationAccepted: status = .delegationAccepted
                case .incomplete: status = .incomplete
                case .blocked: status = .blocked
                case .cancelled: status = .cancelled
                case .failed: status = .failed
                }
                var projection = ScheduledActionRunRecord(id: run.id,
                    startedAt: Date(timeIntervalSince1970: Double(run.startedAtMs ?? run.createdAtMs) / 1_000),
                    finishedAt: Date(timeIntervalSince1970: Double(receipt.terminal.completed_at_ms) / 1_000),
                    isBackground: run.trigger == "background", actionType: "scheduled_url_report", actionId: run.automationID,
                    dedupeKey: run.occurrenceKey, status: status, notificationSent: observation.notificationDelivered,
                    trigger: run.trigger, executionPath: "local")
                projection.runResult = receipt.terminal.result
                projection.resultDigest = receipt.terminal.result_digest
                return projection
            }
            let filtered: [ScheduledActionRunRecord] = {
                if let aid = actionId, !aid.isEmpty {
                    return all.filter { $0.actionId == aid }
                }
                return all
            }()
            let sorted = filtered.sorted { $0.startedAt > $1.startedAt }
            if let l = limit, l >= 0, sorted.count > l {
                return Array(sorted.prefix(l))
            }
            return sorted
        }
    }

    // MARK: - UserTask Management

    public func saveUserTask(_ task: UserTask) throws {
        try queue.sync {
            let id = task.id.uuidString
            let existing = try applicationRuntime.listAutomationDefinitions()
                .first(where: { $0.id == id })
            _ = try applicationRuntime.upsertAutomationDefinition(IOSCanonicalAutomationDefinition(
                id: id,
                name: task.title,
                scheduleFormat: Self.scheduleFormat(for: task),
                enabled: !task.isCompleted,
                revision: (existing?.revision ?? 0) + 1,
                executionSpec: .userTask(try IOSUserTaskSpec(task: task)),
                metadataJSON: "{}",
                createdAtMs: existing?.createdAtMs ?? task.createdAt.millisecondsSince1970,
                updatedAtMs: task.updatedAt.millisecondsSince1970,
                deletedAtMs: nil
            ))
        }
    }
    
    public func getUserTask(id: UUID) throws -> UserTask? {
        try queue.sync {
            try canonicalUserTasks().first(where: { $0.id == id })
        }
    }
    
    public func getAllUserTasks() throws -> [UserTask] {
        try queue.sync { try canonicalUserTasks().sorted { $0.updatedAt > $1.updatedAt } }
    }
    
    public func deleteUserTask(id: UUID) throws {
        try queue.sync {
            _ = try applicationRuntime.deleteAutomationDefinition(
                id: id.uuidString,
                deletedAtMs: Date().millisecondsSince1970
            )
        }
    }

    private func canonicalUserTasks() throws -> [UserTask] {
        try applicationRuntime.listAutomationDefinitions().compactMap { definition in
            guard definition.deletedAtMs == nil, case .userTask(let task) = definition.executionSpec else { return nil }
            return try task.task(definition: definition)
        }
    }

    private static func scheduleFormat(for task: UserTask) -> String {
        if let recurrence = task.recurrenceRule, task.isRecurring {
            return "recurrence:\(recurrence.frequency.rawValue):\(max(1, recurrence.interval))"
        }
        if let target = task.targetDate ?? task.dueDate {
            return "at:\(ISO8601DateFormatter().string(from: target))"
        }
        return "manual"
    }
}

private extension Date {
    var millisecondsSince1970: Int64 { Int64((timeIntervalSince1970 * 1_000).rounded()) }
}
