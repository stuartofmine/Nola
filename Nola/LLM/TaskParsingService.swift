// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements TaskParsingService logic in the iOS LLM integration layer.
// - Interfaces:
//   - TaskParseResult
//   - ScheduledActionDraft
//   - TaskParsingError
//   - TaskParsingService
// - APIs:
//   - none
//
import Foundation
import Combine

/// Lightweight service that uses the injected LLMClient to parse a free-form user instruction
/// into a structured task-like JSON. It asks the LLM to respond with a strict JSON shape and decodes that response.
public struct TaskParseResult {
    public let success: Bool
    public let task: UserTask?
    public let scheduledAction: ScheduledActionDraft?
    public let confidence: Double
    public let raw: String

    public init(success: Bool, task: UserTask?, scheduledAction: ScheduledActionDraft?, confidence: Double, raw: String) {
        self.success = success
        self.task = task
        self.scheduledAction = scheduledAction
        self.confidence = confidence
        self.raw = raw
    }

    public var confirmationIdentity: String {
        if let task {
            return "task:\(task.id.uuidString)"
        }
        if let scheduledAction {
            return [
                "scheduled",
                scheduledAction.name,
                scheduledAction.url,
                scheduledAction.scheduleFrequency,
                scheduledAction.scheduledTimeISO,
                scheduledAction.scheduledWeekday.map(String.init) ?? ""
            ].joined(separator: "|")
        }
        return "raw:\(raw.count):\(confidence)"
    }
}

public struct ScheduledActionDraft {
    public let name: String
    public let url: String
    public let format: URLDataFormat
    public let scheduleFrequency: String
    public let scheduledTimeISO: String
    public let scheduledWeekday: Int?
    public let userPrompt: String?

    public init(
        name: String,
        url: String,
        format: URLDataFormat,
        scheduleFrequency: String,
        scheduledTimeISO: String,
        scheduledWeekday: Int?,
        userPrompt: String?
    ) {
        self.name = name
        self.url = url
        self.format = format
        self.scheduleFrequency = scheduleFrequency
        self.scheduledTimeISO = scheduledTimeISO
        self.scheduledWeekday = scheduledWeekday
        self.userPrompt = userPrompt
    }
}

public enum TaskParsingError: Error {
    case llmError(String)
    case invalidResponse
}

public final class TaskParsingService {
    private let modelClient: any AgentModelClient
    private let logger = Logger.shared

    public init(llmClient: LLMClient) {
        if let agentModelClient = llmClient as? any AgentModelClient {
            self.modelClient = agentModelClient
        } else {
            self.modelClient = TextAgentModelClient(client: llmClient, enableThinking: false)
        }
    }

    public init(agentModelClient: any AgentModelClient) {
        self.modelClient = agentModelClient
    }

    private static let responseSchema: AgentCoreJSONValue = .object([
        "type": .string("object"),
        "additionalProperties": .bool(false),
        "required": .array([
            "intent", "type", "title", "description", "targetDate", "isRecurring",
            "recurrenceRule", "relatedPerson", "amount", "currency", "dueDate",
            "scheduledAction", "confidence", "reason",
        ].map(AgentCoreJSONValue.string)),
        "properties": .object([
            "intent": .object([
                "type": .string("string"),
                "enum": .array(["none", "user_task", "scheduled_action"].map(AgentCoreJSONValue.string)),
            ]),
            "type": .object(["type": .array([.string("string"), .string("null")])]),
            "title": .object(["type": .array([.string("string"), .string("null")])]),
            "description": .object(["type": .array([.string("string"), .string("null")])]),
            "targetDate": .object(["type": .array([.string("string"), .string("null")])]),
            "isRecurring": .object(["type": .array([.string("boolean"), .string("null")])]),
            "recurrenceRule": .object([
                "type": .array([.string("object"), .string("null")]),
                "additionalProperties": .bool(false),
                "required": .array([.string("frequency"), .string("interval")]),
                "properties": .object([
                    "frequency": .object([
                        "type": .string("string"),
                        "enum": .array(["daily", "weekly", "monthly", "yearly"].map(AgentCoreJSONValue.string)),
                    ]),
                    "interval": .object([
                        "type": .string("integer"),
                        "minimum": .number(1),
                    ]),
                ]),
            ]),
            "relatedPerson": .object(["type": .array([.string("string"), .string("null")])]),
            "amount": .object(["type": .array([.string("number"), .string("null")])]),
            "currency": .object(["type": .array([.string("string"), .string("null")])]),
            "dueDate": .object(["type": .array([.string("string"), .string("null")])]),
            "scheduledAction": .object([
                "type": .array([.string("object"), .string("null")]),
                "additionalProperties": .bool(false),
                "required": .array([
                    "name", "url", "format", "scheduleFrequency", "scheduledTimeISO",
                    "scheduledWeekday", "userPrompt",
                ].map(AgentCoreJSONValue.string)),
                "properties": .object([
                    "name": .object(["type": .string("string"), "minLength": .number(1)]),
                    "url": .object(["type": .string("string"), "minLength": .number(1)]),
                    "format": .object([
                        "type": .string("string"),
                        "enum": .array(["html", "rss", "json", "icalendar"].map(AgentCoreJSONValue.string)),
                    ]),
                    "scheduleFrequency": .object([
                        "type": .string("string"),
                        "enum": .array(["daily", "weekly"].map(AgentCoreJSONValue.string)),
                    ]),
                    "scheduledTimeISO": .object(["type": .string("string"), "minLength": .number(4)]),
                    "scheduledWeekday": .object([
                        "type": .array([.string("integer"), .string("null")]),
                        "minimum": .number(1),
                        "maximum": .number(7),
                    ]),
                    "userPrompt": .object(["type": .array([.string("string"), .string("null")])]),
                ]),
            ]),
            "confidence": .object([
                "type": .string("number"),
                "minimum": .number(0),
                "maximum": .number(1),
            ]),
            "reason": .object(["type": .array([.string("string"), .string("null")])]),
        ]),
    ])

