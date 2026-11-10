// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Executes Agent Host calendar and reminder tools inside the signed macOS app.
// - Security: EventKit authorization is checked at execution time; mutation tools return persistent identifiers.

import EventKit
import Foundation

@MainActor
enum MacProductivityToolService {
    private static let eventStore = EKEventStore()
    private static let iso8601 = ISO8601DateFormatter()
    private static let iso8601Fractional: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    nonisolated static func supports(_ toolName: String) -> Bool {
        switch toolName {
        case "search_calendar_events", "create_calendar_event",
             "search_reminders", "create_reminder", "update_reminder":
            return true
        default:
            return false
        }
    }

    static func responseData(
        requestID: String,
        toolName: String,
        arguments: [String: Any]
    ) async -> Data {
        do {
            let result: (output: Any, display: String)
            switch toolName {
            case "search_calendar_events":
                result = try await searchCalendarEvents(arguments)
            case "create_calendar_event":
                result = try await createCalendarEvent(arguments)
            case "search_reminders":
                result = try await searchReminders(arguments)
            case "create_reminder":
                result = try await createReminder(arguments)
            case "update_reminder":
                result = try await updateReminder(arguments)
            default:
                throw ToolError.invalid(
                    String(format: String(localized: "Unsupported productivity tool: %@"), toolName)
                )
            }
            return response(requestID: requestID, output: result.output, display: result.display, isError: false)
        } catch {
            return response(
                requestID: requestID,
                output: ["error": error.localizedDescription],
                display: error.localizedDescription,
                isError: true
            )
        }
    }

    private static func searchCalendarEvents(_ arguments: [String: Any]) async throws -> (Any, String) {
        try await ensureAccess(to: .event)
        let start = date(arguments["start_date"]) ?? .now
        let defaultEnd = Calendar.current.date(byAdding: .day, value: 30, to: start) ?? start.addingTimeInterval(2_592_000)
        let end = date(arguments["end_date"]) ?? defaultEnd
        guard end > start else {
            throw ToolError.invalid(String(localized: "end_date must be after start_date"))
        }
        let limit = boundedInt(arguments["limit"], default: 30, range: 1...100)
        let query = string(arguments["query"]).lowercased()
        let predicate = eventStore.predicateForEvents(withStart: start, end: end, calendars: nil)
        let events = eventStore.events(matching: predicate)
            .filter { event in
                query.isEmpty || [event.title, event.location, event.notes]
                    .compactMap { $0?.lowercased() }
                    .contains { $0.contains(query) }
            }
            .prefix(limit)
            .map { event in
                [
                    "identifier": event.eventIdentifier ?? "",
                    "title": event.title ?? "",
                    "start_date": iso8601.string(from: event.startDate),
                    "end_date": iso8601.string(from: event.endDate),
                    "all_day": event.isAllDay,
                    "location": event.location ?? "",
                    "notes": event.notes ?? "",
                    "calendar": event.calendar.title,
                ] as [String: Any]
            }
        return (["events": Array(events), "count": events.count], "Found \(events.count) calendar events")
    }

    private static func createCalendarEvent(_ arguments: [String: Any]) async throws -> (Any, String) {
        try await ensureAccess(to: .event)
        let title = try requiredString(arguments, "title")
        guard let start = date(arguments["start_date"]) else {
            throw ToolError.invalid(String(localized: "start_date must be a valid ISO-8601 date"))
        }
        let duration = boundedInt(arguments["duration_minutes"], default: 60, range: 1...10_080)
        let end = date(arguments["end_date"]) ?? start.addingTimeInterval(TimeInterval(duration * 60))
        guard end > start else {
            throw ToolError.invalid(String(localized: "end_date must be after start_date"))
        }

        let event = EKEvent(eventStore: eventStore)
        event.title = title
        event.startDate = start
        event.endDate = end
        event.isAllDay = arguments["all_day"] as? Bool ?? false
        event.location = string(arguments["location"])
        event.notes = string(arguments["notes"])
        event.calendar = try calendar(named: string(arguments["calendar"]), entityType: .event)
        try eventStore.save(event, span: .thisEvent, commit: true)
        guard let identifier = event.eventIdentifier, !identifier.isEmpty else {
            throw ToolError.failed(String(localized: "Calendar did not return a persistent identifier"))
        }
        return ([
            "identifier": identifier,
            "title": event.title ?? "",
            "start_date": iso8601.string(from: event.startDate),
            "end_date": iso8601.string(from: event.endDate),
            "calendar": event.calendar.title,
        ], "Created calendar event \(title)")
    }

    private static func searchReminders(_ arguments: [String: Any]) async throws -> (Any, String) {
        try await ensureAccess(to: .reminder)
        let includeCompleted = arguments["include_completed"] as? Bool ?? false
        let query = string(arguments["query"]).lowercased()
        let limit = boundedInt(arguments["limit"], default: 30, range: 1...100)
        let predicate = includeCompleted
            ? eventStore.predicateForReminders(in: nil)
            : eventStore.predicateForIncompleteReminders(withDueDateStarting: nil, ending: nil, calendars: nil)
        let reminders = await withCheckedContinuation { continuation in
            eventStore.fetchReminders(matching: predicate) { continuation.resume(returning: $0 ?? []) }
        }
        let values = reminders
            .filter { query.isEmpty || [$0.title, $0.notes].compactMap { $0?.lowercased() }.contains { $0.contains(query) } }
            .prefix(limit)
            .map(reminderValue)
        return (["reminders": Array(values), "count": values.count], "Found \(values.count) reminders")
    }

