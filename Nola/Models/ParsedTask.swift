// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ParsedTask logic in the shared model layer.
// - Interfaces:
//   - ParsedTask
// - APIs:
//   - none
//
import Foundation

public struct ParsedTask: Codable {
    public var taskType: String
    public var title: String
    public var description: String?
    public var targetDate: String?
    public var isRecurring: Bool?
    public var recurrenceRule: String?
    public var relatedPerson: String?
    public var amount: Double?
    public var currency: String?
    public var confidence: Double?
}