    private struct RawRecurrenceRule: Codable {
        let frequency: String?
        let interval: Int?

        init(frequency: String?, interval: Int?) {
            self.frequency = frequency
            self.interval = interval
        }

        init(from decoder: Decoder) throws {
            if let keyed = try? decoder.container(keyedBy: CodingKeys.self) {
                frequency = try keyed.decodeIfPresent(String.self, forKey: .frequency)
                interval = try keyed.decodeIfPresent(Int.self, forKey: .interval)
                return
            }
            let single = try decoder.singleValueContainer()
            if single.decodeNil() {
                frequency = nil
                interval = nil
                return
            }
            let raw = try single.decode(String.self)
            let parsed = Self.parse(raw)
            frequency = parsed.frequency
            interval = parsed.interval
        }

        private enum CodingKeys: String, CodingKey {
            case frequency
            case interval
        }

        private static func parse(_ raw: String) -> RawRecurrenceRule {
            let normalized = raw
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
            guard !normalized.isEmpty else {
                return RawRecurrenceRule(frequency: nil, interval: nil)
            }

            var frequency: String?
            var interval: Int?
            for part in normalized.split(separator: ";") {
                let pieces = part.split(separator: "=", maxSplits: 1).map(String.init)
                guard pieces.count == 2 else { continue }
                switch pieces[0] {
                case "freq":
                    frequency = pieces[1]
                case "interval":
                    interval = Int(pieces[1])
                default:
                    break
                }
            }
            if frequency == nil {
                frequency = normalized
                    .replacingOccurrences(of: "every ", with: "")
                    .replacingOccurrences(of: "freq=", with: "")
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            }
            return RawRecurrenceRule(frequency: frequency, interval: interval)
        }
    }

    private struct RawParse: Codable {
        let intent: String?
        let type: String?
        let title: String?
        let description: String?
        let targetDate: String?
        let isRecurring: Bool?
        let recurrenceRule: RawRecurrenceRule?
        let relatedPerson: String?
        let amount: Double?
        let currency: String?
        let dueDate: String?
        let confidence: Double?
        let reason: String?
        let scheduledAction: RawScheduledAction?
    }

    private struct RawScheduledAction: Codable {
        let name: String?
        let url: String?
        let format: String?
        let scheduleFrequency: String?
        let scheduledTimeISO: String?
        let scheduledWeekday: Int?
        let userPrompt: String?
    }

