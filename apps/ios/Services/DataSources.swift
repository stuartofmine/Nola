// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DataSources logic in the iOS service layer.
// - Interfaces:
//   - DataSource
//   - ReadableDataSource
//   - ReadableDataSourceItem
//   - RawEvent
//   - EventSource
//   - ContactsDataSource
//   - CalendarsDataSource
//   - RemindersDataSource
// - APIs:
//   - none
//
//
//  DataSources.swift
//  Nola
//
//  Adapters for Contacts, Calendars, and Reminders with preprocessing.
//

import Foundation
import Contacts
import EventKit
import HealthKit

/// Unified data source interface.
public protocol DataSource {
    func fetchEvents(in range: DateInterval) async throws -> [RawEvent]
}

/// Generic readable-source interface for chat context and scheduled operations.
/// Event-based sources can still expose `RawEvent`, but arbitrary documents,
/// folders, search results, and APIs can provide plain readable items.
public protocol ReadableDataSource {
    var readableSourceID: UUID { get }
    var readableKind: DataSourceKind { get }
    var readableCapabilities: [DataSourceCapability] { get }
    func readContextItems(limit: Int) async throws -> [ReadableDataSourceItem]
}

public struct ReadableDataSourceItem: Identifiable, Hashable, Sendable, Codable {
    public let id: String
    public let title: String
    public let summary: String?
    public let content: String?
    public let sourceURL: String?
    public let metadata: [String: String]

    public init(
        id: String,
        title: String,
        summary: String? = nil,
        content: String? = nil,
        sourceURL: String? = nil,
        metadata: [String: String] = [:]
    ) {
        self.id = id
        self.title = title
        self.summary = summary
        self.content = content
        self.sourceURL = sourceURL
        self.metadata = metadata
    }
}

/// Raw event from any source before preprocessing.
public struct RawEvent: Identifiable, Hashable, Sendable, Codable {
    public let id: String
    public let source: EventSource
    public let title: String
    public let date: Date?       // For calendars: start; for reminders: due
    public let notes: String?
    public let metadata: [String: String]
    
    public nonisolated init(id: String, source: EventSource, title: String, date: Date?, notes: String?, metadata: [String: String] = [:]) {
        self.id = id
        self.source = source
        self.title = title
        self.date = date
        self.notes = notes
        self.metadata = metadata
    }
}

public enum EventSource: String, Codable, Hashable {
    case contact
    case calendar
    case reminder
    case url
    case health
}

// MARK: - ContactsDataSource

public final class ContactsDataSource: DataSource, @unchecked Sendable {
    private nonisolated(unsafe) let store = CNContactStore()
    
    public init() {}
    
    /// Fetch contacts with upcoming birthdays in the given range.
    public func fetchEvents(in range: DateInterval) async throws -> [RawEvent] {
        let logger = Logger()
        // Request access if not already authorized
        let currentStatus = CNContactStore.authorizationStatus(for: .contacts)
        logger.log(.debug, "ContactsDataSource", "Permission status check", metadata: ["status": String(currentStatus.rawValue)])
        
        if currentStatus == .notDetermined {
            do {
                let granted = try await store.requestAccess(for: .contacts)
                logger.log(.debug, "ContactsDataSource", "Access request result", metadata: ["granted": granted ? "true" : "false"])
                if !granted {
                    throw DataSourceError.permissionDenied(source: "Contacts")
                }
            } catch {
                logger.log(.error, "ContactsDataSource", "Request failed", metadata: ["error": error.localizedDescription])
                throw DataSourceError.permissionDenied(source: "Contacts")
            }
        } else if currentStatus != .authorized {
            throw DataSourceError.permissionDenied(source: "Contacts")
        }
        
        // Perform contact enumeration on background thread (blocking operation)
        // CNContactStore is safe to use off main thread
        let result = try await Task.detached { [store] in
            let keys: [CNKeyDescriptor] = [
                CNContactGivenNameKey as CNKeyDescriptor,
                CNContactFamilyNameKey as CNKeyDescriptor,
                CNContactBirthdayKey as CNKeyDescriptor
            ]
            
            var events: [RawEvent] = []
            var totalContacts = 0
            var contactsWithBirthday = 0
            let request = CNContactFetchRequest(keysToFetch: keys)
            
            try store.enumerateContacts(with: request) { contact, _ in
                totalContacts += 1
                guard let birthday = contact.birthday else { return }
                contactsWithBirthday += 1
                
                // Convert birthday to Date (approximate, using current year or next)
                var components = DateComponents()
                components.year = Calendar.current.component(.year, from: range.start)
                components.month = birthday.month
                components.day = birthday.day
                
                guard let birthdayDate = Calendar.current.date(from: components) else { return }
                
                // Check if birthday falls in range (this year or next)
                let nextYearDate = Calendar.current.date(byAdding: .year, value: 1, to: birthdayDate)
                let candidates = [birthdayDate, nextYearDate].compactMap { $0 }
                
                for candidate in candidates {
                    if range.contains(candidate) {
                        let name = [contact.givenName, contact.familyName].filter { !$0.isEmpty }.joined(separator: " ")
                        let event = RawEvent(
                            id: contact.identifier,
                            source: .contact,
                            title: L10n.format("data_source.contacts.birthday_title", name),
                            date: candidate,
                            notes: nil,
                            metadata: ["relation": "family"] // Simplified; could infer from name/tags
                        )
                        events.append(event)
                    }
                }
            }
            return (events, totalContacts, contactsWithBirthday)
        }.value

        logger.log(.debug, "ContactsDataSource", NSLocalizedString("autogen.DataSources.33b55947", comment: "AUTOGEN"), metadata: [
            "totalContacts": "\(result.1)",
            "withBirthday": "\(result.2)",
            "inRange": "\(result.0.count)"
        ])

        return result.0
    }
}

