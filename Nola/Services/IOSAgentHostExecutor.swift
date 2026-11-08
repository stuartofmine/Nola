// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Executes the mobile-safe subset of shared agent host calls on iOS.

import Foundation
import CryptoKit
import CoreLocation
import EventKit
import UIKit

nonisolated public struct IOSAgentHostCallResult: Sendable {
    public let callId: String
    public let output: AgentCoreJSONValue
    public let isError: Bool
    public let artifacts: [AgentCoreJSONValue]

    public init(
        callId: String,
        output: AgentCoreJSONValue,
        isError: Bool,
        artifacts: [AgentCoreJSONValue] = []
    ) {
        self.callId = callId
        self.output = output
        self.isError = isError
        self.artifacts = artifacts
    }
}

nonisolated private struct IOSAgentHostCall: Sendable {
    let callId: String
    let toolName: String
    let capability: AgentCoreCapability
    let arguments: AgentCoreJSONValue
    let grant: AgentCoreJSONValue?
    let sessionId: String?
    let executionPermit: IOSRunExecutionPermit?
}

nonisolated public struct IOSAgentSkillSnapshot: Sendable {
    public let id: String
    public let name: String
    public let summary: String
    public let instructions: String
    public let version: String?
    public let requiredTools: [String]
    public let validationSteps: [String]

    public init(
        id: String,
        name: String,
        summary: String,
        instructions: String,
        version: String?,
        requiredTools: [String],
        validationSteps: [String]
    ) {
        self.id = id
        self.name = name
        self.summary = summary
        self.instructions = instructions
        self.version = version
        self.requiredTools = requiredTools
        self.validationSteps = validationSteps
    }
}

