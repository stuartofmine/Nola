// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements RustInterop logic in the iOS service layer.
// - Interfaces:
//   - RustInterop
// - APIs:
//   - none
//
import Foundation
import CryptoKit

/// Native iOS implementations for event processing, rules, and model management.
public struct RustInterop {
    private enum StorageKey {
        static let modelSelectionModels = "com.nola.rustinterop.modelSelection.models"
        static let modelSelectionLastSelected = "com.nola.rustinterop.modelSelection.lastSelected"
    }

    private final class RulesState: @unchecked Sendable {
        let lock = NSLock()
        var config = RulesConfig(priorityThreshold: 0.6, quietHoursStart: 22, quietHoursEnd: 7, throttleWindowHours: 24)
        var recentNotifications: [String: Date] = [:]
    }

    fileprivate final class ModelManagerState: NSObject, @unchecked Sendable {
        let lock = NSLock()
        var tasks: [String: URLSessionDownloadTask] = [:]
        var observations: [String: NSKeyValueObservation] = [:]
        var progresses: [String: Double] = [:]
        var destinations: [String: String] = [:]
    }

    private static let rulesState = RulesState()

    public struct DecisionResult: Codable {
        public let saveSummary: Bool
        public let summaryText: String?
        public let summaryEvent: ProcessedEvent?
        public let candidates: [NotificationCandidate]
    }

    public struct ScheduleDecision: Codable {
        public let event: ProcessedEvent
        public let scheduledTime: String?
    }

    public struct RulesConfig: Codable {
        public var priorityThreshold: Double
        public var quietHoursStart: Int
        public var quietHoursEnd: Int
        public var throttleWindowHours: Int
    }

    public final class ModelManagerHandle {
        fileprivate let state = ModelManagerState()
    }

    public static func generateDedupeKey(source: String, title: String, dateISO: String?, sourceId: String?) -> String? {
        let normalizedTitle = normalizeTitle(title)
        let titleHash = stableHexHash(normalizedTitle)
        let normalizedDate = normalizedDateComponent(from: dateISO)
        let normalizedSourceId = sourceId?.trimmingCharacters(in: .whitespacesAndNewlines)

        if let normalizedSourceId, !normalizedSourceId.isEmpty {
            return "\(source):\(normalizedSourceId):\(titleHash):\(normalizedDate)"
        }
        return "\(source):\(titleHash):\(normalizedDate)"
    }

    public static func parseURLJSON(_ data: Data) -> [RawEvent]? {
        let decoder = makeJSONDecoder()

        if let events = try? decoder.decode([RawEvent].self, from: data) {
            return events
        }

        if let event = try? decoder.decode(RawEvent.self, from: data) {
            return [event]
        }

        Logger.shared.log(.warn, "RustInterop", "Failed to parse URL JSON with Swift decoder")
        return nil
    }

    public static func processRawEvents(_ data: Data) -> [ProcessedEvent]? {
        guard let events = tryDecode([RawEvent].self, from: data) else {
            Logger.shared.log(.warn, "RustInterop", "Failed to decode RawEvent payload for processRawEvents")
            return nil
        }
        return mapRawEventsToProcessedEvents(events)
    }

    public static func preprocessRawEvents(_ data: Data) -> [ProcessedEvent]? {
        processRawEvents(data)
    }

    public static func transformModelOutput(_ modelData: Data, sourceId: String, sourceType: String) -> [ProcessedEvent]? {
        guard let modelOutput = tryDecode(ModelOutput.self, from: modelData) else {
            Logger.shared.log(.warn, "RustInterop", "Failed to decode ModelOutput payload for transformModelOutput")
            return nil
        }

        let resolvedSourceType = ProcessedEvent.SourceType(rawValue: sourceType.lowercased()) ?? .manual
        return DefaultDataTransformationService().transform(
            modelOutput: modelOutput,
            sourceId: sourceId,
            sourceType: resolvedSourceType
        )
    }

    public static func decideActions(processed: Data, modelOutput: Data) -> DecisionResult? {
        guard let processedEvents = tryDecode([ProcessedEvent].self, from: processed),
              let output = tryDecode(ModelOutput.self, from: modelOutput) else {
            Logger.shared.log(.warn, "RustInterop", "Failed to decode payload for decideActions")
            return nil
        }

        let candidates = buildCandidates(processedEvents: processedEvents, modelOutput: output)
        let summaryText = output.explain?.trimmedNilIfEmpty
        let summaryEvent = candidates.first?.event ?? processedEvents.max(by: { $0.priority < $1.priority })

        return DecisionResult(
            saveSummary: summaryText != nil || !output.items.isEmpty,
            summaryText: summaryText,
            summaryEvent: summaryEvent,
            candidates: candidates
        )
    }

