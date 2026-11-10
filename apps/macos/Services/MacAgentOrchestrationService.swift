// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Applies permission-approved Agent Host orchestration requests to native Mac data sources and automations.

import CryptoKit
import Foundation

struct MacAgentOrchestrationRequest: Codable, Sendable {
    struct WebSource: Codable, Sendable {
        var id: String
        var name: String
        var url: String
        var format: String?
        var refreshFrequency: String?

        enum CodingKeys: String, CodingKey {
            case id
            case name
            case url
            case format
            case refreshFrequency = "refresh_frequency"
        }
    }

    var name: String
    var prompt: String
    var skillID: String
    var frequency: String
    var weekday: Int?
    var hour: Int
    var minute: Int
    var enabled: Bool?
    var runWhenDataChanges: Bool?
    var contextSources: [String]?
    var webSources: [WebSource]?
    var maximumDataAgeMinutes: Int?
    var timeoutMinutes: Int?
    var retryCount: Int?
    var executionSpec: MacAutomationDefinitionSpec

    enum CodingKeys: String, CodingKey {
        case name
        case prompt
        case skillID = "skill_id"
        case frequency
        case weekday
        case hour
        case minute
        case enabled
        case runWhenDataChanges = "run_when_data_changes"
        case contextSources = "context_sources"
        case webSources = "web_sources"
        case maximumDataAgeMinutes = "maximum_data_age_minutes"
        case timeoutMinutes = "timeout_minutes"
        case retryCount = "retry_count"
        case executionSpec = "execution_spec"
    }

    static func decode(arguments: String) throws -> Self {
        guard let data = arguments.data(using: .utf8) else {
            throw MacAgentOrchestrationError.invalidArguments
        }
        let decoder = JSONDecoder()
        do {
            return try decoder.decode(Self.self, from: data)
        } catch {
            throw MacAgentOrchestrationError.invalidArguments
        }
    }
}

struct MacAgentOrchestrationResult: Sendable {
    let automation: MacAutomation
    let webSources: [MacWebSource]
    let mutation: MacAutomationMutationVerification

    var executionReceipt: [String: Any] {
        let resource = "nola://automation/\(mutation.committed.id)"
        let version = "revision:\(mutation.committed.revision)"
        return [
            "state": mutation.verificationError == nil ? "completed" : "partial",
            "local_stop": NSNull(),
            "effects": [["resource_uri": resource, "operation": "automation.upsert",
                         "before_version": NSNull(), "after_version": version]],
            "checks": [["kind": "postcondition", "resource_uri": resource, "version": version,
                        "predicate": "durable_definition_matches",
                        "passed": mutation.verificationError == nil]],
        ]
    }

    var activityDetail: String {
        guard !webSources.isEmpty else {
            return "\(automation.name) · \(automation.scheduleDescription)"
        }
        return "\(automation.name) · \(automation.scheduleDescription) · \(webSources.count) \(String(localized: "Web Sources"))"
    }
}

enum MacAgentOrchestrationError: LocalizedError {
    case invalidArguments
    case invalidName
    case invalidPrompt
    case invalidSchedule
    case invalidWebSource
    case unavailableSkill
    case unauthorizedWorkspace
    case saveFailed(String)

    var errorDescription: String? {
        switch self {
        case .invalidArguments:
            return String(localized: "The automation request is invalid.")
        case .invalidName:
            return String(localized: "The automation needs a name.")
        case .invalidPrompt:
            return String(localized: "The automation needs a task prompt.")
        case .invalidSchedule:
            return String(localized: "The automation schedule is invalid.")
        case .invalidWebSource:
            return String(localized: "A web data source URL is invalid.")
        case .unavailableSkill:
            return String(localized: "The requested Skill is disabled, removed, or unavailable.")
        case .unauthorizedWorkspace:
            return String(localized: "Choose an authorized workspace before saving this automation.")
        case .saveFailed(let detail):
            return detail
        }
    }
}

@MainActor
enum MacAgentOrchestrationService {
    nonisolated static let toolName = "nola_orchestrate"