public actor IOSAgentHostExecutor {
    private static let maxFileBytes = 1_000_000
    private static let maxNetworkBytes = 2_000_000
    private let session: URLSession
    private let eventStore = EKEventStore()
    private var activeAgentSessions: Set<String> = []
    private var agentSessionWaiters: [String: [CheckedContinuation<Void, Never>]] = [:]
    private var skillSnapshotsBySession: [String: [IOSAgentSkillSnapshot]] = [:]

    public init(session: URLSession = .shared) {
        self.session = session
    }


    /// Acquires the single-writer lease for a Core Session. Every foreground
    /// Turn and background worker must hold this lease from checkpoint restore
    /// through its final checkpoint write.
    public func acquireAgentSession(sessionId: String) async {
        if activeAgentSessions.insert(sessionId).inserted { return }
        await withCheckedContinuation { continuation in
            agentSessionWaiters[sessionId, default: []].append(continuation)
        }
    }

    public func releaseAgentSession(sessionId: String) {
        guard var waiters = agentSessionWaiters[sessionId], !waiters.isEmpty else {
            activeAgentSessions.remove(sessionId)
            agentSessionWaiters.removeValue(forKey: sessionId)
            skillSnapshotsBySession.removeValue(forKey: sessionId)
            return
        }
        let next = waiters.removeFirst()
        if waiters.isEmpty {
            agentSessionWaiters.removeValue(forKey: sessionId)
        } else {
            agentSessionWaiters[sessionId] = waiters
        }
        next.resume()
    }

    public func installSkillSnapshot(
        _ skills: [IOSAgentSkillSnapshot],
        sessionId: String
    ) {
        skillSnapshotsBySession[sessionId] = skills
    }


    private func execute(_ request: IOSAgentHostCall) async -> IOSAgentHostCallResult {
        do {
            try request.executionPermit?.checkAdmission()
            let toolName = request.toolName
            guard !toolName.hasPrefix("host.") && !toolName.hasPrefix("host_") else {
                throw HostExecutorError.unsupported("host")
            }
            let output: AgentCoreJSONValue
            switch request.capability {
            case .none:
                switch toolName {
                case "skill_invoke":
                    output = try invokeSkill(request)
                case "memory_search":
                    output = try searchMemory(request)
                default:
                    throw HostExecutorError.unsupported(toolName)
                }
            case .filesRead:
                output = try readScopedFile(request)
            case .filesWrite:
                output = try writeScopedFile(request)
            case .network:
                output = try await fetchNetworkResource(request)
            case .clipboard:
                output = try await accessClipboard(request)
            case .location:
                output = try await currentLocation(executionPermit: request.executionPermit)
            case .calendar:
                output = try accessCalendar(request)
            default:
                throw HostExecutorError.unsupported(request.capability.rawValue)
            }
            return IOSAgentHostCallResult(
                callId: request.callId,
                output: output,
                isError: false,
                artifacts: iosAgentFileArtifacts(
                    output: output,
                    capability: request.capability
                )
            )
        } catch {
            return IOSAgentHostCallResult(
                callId: request.callId,
                output: .object([
                    "error": .string(error.localizedDescription),
                    "capability": .string(request.capability.rawValue),
                    "tool_name": .string(request.toolName)
                ]),
                isError: true
            )
        }
    }

    public func executeBridge(
        callId: String,
        toolName: String,
        capability: AgentCoreCapability,
        arguments: AgentCoreJSONValue,
        grant: AgentCoreJSONValue?,
        sessionId: String? = nil,
        executionPermit: IOSRunExecutionPermit? = nil
    ) async -> IOSAgentHostCallResult {
        guard Self.installedToolCapabilities[toolName] == capability else {
            return IOSAgentHostCallResult(
                callId: callId,
                output: .object([
                    "error": .string("Tool is not present in the installed iOS handler manifest.")
                ]),
                isError: true
            )
        }
        return await execute(
            IOSAgentHostCall(
                callId: callId,
                toolName: toolName,
                capability: capability,
                arguments: arguments,
                grant: grant,
                sessionId: sessionId,
                executionPermit: executionPermit
            )
        )
    }

    private func invokeSkill(_ request: IOSAgentHostCall) throws -> AgentCoreJSONValue {
        guard let sessionId = request.sessionId,
              let object = request.arguments.objectValue,
              let requestedName = object["skill"]?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines),
              !requestedName.isEmpty else {
            throw HostExecutorError.missingArgument("skill")
        }
        let matches = (skillSnapshotsBySession[sessionId] ?? []).filter {
            $0.name.trimmingCharacters(in: .whitespacesAndNewlines) == requestedName
        }
        guard matches.count == 1, let skill = matches.first else {
            throw HostExecutorError.unsupported(matches.isEmpty ? "unknown or disabled Skill" : "ambiguous Skill name")
        }
        let instructionsData = Data(skill.instructions.utf8)
        guard instructionsData.count <= 256 * 1_024 else {
            throw HostExecutorError.outputTooLarge
        }
        let digest = SHA256.hash(data: instructionsData)
            .map { String(format: "%02x", $0) }
            .joined()
        let activation = object["activation"]?.stringValue == "session" ? "session" : "turn"
        let declaredVersion = skill.version?.trimmingCharacters(in: .whitespacesAndNewlines)
        let version = declaredVersion?.isEmpty == false ? declaredVersion! : "sha256-\(digest.prefix(12))"
        return .object([
            "type": .string("invoked_skill"),
            "skill_id": .string(skill.id),
            "name": .string(skill.name),
            "display_name": .string(skill.name),
            "summary": .string(skill.summary),
            "version": .string(version),
            "instructions_ref": .string("nola-skill://\(skill.id)#sha256=\(digest)"),
            "instructions_digest": .string(digest),
            "base_directory": .string("ios://skills/\(skill.id)"),
            "activation": .string(activation),
            "mode": .string("inline"),
            "budget": .object([
                "max_model_turns": .number(8),
                "max_tool_calls": .number(32),
                "max_elapsed_millis": .number(300_000)
            ]),
            "arguments": object["args"] ?? .object([:]),
            "instructions": .string(skill.instructions),
            "required_tools": .array(skill.requiredTools.map { toolName in
                .object([
                    "provider": .string("builtin"),
                    "tool_name": .string(toolName)
                ])
            }),
            "validation_steps": .array(skill.validationSteps.map(AgentCoreJSONValue.string))
        ])
    }

    private func searchMemory(_ request: IOSAgentHostCall) throws -> AgentCoreJSONValue {
        guard let object = request.arguments.objectValue,
              let query = object["query"]?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines),
              !query.isEmpty else {
            throw HostExecutorError.missingArgument("query")
        }
        guard Set(object.keys).isSubset(of: ["query", "limit"]) else {
            throw HostExecutorError.invalidMemoryArguments
        }
        let requestedLimit: Int
        if case .number(let value)? = object["limit"] {
            requestedLimit = Int(value)
        } else {
            requestedLimit = 8
        }
        let limit = max(1, min(requestedLimit, 20))
        let candidates = try IOSApplicationRuntime.requireShared().recallProfileMemories(query: query, limit: 50)
        let formatter = ISO8601DateFormatter()
        let records = try candidates.map {
            try JSONDecoder().decode(CoreMemoryRecord.self, from: JSONEncoder().encode($0.record))
        }
        guard let plan = PersonalIntelligenceCore.selectMemories(
            CoreMemorySelectionInput(
                policy: CoreMemoryPolicy(
                    longTermEnabled: true,
                    automaticExtractionEnabled: true,
                    searchLimit: limit,
                    minimumConfidence: 0.5,
                    maximumAgeDays: 365
                ),
                records: Array(records),
                mode: .onDemand,
                allowedNamespaces: [],
                includeSessionRetention: false,
                now: formatter.string(from: Date())
            )
        ) else { throw HostExecutorError.memoryUnavailable }
        let matches = plan.selected.map { selected in
            AgentCoreJSONValue.object([
                "memory_id": .string(selected.record.id),
                "content": .string(selected.record.content),
                "kind": .string(selected.record.kind.rawValue),
                "source": .string(selected.record.provenance.source),
                "requires_current_state_verification": .bool(selected.requiresCurrentStateVerification)
            ])
        }
        return .object([
            "type": .string("memory_search_result"),
            "query": .string(query),
            "matches": .array(matches),
            "plan_digest": .string(plan.planDigest)
        ])
    }

    private func readScopedFile(_ request: IOSAgentHostCall) throws -> AgentCoreJSONValue {
        try withScopedURL(request) { url in
            let values = try url.resourceValues(forKeys: [.fileSizeKey])
            if let size = values.fileSize, size > Self.maxFileBytes {
                throw HostExecutorError.outputTooLarge
            }
            let data = try Data(contentsOf: url)
            guard data.count <= Self.maxFileBytes else { throw HostExecutorError.outputTooLarge }
            return .object([
                "uri": .string(url.absoluteString),
                "content": .string(String(decoding: data, as: UTF8.self)),
                "bytes": .number(Double(data.count))
            ])
        }
    }

    private func writeScopedFile(_ request: IOSAgentHostCall) throws -> AgentCoreJSONValue {
        let content = request.arguments.objectValue?["content"]?.stringValue
            ?? request.arguments.objectValue?["text"]?.stringValue
        guard let content else { throw HostExecutorError.missingArgument("content") }
        let data = Data(content.utf8)
        guard data.count <= Self.maxFileBytes else { throw HostExecutorError.outputTooLarge }
        return try withScopedURL(request) { url in
            try data.write(to: url, options: .atomic)
            return .object([
                "uri": .string(url.absoluteString),
                "bytes_written": .number(Double(data.count))
            ])
        }
    }

    private func fetchNetworkResource(_ request: IOSAgentHostCall) async throws -> AgentCoreJSONValue {
        guard let raw = request.arguments.objectValue?["url"]?.stringValue,
              let url = URL(string: raw),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https" else {
            throw HostExecutorError.missingArgument("HTTP(S) url")
        }
        var request = URLRequest(url: url)
        request.timeoutInterval = 15
        request.setValue("application/json, text/plain, text/html;q=0.8, */*;q=0.1", forHTTPHeaderField: "Accept")
        let (data, response) = try await session.data(for: request)
        guard data.count <= Self.maxNetworkBytes else { throw HostExecutorError.outputTooLarge }
        let http = response as? HTTPURLResponse
        return .object([
            "url": .string(url.absoluteString),
            "status": .number(Double(http?.statusCode ?? 0)),
            "content_type": .string(http?.value(forHTTPHeaderField: "Content-Type") ?? ""),
            "body": .string(String(decoding: data, as: UTF8.self))
        ])
    }

    private func accessClipboard(_ request: IOSAgentHostCall) async throws -> AgentCoreJSONValue {
        let writeText = request.arguments.objectValue?["content"]?.stringValue
            ?? request.arguments.objectValue?["text"]?.stringValue
        return try await MainActor.run {
            try request.executionPermit?.checkAdmission()
            if let writeText {
                UIPasteboard.general.string = writeText
                return .object(["written": .bool(true)])
            }
            return .object(["content": .string(UIPasteboard.general.string ?? "")])
        }
    }

    private func currentLocation(executionPermit: IOSRunExecutionPermit?) async throws -> AgentCoreJSONValue {
        let location = try await requestAgentLocation(executionPermit: executionPermit)
        return .object([
            "latitude": .number(location.coordinate.latitude),
            "longitude": .number(location.coordinate.longitude),
            "horizontal_accuracy_m": .number(location.horizontalAccuracy),
            "timestamp_iso": .string(Self.formatISODate(location.timestamp))
        ])
    }

    private func accessCalendar(_ request: IOSAgentHostCall) throws -> AgentCoreJSONValue {
        if request.toolName == "device_create_calendar_event" {
            return try createCalendarEvent(request)
        }
        let now = Date()
        let start = try optionalDate(request, key: "start_iso") ?? now
        let end = try optionalDate(request, key: "end_iso") ?? start.addingTimeInterval(86_400)
        guard end > start, end.timeIntervalSince(start) <= 31 * 86_400 else {
            throw HostExecutorError.invalidTimeRange
        }
        let predicate = eventStore.predicateForEvents(withStart: start, end: end, calendars: nil)
        let events = eventStore.events(matching: predicate).prefix(200).map { item in
            AgentCoreJSONValue.object([
                "id": .string(item.calendarItemIdentifier),
                "title": .string(item.title ?? ""),
                "start_iso": .string(Self.formatISODate(item.startDate)),
                "end_iso": .string(Self.formatISODate(item.endDate)),
                "all_day": .bool(item.isAllDay),
                "calendar": .string(item.calendar.title)
            ])
        }
        return .object([
            "events": .array(Array(events)),
            "truncated": .bool(events.count == 200)
        ])
    }

    private func createCalendarEvent(_ request: IOSAgentHostCall) throws -> AgentCoreJSONValue {
        guard let title = request.arguments.objectValue?["title"]?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines),
              !title.isEmpty else {
            throw HostExecutorError.missingArgument("title")
        }
        guard let start = try optionalDate(request, key: "start_iso"),
              let end = try optionalDate(request, key: "end_iso"),
              end > start,
              end.timeIntervalSince(start) <= 31 * 86_400 else {
            throw HostExecutorError.invalidTimeRange
        }
        guard let calendar = eventStore.defaultCalendarForNewEvents else {
            throw HostExecutorError.calendarUnavailable
        }
        let item = EKEvent(eventStore: eventStore)
        item.calendar = calendar
        item.title = title
        item.startDate = start
        item.endDate = end
        item.notes = request.arguments.objectValue?["notes"]?.stringValue
        try eventStore.save(item, span: .thisEvent, commit: true)
        return .object([
            "created": .bool(true),
            "id": .string(item.calendarItemIdentifier),
            "calendar": .string(calendar.title)
        ])
    }

    private func optionalDate(_ request: IOSAgentHostCall, key: String) throws -> Date? {
        guard let raw = request.arguments.objectValue?[key]?.stringValue else { return nil }
        guard let date = Self.parseISODate(raw) else {
            throw HostExecutorError.invalidDate(key)
        }
        return date
    }

    private static func formatISODate(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }

    private static func parseISODate(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) { return date }
        let standard = ISO8601DateFormatter()
        standard.formatOptions = [.withInternetDateTime]
        return standard.date(from: value)
    }

    private func withScopedURL<T>(
        _ request: IOSAgentHostCall,
        operation: (URL) throws -> T
    ) throws -> T {
        guard let grant = request.grant?.objectValue else { throw HostExecutorError.missingGrant }
        let resolvedURL: URL?
        if let encoded = grant["bookmark"]?.stringValue,
           let data = Data(base64Encoded: encoded) {
            var stale = false
            resolvedURL = try? URL(
                resolvingBookmarkData: data,
                options: [],
                relativeTo: nil,
                bookmarkDataIsStale: &stale
            )
        } else if let raw = grant["uri"]?.stringValue {
            resolvedURL = URL(string: raw)
        } else {
            resolvedURL = nil
        }
        guard let url = resolvedURL, url.isFileURL else { throw HostExecutorError.invalidGrant }
        let accessing = url.startAccessingSecurityScopedResource()
        defer { if accessing { url.stopAccessingSecurityScopedResource() } }
        guard accessing || url.path.hasPrefix(NSHomeDirectory()) else {
            throw HostExecutorError.invalidGrant
        }
        return try operation(url)
    }
}