    /// Parse a user instruction into a UserTask. Returns TaskParseResult containing optional UserTask and raw model text.
    public func parseInstruction(_ text: String, timeout: TimeInterval? = 10.0) async -> TaskParseResult {
        logger.log("Parsing an explicit task-editor request with the configured model", level: .debug, category: "TaskParsingService", metadata: [
            "text": text
        ])

        // Get current date and time for context
        let now = Date()
        
        // Get local timezone identifier and offset
        let timeZone = TimeZone.current
        let timeZoneOffset = timeZone.secondsFromGMT() / 3600
        let tzSign = timeZoneOffset >= 0 ? "+" : ""
        let tzStr = String(format: "%@%02d:00", tzSign, abs(timeZoneOffset))

                // Prepare a few concrete examples to include in the system prompt so the LLM learns mapping
                func isoString(for date: Date) -> String {
                    // Produce a naive local ISO-like string WITHOUT timezone suffix
                    let df = DateFormatter()
                    df.locale = Locale(identifier: "en_US_POSIX")
                    df.timeZone = TimeZone.current
                    df.dateFormat = "yyyy-MM-dd'T'HH:mm:ss"
                    return df.string(from: date)
                }

                let calendar = Calendar.current

                func dateByAddingDays(_ days: Int, hour: Int, minute: Int) -> Date? {
                    guard let base = calendar.date(byAdding: .day, value: days, to: calendar.startOfDay(for: now)) else {
                        return nil
                    }
                    var components = calendar.dateComponents([.year, .month, .day], from: base)
                    components.hour = hour
                    components.minute = minute
                    components.second = 0
                    return calendar.date(from: components)
                }

                guard let dayAfterTomorrow19 = dateByAddingDays(2, hour: 19, minute: 0),
                      let dayAfterTomorrow08 = dateByAddingDays(2, hour: 8, minute: 0),
                      let dayAfterTomorrow09 = dateByAddingDays(2, hour: 9, minute: 0),
                      let tomorrow1530 = dateByAddingDays(1, hour: 15, minute: 30) else {
                    logger.log("TaskParsingService failed to calculate prompt example dates", level: .error, category: "TaskParsingService")
                    return TaskParseResult(success: false, task: nil, scheduledAction: nil, confidence: 0.0, raw: "")
                }

                let exampleDayAfterTomorrow19 = isoString(for: dayAfterTomorrow19)
                let exampleDayAfterTomorrow08 = isoString(for: dayAfterTomorrow08)
                let exampleDayAfterTomorrow09 = isoString(for: dayAfterTomorrow09)
                let exampleTomorrow1530 = isoString(for: tomorrow1530)

                // explicit absolute example
                let explicitExample = "2025-12-01T09:30:00\(tzStr)"

                // Build the system prompt using the central PromptTemplates helper so templates are centralized.
                let replacements: [String: String] = [
                    "CURRENT_LOCAL_DATETIME": isoString(for: now),
                    "TIMEZONE": "\(timeZone.identifier) UTC\(tzStr)",
                    "DAY_AFTER_TOMORROW_19": exampleDayAfterTomorrow19,
                    "DAY_AFTER_TOMORROW_08": exampleDayAfterTomorrow08,
                    "DAY_AFTER_TOMORROW_09": exampleDayAfterTomorrow09,
                    "TOMORROW_1530": exampleTomorrow1530,
                    "EXPLICIT": explicitExample
                ]

                let systemPrompt = PromptTemplates.taskParsingSystem(replacements: replacements)

                do {
                    let operationId = "task-parser:\(UUID().uuidString)"
                    let structured = try await AgentStructuredOutputExecutor.execute(
                        sessionId: operationId,
                        turnId: operationId,
                        systemInstructions: systemPrompt + "\nReturn the complete result as the arguments of submit_task_parse. Do not emit marker wrappers or prose.",
                        modelInput: .object(["user_request": .string(text)]),
                        responseSchema: Self.responseSchema,
                        submissionToolName: "submit_task_parse",
                        submissionToolDescription: "Submit the parsed task, scheduled action, or explicit no-action verdict.",
                        client: modelClient,
                        reservedOutputTokens: 2_048
                    )
                    let data = try JSONEncoder().encode(structured)
                    let rawResponse = String(decoding: data, as: UTF8.self)
                    logger.log("TaskParsingService structured response", level: .debug, category: "TaskParsingService", metadata: [
                        "preview": String(rawResponse.prefix(800))
                    ])
                    return decodeAndMap(data: data, rawResponse: rawResponse, userText: text)
                } catch {
                    logger.log("TaskParsingService rejected structured output: \(error)", level: .error, category: "TaskParsingService")
                    return TaskParseResult(success: false, task: nil, scheduledAction: nil, confidence: 0.0, raw: "")
                }
        }