    static func responseData(
        requestID: String,
        arguments: [String: Any],
        workspaceURL: URL?,
        appEnvironment: MacAppEnvironment = .shared
    ) async -> Data {
        let response: [String: Any]
        do {
            let data = try JSONSerialization.data(withJSONObject: arguments, options: [.sortedKeys])
            let request = try JSONDecoder().decode(MacAgentOrchestrationRequest.self, from: data)
            let result: MacAgentOrchestrationResult
            if appEnvironment.supportRoot == MacAppEnvironment.shared.supportRoot {
                result = try await apply(request, workspaceURL: workspaceURL)
            } else {
                let registry = MacDataSourceRegistry(
                    storeURL: appEnvironment.supportRoot
                        .appendingPathComponent("DataSources", isDirectory: true)
                        .appendingPathComponent("source-registry.json")
                )
                let webSourceStore = MacWebSourceStore(
                    storeURL: appEnvironment.supportRoot
                        .appendingPathComponent("WebSources", isDirectory: true)
                        .appendingPathComponent("web-sources.json"),
                    registry: registry
                )
                let automationStore = try MacAutomationStore(
                    synchronizeSystemJobs: false,
                    appEnvironment: appEnvironment,
                    authorizationResolver: { candidate in
                        guard let workspaceURL else { return false }
                        return MacAppEnvironment.contains(candidate, in: workspaceURL)
                    },
                    activityStore: MacActivityStore(
                        baseDirectory: appEnvironment.supportRoot
                            .appendingPathComponent("Activity", isDirectory: true)
                    ),
                    completionNotifier: { _, _ in },
                    dataSourceRegistry: registry,
                    webSourceStore: webSourceStore
                )
                result = try await apply(
                    request,
                    workspaceURL: workspaceURL,
                    automationStore: automationStore,
                    workspaceIsAuthorized: { candidate in
                        guard let workspaceURL else { return false }
                        return MacAppEnvironment.contains(candidate, in: workspaceURL)
                    },
                    webSourceStore: webSourceStore
                )
            }
            response = [
                "type": "device_tool_result",
                "request_id": requestID,
                "output": [
                    "automation_id": result.automation.id.uuidString,
                    "name": result.automation.name,
                    "skill_id": result.automation.skillID?.rawValue ?? request.skillID,
                    "schedule": result.automation.scheduleDescription,
                    "enabled": result.automation.isEnabled,
                    "persisted": true,
                    "read_back_verified": result.mutation.verificationError == nil,
                    "submitted_execution_spec_sha256": result.mutation.submittedExecutionSpecDigest,
                    "revision": result.mutation.committed.revision,
                    "native_commit_required": false,
                    "status": result.mutation.verificationError == nil ? "persisted" : "committed_verification_failed",
                    "verification_error": result.mutation.verificationError.map { $0 as Any } ?? NSNull(),
                ],
                "display": result.mutation.verificationError.map { "Automation was committed, but verification failed: \($0)" } ?? result.activityDetail,
                "is_error": result.mutation.verificationError != nil,
                "execution": result.executionReceipt,
                "artifacts": [[
                    "uri": "nola://automation/\(result.automation.id.uuidString.lowercased())",
                    "name": "\(result.automation.id.uuidString.lowercased()).json",
                    "mime_type": "application/json",
                    "operation": "upsert",
                ]],
            ]
        } catch {
            response = [
                "type": "device_tool_result",
                "request_id": requestID,
                "output": ["error": error.localizedDescription],
                "display": error.localizedDescription,
                "is_error": true,
            ]
        }
        return (try? JSONSerialization.data(withJSONObject: response, options: [.sortedKeys]))
            ?? Data("{\"type\":\"device_tool_result\",\"is_error\":true}".utf8)
    }

    static func apply(
        arguments: String,
        workspaceURL: URL?
    ) async throws -> MacAgentOrchestrationResult {
        try await apply(
            MacAgentOrchestrationRequest.decode(arguments: arguments),
            workspaceURL: workspaceURL
        )
    }

    static func apply(
        _ request: MacAgentOrchestrationRequest,
        workspaceURL: URL?
    ) async throws -> MacAgentOrchestrationResult {
        try await apply(
            request,
            workspaceURL: workspaceURL,
            automationStore: .shared,
            workspaceIsAuthorized: { workspace in
                MacSecurityScopedBookmarkStore.shared.isAuthorizedWorkspace(workspace)
            }
        )
    }

