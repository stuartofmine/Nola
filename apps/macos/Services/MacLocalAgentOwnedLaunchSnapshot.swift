// SPDX-License-Identifier: Apache-2.0
// In-memory launch ownership. This is not persisted authority or a new turn.
import Foundation

/// External executors own authentication and model routing. Recovery takes the
/// exact identity from the immutable Host binding, never current model settings.
nonisolated struct MacFrozenExternalExecutorIdentity: Equatable {
    let backend: String
    let model: String?
    let provider: String?

    static func resolve(primary: [String: Any], modelProfileID: String?, fallbackCount: Int) throws -> Self? {
        guard primary["kind"] as? String == "external" else { return nil }
        func valid(_ value: String) -> Bool {
            !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && value.utf8.count <= 1_024
                && value.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
        }
        guard let backend = primary["backend"] as? String, valid(backend), backend != "nola",
              modelProfileID == nil, fallbackCount == 0,
              Set(primary.keys).isSubset(of: ["kind", "backend", "model", "provider"]) else {
            throw MacLocalAgentResumeError.invalidFrame
        }
        func optional(_ key: String) throws -> String? {
            guard let value = primary[key], !(value is NSNull) else { return nil }
            guard let text = value as? String, valid(text) else { throw MacLocalAgentResumeError.invalidFrame }
            return text
        }
        return try Self(backend: backend, model: optional("model"), provider: optional("provider"))
    }
}

/// Decoded only from Application's validated immutable Run invocation.
nonisolated struct MacFrozenLaunchDescriptor: Decodable, Sendable {
    let version: Int
    let input_origin: String
    let project_path: String
    let authorized_workspace_paths: [String]
    let allowed_directory_arguments: [String]
    let skill_directory_arguments: [String]
    let active_skill_ids: [String]
    let model_profile_id: String?
    let model_profile_revision: Int64?
    let fallback_profile_ids: [String]
    let permission_mode: String
    let memory_enabled: Bool
    let context_window_tokens: Int?
    let max_iterations: Int
    let model_capabilities: [String: Bool]
    let vision_input: Bool
    let permission_rules: [MacCanonicalJSONValue]
    let host_attachments: [MacCanonicalJSONValue]

    static func load(_ exact: MacExactRunInvocation) throws -> Self {
        guard let binding = try JSONSerialization.jsonObject(with: JSONEncoder().encode(exact.hostInvocation)) as? [String: Any],
              let value = binding["mac_launch"] as? [String: Any], value.keys.contains("model_profile_revision") else {
            throw MacLocalAgentResumeError.snapshotUnavailable
        }
        let encoded = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
        guard encoded.count <= 131_072 else { throw MacLocalAgentResumeError.invalidFrame }
        let descriptor = try JSONDecoder().decode(Self.self, from: encoded)
        // Recovery provenance is a closed wire domain, not a grant of authority.
        // Permissions and the original Run are revalidated independently.
        guard descriptor.version == 1,
              ["interactive_user", "scheduled_action"].contains(descriptor.input_origin) else {
            throw MacLocalAgentResumeError.invalidFrame
        }
        return descriptor
    }

    /// A grant made while this Run was executing must not invalidate its frozen
    /// authority. Recovery still sends only the original rules; it never adopts
    /// these additional grants. Revocations, changed rules and new restrictions
    /// remain fail-closed both during preparation and immediately before launch.
    static func validatePermissionRules(frozen: [MacCanonicalJSONValue], current: [MacCanonicalJSONValue]) throws {
        let original = Set(frozen)
        let latest = Set(current)
        guard original.isSubset(of: latest), latest.subtracting(original).allSatisfy({ rule in
            guard case .object(let fields) = rule,
                  case .string("allow") = fields["decision"] else { return false }
            return true
        }) else { throw MacLocalAgentResumeError.authorizationChanged }
    }

    func validate(_ authorization: MacLocalAgentResumeAuthorization) throws {
        func canonical(_ url: URL) -> String { url.standardizedFileURL.resolvingSymlinksInPath().path }
        let current = Set(([authorization.projectURL] + authorization.authorizedWorkspaceURLs).map(canonical))
        guard canonical(authorization.projectURL) == project_path,
              let frozenMode = MacPermissionMode(rawValue: permission_mode),
              authorization.permissionMode.permitsResuming(frozen: frozenMode),
              Set([project_path] + authorized_workspace_paths).isSubset(of: current) else {
            throw MacLocalAgentResumeError.authorizationChanged
        }
    }
}

nonisolated struct MacLocalAgentResumeAuthorization: Sendable {
    let projectURL: URL
    let authorizedWorkspaceURLs: [URL]
    let permissionMode: MacPermissionMode
}

