// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements UserTask logic in the shared model layer.
// - Interfaces:
//   - TaskType
//   - RecurrenceFrequency
//   - RecurrenceRule
//   - UserTask
// - APIs:
//   - none
//
import Foundation

public enum TaskType: String, Codable, CaseIterable {
    case reminder
    case birthday
    case debt
    case appointment
    case custom
}

public enum RecurrenceFrequency: String, Codable, CaseIterable {
    case daily
    case weekly
    case monthly
    case yearly
}

// Pure persisted schedule data is reconstructed on the storage queue as well
// as the UI actor; it does not own UI state.
nonisolated public struct RecurrenceRule: Codable, Equatable {
    public var frequency: RecurrenceFrequency
    public var interval: Int
    public var daysOfWeek: [Int]?
    public var daysOfMonth: [Int]?
    public var monthsOfYear: [Int]?
    public var endDate: Date?

    public init(frequency: RecurrenceFrequency = .daily,
                interval: Int = 1,
                daysOfWeek: [Int]? = nil,
                daysOfMonth: [Int]? = nil,
                monthsOfYear: [Int]? = nil,
                endDate: Date? = nil) {
        self.frequency = frequency
        self.interval = interval
        self.daysOfWeek = daysOfWeek
        self.daysOfMonth = daysOfMonth
        self.monthsOfYear = monthsOfYear
        self.endDate = endDate
    }
}

nonisolated public struct UserTask: Identifiable, Codable, Equatable {
    public let id: UUID
    public var type: TaskType
    public var title: String
    public var description: String?
    public var targetDate: Date?
    public var isRecurring: Bool
    public var recurrenceRule: RecurrenceRule?
    public var relatedPerson: String?
    public var amount: Decimal?
    public var currency: String?
    public var dueDate: Date?
    public var isCompleted: Bool
    public let createdAt: Date
    public var updatedAt: Date
    public var notificationIds: [String]

    public init(id: UUID = UUID(),
                type: TaskType = .custom,
                title: String,
                description: String? = nil,
                targetDate: Date? = nil,
                isRecurring: Bool = false,
                recurrenceRule: RecurrenceRule? = nil,
                relatedPerson: String? = nil,
                amount: Decimal? = nil,
                currency: String? = nil,
                dueDate: Date? = nil,
                isCompleted: Bool = false,
                createdAt: Date = Date(),
                updatedAt: Date = Date(),
                notificationIds: [String] = []) {
        self.id = id
        self.type = type
        self.title = title
        self.description = description
        self.targetDate = targetDate
        self.isRecurring = isRecurring
        self.recurrenceRule = recurrenceRule
        self.relatedPerson = relatedPerson
        self.amount = amount
        self.currency = currency
        self.dueDate = dueDate
        self.isCompleted = isCompleted
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.notificationIds = notificationIds
    }

    public static func empty() -> UserTask {
        return UserTask(title: "")
    }
}
