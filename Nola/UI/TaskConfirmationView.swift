// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements TaskConfirmationView logic in the iOS UI layer.
// - Interfaces:
//   - TaskConfirmationView
//   - TaskConfirmationView_Previews
//   - TaskConfirmationInlineView
//   - TaskServiceWrapper
//   - Binding
// - APIs:
//   - none
//
import SwiftUI
import Combine

public struct TaskConfirmationView: View {
    @Environment(\.presentationMode) var presentationMode
    @State private var title: String
    @State private var notes: String
    @State private var date: Date?
    @State private var isRecurring: Bool
    @State private var recurrenceRule: String
    @State private var relatedPerson: String
    @State private var amountText: String

    public var onSave: (UserTask) -> Void

    public init(parsed: ParsedTask, onSave: @escaping (UserTask) -> Void) {
        _title = State(initialValue: parsed.title)
        _notes = State(initialValue: parsed.description ?? "")
        if let td = parsed.targetDate, let d = Self.parseModelDate(td) {
            _date = State(initialValue: d)
        } else {
            _date = State(initialValue: nil)
        }
        _isRecurring = State(initialValue: parsed.isRecurring ?? false)
        _recurrenceRule = State(initialValue: parsed.recurrenceRule ?? "")
        _relatedPerson = State(initialValue: parsed.relatedPerson ?? "")
        _amountText = State(initialValue: parsed.amount.map { String($0) } ?? "")
        self.onSave = onSave
    }

    public var body: some View {
        NavigationView {
            Form {
                Section(header: Text(L10n.taskSectionTask)) {
                    TextField(L10n.taskTitlePlaceholder, text: $title)
                    TextField(L10n.taskNotesPlaceholder, text: $notes)
                }
                Section(header: Text(L10n.taskSectionWhen)) {
                    Toggle(L10n.taskToggleRecurring, isOn: $isRecurring)
                    if let _ = date {
                        DatePicker(L10n.taskDate, selection: Binding(get: { date ?? Date() }, set: { date = $0 }), displayedComponents: [.date, .hourAndMinute])
                    } else {
                        DatePicker(L10n.taskDateOptional, selection: Binding(get: { Date() }, set: { date = $0 }), displayedComponents: [.date, .hourAndMinute])
                            .labelsHidden()
                    }
                    TextField(L10n.taskRecurrenceRule, text: $recurrenceRule)
                }
                Section(header: Text(L10n.taskSectionExtra)) {
                    TextField(L10n.taskRelatedPersonPlaceholder, text: $relatedPerson)
                    TextField(L10n.taskAmountPlaceholder, text: $amountText).keyboardType(.decimalPad)
                }
            }
            .navigationBarTitle(L10n.taskConfirm, displayMode: .inline)
            .navigationBarItems(leading: Button(L10n.cancel) { presentationMode.wrappedValue.dismiss() }, trailing: Button(L10n.done) {
                var target: Date? = date
                if target == nil { target = nil }
                let amt = Double(amountText).map { Decimal($0) }
                
                let rule = Self.recurrenceRule(from: recurrenceRule)
                
                let task = UserTask(
                    title: title,
                    description: notes.isEmpty ? nil : notes,
                    targetDate: target,
                    isRecurring: isRecurring,
                    recurrenceRule: rule,
                    relatedPerson: relatedPerson.isEmpty ? nil : relatedPerson,
                    amount: amt,
                    currency: nil
                )
                onSave(task)
                presentationMode.wrappedValue.dismiss()
            })
        }
    }

    private static func parseModelDate(_ text: String) -> Date? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = iso.date(from: trimmed) { return date }
        iso.formatOptions = [.withInternetDateTime]
        if let date = iso.date(from: trimmed) { return date }