    public static func decideActionsRawJSON(processed: Data, modelOutput: Data) -> String? {
        guard let result = decideActions(processed: processed, modelOutput: modelOutput),
              let data = try? makeJSONEncoder().encode(result) else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    public static func schedulerSchedule(_ processedData: Data, targetHour: Int) -> [ScheduleDecision]? {
        guard let processedEvents = tryDecode([ProcessedEvent].self, from: processedData) else {
            Logger.shared.log(.warn, "RustInterop", "Failed to decode payload for schedulerSchedule")
            return nil
        }

        let calendar = Calendar.current
        let decisions = processedEvents.map { event in
            guard let date = event.date else {
                return ScheduleDecision(event: event, scheduledTime: nil)
            }

            var components = calendar.dateComponents([.year, .month, .day], from: date)
            components.hour = max(0, min(targetHour, 23))
            components.minute = 0
            components.second = 0
            let scheduledDate = calendar.date(from: components)
            return ScheduleDecision(event: event, scheduledTime: scheduledDate.map(formatISO8601))
        }

        return decisions
    }

    public static func modelManagerNew() -> ModelManagerHandle? {
        ModelManagerHandle()
    }

    public static func modelManagerFree(_ handle: ModelManagerHandle?) {
        guard let handle else { return }
        handle.state.lock.lock()
        let tasks = Array(handle.state.tasks.values)
        handle.state.tasks.removeAll()
        handle.state.observations.removeAll()
        handle.state.progresses.removeAll()
        handle.state.destinations.removeAll()
        handle.state.lock.unlock()
        tasks.forEach { $0.cancel() }
    }

    public static func modelSHA256(path: String) -> String? {
        let url = URL(fileURLWithPath: path)

        guard FileManager.default.fileExists(atPath: url.path) else { return nil }

        do {
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }

            var hasher = SHA256()
            while true {
                let chunk = try handle.read(upToCount: 1_048_576) ?? Data()
                if chunk.isEmpty { break }
                hasher.update(data: chunk)
            }
            let digest = hasher.finalize()
            return digest.map { String(format: "%02x", $0) }.joined()
        } catch {
            Logger.shared.log(.error, "RustInterop", "Failed to compute model SHA256", metadata: ["error": error.localizedDescription, "path": path])
            return nil
        }
    }

    public static func modelManagerDownload(_ handle: ModelManagerHandle?, url: String, destPath: String) -> Bool {
        guard let handle,
              let sourceURL = URL(string: url),
              !destPath.isEmpty else {
            return false
        }

        let destinationURL = URL(fileURLWithPath: destPath)
        do {
            try FileManager.default.createDirectory(
                at: destinationURL.deletingLastPathComponent(),
                withIntermediateDirectories: true,
                attributes: nil
            )
        } catch {
            Logger.shared.log(.error, "RustInterop", "Failed to prepare download destination", metadata: ["error": error.localizedDescription, "destPath": destPath])
            return false
        }

        let state = handle.state
        let task = URLSession.shared.downloadTask(with: sourceURL) { tempURL, _, error in
            defer {
                state.lock.lock()
                state.tasks.removeValue(forKey: url)
                state.observations.removeValue(forKey: url)
                state.progresses[url] = error == nil ? 1.0 : -1.0
                state.lock.unlock()
            }

            if let error {
                Logger.shared.log(.warn, "RustInterop", "Model download failed", metadata: ["error": error.localizedDescription, "url": url])
                return
            }

            guard let tempURL else {
                Logger.shared.log(.warn, "RustInterop", "Model download finished without a temp file", metadata: ["url": url])
                return
            }

            do {
                if FileManager.default.fileExists(atPath: destinationURL.path) {
                    try FileManager.default.removeItem(at: destinationURL)
                }
                try FileManager.default.moveItem(at: tempURL, to: destinationURL)
            } catch {
                Logger.shared.log(.error, "RustInterop", "Failed to move downloaded model", metadata: ["error": error.localizedDescription, "destPath": destPath])
            }
        }

        let observation = task.progress.observe(\.fractionCompleted, options: [.initial, .new]) { [weak state] progress, _ in
            state?.lock.lock()
            state?.progresses[url] = progress.fractionCompleted
            state?.lock.unlock()
        }

        state.lock.lock()
        state.tasks[url] = task
        state.observations[url] = observation
        state.progresses[url] = 0
        state.destinations[url] = destPath
        state.destinations[sourceURL.lastPathComponent] = destPath
        state.destinations[destinationURL.lastPathComponent] = destPath
        state.lock.unlock()

        task.resume()
        return true
    }

    public static func modelManagerProgress(_ handle: ModelManagerHandle?, url: String) -> Double {
        guard let handle else { return -1.0 }
        handle.state.lock.lock()
        defer { handle.state.lock.unlock() }
        return handle.state.progresses[url] ?? handle.state.tasks[url]?.progress.fractionCompleted ?? -1.0
    }