    static func apply(
        _ request: MacAgentOrchestrationRequest,
        workspaceURL: URL?,
        automationStore: MacAutomationStore,
        workspaceIsAuthorized: (URL) -> Bool,
        webSourceStore: MacWebSourceStore? = nil
    ) async throws -> MacAgentOrchestrationResult {
        let name = request.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = request.prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { throw MacAgentOrchestrationError.invalidName }
        guard !prompt.isEmpty else { throw MacAgentOrchestrationError.invalidPrompt }
        guard let skillID = MacBuiltInSkillID(rawValue: request.skillID),
              try automationStore.isSkillActive(skillID) else {
            throw MacAgentOrchestrationError.unavailableSkill
        }
        guard let frequency = MacAutomation.Frequency(rawValue: request.frequency),
              (0...23).contains(request.hour),
              (0...59).contains(request.minute) else {
            throw MacAgentOrchestrationError.invalidSchedule
        }
        let weekday = request.weekday ?? 1
        guard (0...6).contains(weekday) else {
            throw MacAgentOrchestrationError.invalidSchedule
        }

        let workspace = workspaceURL ?? MacAppEnvironment.shared.scratchWorkspaceURL
        guard workspaceIsAuthorized(workspace) else {
            throw MacAgentOrchestrationError.unauthorizedWorkspace
        }
        guard case .agent(let submittedSpec) = request.executionSpec,
              submittedSpec.prompt == request.prompt,
              submittedSpec.executionScope == .localWorkspace(root: workspace.path),
              ["cautious", "allow_all"].contains(submittedSpec.permissionProfile),
              submittedSpec.modelSelection == "use_current_default_at_admission",
              submittedSpec.skillRefs == [request.skillID],
              Set(submittedSpec.contextRefs.map { "\($0.kind):\($0.id)" }).count == submittedSpec.contextRefs.count else {
            throw MacAgentOrchestrationError.invalidArguments
        }
        for reference in submittedSpec.contextRefs {
            switch reference.kind {
            case "personal_data": _ = try MacPlatformDataService.automationKind(for: reference.id)
            case "web_source":
                guard UUID(uuidString: reference.id) != nil else { throw MacAgentOrchestrationError.invalidArguments }
            default: throw MacAgentOrchestrationError.invalidArguments
            }
        }

        var contextSources = Set<MacAutomation.ContextSource>()
        for rawValue in request.contextSources ?? [] {
            switch rawValue {
            case "personal_data": contextSources.insert(.personalData)
            case "web_sources": contextSources.insert(.webSources)
            default: throw MacAgentOrchestrationError.invalidArguments
            }
        }

        let webSourceStore = webSourceStore ?? .shared
        var resolvedWebSources: [MacWebSource] = []
        for sourceRequest in request.webSources ?? [] {
            guard let sourceID = UUID(uuidString: sourceRequest.id),
                  submittedSpec.contextRefs.contains(.init(kind: "web_source", id: sourceRequest.id.lowercased())) else {
                throw MacAgentOrchestrationError.invalidArguments
            }
            let canonicalURL = try canonicalWebURL(sourceRequest.url)
            let existing = webSourceStore.sources.first {
                $0.id == sourceID
            }
            let format = try webSourceFormat(sourceRequest.format)
            let schedule = try webSourceSchedule(sourceRequest.refreshFrequency)
            let source = MacWebSource(
                id: sourceID,
                name: normalizedSourceName(sourceRequest.name, url: canonicalURL),
                url: canonicalURL,
                format: format ?? existing?.format ?? .automatic,
                isEnabled: true,
                cachedText: existing?.cachedText ?? "",
                lastFetchedAt: existing?.lastFetchedAt,
                lastAttemptAt: existing?.lastAttemptAt,
                lastError: existing?.lastError,
                createdAt: existing?.createdAt ?? .now,
                scheduleFrequency: schedule ?? existing?.scheduleFrequency,
                scheduledWeekday: schedule == .weekly ? weekday + 1 : existing?.scheduledWeekday,
                scheduledHour: schedule == nil ? existing?.scheduledHour : request.hour,
                scheduledMinute: schedule == nil ? existing?.scheduledMinute : request.minute,
                scheduledDate: existing?.scheduledDate,
                runs: existing?.runs
            )
            try await webSourceStore.validateAndUpsert(source)
            resolvedWebSources.append(source)
        }
        if !resolvedWebSources.isEmpty {
            contextSources.insert(.webSources)
        }
        let declaredSources = Set(submittedSpec.contextRefs.map { $0.kind == "personal_data" ? MacAutomation.ContextSource.personalData : .webSources })
        guard contextSources == declaredSources,
              submittedSpec.contextRefs.filter({ $0.kind == "web_source" }).allSatisfy({ reference in
                  webSourceStore.sources.contains { $0.id.uuidString.lowercased() == reference.id && $0.isEnabled }
              }) else { throw MacAgentOrchestrationError.invalidArguments }

        let key = orchestrationKey(name: name, workspacePath: workspace.path)
        let existingAutomation = automationStore.automations.first {
            $0.orchestrationKey == key
        }
        let automation = MacAutomation(
            id: existingAutomation?.id ?? UUID(),
            name: name,
            prompt: submittedSpec.prompt,
            frequency: frequency,
            weekday: weekday,
            hour: request.hour,
            minute: request.minute,
            workspacePath: workspace.path,
            isEnabled: request.enabled ?? true,
            createdAt: existingAutomation?.createdAt ?? .now,
            contextSources: Set(submittedSpec.contextRefs.map { $0.kind == "personal_data" ? .personalData : .webSources }),
            webSourceIDs: Set(submittedSpec.contextRefs.filter { $0.kind == "web_source" }.compactMap { UUID(uuidString: $0.id) }),
            maximumDataAgeMinutes: min(10_080, max(5, request.maximumDataAgeMinutes ?? 60)),
            timeoutMinutes: min(720, max(1, request.timeoutMinutes ?? 30)),
            retryCount: min(3, max(0, request.retryCount ?? 0)),
            runWhenDataChanges: request.runWhenDataChanges ?? false,
            skillID: skillID,
            orchestrationKey: key,
            executionSpec: submittedSpec
        )
        let mutation = try automationStore.upsertWithVerification(automation)
        for source in resolvedWebSources where source.cachedText.isEmpty {
            Task { await webSourceStore.refresh(source) }
        }
        return MacAgentOrchestrationResult(
            automation: automation,
            webSources: resolvedWebSources,
            mutation: mutation
        )
    }

