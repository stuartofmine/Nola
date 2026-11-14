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

    /// Private desktop-control callback. Only the real started Main request is
    /// accepted; template bodies and identity never come from model arguments.
    static func captureMainRequest(_ request: MacCanonicalJSONValue, conversationID: String, runID: String,
                                   using store: MacApplicationPort) throws -> AppleExpertCaptureContext<MacCanonicalJSONValue> {
        guard case .object(let envelope) = request,
              envelope["session_id"] == .string(conversationID), envelope["turn_id"] == .string(runID),
              case .object(let payload)? = envelope["payload"], payload["capability_id"] == .string("delegate_expert"),
              case .string(let call)? = payload["call_id"], !call.isEmpty,
              case .object(let input)? = payload["input"], input["action"] == .string("request"),
              case .string(let target)? = input["target_participant_id"], case .string(let text)? = input["text"] else {
            throw MacApplicationPortError.invalidResponse
        }
        let authority: AppleExpertTemplateAuthority = try store.executeExpertHost(.derivePeerAuthority(request: request, authority: .local()))
        let inReplyTo: String?
        if case .string(let id)? = input["in_reply_to"] { inReplyTo = id } else { inReplyTo = nil }
        let peer = AppleExpertPeerInput(sourceContextID: conversationID, sourceRunID: runID, callID: call,
            targetParticipantID: target, kind: "request", text: text, inReplyTo: inReplyTo)
        let preview: AppleExpertCapturePreview<MacCanonicalJSONValue> = try store.executeExpertHost(.previewPeer(input: peer, authority: authority))
        guard preview.participants.count == 1, preview.participants.first?.participant.participant_id == target else { throw MacApplicationPortError.invalidResponse }
        return try capture(preview, using: store, authority: authority)
    }

    static func capture(_ preview: AppleExpertCapturePreview<MacCanonicalJSONValue>,
                        using store: MacApplicationPort = .shared,
                        authority: AppleExpertTemplateAuthority = .local()) throws -> AppleExpertCaptureContext<MacCanonicalJSONValue> {
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
                selected = try MacPromptLibrary.defaultTemplateID(using: store)?.uuidString
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
        let selected = Set(selections.compactMap { $0.template_id.flatMap(UUID.init(uuidString:)) })
        return .init(actor: authority.actor, request_host_id: authority.request_host_id, selections: selections,
            builtins: BuiltInPromptTemplates.templates().filter { selected.contains($0.id) }.map { .init(provider_id: "apple.prompt-templates",
                template_id: $0.id.uuidString, content: $0.content, platform: .string("macos")) })
    }
}