    public static func modelManagerCancel(_ handle: ModelManagerHandle?, url: String) -> Bool {
        guard let handle else { return false }

        handle.state.lock.lock()
        let task = handle.state.tasks.removeValue(forKey: url)
        handle.state.observations.removeValue(forKey: url)
        handle.state.progresses[url] = -1.0
        handle.state.lock.unlock()

        task?.cancel()
        return task != nil
    }

    public static func rulesInit(config: RulesConfig) -> Bool {
        rulesState.lock.lock()
        rulesState.config = config
        pruneRecentNotifications(now: Date())
        rulesState.lock.unlock()
        return true
    }

    public static func rulesFilter(_ processedData: Data) -> [NotificationCandidate]? {
        guard let processedEvents = tryDecode([ProcessedEvent].self, from: processedData) else {
            Logger.shared.log(.warn, "RustInterop", "Failed to decode payload for rulesFilter")
            return nil
        }

        let now = Date()
        rulesState.lock.lock()
        let config = rulesState.config
        pruneRecentNotifications(now: now)
        let recent = rulesState.recentNotifications
        rulesState.lock.unlock()

        return processedEvents.compactMap { event in
            guard event.priority >= config.priorityThreshold else { return nil }

            let key = event.dedupeKey.isEmpty
                ? (generateDedupeKey(source: event.source.rawValue, title: event.title, dateISO: event.start ?? event.due, sourceId: event.sourceId) ?? event.id)
                : event.dedupeKey

            if let sentAt = recent[key],
               now.timeIntervalSince(sentAt) < TimeInterval(max(0, config.throttleWindowHours)) * 3600 {
                return nil
            }

            return NotificationCandidate(
                event: event,
                dedupeKey: key,
                text: event.title,
                actionType: actionType(for: event)
            )
        }
    }

    public static func rulesMarkSent(_ dedupe: String) {
        rulesState.lock.lock()
        rulesState.recentNotifications[dedupe] = Date()
        pruneRecentNotifications(now: Date())
        rulesState.lock.unlock()
    }

    public static func rulesIsQuietHours() -> Bool {
        rulesState.lock.lock()
        let config = rulesState.config
        rulesState.lock.unlock()

        let hour = Calendar.current.component(.hour, from: Date())
        if config.quietHoursStart == config.quietHoursEnd {
            return false
        }
        if config.quietHoursStart < config.quietHoursEnd {
            return hour >= config.quietHoursStart && hour < config.quietHoursEnd
        }
        return hour >= config.quietHoursStart || hour < config.quietHoursEnd
    }

    public static func rulesClearHistory() {
        rulesState.lock.lock()
        rulesState.recentNotifications.removeAll()
        rulesState.lock.unlock()
    }

    public static func modelManagerModelPath(_ handle: ModelManagerHandle?, modelId: String) -> String? {
        guard let handle else { return nil }
        handle.state.lock.lock()
        defer { handle.state.lock.unlock() }
        return handle.state.destinations[modelId]
    }

    public static func modelManagerIsDownloaded(_ handle: ModelManagerHandle?, modelId: String) -> Bool {
        guard let path = modelManagerModelPath(handle, modelId: modelId) else { return false }
        return FileManager.default.fileExists(atPath: path)
    }

    public static func findProcessedEventInList(processed: [ProcessedEvent], key: String) -> ProcessedEvent? {
        if let exact = processed.first(where: { $0.dedupeKey == key || $0.id == key || $0.sourceId == key }) {
            return exact
        }

        return processed.first(where: {
            $0.dedupeKey.contains(key) ||
            key.contains($0.dedupeKey) ||
            $0.title.localizedCaseInsensitiveContains(key)
        })
    }

    private static func buildCandidates(processedEvents: [ProcessedEvent], modelOutput: ModelOutput) -> [NotificationCandidate] {
        var usedIndexes = Set<Int>()

        return modelOutput.items.compactMap { item in
            let exactIndex = processedEvents.indices.first(where: {
                !usedIndexes.contains($0) &&
                processedEvents[$0].dedupeKey == item.dedupeKey
            })
            let titleIndex = processedEvents.indices.first(where: {
                !usedIndexes.contains($0) &&
                processedEvents[$0].title.localizedCaseInsensitiveContains(item.text)
            })
            let fallbackIndex = processedEvents.indices
                .filter { !usedIndexes.contains($0) }
                .max(by: { processedEvents[$0].priority < processedEvents[$1].priority })
            let eventIndex = exactIndex ?? titleIndex ?? fallbackIndex

            guard let eventIndex else { return nil }
            usedIndexes.insert(eventIndex)

            let event = processedEvents[eventIndex]
            let dedupeKey = item.dedupeKey.isEmpty ? event.dedupeKey : item.dedupeKey

            return NotificationCandidate(
                event: event,
                dedupeKey: dedupeKey,
                text: item.text,
                actionType: actionType(for: event, preferredAction: item.action)
            )
        }
    }