nonisolated enum MacLocalAgentResumeError: LocalizedError {
    case snapshotUnavailable
    case authorizationChanged
    case invalidFrame
    case credentialUnavailable

    var errorDescription: String? {
        switch self {
        case .snapshotUnavailable: String(localized: "The original local Run launch snapshot is unavailable. The prompt was not resubmitted.", table: "Chat")
        case .authorizationChanged: String(localized: "The original Run workspace or permission authorization has changed.", table: "Chat")
        case .invalidFrame: String(localized: "The original local Run launch configuration is invalid.", table: "Chat")
        case .credentialUnavailable: String(localized: "The original model credential is unavailable or its authorization has changed.", table: "Chat")
        }
    }
}

nonisolated struct MacLocalAgentOwnedLaunchSnapshot: Sendable {
    let request: MacLocalAgentRunRequest
    let executableURL: URL
    let arguments: [String]
    let environment: [String: String]
    let initialRequestData: Data
    let toolPlan: MacCoreToolPlanReceipt
    let credentialSelection: MacLocalAgentCredentialSelection
    private let authorization: MacLocalAgentFrozenAuthorization

    init(request: MacLocalAgentRunRequest, executableURL: URL, arguments: [String],
         environment: [String: String], initialRequestData: Data, toolPlan: MacCoreToolPlanReceipt,
         credentialSelection: MacLocalAgentCredentialSelection = .unavailable) {
        self.request = request
        self.executableURL = executableURL
        self.arguments = arguments
        self.environment = environment
        self.initialRequestData = initialRequestData
        self.toolPlan = toolPlan
        self.credentialSelection = credentialSelection
        self.authorization = MacLocalAgentFrozenAuthorization(projectURL: request.projectURL,
            authorizedWorkspaceURLs: request.authorizedWorkspaceURLs, permissionMode: request.permissionMode)
    }

    func validate(_ authorization: MacLocalAgentResumeAuthorization) throws {
        try self.authorization.validate(authorization)
    }

    static func resumeRequestData(original: Data, conversationID: String, runID: String,
                                  assistantMessageID: String) throws -> Data {
        guard var configuration = try JSONSerialization.jsonObject(with: original) as? [String: Any],
              configuration["submission_id"] as? String == runID,
              configuration["assistant_message_id"] as? String == assistantMessageID else {
            throw MacLocalAgentResumeError.invalidFrame
        }
        for key in ["prompt", "submission_id", "user_message_id", "assistant_message_id", "expected_configuration_revision", "expected_parent_message_id", "context_components", "selected_experts", "expected_source_store_epoch", "expert_template_authority", "input_origin", "automation_execution_lease", "automation_execution_spec", "automation_artifacts"] {
            configuration.removeValue(forKey: key)
        }
        if let attachments = configuration.removeValue(forKey: "attachments") {
            configuration["host_attachments"] = attachments
        }
        var data = try JSONSerialization.data(withJSONObject: [
            "operation": "resume", "session_id": conversationID, "expected_run_id": runID,
            "assistant_message_id": assistantMessageID, "configuration": configuration,
            "expert_template_authority": try JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertTemplateAuthority.local()))
        ], options: [.sortedKeys])
        data.append(0x0a)
        return data
    }
}

/// Filesystem authority is captured independently from launch data so that
/// reauthorization compares against original paths, not re-resolved symlinks.
nonisolated struct MacLocalAgentFrozenAuthorization: Sendable {
    private let originalURLs: [URL]
    private let authorizedPaths: Set<String>
    private let projectPath: String
    private let permissionMode: MacPermissionMode

    init(projectURL: URL, authorizedWorkspaceURLs: [URL], permissionMode: MacPermissionMode) {
        self.originalURLs = [projectURL] + authorizedWorkspaceURLs
        self.projectPath = Self.path(projectURL)
        self.authorizedPaths = Set(([projectURL] + authorizedWorkspaceURLs).map(Self.path))
        self.permissionMode = permissionMode
    }

    private static func path(_ url: URL) -> String {
        url.standardizedFileURL.resolvingSymlinksInPath().path
    }

    func validate(_ current: MacLocalAgentResumeAuthorization) throws {
        let paths = Set(([current.projectURL] + current.authorizedWorkspaceURLs).map(Self.path))
        guard Self.path(current.projectURL) == projectPath,
              current.permissionMode.permitsResuming(frozen: permissionMode),
              Set(originalURLs.map(Self.path)) == authorizedPaths,
              authorizedPaths.isSubset(of: paths) else {
            throw MacLocalAgentResumeError.authorizationChanged
        }
    }
}
