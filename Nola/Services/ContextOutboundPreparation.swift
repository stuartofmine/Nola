import Foundation

enum ContextOutboundPreparationError: LocalizedError, Equatable {
    case coreUnavailable
    case summaryStillRequired
    case cannotFit
    case invalidCoreAction(String)

    var errorDescription: String? {
        switch self {
        case .coreUnavailable:
            return "The shared context engine is unavailable."
        case .summaryStillRequired:
            return "The conversation still requires summarization before it can be sent."
        case .cannotFit:
            return "The prepared conversation cannot fit within the model context window."
        case .invalidCoreAction(let action):
            return "The shared context engine returned an unsupported action: \(action)."
        }
    }
}

public struct CorePreparedOutboundPayload {
    let messages: [AgentCoreContextPrepareMessage]
    let segments: [PromptSegment]
    let tools: [AgentCoreJSONValue]
    /// Raw typed inputs for the Agent Turn. Provider-only callers consume the
    /// budgeted `messages`; Core-hosted callers must pass these components
    /// back to Core so it can preserve source kind, authorization, freshness,
    /// and audit identity while applying the active Turn budget.
    let contextComponents: [AgentCoreContextComponent]
    let payloadDigest: String
    let metrics: AgentCoreContextPrepareMetrics
    let componentMetrics: [AgentCoreContextComponentMetric]

    /// Data sources that survived Core budgeting and are present in the provider payload.
    /// Truncated components still count as used; dropped components do not.
    var usedDataSourceIDs: [UUID] {
        var seen = Set<UUID>()
        return componentMetrics
            .filter { $0.kind == .platformData }
            .flatMap { $0.usedIds + $0.truncatedIds }
            .compactMap { componentID -> UUID? in
                let prefix = "data_source:"
                guard componentID.hasPrefix(prefix) else { return nil }
                return UUID(uuidString: String(componentID.dropFirst(prefix.count)))
            }
            .filter { seen.insert($0).inserted }
    }

    private struct ProviderEnvelope: Encodable {
        let schemaVersion = 1
        let payloadDigest: String
        let systemPrompt: String
        let messages: [AgentCoreContextPrepareMessage]
        let tools: [AgentCoreJSONValue]

        enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case payloadDigest = "payload_digest"
            case systemPrompt = "system_prompt"
            case messages, tools
        }
    }

    init(
        response: AgentCoreContextPrepareResponse,
        contextComponents: [AgentCoreContextComponent] = []
    ) throws {
        guard response.schemaVersion == 1 else {
            throw ContextOutboundPreparationError.invalidCoreAction("schema_\(response.schemaVersion)")
        }
        guard response.action == "ready" else {
            switch response.action {
            case "summary_required": throw ContextOutboundPreparationError.summaryStillRequired
            case "cannot_fit": throw ContextOutboundPreparationError.cannotFit
            default: throw ContextOutboundPreparationError.invalidCoreAction(response.action)
            }
        }
        messages = response.messages
        segments = response.messages.map { PromptSegment(role: $0.role, content: $0.content) }
        tools = response.tools
        self.contextComponents = contextComponents
        payloadDigest = response.payloadDigest
        metrics = response.metrics
        componentMetrics = response.componentMetrics
    }

    func providerEnvelopeJSON(systemPrompt: String, userQuery: String) throws -> String {
        var providerMessages = messages.filter { $0.role != "system" }
        if providerMessages.last?.role == "user" {
            providerMessages.removeLast()
        }
        providerMessages.append(AgentCoreContextPrepareMessage(
            id: "nola-current-user-query",
            role: "user",
            content: userQuery
        ))
        let envelope = ProviderEnvelope(
            payloadDigest: payloadDigest,
            systemPrompt: systemPrompt,
            messages: providerMessages,
            tools: tools
        )
        let data = try JSONEncoder().encode(envelope)
        guard let value = String(data: data, encoding: .utf8) else {
            throw ContextOutboundPreparationError.cannotFit
        }
        return value
    }
}
