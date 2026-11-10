// SPDX-License-Identifier: Apache-2.0
// Trusted native Host inputs. These are not public renderer commands or grants.
import Foundation

nonisolated struct AppleExpertTemplateAuthority: Codable, Sendable {
    let actor: AgentCoreResourcePrincipal
    /// Native template resources are currently unbound. Null does not prove a
    /// Host identity; the authenticated native transport supplies Store ownership.
    let request_host_id: String?
    static func local() -> Self { .init(actor: AgentCoreResourceAccess.localPrincipal(), request_host_id: nil) }
    private enum CodingKeys: String, CodingKey { case actor, request_host_id }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(actor, forKey: .actor)
        try values.encode(request_host_id, forKey: .request_host_id)
    }
}

nonisolated struct AppleExpertCapturePreview<JSON: JournalJSONValue>: Decodable, Sendable {
    struct Participant: Decodable, Sendable {
        let participant: AppleExpertParticipantReference
        let context_configuration_revision: Int64
        let defaults: JSON
        let client_model_presentation: JSON
        let model_declaration: AppleExpertModelDeclaration<JSON>
        /// Core's same-snapshot generation resolution; never client defaults.
        let effective_parameters: JSON
    }
    let participants: [Participant]
}

nonisolated struct AppleExpertCaptureSelection<JSON: JournalJSONValue>: Encodable, Sendable {
    enum Source: String, Encodable, Sendable { case explicit, hostDefault = "host_default", noTemplate = "no_template" }
    struct ModelProfile: Encodable, Sendable { let profile_id: String; let admitted_revision: Int64 }
    let context_conversation_id: String
    let context_configuration_revision: Int64
    let model_profile: ModelProfile
    let source: Source
    let template_id: String?
    let host_user_instructions: [AppleExpertPromptDefinition<JSON>]
    private enum CodingKeys: String, CodingKey {
        case context_conversation_id, context_configuration_revision, model_profile, source, template_id, host_user_instructions
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(context_conversation_id, forKey: .context_conversation_id)
        try values.encode(context_configuration_revision, forKey: .context_configuration_revision)
        try values.encode(model_profile, forKey: .model_profile)
        try values.encode(source, forKey: .source)
        try values.encode(template_id, forKey: .template_id)
        try values.encode(host_user_instructions, forKey: .host_user_instructions)
    }
}

nonisolated struct AppleExpertBuiltinDefinition<JSON: JournalJSONValue>: Encodable, Sendable {
    let provider_id: String
    let template_id: String
    let content: String
    let platform: JSON
}

nonisolated struct AppleExpertCaptureContext<JSON: JournalJSONValue>: Encodable, Sendable {
    let actor: AgentCoreResourcePrincipal
    let request_host_id: String?
    let selections: [AppleExpertCaptureSelection<JSON>]
    let builtins: [AppleExpertBuiltinDefinition<JSON>]
    private enum CodingKeys: String, CodingKey { case actor, request_host_id, selections, builtins }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(actor, forKey: .actor)
        try values.encode(request_host_id, forKey: .request_host_id)
        try values.encode(selections, forKey: .selections)
        try values.encode(builtins, forKey: .builtins)
    }
}

