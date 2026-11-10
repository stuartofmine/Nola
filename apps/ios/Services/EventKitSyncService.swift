// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements EventKitSyncService logic in the iOS service layer.
// - Interfaces:
//   - EventKitSyncService
// - APIs:
//   - none
//
//
//  EventKitSyncService.swift
//  Nola
//
//  Simple EventKit integration for syncing events with reminders

import Foundation
import EventKit

public final class EventKitSyncService {
    private let eventStore = EKEventStore()
    
    public init() { }
    
    // MARK: - Create Calendar Event
    
    /// Create a calendar event
    public func createCalendarEvent(
        title: String,
        description: String? = nil,
        startDate: Date,
        endDate: Date? = nil
    ) throws -> String {
        let event = EKEvent(eventStore: eventStore)
        event.title = title
        event.notes = description
        event.startDate = startDate
        event.endDate = endDate ?? startDate.addingTimeInterval(3600)
        event.calendar = eventStore.defaultCalendarForNewEvents
        
        try eventStore.save(event, span: .thisEvent, commit: true)
        return event.calendarItemIdentifier
    }
    
    // MARK: - Create Reminder
    
    /// Create a reminder
    public func createReminder(
        title: String,
        description: String? = nil,
        dueDate: Date
    ) throws -> String {
        let reminder = EKReminder(eventStore: eventStore)
        reminder.title = title
        reminder.notes = description
        reminder.dueDateComponents = Calendar.current.dateComponents(
            [.year, .month, .day, .hour, .minute],
            from: dueDate
        )
        reminder.calendar = eventStore.defaultCalendarForNewReminders()
        
        try eventStore.save(reminder, commit: true)
        return reminder.calendarItemIdentifier
    }
    
    // MARK: - Update Event
    
    /// Update an event
    public func updateEvent(
        ekIdentifier: String,
        title: String? = nil,
        description: String? = nil,
        startDate: Date? = nil
    ) throws {
        guard let event = eventStore.event(withIdentifier: ekIdentifier) else {
            throw NSError(domain: "EventKit", code: 404)
        }
        
        if let title = title { event.title = title }
        if let description = description { event.notes = description }
        if let startDate = startDate { event.startDate = startDate }
        
        try eventStore.save(event, span: .thisEvent, commit: true)
    }
    
    // MARK: - Delete Event
    
    /// Delete an event
    public func deleteEvent(ekIdentifier: String) throws {
        guard let event = eventStore.event(withIdentifier: ekIdentifier) else {
            throw NSError(domain: "EventKit", code: 404)
        }
        
        try eventStore.remove(event, span: .thisEvent, commit: true)
    }
    
    // MARK: - Check Access
    
    /// Check if we have calendar access
    public func hasCalendarAccess() -> Bool {
        let status = EKEventStore.authorizationStatus(for: .event)
        // iOS 17+: check for fullAccess or writeOnly
        // iOS 16 and earlier: .authorized (rawValue=3)
        if #available(iOS 17, *) {
            return status == .fullAccess || status == .writeOnly
        } else {
            // In iOS 16 and earlier, authorized status is what we need
            return status.rawValue == 3 || status == .authorized
        }
    }
    
    /// Check if we have reminder access
    public func hasReminderAccess() -> Bool {
        let status = EKEventStore.authorizationStatus(for: .reminder)
        // iOS 17+: check for fullAccess
        // iOS 16 and earlier: .authorized (rawValue=3)
        if #available(iOS 17, *) {
            return status == .fullAccess
        } else {
            // In iOS 16 and earlier, authorized status is what we need
            return status.rawValue == 3 || status == .authorized
        }
    }
    
    // MARK: - Request Authorization
    
    /// Request calendar access
    public func requestCalendarAccess() async -> Bool {
        if #available(iOS 17, *) {
            do {
                return try await eventStore.requestWriteOnlyAccessToEvents()
            } catch {
                return false
            }
        } else {
            return await withCheckedContinuation { continuation in
                eventStore.requestAccess(to: .event) { granted, error in
                    continuation.resume(returning: granted)
                }
            }
        }
    }
    
    /// Request reminder access
    public func requestReminderAccess() async -> Bool {
        if #available(iOS 17, *) {
            do {
                return try await eventStore.requestFullAccessToReminders()
            } catch {
                return false
            }
        } else {
            return await withCheckedContinuation { continuation in
                eventStore.requestAccess(to: .reminder) { granted, error in
                    continuation.resume(returning: granted)
                }
            }
        }
    }
}
