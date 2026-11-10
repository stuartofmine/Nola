// SPDX-License-Identifier: Apache-2.0
// Native Host selection and instruction producer; managed bodies stay Core-authorized.
import Foundation
import CryptoKit

@MainActor enum MacExpertInstructionProducer {
    static func definitions(style: MacResponseStyle, customSystemPrompt: String) -> [AppleExpertPromptDefinition<MacCanonicalJSONValue>] {
        var values = [("macos:response-depth", style.responseInstruction)]
        let custom = customSystemPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if !custom.isEmpty { values.append(("macos:user-custom-instructions", custom)) }
        return values.map { id, body in .init(component_id: id, content: body,
            content_sha256: SHA256.hash(data: Data(body.utf8)).map { String(format: "%02x", $0) }.joined(),
            definition_revision: nil, platform: .string("macos")) }
    }

    static func capture(_ preview: AppleExpertCapturePreview<MacCanonicalJSONValue>) throws -> AppleExpertCaptureContext<MacCanonicalJSONValue> {
        let authority = AppleExpertTemplateAuthority.local()
        let selections = try preview.participants.map { peer -> AppleExpertCaptureSelection<MacCanonicalJSONValue> in
            guard case .object(let defaults) = peer.defaults else { throw MacApplicationPortError.invalidResponse }
            let selected: String?
            let source: AppleExpertCaptureSelection<MacCanonicalJSONValue>.Source
            if let raw = defaults["selectedPromptTemplateID"], raw != .null {
                guard case .string(let id) = raw, UUID(uuidString: id) != nil else { throw MacApplicationPortError.invalidResponse }
                selected = id; source = .explicit
            } else {
                // A missing desktop default is an intentional absent managed
                // template. An invalid/unreadable explicit default throws.
                selected = try MacPromptLibrary.defaultTemplateID()?.uuidString
                source = selected == nil ? .noTemplate : .hostDefault
            }
            let style: MacResponseStyle
            if let raw = defaults["promptResponseStyle"], raw != .null {
                guard case .string(let value) = raw, let resolved = MacResponseStyle(rawValue: value) else { throw MacApplicationPortError.invalidResponse }
                style = resolved
            } else { style = .balanced }
            let parameters = try ModelParameters.decodeProfile(
                String(decoding: JSONEncoder().encode(peer.model_declaration.parameters), as: UTF8.self),
                inheriting: .defaults(isLocal: peer.model_declaration.runtime != "remote"))
            return .init(context_conversation_id: peer.participant.context_conversation_id,
                context_configuration_revision: peer.context_configuration_revision,
                model_profile: .init(profile_id: peer.model_declaration.profile_id, admitted_revision: peer.model_declaration.profile_revision),
                source: source, template_id: selected,
                host_user_instructions: definitions(style: style, customSystemPrompt: parameters.customSystemPrompt))
        }
        return .init(actor: authority.actor, request_host_id: authority.request_host_id, selections: selections,
            builtins: BuiltInPromptTemplates.templates().map { .init(provider_id: "apple.prompt-templates",
                template_id: $0.id.uuidString, content: $0.content, platform: .string("macos")) })
    }
}