nonisolated enum AppleExpertHostOperation<JSON: JournalJSONValue>: Encodable, Sendable {
    case preview(input: AppleAdmitExpertConversationMessage, authority: AppleExpertTemplateAuthority)
    case admit(input: AppleAdmitExpertConversationMessage, context: AppleExpertCaptureContext<JSON>)
    case prepareResend(input: ApplePrepareEditedResend, authority: AppleExpertTemplateAuthority)
    case loadManifest(conversationID: String, messageID: String, authority: AppleExpertTemplateAuthority)
    case loadPreparedResend(conversationID: String, authority: AppleExpertTemplateAuthority)
    case loadChildInstructions(conversationID: String, expectedRunID: String, authority: AppleExpertTemplateAuthority)
    case readableTemplates(authority: AppleExpertTemplateAuthority)

    var isReadOnly: Bool {
        switch self { case .admit, .prepareResend: false; default: true }
    }
    private enum CodingKeys: String, CodingKey {
        case kind, input, context, authority, conversation_id, message_id, expected_run_id
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .preview(let input, let authority):
            try values.encode("preview_capture", forKey: .kind)
            try values.encode(input, forKey: .input); try values.encode(authority, forKey: .authority)
        case .admit(let input, let context):
            try values.encode("admit", forKey: .kind)
            try values.encode(input, forKey: .input); try values.encode(context, forKey: .context)
        case .prepareResend(let input, let authority):
            try values.encode("prepare_edited_resend", forKey: .kind)
            try values.encode(input, forKey: .input); try values.encode(authority, forKey: .authority)
        case .loadManifest(let conversationID, let messageID, let authority):
            try values.encode("load_manifest", forKey: .kind)
            try values.encode(conversationID, forKey: .conversation_id)
            try values.encode(messageID, forKey: .message_id); try values.encode(authority, forKey: .authority)
        case .loadPreparedResend(let conversationID, let authority):
            try values.encode("load_prepared_resend", forKey: .kind)
            try values.encode(conversationID, forKey: .conversation_id); try values.encode(authority, forKey: .authority)
        case .loadChildInstructions(let conversationID, let expectedRunID, let authority):
            try values.encode("load_child_instructions", forKey: .kind)
            try values.encode(conversationID, forKey: .conversation_id)
            try values.encode(expectedRunID, forKey: .expected_run_id); try values.encode(authority, forKey: .authority)
        case .readableTemplates(let authority):
            try values.encode("readable_templates", forKey: .kind); try values.encode(authority, forKey: .authority)
        }
    }
}

/// Transport ordering across original owners, never execution authorization.
/// The full original operation is retained separately and Core validates it.
nonisolated enum AppleExpertHostIntentScope {
    static func operation(in envelope: [String: Any]) throws -> [String: Any] {
        guard let command = envelope["command"] as? [String: Any], command["type"] as? String == "host_expert_template_operation",
              let arguments = command["arguments"] as? [String: Any], arguments.count == 1,
              let operation = arguments["operation"] as? [String: Any] else { throw failure() }
        return operation
    }
    static func authority(_ operation: [String: Any]) throws -> [String: Any] {
        if let authority = operation["authority"] as? [String: Any] { return authority }
        guard let context = operation["context"] as? [String: Any], let actor = context["actor"] else { throw failure() }
        return ["actor": actor, "request_host_id": context["request_host_id"] ?? NSNull()]
    }
    static func stableIdentity(_ operation: [String: Any]) throws -> Data {
        try JSONSerialization.data(withJSONObject: ["kind": operation["kind"] ?? NSNull(),
            "input": operation["input"] ?? NSNull(), "authority": try authority(operation)], options: [.sortedKeys])
    }
    static func owners(_ operation: [String: Any]) -> Set<String>? {
        guard let input = operation["input"] as? [String: Any] else { return nil }
        func id(_ key: String, in fields: [String: Any]) -> String? {
            guard let value = fields[key] as? String, !value.isEmpty, value.utf8.count <= 512,
                  value.trimmingCharacters(in: .whitespacesAndNewlines) == value,
                  !value.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else { return nil }
            return value
        }
        switch operation["kind"] as? String {
        case "admit":
            guard let parent = id("conversation_id", in: input),
                  let participants = input["selected_participants"] as? [[String: Any]], !participants.isEmpty else { return nil }
            var owners: Set<String> = [parent]
            for participant in participants {
                guard let context = id("context_conversation_id", in: participant) else { return nil }
                owners.insert(context)
            }
            return owners
        case "prepare_edited_resend":
            guard let source = id("source_conversation_id", in: input), let target = id("target_conversation_id", in: input) else { return nil }
            return [source, target]
        default: return nil
        }
    }
    static func intersects(_ first: [String: Any], _ second: [String: Any]) -> Bool {
        guard let a = owners(first), let b = owners(second) else { return true }
        return !a.isDisjoint(with: b)
    }
    static func validateJournalOwners(_ pending: [String: [String: Any]]) throws {
        var operations: [[String: Any]] = []
        for envelope in pending.values {
            let candidate = try operation(in: envelope)
            guard !operations.contains(where: { intersects($0, candidate) }) else { throw failure() }
            operations.append(candidate)
        }
    }
    private static func failure() -> NSError {
        NSError(domain: "NolaExpertHostIntent", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "The original expert Host intent could not be confirmed."])
    }
}