// MARK: - CalendarsDataSource

public final class CalendarsDataSource: DataSource, @unchecked Sendable {
    public init() {}
    
    @MainActor
    public func fetchEvents(in range: DateInterval) async throws -> [RawEvent] {
        // Create a new store instance each time to ensure we get the latest authorization status
        let store = EKEventStore()
        
        // Request access if needed (iOS 17+: use requestFullAccessToEvents)
        if #available(iOS 17.0, *) {
            do {
                let granted = try await store.requestFullAccessToEvents()
                Logger().log(.debug, "CalendarsDataSource", "iOS 17+ requestFullAccessToEvents", metadata: ["granted": "\(granted)"])
                if !granted {
                    throw DataSourceError.permissionDenied(source: "Calendars")
                }
            } catch {
                Logger().log(.error, "CalendarsDataSource", "Access request failed", metadata: ["error": "\(error)"])
                throw DataSourceError.permissionDenied(source: "Calendars")
            }
        } else {
            // iOS 16 and earlier
            do {
                let granted = try await store.requestAccess(to: .event)
                Logger().log(.debug, "CalendarsDataSource", "iOS 16- requestAccess", metadata: ["granted": "\(granted)"])
                if !granted {
                    throw DataSourceError.permissionDenied(source: "Calendars")
                }
            } catch {
                Logger().log(.error, "CalendarsDataSource", "Access request failed", metadata: ["error": "\(error)"])
                throw DataSourceError.permissionDenied(source: "Calendars")
            }
        }
        
        let status = EKEventStore.authorizationStatus(for: .event)
        Logger().log(.debug, "CalendarsDataSource", "Permission status", metadata: ["status": "\(status.rawValue)"])
        
        let predicate = store.predicateForEvents(withStart: range.start, end: range.end, calendars: nil)
        let ekEvents = store.events(matching: predicate)
        
        Logger().log(.debug, "CalendarsDataSource", "Event query results", metadata: ["count": "\(ekEvents.count)"])
        
        return ekEvents.map { ek in
            RawEvent(
                id: ek.eventIdentifier,
                source: .calendar,
                title: ek.title ?? L10n.string("data_source.calendar.untitled_event"),
                date: ek.startDate,
                notes: ek.notes,
                metadata: [
                    "duration": "\(ek.endDate.timeIntervalSince(ek.startDate))",
                    "calendar": ek.calendar.title
                ]
            )
        }
    }
}

// MARK: - RemindersDataSource

public final class RemindersDataSource: DataSource, @unchecked Sendable {
    public init() {}
    
    @MainActor
    public func fetchEvents(in range: DateInterval) async throws -> [RawEvent] {
        // Create a new store instance each time to ensure we get the latest authorization status
        let store = EKEventStore()
        
        // Request access if needed (iOS 17+: use requestFullAccessToReminders)
        if #available(iOS 17.0, *) {
            do {
                let granted = try await store.requestFullAccessToReminders()
                Logger().log(.debug, "RemindersDataSource", "iOS 17+ requestFullAccessToReminders", metadata: ["granted": "\(granted)"])
                if !granted {
                    throw DataSourceError.permissionDenied(source: "Reminders")
                }
            } catch {
                Logger().log(.error, "RemindersDataSource", "Access request failed", metadata: ["error": "\(error)"])
                throw DataSourceError.permissionDenied(source: "Reminders")
            }
        } else {
            // iOS 16 and earlier
            do {
                let granted = try await store.requestAccess(to: .reminder)
                Logger().log(.debug, "RemindersDataSource", "iOS 16- requestAccess", metadata: ["granted": "\(granted)"])
                if !granted {
                    throw DataSourceError.permissionDenied(source: "Reminders")
                }
            } catch {
                Logger().log(.error, "RemindersDataSource", "Access request failed", metadata: ["error": "\(error)"])
                throw DataSourceError.permissionDenied(source: "Reminders")
            }
        }
        
