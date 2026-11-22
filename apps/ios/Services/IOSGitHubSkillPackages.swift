// SPDX-License-Identifier: Apache-2.0
import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

/// Hosts own registration and activation. The shared installer prepares immutable
/// package data inside this Application database's private storage.
actor IOSGitHubSkillPackages {
    static let shared = IOSGitHubSkillPackages()
    nonisolated static let referencePrefix = "nola://github-skills/"

    nonisolated static func sourceURL(reference: String?) -> URL? {
        guard let reference, reference.hasPrefix(referencePrefix) else { return nil }
        return URL(string: "https://github.com/" + String(reference.dropFirst(referencePrefix.count)))
    }

    nonisolated static func request(_ payload: [String: String], runtime: IOSApplicationRuntime) async throws -> AgentCoreJSONValue {
        let root = runtime.databaseURL.appendingPathExtension("skill-packages")
        return try await Task.detached(priority: .userInitiated) {
#if canImport(NolaCore)
            let json = String(decoding: try JSONEncoder().encode(payload), as: UTF8.self)
            let pointer = root.path.withCString { path in
                json.withCString { nola_mobile_github_skill_package_json(path, $0) }
            }
            guard let pointer else { throw IOSApplicationRuntimeError.invalidResponse }
            defer { nola_core_free_c_string(pointer) }
            let response = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(String(cString: pointer).utf8))
            guard let fields = response.objectFields else { throw IOSApplicationRuntimeError.invalidResponse }
            guard fields["ok"] == .bool(true), let value = fields["value"] else {
                throw IOSApplicationRuntimeError.rejected(fields["error"]?.stringScalar ?? "GitHub Skill request failed.")
            }
            return value
#else
            throw IOSApplicationRuntimeError.applicationUnavailable
#endif
        }.value
    }

    func register(_ package: AgentCoreJSONValue, runtime: IOSApplicationRuntime,
                  permit: IOSRunExecutionPermit?) throws -> AgentCoreJSONValue {
        guard let fields = package.objectFields else { throw IOSApplicationRuntimeError.invalidResponse }
        if fields["status"] == .string("selection_required") { return package }
        guard fields["status"] == .string("prepared"), let name = fields["id"]?.stringScalar,
              let reference = fields["instructions_ref"]?.stringScalar, Self.sourceURL(reference: reference) != nil,
              let version = fields["version"]?.stringScalar, let content = fields["content"]?.stringScalar,
              let summary = fields["description"]?.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }

        try permit?.checkAdmission()
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let skills = try runtime.listSkills()
        if let previous = skills.first(where: { $0.instructionsRef == reference }) {
            guard !skills.contains(where: { $0.id != previous.id && $0.displayName == previous.displayName }),
                  previous.source == "user", let editor = try runtime.skillEditor(id: previous.id),
                  editor.record.instructionsRef == reference else {
                throw IOSApplicationRuntimeError.rejected("This Skill conflicts with another installed Skill.")
            }
            if !editor.record.enabled { _ = try runtime.setSkillEnabled(true, expected: editor, updatedAtMs: now) }
            return .object(["status": .string("installed"), "id": .string(name), "enabled": .bool(true),
                            "source_url": fields["source_url"] ?? .null])
        }
        guard !skills.contains(where: { $0.displayName == name || $0.id == name }) else {
            throw IOSApplicationRuntimeError.rejected("A Skill named \(name) is already installed from another source or revision.")
        }
        _ = try runtime.createUserSkill(.init(id: UUID().uuidString.lowercased(), source: "user", version: version,
            displayName: name, summary: summary, category: "custom", instructionsRef: reference,
            instructionsText: content, requiredTools: [], defaultMode: .inline, enabled: true,
            createdAtMs: now, updatedAtMs: now, deletedAtMs: nil))
        return .object(["status": .string("installed"), "id": .string(name), "enabled": .bool(true),
                        "source_url": fields["source_url"] ?? .null])
    }
}