    private static func canonicalWebURL(_ rawValue: String) throws -> String {
        let trimmed = rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              let host = components.host, !host.isEmpty else {
            throw MacAgentOrchestrationError.invalidWebSource
        }
        components.scheme = scheme
        components.host = host.lowercased()
        components.fragment = nil
        if components.path == "/" { components.path = "" }
        guard let value = components.string else {
            throw MacAgentOrchestrationError.invalidWebSource
        }
        return value
    }

    private static func normalizedSourceName(_ rawValue: String, url: String) -> String {
        let trimmed = rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty
            ? URL(string: url)?.host ?? String(localized: "Web Source")
            : trimmed
    }

    private static func webSourceFormat(_ rawValue: String?) throws -> MacWebSource.Format? {
        guard let rawValue, !rawValue.isEmpty else { return nil }
        guard let value = MacWebSource.Format(rawValue: rawValue) else {
            throw MacAgentOrchestrationError.invalidArguments
        }
        return value
    }

    private static func webSourceSchedule(
        _ rawValue: String?
    ) throws -> MacWebSource.ScheduleFrequency? {
        guard let rawValue, rawValue != "manual", !rawValue.isEmpty else { return nil }
        guard let value = MacWebSource.ScheduleFrequency(rawValue: rawValue) else {
            throw MacAgentOrchestrationError.invalidArguments
        }
        return value
    }

    private static func orchestrationKey(name: String, workspacePath: String) -> String {
        let identity = "\(workspacePath)\n\(name.lowercased())"
        return SHA256.hash(data: Data(identity.utf8))
            .map { String(format: "%02x", $0) }
            .joined()
    }
}