        for pattern in ["yyyy-MM-dd'T'HH:mm:ssZZZ", "yyyy-MM-dd'T'HH:mm:ss", "yyyy-MM-dd HH:mm:ss", "yyyy-MM-dd HH:mm", "yyyy-MM-dd"] {
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.timeZone = TimeZone.current
            formatter.dateFormat = pattern
            if let date = formatter.date(from: trimmed) {
                return date
            }
        }
        return nil
    }

    private static func recurrenceRule(from text: String) -> RecurrenceRule? {
        let normalized = text
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        guard !normalized.isEmpty else { return nil }

        var frequencyText = normalized
        var interval = 1
        for part in normalized.split(separator: ";") {
            let pieces = part.split(separator: "=", maxSplits: 1).map(String.init)
            guard pieces.count == 2 else { continue }
            switch pieces[0] {
            case "freq":
                frequencyText = pieces[1]
            case "interval":
                interval = max(1, Int(pieces[1]) ?? 1)
            default:
                break
            }
        }
        frequencyText = frequencyText
            .replacingOccurrences(of: "every ", with: "")
            .replacingOccurrences(of: "freq=", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard let frequency = RecurrenceFrequency(rawValue: frequencyText) else { return nil }
        return RecurrenceRule(frequency: frequency, interval: interval)
    }
}

// Preview (optional)
#if DEBUG
struct TaskConfirmationView_Previews: PreviewProvider {
    static var previews: some View {
        TaskConfirmationView(parsed: ParsedTask(taskType: "reminder", title: NSLocalizedString("autogen.TaskConfirmationView.c63d4b22", comment: "AUTOGEN: localized text"), description: nil, targetDate: nil, isRecurring: true, recurrenceRule: "FREQ=DAILY", relatedPerson: nil, amount: nil, currency: nil, confidence: 0.8), onSave: { _ in })
    }
}
#endif

// MARK: - TaskConfirmationInlineView
public struct TaskConfirmationInlineView: View {
    let parse: TaskParseResult
    @EnvironmentObject var container: ServiceContainer
    let onComplete: () -> Void
    