nonisolated func iosAgentFileArtifacts(
    output: AgentCoreJSONValue,
    capability: AgentCoreCapability
) -> [AgentCoreJSONValue] {
    guard capability == .filesRead || capability == .filesWrite,
          let rawURI = output.objectValue?["uri"]?.stringValue,
          let url = URL(string: rawURI),
          url.scheme != nil else {
        return []
    }
    let name = url.lastPathComponent.nonEmpty ?? "File"
    return [.object([
        "uri": .string(rawURI),
        "name": .string(name),
        "mime_type": .null,
        "operation": .string(capability == .filesWrite ? "create" : "read"),
    ])]
}

nonisolated private extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}

nonisolated private enum HostExecutorError: LocalizedError {
    case missingGrant
    case invalidGrant
    case missingArgument(String)
    case outputTooLarge
    case unsupported(String)
    case invalidDate(String)
    case invalidTimeRange
    case invalidMemoryArguments
    case memoryUnavailable
    case calendarUnavailable

    var errorDescription: String? {
        switch self {
        case .missingGrant: return "The host call has no scoped resource grant"
        case .invalidGrant: return "The scoped resource grant is invalid"
        case .missingArgument(let name): return "The host call requires \(name)"
        case .outputTooLarge: return "Host output exceeds the mobile size limit"
        case .unsupported(let capability): return "Capability \(capability) has no iOS host executor"
        case .invalidDate(let key): return "The host call requires a valid ISO 8601 \(key)"
        case .invalidTimeRange: return "The calendar time range must be positive and no longer than 31 days"
        case .invalidMemoryArguments: return "Personal memory search accepts only query and limit. Local conversation/workspace scopes are unavailable in this tool."
        case .memoryUnavailable: return "The memory engine is unavailable. Rebuild or update Nola before retrying."
        case .calendarUnavailable: return "No writable default calendar is available"
        }
    }
}

@MainActor
private final class AgentExecutorLocationRequester: NSObject, CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private var continuation: CheckedContinuation<CLLocation, Error>?

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
    }

    func request() async throws -> CLLocation {
        guard CLLocationManager.locationServicesEnabled(),
              manager.authorizationStatus == .authorizedAlways
                || manager.authorizationStatus == .authorizedWhenInUse else {
            throw HostExecutorError.unsupported("location permission")
        }
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            manager.requestLocation()
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let location = locations.last, let continuation else { return }
        self.continuation = nil
        continuation.resume(returning: location)
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        guard let continuation else { return }
        self.continuation = nil
        continuation.resume(throwing: error)
    }
}

@MainActor
private func requestAgentLocation(executionPermit: IOSRunExecutionPermit?) async throws -> CLLocation {
    try executionPermit?.checkAdmission()
    return try await AgentExecutorLocationRequester().request()
}

private extension AgentCoreJSONValue {
    nonisolated var stringValue: String? {
        guard case .string(let value) = self else { return nil }
        return value
    }

    nonisolated var objectValue: [String: AgentCoreJSONValue]? {
        guard case .object(let value) = self else { return nil }
        return value
    }
}