    private func decodeAndMap(data: Data, rawResponse: String, userText: String) -> TaskParseResult {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601

        do {
            let parsed = try decoder.decode(RawParse.self, from: data)
            let confidence = parsed.confidence ?? 0.5
            let normalizedIntent = parsed.intent?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
            let normalizedReason = parsed.reason?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            
            logger.log("TaskParsingService LLM raw JSON", level: .debug, category: "TaskParsingService", metadata: [
                "json": rawResponse
            ])
            logger.log("TaskParsingService LLM parsed", level: .debug, category: "TaskParsingService", metadata: [
                "intent": parsed.intent ?? "null",
                "type": parsed.type ?? "null",
                "title": parsed.title ?? "null",
                "targetDate": parsed.targetDate ?? "null",
                "confidence": String(confidence)
            ])

            if isNoActionIntent(normalizedIntent) || indicatesNoActionReason(normalizedReason) {
                logger.log("TaskParsingService detected non-action parse result; fallback to normal chat", level: .info, category: "TaskParsingService", metadata: [
                    "intent": normalizedIntent.isEmpty ? "nil" : normalizedIntent,
                    "reason": normalizedReason.isEmpty ? "nil" : normalizedReason,
                    "confidence": String(confidence)
                ])
                return TaskParseResult(success: false, task: nil, scheduledAction: nil, confidence: confidence, raw: rawResponse)
            }

            if let scheduledAction = mapScheduledAction(from: parsed, userText: userText) {
                let confidence = parsed.confidence ?? 0.8
                logger.log("TaskParsingService parsed scheduled action (name=\(scheduledAction.name), confidence=\(confidence))", level: .info, category: "TaskParsingService")
                return TaskParseResult(success: true, task: nil, scheduledAction: scheduledAction, confidence: confidence, raw: rawResponse)
            }

            let type = TaskType(rawValue: parsed.type ?? "custom") ?? .custom
            let title = parsed.title ?? ""
            let description = parsed.description

            let targetDate = parsed.targetDate.flatMap(parseModelDate)
            
            // Log parsing result
            if let td = targetDate {
                logger.log("TaskParsingService targetDate parsed successfully", level: .debug, category: "TaskParsingService", metadata: [
                    "targetDate": td.ISO8601Format()
                ])
            } else if parsed.targetDate != nil {
                logger.log("TaskParsingService failed to parse targetDate string: \(parsed.targetDate ?? "nil")", level: .error, category: "TaskParsingService")
            } else {
                logger.log("TaskParsingService targetDate is null from LLM", level: .debug, category: "TaskParsingService")
            }

            let isRecurring = parsed.isRecurring ?? false
            var recurrence: RecurrenceRule? = nil
            if let rr = parsed.recurrenceRule {
                let freq = RecurrenceFrequency(rawValue: rr.frequency ?? "daily") ?? .daily
                recurrence = RecurrenceRule(frequency: freq, interval: rr.interval ?? 1)
            }

            let relatedPerson = parsed.relatedPerson
            let amount: Decimal? = parsed.amount.map { Decimal($0) }
            let currency = parsed.currency
            let dueDate = parsed.dueDate.flatMap(parseModelDate)

            let now = Date()
            var ut = UserTask(
                id: UUID(),
                type: type,
                title: title,
                description: description,
                targetDate: targetDate,
                isRecurring: isRecurring,
                recurrenceRule: recurrence,
                relatedPerson: relatedPerson,
                amount: amount,
                currency: currency,
                dueDate: dueDate,
                isCompleted: false,
                createdAt: now,
                updatedAt: now,
                notificationIds: []
            )

            // Normalize dates
            if ut.type == .birthday, let t = ut.targetDate {
                ut.targetDate = nextOccurrenceForBirthday(t, reference: now)
            }

            if ut.type == .debt {
                // Try parsing dueDate from LLM output or user text using enhanced non-ISO parsing
                if ut.dueDate == nil, let userInferred = tryParseDateString(userText, reference: now) {
                    ut.dueDate = userInferred
                } else if ut.dueDate == nil, let rawInferred = tryParseDateString(rawResponse, reference: now) {
                    ut.dueDate = rawInferred
                }
            }

            if ut.type == .reminder && ut.isRecurring && ut.targetDate == nil {
                if let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: Calendar.current.startOfDay(for: now)) {
                    var comps = Calendar.current.dateComponents([.year, .month, .day], from: tomorrow)
                    comps.hour = 9; comps.minute = 0
                    ut.targetDate = Calendar.current.date(from: comps)
                }
            }
            // If reminder has no explicit targetDate parsed from LLM, try enhanced parsing from user/raw text
            if ut.type == .reminder && ut.targetDate == nil {
                if let inferred = tryParseDateString(userText, reference: now) {
                    ut.targetDate = inferred
                    logger.log("TaskParsingService inferred targetDate from userText: \(inferred.ISO8601Format())", level: .info, category: "TaskParsingService")
                } else if let inferred2 = tryParseDateString(rawResponse, reference: now) {
                    ut.targetDate = inferred2
                    logger.log("TaskParsingService inferred targetDate from rawResponse: \(inferred2.ISO8601Format())", level: .info, category: "TaskParsingService")
                }
            }
            if ut.type == .reminder, let targetDate = ut.targetDate, targetDate <= now {
                if let inferred = tryParseDateString(userText, reference: now), inferred > now {
                    ut.targetDate = inferred
                    logger.log("TaskParsingService replaced past reminder targetDate from userText: \(inferred.ISO8601Format())", level: .warn, category: "TaskParsingService")
                } else if let next = nextFutureOccurrence(preservingTimeOfDayFrom: targetDate, reference: now) {
                    ut.targetDate = next
                    logger.log("TaskParsingService moved past reminder targetDate to future occurrence: \(next.ISO8601Format())", level: .warn, category: "TaskParsingService")
                }
            }

            logger.log("TaskParsingService parsed task (title=\(ut.title), confidence=\(confidence))", level: .info, category: "TaskParsingService")
            return TaskParseResult(success: true, task: ut, scheduledAction: nil, confidence: confidence, raw: rawResponse)

        } catch {
            logger.log("TaskParsingService decode error: \(error)", level: .error, category: "TaskParsingService")
            return TaskParseResult(success: false, task: nil, scheduledAction: nil, confidence: 0.0, raw: rawResponse)
        }
    }

    private func isNoActionIntent(_ intent: String) -> Bool {
        guard !intent.isEmpty else { return false }
        return intent == "none" || intent == "no_action" || intent == "non_task" || intent == "chat" || intent == "question"
    }

    private func indicatesNoActionReason(_ reason: String) -> Bool {
        guard !reason.isEmpty else { return false }
        let normalized = reason.lowercased()
        let markers = [
            "no actionable",
            "not actionable",
            "no task detected",
            "not a task",
            "no executable task",
            "不是任务",
            "无可执行",
            "未检测到任务",
            "不需要执行",
            "仅在询问",
            "无法执行任务"
        ]
        return markers.contains { normalized.contains($0) }
    }

    private func mapScheduledAction(from parsed: RawParse, userText: String) -> ScheduledActionDraft? {
        let explicitIntent = parsed.intent?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let raw = parsed.scheduledAction

        let url = normalizeScheduledActionURL(raw?.url)
        let hasURL = !url.isEmpty && URL(string: url)?.host != nil
        let shouldTreatAsScheduledAction = explicitIntent == "scheduled_action" || (raw != nil && hasURL)
        guard shouldTreatAsScheduledAction, hasURL else {
            return nil
        }

        let normalizedTime = ScheduledTimeParser.normalizeToHHmm(raw?.scheduledTimeISO ?? parsed.targetDate ?? "09:00") ?? "09:00"
        let normalizedFrequency = ScheduledTimeParser.normalizeFrequency(raw?.scheduleFrequency) ?? "daily"
        let weekday: Int? = {
            guard normalizedFrequency == "weekly" else { return nil }
            let candidate = raw?.scheduledWeekday ?? Calendar.current.component(.weekday, from: Date())
            return (1...7).contains(candidate) ? candidate : Calendar.current.component(.weekday, from: Date())
        }()

        let trimmedPrompt = raw?.userPrompt?.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = (trimmedPrompt?.isEmpty == false) ? trimmedPrompt : parsed.description?.trimmingCharacters(in: .whitespacesAndNewlines)

        let fallbackName = URL(string: url)?.host?.replacingOccurrences(of: "www.", with: "") ?? userText.trimmingCharacters(in: .whitespacesAndNewlines)
        let candidateName = raw?.name?.trimmingCharacters(in: .whitespacesAndNewlines)
        let candidateTitle = parsed.title?.trimmingCharacters(in: .whitespacesAndNewlines)
        let name: String
        if let candidateName, !candidateName.isEmpty {
            name = candidateName
        } else if let candidateTitle, !candidateTitle.isEmpty {
            name = candidateTitle
        } else {
            name = fallbackName
        }

        return ScheduledActionDraft(
            name: name,
            url: url,
            format: parseFormat(raw?.format, url: url),
            scheduleFrequency: normalizedFrequency,
            scheduledTimeISO: normalizedTime,
            scheduledWeekday: weekday,
            userPrompt: prompt
        )
    }

    private func parseFormat(_ raw: String?, url: String) -> URLDataFormat {
        if let raw = raw?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased(), let format = URLDataFormat(rawValue: raw) {
            return format
        }

        let lower = url.lowercased()
        if lower.hasSuffix(".ics") {
            return .icalendar
        }
        if lower.contains("rss") || lower.contains("feed") || lower.contains("atom") {
            return .rss
        }
        if lower.hasSuffix(".json") {
            return .json
        }
        return .html
    }

    private func normalizeScheduledActionURL(_ raw: String?) -> String {
        let trimmed = raw?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !trimmed.isEmpty else { return "" }
        if let url = URL(string: trimmed), url.scheme != nil {
            return trimmed
        }
        if trimmed.hasPrefix("www.") {
            return "https://\(trimmed)"
        }
        return trimmed
    }

    private func parseModelDate(_ dateStr: String) -> Date? {
        let trimmed = dateStr.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        let isoWithFractionalSeconds = ISO8601DateFormatter()
        isoWithFractionalSeconds.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = isoWithFractionalSeconds.date(from: trimmed) { return date }

        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime]
        if let date = iso.date(from: trimmed) { return date }

        let patterns = [
            "yyyy-MM-dd'T'HH:mm:ssZZZ",
            "yyyy-MM-dd'T'HH:mm:ss",
            "yyyy-MM-dd HH:mm:ss",
            "yyyy-MM-dd HH:mm",
            "yyyy-MM-dd"
        ]
        for pattern in patterns {
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

    private func extractFirstJSONObject(from text: String) -> String? {
        // Prefer explicit markers placed by the model to avoid truncation issues
        if let startMarkerRange = text.range(of: "###JSON_START###"), let endMarkerRange = text.range(of: "###JSON_END###", range: startMarkerRange.upperBound..<text.endIndex) {
            let inner = text[startMarkerRange.upperBound..<endMarkerRange.lowerBound]
            // Trim surrounding whitespace and code fences
            var candidate = inner.trimmingCharacters(in: .whitespacesAndNewlines)
            if candidate.hasPrefix("```json") {
                candidate = candidate.replacingOccurrences(of: "```json", with: "")
            }
            candidate = candidate.replacingOccurrences(of: "```", with: "").trimmingCharacters(in: .whitespacesAndNewlines)
            return candidate
        }

        // Fallback: Find the first '{' and the matching '}' by scanning
        guard let start = text.firstIndex(of: "{") else { return nil }
        var depth = 0
        var index = start
        while index < text.endIndex {
            let ch = text[index]
            if ch == "{" { depth += 1 }
            else if ch == "}" { depth -= 1 }
            if depth == 0 {
                let json = String(text[start...index])
                return json
            }
            index = text.index(after: index)
        }
        return nil
    }

    private func nextOccurrenceForBirthday(_ date: Date, reference now: Date) -> Date {
        var comps = Calendar.current.dateComponents([.month, .day, .hour, .minute, .second], from: date)
        let currentYear = Calendar.current.component(.year, from: now)
        comps.year = currentYear
        if let candidate = Calendar.current.date(from: comps) {
            if candidate >= now { return candidate }
            var nextYearComps = comps
            nextYearComps.year = currentYear + 1
            return Calendar.current.date(from: nextYearComps) ?? candidate
        }
        return date
    }
    /// Enhanced non-ISO date parsing.
    /// Attempts multiple strategies (ISO8601 variants, many DateFormatter patterns, and localized heuristics)
    /// to parse a wide range of date/time strings including common Chinese formats.
    private func tryParseDateString(_ raw: String, reference now: Date) -> Date? {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.isEmpty { return nil }

        // Quick-extract potential date-like substring between markers if long LLM output
        var candidateStrings: [String] = [s]
        // If there's a line containing typical date separators, prioritize those substrings
        if s.contains("年") || s.contains("月") || s.contains("/") || s.contains("-") || s.contains(":" ) {
            let lines = s.components(separatedBy: CharacterSet.newlines)
            for line in lines where line.contains("年") || line.contains("月") || line.contains("/") || line.contains("-") {
                candidateStrings.insert(line.trimmingCharacters(in: .whitespaces), at: 0)
            }
        }

        // Common date formats to try (both Chinese locale and en_US_POSIX)
        let formatPatterns = [
            "yyyy-MM-dd'T'HH:mm:ssZ",
            "yyyy-MM-dd'T'HH:mm:ss",
            "yyyy-MM-dd'T'HH:mm:ss.SSSZ",
            "yyyy-MM-dd HH:mm",
            "yyyy/MM/dd HH:mm",
            "yyyy-MM-dd",
            "MM/dd/yyyy HH:mm",
            "MM/dd HH:mm",
            NSLocalizedString("autogen.TaskParsingService.29b8a027", comment: "AUTOGEN"),
            NSLocalizedString("autogen.TaskParsingService.fbe2ad89", comment: "AUTOGEN"),
            NSLocalizedString("autogen.TaskParsingService.01f7fe78", comment: "AUTOGEN"),
            NSLocalizedString("autogen.TaskParsingService.3054424b", comment: "AUTOGEN"),
            "HH:mm",
            "H:mm"
        ]

        // Try ISO8601 first
        let iso1 = ISO8601DateFormatter()
        iso1.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = iso1.date(from: s) { return d }
        let iso2 = ISO8601DateFormatter()
        iso2.formatOptions = [.withInternetDateTime]
        if let d = iso2.date(from: s) { return d }

        // Try the format patterns on candidate substrings
        for cand in candidateStrings {
                for pattern in formatPatterns {
                let df = DateFormatter()
                df.locale = Locale(identifier: pattern.contains("年") || pattern.contains("月") ? "zh_CN" : "en_US_POSIX")
                df.dateFormat = pattern
                // Interpret naive formats as local time
                df.timeZone = TimeZone.current
                if let d = df.date(from: cand) {
                    return d
                }
            }
        }

        // Localized heuristics: handle Chinese relative keywords (integrated, not separate fallback)
        let lower = s.replacingOccurrences(of: "：", with: ":").lowercased()
        let calendar = Calendar.current
        if lower.contains(NSLocalizedString("autogen.TaskParsingService.f40a330d", comment: "AUTOGEN")) || lower.contains(NSLocalizedString("autogen.TaskParsingService.639e8f00", comment: "AUTOGEN")) || lower.contains(NSLocalizedString("autogen.TaskParsingService.b76ce230", comment: "AUTOGEN")) || lower.contains(NSLocalizedString("autogen.TaskParsingService.17e83cc2", comment: "AUTOGEN")) {
            var dayOffset = 0
            if lower.contains(NSLocalizedString("autogen.TaskParsingService.f40a330d", comment: "AUTOGEN")) { dayOffset = 3 }
            else if lower.contains(NSLocalizedString("autogen.TaskParsingService.639e8f00", comment: "AUTOGEN")) { dayOffset = 2 }
            else if lower.contains(NSLocalizedString("autogen.TaskParsingService.b76ce230", comment: "AUTOGEN")) { dayOffset = 1 }
            else if lower.contains(NSLocalizedString("autogen.TaskParsingService.17e83cc2", comment: "AUTOGEN")) { dayOffset = 0 }

            // extract time if present
            var hour: Int? = nil
            var minute = 0
            if let m = try? NSRegularExpression(pattern: #"([0-9]{1,2})\s*[:：]\s*([0-9]{1,2})"#, options: []) {
                if let match = m.firstMatch(in: lower, options: [], range: NSRange(lower.startIndex..., in: lower)) {
                    if let hRange = Range(match.range(at: 1), in: lower), let miRange = Range(match.range(at: 2), in: lower) {
                        hour = Int(lower[hRange])
                        minute = Int(lower[miRange]) ?? 0
                    }
                }
            }
            if hour == nil {
                if let m2 = try? NSRegularExpression(pattern: #"([0-9]{1,2})\s*点半"#, options: []) {
                    if let match = m2.firstMatch(in: lower, options: [], range: NSRange(lower.startIndex..., in: lower)) {
                        if let hRange = Range(match.range(at: 1), in: lower) { hour = Int(lower[hRange]); minute = 30 }
                    }
                }
            }
            if hour == nil {
                if let m3 = try? NSRegularExpression(pattern: #"([0-9]{1,2})\s*点"#, options: []) {
                    if let match = m3.firstMatch(in: lower, options: [], range: NSRange(lower.startIndex..., in: lower)) {
                        if let hRange = Range(match.range(at: 1), in: lower) { hour = Int(lower[hRange]); minute = 0 }
                    }
                }
            }

            // reasonable defaults
            let defaultHour = lower.contains(NSLocalizedString("autogen.TaskParsingService.4ca5b0f2", comment: "AUTOGEN")) ? 5 : (lower.contains(NSLocalizedString("autogen.TaskParsingService.172fb719", comment: "AUTOGEN")) ? 8 : (lower.contains(NSLocalizedString("autogen.TaskParsingService.e214ee22", comment: "AUTOGEN")) ? 9 : (lower.contains(NSLocalizedString("autogen.TaskParsingService.0c5382c1", comment: "AUTOGEN")) ? 12 : (lower.contains(NSLocalizedString("autogen.TaskParsingService.1dfaab65", comment: "AUTOGEN")) ? 15 : (lower.contains(NSLocalizedString("autogen.TaskParsingService.5ef2d60e", comment: "AUTOGEN")) ? 18 : (lower.contains(NSLocalizedString("autogen.TaskParsingService.76a41594", comment: "AUTOGEN")) ? 19 : 9))))))
            let finalHour = normalizeHourForDayPeriod(hour ?? defaultHour, in: lower)

            if let base = calendar.date(byAdding: .day, value: dayOffset, to: calendar.startOfDay(for: now)) {
                var comps = calendar.dateComponents([.year, .month, .day], from: base)
                comps.hour = finalHour; comps.minute = minute
                if let d = calendar.date(from: comps) { return d }
            }
        }

        if let timeOnly = timeOnlyDate(from: lower, reference: now), timeOnly > now {
            return timeOnly
        } else if let timeOnly = timeOnlyDate(from: lower, reference: now),
                  let tomorrow = calendar.date(byAdding: .day, value: 1, to: timeOnly) {
            return tomorrow
        }

        // English relative heuristics (tomorrow / day after tomorrow)
        if lower.contains("tomorrow") || lower.contains("day after tomorrow") {
            var dayOffset = 1
            if lower.contains("day after tomorrow") { dayOffset = 2 }
            var hour: Int? = nil; var minute = 0
            if let m = try? NSRegularExpression(pattern: #"([0-9]{1,2}):([0-9]{2})"#, options: []) {
                if let match = m.firstMatch(in: lower, options: [], range: NSRange(lower.startIndex..., in: lower)) {
                    if let hRange = Range(match.range(at: 1), in: lower), let miRange = Range(match.range(at: 2), in: lower) {
                        hour = Int(lower[hRange]); minute = Int(lower[miRange]) ?? 0
                    }
                }
            }
            let finalHour = hour ?? 9
            if let base = calendar.date(byAdding: .day, value: dayOffset, to: calendar.startOfDay(for: now)) {
                var comps = calendar.dateComponents([.year, .month, .day], from: base)
                comps.hour = finalHour; comps.minute = minute
                if let d = calendar.date(from: comps) { return d }
            }
        }

        return nil
    }

    private func timeOnlyDate(from lower: String, reference now: Date) -> Date? {
        let calendar = Calendar.current
        var hour: Int?
        var minute = 0

        if let m = try? NSRegularExpression(pattern: #"([0-9]{1,2})\s*[:：]\s*([0-9]{1,2})"#, options: []),
           let match = m.firstMatch(in: lower, options: [], range: NSRange(lower.startIndex..., in: lower)),
           let hRange = Range(match.range(at: 1), in: lower),
           let miRange = Range(match.range(at: 2), in: lower) {
            hour = Int(lower[hRange])
            minute = Int(lower[miRange]) ?? 0
        }

        if hour == nil,
           let m = try? NSRegularExpression(pattern: #"([0-9]{1,2})\s*点半"#, options: []),
           let match = m.firstMatch(in: lower, options: [], range: NSRange(lower.startIndex..., in: lower)),
           let hRange = Range(match.range(at: 1), in: lower) {
            hour = Int(lower[hRange])
            minute = 30
        }

        if hour == nil,
           let m = try? NSRegularExpression(pattern: #"([0-9]{1,2})\s*点"#, options: []),
           let match = m.firstMatch(in: lower, options: [], range: NSRange(lower.startIndex..., in: lower)),
           let hRange = Range(match.range(at: 1), in: lower) {
            hour = Int(lower[hRange])
            minute = 0
        }

        if hour == nil {
            if lower.contains(NSLocalizedString("autogen.TaskParsingService.4ca5b0f2", comment: "AUTOGEN")) {
                hour = 5
            } else if lower.contains(NSLocalizedString("autogen.TaskParsingService.172fb719", comment: "AUTOGEN")) {
                hour = 8
            } else if lower.contains(NSLocalizedString("autogen.TaskParsingService.e214ee22", comment: "AUTOGEN")) {
                hour = 9
            } else if lower.contains(NSLocalizedString("autogen.TaskParsingService.0c5382c1", comment: "AUTOGEN")) {
                hour = 12
            } else if lower.contains(NSLocalizedString("autogen.TaskParsingService.1dfaab65", comment: "AUTOGEN")) {
                hour = 15
            } else if lower.contains(NSLocalizedString("autogen.TaskParsingService.5ef2d60e", comment: "AUTOGEN")) {
                hour = 18
            } else if lower.contains(NSLocalizedString("autogen.TaskParsingService.76a41594", comment: "AUTOGEN")) {
                hour = 19
            }
        }

        guard let rawHour = hour else {
            return nil
        }
        let finalHour = normalizeHourForDayPeriod(rawHour, in: lower)
        guard (0...23).contains(finalHour), (0...59).contains(minute) else {
            return nil
        }

        var comps = calendar.dateComponents([.year, .month, .day], from: now)
        comps.hour = finalHour
        comps.minute = minute
        comps.second = 0
        return calendar.date(from: comps)
    }

    private func normalizeHourForDayPeriod(_ hour: Int, in lower: String) -> Int {
        var normalized = hour
        let isAfternoonLike = lower.contains(NSLocalizedString("autogen.TaskParsingService.1dfaab65", comment: "AUTOGEN"))
            || lower.contains(NSLocalizedString("autogen.TaskParsingService.5ef2d60e", comment: "AUTOGEN"))
            || lower.contains(NSLocalizedString("autogen.TaskParsingService.76a41594", comment: "AUTOGEN"))
        if isAfternoonLike && (1...11).contains(normalized) {
            normalized += 12
        }
        if lower.contains(NSLocalizedString("autogen.TaskParsingService.0c5382c1", comment: "AUTOGEN")),
           (1...10).contains(normalized) {
            normalized += 12
        }
        if lower.contains(NSLocalizedString("autogen.TaskParsingService.4ca5b0f2", comment: "AUTOGEN")),
           normalized == 12 {
            normalized = 0
        }
        if lower.contains(NSLocalizedString("autogen.TaskParsingService.e214ee22", comment: "AUTOGEN")),
           normalized == 12 {
            normalized = 0
        }
        return normalized
    }

    private func nextFutureOccurrence(preservingTimeOfDayFrom date: Date, reference now: Date) -> Date? {
        let calendar = Calendar.current
        let time = calendar.dateComponents([.hour, .minute, .second], from: date)
        var comps = calendar.dateComponents([.year, .month, .day], from: now)
        comps.hour = time.hour
        comps.minute = time.minute
        comps.second = time.second ?? 0
        guard var candidate = calendar.date(from: comps) else { return nil }
        while candidate <= now {
            guard let next = calendar.date(byAdding: .day, value: 1, to: candidate) else { return nil }
            candidate = next
        }
        return candidate
    }
}