        let status = EKEventStore.authorizationStatus(for: .reminder)
        Logger().log(.debug, "RemindersDataSource", "Permission status", metadata: ["status": "\(status.rawValue)"])
        
        let predicate = store.predicateForReminders(in: nil) // All calendars
        
        // fetchReminders callback executes on main thread
        return try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<[RawEvent], Error>) in
            store.fetchReminders(matching: predicate) { items in
                let reminders = items ?? []
                
                Logger().log(.debug, "RemindersDataSource", "Fetched reminders", metadata: ["total": "\(reminders.count)"])
                
                let filtered = reminders.filter { reminder in
                    guard let due = reminder.dueDateComponents?.date else { return false }
                    return range.contains(due)
                }
                
                Logger().log(.debug, "RemindersDataSource", "Filtered reminders in range", metadata: ["count": "\(filtered.count)"])
                
                let events = filtered.map { rem in
                    RawEvent(
                        id: rem.calendarItemIdentifier,
                        source: .reminder,
                        title: rem.title ?? L10n.string("data_source.reminder.untitled_reminder"),
                        date: rem.dueDateComponents?.date,
                        notes: rem.notes,
                        metadata: [
                            "priority": "\(rem.priority)",
                            "completed": "\(rem.isCompleted)"
                        ]
                    )
                }
                
                continuation.resume(returning: events)
            }
        }
    }
}

// MARK: - Preprocessing

/// Preprocessor: dedupe, score, timeline sort, PII minimization.
public final class EventPreprocessor {
    public init() {}
    
    public func process(_ events: [RawEvent], config: PreprocessConfig = .default) -> [ProcessedEvent] {
        var deduplicated = deduplicate(events)
        deduplicated = deduplicated.sorted { ($0.date ?? .distantFuture) < ($1.date ?? .distantFuture) }
        
        let scored = deduplicated.map { raw -> ProcessedEvent in
            let score = computeScore(raw, config: config)
            let sanitizedTitle = sanitize(raw.title)

            // Preserve a meaningful sourceId when possible (especially for URL sources).
            // - For URL events: prefer URLDataSource.id from metadata["sourceId"]
            // - Otherwise: fall back to raw.id
            let sourceId: String = {
                if let sid = raw.metadata["sourceId"], !sid.isEmpty { return sid }
                return raw.id
            }()

            // Stable dedupeKey used across storage/notifications/navigation.
            let dedupe = DedupeKeyGenerator.generate(
                source: raw.source.rawValue,
                title: sanitizedTitle,
                date: raw.date,
                sourceId: sourceId
            )

            // Map EventSource to SourceType
            let srcType: ProcessedEvent.SourceType = {
                switch raw.source {
                case .calendar: return .calendar
                case .reminder: return .reminder
                case .contact: return .manual
                case .health: return .health
                case .url: return .url
                }
            }()

            // Provide a deterministic hash for UI/debug (do not use Swift hashValue).
            let stableHash: Int = {
                // 64-bit FNV-1a reduced to Int
                var h: UInt64 = 0xcbf29ce484222325
                let p: UInt64 = 0x00000100000001B3
                for b in dedupe.utf8 {
                    h ^= UInt64(b)
                    h &*= p
                }
                return Int(truncatingIfNeeded: h)
            }()

            return ProcessedEvent(
                id: raw.id,
                source: raw.source,
                sourceType: srcType,
                sourceId: sourceId,
                title: sanitizedTitle,
                date: raw.date,
                start: nil,
                due: nil,
                priority: score,
                tags: nil,
                dedupeKey: dedupe,
                provenance: [:] as [String: JSONValue]?,
                metadata: minimizePII(raw.metadata),
                notes: raw.notes,
                hash: stableHash
            )
        }
        
        return scored.filter { $0.priority >= config.minScore }
    }
    