    @State private var editingDate = false
    @State private var selectedDate: Date = Date()
    @State private var inlineStatusMessage: String? = nil
    @State private var isSaving = false
    
    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let draft = parse.scheduledAction {
                VStack(alignment: .leading, spacing: 12) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(draft.name)
                            .font(.headline)
                            .foregroundColor(.primary)

                        Text(draft.url)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .textSelection(.enabled)

                        HStack(spacing: 8) {
                            Image(systemName: "clock")
                                .font(.system(size: 14))
                                .foregroundColor(.blue)

                            Text(scheduleSummary(for: draft))
                                .font(.caption)
                                .foregroundColor(.primary)
                        }

                        if let prompt = draft.userPrompt, !prompt.isEmpty {
                            Text(prompt)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }

                        HStack(spacing: 8) {
                            Image(systemName: "iphone")
                                .font(.system(size: 14))
                                .foregroundColor(.secondary)
                            Text(L10n.scheduledActionsModeLocal)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                    .padding(12)
                    .background(Color(.systemGray6))
                    .cornerRadius(10)

                    if let inlineStatusMessage, !inlineStatusMessage.isEmpty {
                        Text(inlineStatusMessage)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }

                    HStack(spacing: 12) {
                        Button(action: { saveScheduledAction(draft) }) {
                            Text(L10n.commonSave)
                                .font(.subheadline)
                                .fontWeight(.semibold)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 10)
                                .background(Color.green)
                                .foregroundColor(.white)
                                .cornerRadius(8)
                        }
                        .disabled(isSaving)

                        Button(action: { onComplete() }) {
                            Text(L10n.taskIgnore)
                                .font(.subheadline)
                                .fontWeight(.semibold)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 10)
                                .background(Color(.systemGray5))
                                .foregroundColor(.primary)
                                .cornerRadius(8)
                        }
                        .disabled(isSaving)
                    }
                }
            } else if let task = parse.task {
                VStack(alignment: .leading, spacing: 8) {
                    // Task title.
                    Text(task.title)
                        .font(.headline)
                        .foregroundColor(.primary)
                    
                    // Task description.
                    if let desc = task.description {
                        Text(desc)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                    
                    // Time information card; editable when a time is detected.
                    if let dt = task.targetDate {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack(spacing: 8) {
                                Image(systemName: "calendar")
                                    .font(.system(size: 14))
                                    .foregroundColor(.blue)
                                
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(L10n.taskInlinePlannedTime)
                                        .font(.caption2)
                                        .foregroundColor(.secondary)
                                    Text(dt.formatted(date: .abbreviated, time: .shortened))
                                        .font(.caption)
                                        .foregroundColor(.primary)
                                        .fontWeight(.semibold)
                                }
                                
                                Spacer()
                                
                                Button(action: {
                                    selectedDate = dt
                                    editingDate = true
                                }) {
                                    Image(systemName: "pencil.circle.fill")
                                        .font(.system(size: 16))
                                        .foregroundColor(.blue)
                                }
                            }
                            .padding(10)
                            .background(Color.blue.opacity(0.08))
                            .cornerRadius(8)
                        }
                    } else {
                        // No time parsed; prompt the user to set one.
                        VStack(alignment: .leading, spacing: 8) {
                            HStack(spacing: 8) {
                                Image(systemName: "clock.badge.exclamationmark")
                                    .font(.system(size: 14))
                                    .foregroundColor(.orange)
                                
                                Text(L10n.taskInlineNoTimeDetected)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                
                                Spacer()
                                
                                Button(action: {
                                    selectedDate = Date().addingTimeInterval(86400) // Tomorrow
                                    editingDate = true
                                }) {
                                    Image(systemName: "plus.circle.fill")
                                        .font(.system(size: 16))
                                        .foregroundColor(.blue)
                                }
                            }
                            .padding(10)
                            .background(Color.orange.opacity(0.08))
                            .cornerRadius(8)
                        }
                    }
                }
                .padding(12)
                .background(Color(.systemGray6))
                .cornerRadius(10)
                
                // Time edit sheet.
                if editingDate {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(L10n.taskInlineSetReminderTime)
                            .font(.subheadline)
                            .fontWeight(.semibold)
                            .foregroundColor(.primary)
                        
                        DatePicker(
                            L10n.taskInlineSelectTime,
                            selection: $selectedDate,
                            displayedComponents: [.date, .hourAndMinute]
                        )
                        .datePickerStyle(.graphical)
                        
                        HStack(spacing: 12) {
                            Button(action: {
                                editingDate = false
                            }) {
                                Text(L10n.cancel)
                                    .font(.subheadline)
                                    .fontWeight(.semibold)
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 10)
                                    .background(Color(.systemGray5))
                                    .foregroundColor(.primary)
                                    .cornerRadius(8)
                            }
                             
                            Button(action: {
                                guard var updatedTask = parse.task else {
                                    editingDate = false
                                    return
                                }
                                updatedTask.targetDate = selectedDate
                                saveTaskWithDate(updatedTask)
                                editingDate = false
                            }) {
                                Text(L10n.confirm)
                                    .font(.subheadline)
                                    .fontWeight(.semibold)
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 10)
                                    .background(Color.blue)
                                    .foregroundColor(.white)
                                    .cornerRadius(8)
                            }
                            .disabled(isSaving)
                        }
                    }
                    .padding(12)
                    .background(Color.blue.opacity(0.08))
                    .cornerRadius(10)
                }
                
                // Confirm/ignore actions (shown only before edits).
                if !editingDate {
                    HStack(spacing: 12) {
                        Button(action: saveTask) {
                            Text(L10n.taskConfirm)
                                .font(.subheadline)
                                .fontWeight(.semibold)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 10)
                                .background(Color.green)
                                .foregroundColor(.white)
                                .cornerRadius(8)
                        }
                        .disabled(isSaving)
                        
                        Button(action: { onComplete() }) {
                            Text(L10n.taskIgnore)
                                .font(.subheadline)
                                .fontWeight(.semibold)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 10)
                                .background(Color(.systemGray5))
                                .foregroundColor(.primary)
                                .cornerRadius(8)
                        }
                        .disabled(isSaving)
                    }
                }
            } else {
                Text(L10n.errorParseFailed.localized(parse.raw))
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding(12)
        .background(Color(.systemGray6))
        .cornerRadius(10)
        .padding(.horizontal)
    }

    private func saveTask() {
        guard let service = buildTaskService() else { onComplete(); return }
        if let task = parse.task {
            guard !isSaving else { return }
            isSaving = true
            inlineStatusMessage = nil
            Task {
                do {
                    Logger().log(.info, "TaskConfirmationInlineView", "Saving task", metadata: ["taskId": task.id.uuidString, "title": task.title, "targetDate": task.targetDate?.ISO8601Format() ?? "nil"])
                    try service.saveTask(task)
                    Logger().log(.debug, "TaskConfirmationInlineView", "Task saved", metadata: ["taskId": task.id.uuidString])
                    await MainActor.run {
                        isSaving = false
                        onComplete()
                    }
                } catch {
                    Logger().log(.error, "TaskConfirmationInlineView", L10n.taskSaveFailed.localized("\(error)"))
                    await MainActor.run {
                        inlineStatusMessage = String(format: L10n.taskSaveFailed, error.localizedDescription)
                        isSaving = false
                    }
                }
            }
        } else { onComplete() }
    }
    
    private func saveTaskWithDate(_ task: UserTask) {
        guard let service = buildTaskService() else { onComplete(); return }
        guard !isSaving else { return }
        isSaving = true
        inlineStatusMessage = nil
        Task {
            do {
                Logger().log(.info, "TaskConfirmationInlineView", "Saving task with custom date", metadata: ["taskId": task.id.uuidString, "title": task.title, "targetDate": task.targetDate?.ISO8601Format() ?? "nil"])
                try service.saveTask(task)
                Logger().log(.debug, "TaskConfirmationInlineView", "Task saved with custom date", metadata: ["taskId": task.id.uuidString])
                await MainActor.run {
                    isSaving = false
                    onComplete()
                }
            } catch {
                Logger().log(.error, "TaskConfirmationInlineView", L10n.taskSaveFailed.localized("\(error)"))
                await MainActor.run {
                    inlineStatusMessage = String(format: L10n.taskSaveFailed, error.localizedDescription)
                    isSaving = false
                }
            }
        }
    }

    private func buildTaskService() -> UserTaskService? {
        guard let storage = container.localJSONStorage, let unified = container.unifiedStorage else { return nil }
        return UserTaskService(storage: storage, unifiedStorage: unified, notificationService: container.unifiedNotificationService, notificationManager: container.notificationManager)
    }

    private func saveScheduledAction(_ draft: ScheduledActionDraft) {
        guard let fetcher = container.urlFetcher else {
            inlineStatusMessage = L10n.scheduledActionsSchedulerUnavailable
            return
        }
        guard !isSaving else { return }
        isSaving = true
        inlineStatusMessage = nil

        var source = URLDataSource(url: draft.url, format: draft.format, name: draft.name)
        source.enabled = true
        source.userPrompt = draft.userPrompt
        source.scheduleFrequency = draft.scheduleFrequency
        source.scheduledTimeISO = draft.scheduledTimeISO
        source.scheduledWeekday = draft.scheduledWeekday

        if let currentModel = container.chatManager.getCurrentModel(), currentModel.isLocal {
            source.preferredModel = currentModel.id
        }

        do {
            try fetcher.addSource(source)
            container.scheduler.scheduleNextRefresh()
            isSaving = false
            onComplete()
        } catch {
            inlineStatusMessage = error.localizedDescription
            isSaving = false
        }
    }

    private func scheduleSummary(for draft: ScheduledActionDraft) -> String {
        if draft.scheduleFrequency == "weekly", let weekday = draft.scheduledWeekday {
            return "\(weekdayName(for: weekday)) · \(draft.scheduledTimeISO)"
        }
        return "\(scheduleFrequencyLabel(for: draft.scheduleFrequency)) · \(draft.scheduledTimeISO)"
    }

    private func scheduleFrequencyLabel(for frequency: String) -> String {
        switch frequency {
        case "daily":
            return NSLocalizedString("common.frequency.daily", value: "Daily", comment: "Daily frequency")
        case "weekly":
            return NSLocalizedString("common.frequency.weekly", value: "Weekly", comment: "Weekly frequency")
        default:
            return frequency.capitalized
        }
    }

    private func weekdayName(for weekday: Int) -> String {
        let symbols = DateFormatter().weekdaySymbols ?? []
        if symbols.count == 7 {
            return symbols[max(0, min(6, weekday - 1))]
        }
        return String(format: L10n.scheduledActionsWeekdayFormat, weekday)
    }
}

// Helper ObservableObject wrapper so we can use class-based service in View
public final class TaskServiceWrapper: ObservableObject {
    public let service: UserTaskService
    public init(service: UserTaskService) { self.service = service }
}

// Binding helpers: create non-optional bindings from optional-backed state
extension Binding where Value == String {
    init(_ source: Binding<String?>, replacingNilWith defaultValue: String) {
        self.init(get: { source.wrappedValue ?? defaultValue }, set: { source.wrappedValue = $0 })
    }
}

extension Binding where Value == Date {
    init(_ source: Binding<Date?>, replacingNilWith defaultValue: Date) {
        self.init(get: { source.wrappedValue ?? defaultValue }, set: { source.wrappedValue = $0 })
    }
}