    private static func createReminder(_ arguments: [String: Any]) async throws -> (Any, String) {
        try await ensureAccess(to: .reminder)
        let title = try requiredString(arguments, "title")
        let reminder = EKReminder(eventStore: eventStore)
        reminder.title = title
        reminder.notes = string(arguments["notes"])
        reminder.priority = boundedInt(arguments["priority"], default: 0, range: 0...9)
        reminder.calendar = try calendar(named: string(arguments["list"]), entityType: .reminder)
        if let due = date(arguments["due_date"]) {
            reminder.dueDateComponents = Calendar.current.dateComponents(in: .current, from: due)
        }
        try eventStore.save(reminder, commit: true)
        guard !reminder.calendarItemIdentifier.isEmpty else {
            throw ToolError.failed(String(localized: "Reminders did not return a persistent identifier"))
        }
        return (reminderValue(reminder), "Created reminder \(title)")
    }

    private static func updateReminder(_ arguments: [String: Any]) async throws -> (Any, String) {
        try await ensureAccess(to: .reminder)
        let identifier = try requiredString(arguments, "identifier")
        guard let reminder = eventStore.calendarItem(withIdentifier: identifier) as? EKReminder else {
            throw ToolError.invalid(
                String(
                    format: String(localized: "No reminder exists for identifier %@"),
                    identifier
                )
            )
        }
        if arguments.keys.contains("title") { reminder.title = try requiredString(arguments, "title") }
        if arguments.keys.contains("notes") { reminder.notes = arguments["notes"] as? String }
        if let completed = arguments["completed"] as? Bool { reminder.isCompleted = completed }
        if arguments.keys.contains("priority") {
            reminder.priority = boundedInt(arguments["priority"], default: reminder.priority, range: 0...9)
        }
        if arguments.keys.contains("due_date") {
            reminder.dueDateComponents = date(arguments["due_date"])
                .map { Calendar.current.dateComponents(in: .current, from: $0) }
        }
        try eventStore.save(reminder, commit: true)
        return (reminderValue(reminder), "Updated reminder \(reminder.title ?? identifier)")
    }

    private static func ensureAccess(to entityType: EKEntityType) async throws {
        let status = EKEventStore.authorizationStatus(for: entityType)
        if #available(macOS 14.0, *) {
            if status == .fullAccess { return }
            if status == .denied || status == .restricted { throw ToolError.permission }
            let granted: Bool
            switch entityType {
            case .event: granted = try await eventStore.requestFullAccessToEvents()
            case .reminder: granted = try await eventStore.requestFullAccessToReminders()
            @unknown default: granted = false
            }
            guard granted else { throw ToolError.permission }
        } else {
            if status == .authorized { return }
            if status == .denied || status == .restricted { throw ToolError.permission }
            let granted: Bool = try await withCheckedThrowingContinuation {
                (continuation: CheckedContinuation<Bool, Error>) in
                eventStore.requestAccess(to: entityType) { value, error in
                    if let error { continuation.resume(throwing: error) }
                    else { continuation.resume(returning: value) }
                }
            }
            guard granted else { throw ToolError.permission }
        }
    }

    private static func calendar(named name: String, entityType: EKEntityType) throws -> EKCalendar {
        if !name.isEmpty,
           let match = eventStore.calendars(for: entityType).first(where: { $0.title.caseInsensitiveCompare(name) == .orderedSame }) {
            return match
        }
        let defaultCalendar: EKCalendar?
        switch entityType {
        case .event: defaultCalendar = eventStore.defaultCalendarForNewEvents
        case .reminder: defaultCalendar = eventStore.defaultCalendarForNewReminders()
        @unknown default: defaultCalendar = nil
        }
        guard let value = defaultCalendar else {
            throw ToolError.failed(String(localized: "No writable Calendar or Reminders list is available"))
        }
        return value
    }

    private static func reminderValue(_ reminder: EKReminder) -> [String: Any] {
        var value: [String: Any] = [
            "identifier": reminder.calendarItemIdentifier,
            "title": reminder.title ?? "",
            "notes": reminder.notes ?? "",
            "completed": reminder.isCompleted,
            "priority": reminder.priority,
            "list": reminder.calendar.title,
        ]
        if let due = reminder.dueDateComponents?.date { value["due_date"] = iso8601.string(from: due) }
        return value
    }

    private static func requiredString(_ arguments: [String: Any], _ key: String) throws -> String {
        let value = string(arguments[key])
        guard !value.isEmpty else { throw ToolError.invalid("\(key) is required") }
        return value
    }

    private static func string(_ value: Any?) -> String {
        (value as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    private static func date(_ value: Any?) -> Date? {
        let text = string(value)
        guard !text.isEmpty else { return nil }
        return iso8601Fractional.date(from: text) ?? iso8601.date(from: text)
    }

    private static func boundedInt(_ value: Any?, default fallback: Int, range: ClosedRange<Int>) -> Int {
        let parsed = (value as? NSNumber)?.intValue ?? value as? Int ?? fallback
        return min(range.upperBound, max(range.lowerBound, parsed))
    }

    private static func response(requestID: String, output: Any, display: String, isError: Bool) -> Data {
        let payload: [String: Any] = [
            "type": "device_tool_result",
            "request_id": requestID,
            "output": output,
            "display": display,
            "is_error": isError,
        ]
        return (try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys]))
            ?? Data("{\"type\":\"device_tool_result\",\"is_error\":true}".utf8)
    }

    private enum ToolError: LocalizedError {
        case invalid(String)
        case failed(String)
        case permission

        var errorDescription: String? {
            switch self {
            case .invalid(let message), .failed(let message): return message
            case .permission:
                return String(localized: "Calendar or Reminders access is not authorized for Nola.")
            }
        }
    }
}