    private func deduplicate(_ events: [RawEvent]) -> [RawEvent] {
        var seen = Set<String>()
        return events.filter { event in
            let title = sanitize(event.title)
            let sourceId: String = {
                if let sid = event.metadata["sourceId"], !sid.isEmpty { return sid }
                return event.id
            }()
            let key = DedupeKeyGenerator.generate(source: event.source.rawValue, title: title, date: event.date, sourceId: sourceId)
            if seen.contains(key) { return false }
            seen.insert(key)
            return true
        }
    }
    
    private func computeScore(_ event: RawEvent, config: PreprocessConfig) -> Double {
        var score = 0.5 // Base
        
        // Time sensitivity: closer dates = higher score
        if let date = event.date {
            let days = Calendar.current.dateComponents([.day], from: Date(), to: date).day ?? 0
            if days >= 0 && days <= 3 { score += 0.3 }
            else if days > 3 && days <= 7 { score += 0.2 }
        }
        
        // Relation tag boost
        if event.metadata["relation"] == "family" { score += 0.2 }
        
        // Priority (for reminders)
        if let prio = event.metadata["priority"], let p = Int(prio), p <= 3 {
            score += 0.1
        }
        
        // Message category boost
        if let category = event.metadata["category"] {
            switch category {
            case "urgent": score += 0.3
            case "bill": score += 0.25
            case "appointment": score += 0.2
            case "delivery": score += 0.15
            default: break
            }
        }
        
        return min(score, 1.0)
    }
    
    private func sanitize(_ text: String) -> String {
        // Remove excess whitespace, limit length
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return String(trimmed.prefix(100))
    }
    
    private func minimizePII(_ metadata: [String: String]) -> [String: String] {
        // Only keep safe tags
        // Keep URL/link fields for user-facing actions (e.g. open original content) and
        // lightweight context fields that are not typically PII.
        let allowed = Set([
            "relation",
            "priority",
            "completed",
            "category",
            "keywords",
            "url",
            "link",
            "sourceName",
            "location",
            "tags",
            "type"
        ])
        return metadata.filter { allowed.contains($0.key) }
    }
}

public struct PreprocessConfig {
    public var minScore: Double
    public static let `default` = PreprocessConfig(minScore: 0.4)
}

/// Processed event with score and hash.
public struct ProcessedEvent: Identifiable, Codable {
    public let id: String
    // Concrete source category used by platform preprocessing and presentation.
    public let source: EventSource
    // Canonical source type for cross-service contracts
    public let sourceType: SourceType
    // Original source identifier (e.g., ekIdentifier, urlSourceId, manualId)
    public let sourceId: String
    public let title: String
    // Optional date fields (kept both as Date and ISO string in other layers)
    public let date: Date?
    public let start: String?
    public let due: String?
    public let priority: Double
    public let tags: [String]?
    // Dedupe key used across notification & storage layers
    public let dedupeKey: String
    // Arbitrary provenance info (LLM output, fetch metadata)
    public let provenance: [String: JSONValue]?
    // Source metadata exposed to context consumers.
    public let metadata: [String: String]
    public let notes: String?
    public let hash: Int

    public enum SourceType: String, Codable {
        case calendar
        case reminder
        case url
        case manual
        case health
    }
    
    /// Full memberwise initializer
    public init(id: String, source: EventSource, sourceType: SourceType, sourceId: String, title: String, date: Date?, start: String?, due: String?, priority: Double, tags: [String]?, dedupeKey: String, provenance: [String: JSONValue]?, metadata: [String: String], notes: String?, hash: Int) {
        self.id = id
        self.source = source
        self.sourceType = sourceType
        self.sourceId = sourceId
        self.title = title
        self.date = date
        self.start = start
        self.due = due
        self.priority = priority
        self.tags = tags
        self.dedupeKey = dedupeKey
        self.provenance = provenance
        self.metadata = metadata
        self.notes = notes
        self.hash = hash
    }
}
// Default Codable synthesis should be fine; if format needs control
// (e.g. ISO8601), add custom CodingKeys/encoder later.

// MARK: - URLFetcherDataSource Adapter

public final class URLFetcherDataSource: DataSource {
    private let fetcher: URLFetcherService
    
    public init(fetcher: URLFetcherService) {
        self.fetcher = fetcher
    }
    
    public func fetchEvents(in range: DateInterval) async throws -> [RawEvent] {
        do {
            let allURLEvents = try await fetcher.fetchAllEvents()

            return allURLEvents.filter { ev in
                guard let d = ev.date else { return true }
                return range.contains(d)
            }
        } catch {
            throw DataSourceError.unknown(error)
        }
    }
}
