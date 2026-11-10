// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduleItemDetailSheets logic in the iOS UI layer.
// - Interfaces:
//   - ReminderDetailSheet
//   - CalendarEventDetailSheet
// - APIs:
//   - none
//
import SwiftUI
import EventKit

struct ReminderDetailSheet: View {
    let reminderId: String
    @Environment(\.dismiss) private var dismiss
    @State private var reminder: EKReminder? = nil
    @State private var errorText: String? = nil

    var body: some View {
        NavigationStack {
            ScrollView {
                if let rem = reminder {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(rem.title)
                            .font(.title3.weight(.semibold))
                        if let due = rem.dueDateComponents?.date {
                            Text(due.formatted(date: .abbreviated, time: .shortened))
                                .font(.subheadline)
                                .foregroundColor(.secondary)
                        }
                        if let notes = rem.notes, !notes.isEmpty {
                            Text(notes)
                                .font(.body)
                                .foregroundColor(.primary)
                        }
                        if let cal = rem.calendar {
                            Text(cal.title)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                    .padding()
                } else if let err = errorText {
                    VStack(spacing: 12) {
                        Text(err).foregroundStyle(.secondary)
                        Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry loading")) { loadReminder() }
                    }.padding()
                } else {
                    ProgressView()
                        .padding()
                }
            }
            .textSelection(.enabled)
            .navigationTitle(L10n.homeSectionReminders)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(L10n.close) { dismiss() }
                }
            }
            .onAppear { loadReminder() }
        }
    }

    private func loadReminder() {
        reminder = nil
        errorText = nil
        let store = EKEventStore()
        let status = EKEventStore.authorizationStatus(for: .reminder)
        guard status == .fullAccess else {
            errorText = L10n.permissionInsufficientDescription
            return
        }
        if let item = store.calendarItem(withIdentifier: reminderId) as? EKReminder {
            reminder = item
        } else {
            errorText = L10n.notificationContentUnavailable
        }
    }
}

struct CalendarEventDetailSheet: View {
    let eventId: String
    @Environment(\.dismiss) private var dismiss
    @State private var event: EKEvent? = nil
    @State private var errorText: String? = nil

    var body: some View {
        NavigationStack {
            ScrollView {
                if let ev = event {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(ev.title)
                            .font(.title3.weight(.semibold))
                        Text(ev.startDate.formatted(date: .abbreviated, time: .shortened))
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                        if let loc = ev.location, !loc.isEmpty {
                            Text(loc)
                                .font(.body)
                        }
                        if let notes = ev.notes, !notes.isEmpty {
                            Text(notes)
                                .font(.body)
                                .foregroundColor(.primary)
                        }
                    }
                    .padding()
                } else if let err = errorText {
                    VStack(spacing: 12) {
                        Text(err).foregroundStyle(.secondary)
                        Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry loading")) { loadEvent() }
                    }.padding()
                } else {
                    ProgressView()
                        .padding()
                }
            }
            .textSelection(.enabled)
            .navigationTitle(L10n.homeSectionCalendar)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(L10n.close) { dismiss() }
                }
            }
            .onAppear { loadEvent() }
        }
    }

    private func loadEvent() {
        event = nil
        errorText = nil
        let store = EKEventStore()
        let status = EKEventStore.authorizationStatus(for: .event)
        guard status == .fullAccess else {
            errorText = L10n.permissionInsufficientDescription
            return
        }
        if let ev = store.event(withIdentifier: eventId) {
            event = ev
        } else {
            errorText = L10n.notificationContentUnavailable
        }
    }
}