    private static func mapRawEventsToProcessedEvents(_ events: [RawEvent]) -> [ProcessedEvent] {
        events.map { event in
            let sourceType: ProcessedEvent.SourceType = {
                switch event.source {
                case .calendar: return .calendar
                case .reminder: return .reminder
                case .url: return .url
                case .contact: return .manual
                case .health: return .health
                }
            }()

            let sourceId = event.metadata["sourceId"]?.trimmedNilIfEmpty ?? event.id
            let priority = Double(event.metadata["priority"] ?? "") ?? 0.5
            let tags = event.metadata["tags"]?.split(separator: ",").map { String($0) }
            let dedupeKey = generateDedupeKey(
                source: event.source.rawValue,
                title: event.title,
                dateISO: event.date.map(formatISO8601),
                sourceId: sourceId
            ) ?? event.id

            let stableHash: Int = {
                var hash: UInt64 = 0xcbf29ce484222325
                let prime: UInt64 = 0x00000100000001B3
                for byte in dedupeKey.utf8 {
                    hash ^= UInt64(byte)
                    hash &*= prime
                }
                return Int(truncatingIfNeeded: hash)
            }()

            return ProcessedEvent(
                id: event.id,
                source: event.source,
                sourceType: sourceType,
                sourceId: sourceId,
                title: event.title,
                date: event.date,
                start: nil,
                due: nil,
                priority: priority,
                tags: tags,
                dedupeKey: dedupeKey,
                provenance: nil,
                metadata: event.metadata,
                notes: event.notes,
                hash: stableHash
            )
        }
    }

    private static func actionType(for event: ProcessedEvent, preferredAction: String? = nil) -> ActionType {
        switch preferredAction?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "openreminder", "open_reminder":
            return .openReminder
        case "openmessages", "open_messages":
            return .openMessages
        case "openhealth", "open_health":
            return .openHealth
        case "dismiss":
            return .dismiss
        default:
            switch event.source {
            case .calendar, .contact, .url:
                return .openCalendar
            case .reminder:
                return .openReminder
            case .health:
                return .openHealth
            }
        }
    }

    private static func pruneRecentNotifications(now: Date) {
        let window = TimeInterval(max(0, rulesState.config.throttleWindowHours)) * 3600
        guard window > 0 else {
            rulesState.recentNotifications.removeAll()
            return
        }

        let cutoff = now.addingTimeInterval(-window)
        rulesState.recentNotifications = rulesState.recentNotifications.filter { $0.value >= cutoff }
    }

    private static func makeJSONDecoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }

    private static func makeJSONEncoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }

    private static func tryDecode<T: Decodable>(_ type: T.Type, from data: Data) -> T? {
        try? makeJSONDecoder().decode(type, from: data)
    }

    nonisolated private static func stableHexHash(_ value: String) -> String {
        var hash: UInt64 = 0xcbf29ce484222325
        let prime: UInt64 = 0x00000100000001B3
        for byte in value.utf8 {
            hash ^= UInt64(byte)
            hash &*= prime
        }
        return String(format: "%016llx", hash)
    }

    nonisolated private static func normalizeTitle(_ value: String) -> String {
        let lowercase = value.lowercased()
        let singleSpaces = lowercase.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
        let trimmed = singleSpaces.trimmingCharacters(in: .whitespacesAndNewlines)
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: " "))
        return trimmed.unicodeScalars
            .filter { allowed.contains($0) }
            .map(String.init)
            .joined()
    }

    nonisolated private static func normalizedDateComponent(from dateISO: String?) -> String {
        guard let dateISO = dateISO?.trimmedNilIfEmpty else { return "nodate" }
        if let date = parseFlexibleDate(dateISO) {
            return fullDateString(from: date)
        }
        if dateISO.count >= 10 {
            return String(dateISO.prefix(10))
        }
        return dateISO
    }

    nonisolated fileprivate static func parseFlexibleDate(_ value: String) -> Date? {
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = iso.date(from: value) { return date }

        iso.formatOptions = [.withInternetDateTime]
        if let date = iso.date(from: value) { return date }

        iso.formatOptions = [.withFullDate]
        if let date = iso.date(from: value) { return date }

        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd HH:mm"
        if let date = formatter.date(from: value) { return date }

        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.date(from: value)
    }

    nonisolated private static func fullDateString(from date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withFullDate]
        return formatter.string(from: date)
    }

    nonisolated private static func formatISO8601(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }

}

private extension String {
    nonisolated var trimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    nonisolated var nonEmpty: String? {
        isEmpty ? nil : self
    }
}
