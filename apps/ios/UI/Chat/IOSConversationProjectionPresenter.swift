// SPDX-License-Identifier: Apache-2.0
// Stateless adapter from the versioned Core projection to iOS view models.

import Foundation

@MainActor enum IOSConversationProjectionPresenter {
    static func messages(from projection: IOSConversationProjectionV1) -> [ChatMessage] {
        let byID = Dictionary(uniqueKeysWithValues: projection.messages.map { ($0.messageID, $0) })
        var seen = Set<String>()
        let orderedIDs = (
            projection.surface.committedMessageIDs
                + [projection.surface.liveMessageID].compactMap { $0 }
                + projection.messages.map(\.messageID)
        ).filter { seen.insert($0).inserted }
        return orderedIDs.compactMap { id in
            byID[id].flatMap {
                message(from: $0, activeRun: projection.activeRun, surface: projection.surface)
            }
        }
    }

    static func message(
        from projected: IOSConversationProjectionV1.Message,
        activeRun: IOSConversationProjectionV1.RunActivity?,
        surface: IOSConversationProjectionV1.Surface? = nil
    ) -> ChatMessage? {
        guard projected.role != .system else { return nil }
        let role: ChatMessage.Role = projected.role == .user ? .user : .assistant
        let content = projected.parts.compactMap { part -> String? in
            switch part {
            case .text(let value): return value.isDisclosed ? value.markdown : nil
            case .uiAction(let value): return value.summary
            case .historical(_, _, let projection):
                if projection.canonicalKind == "reasoning", projection.decodedText != nil { return nil }
                return projection.timelineText
            default: return nil
            }
        }.joined()
        let reasoning = projected.parts.compactMap { part -> String? in
            switch part {
            case .reasoning(let value): return value.summary
            case .historical(_, _, let projection) where projection.canonicalKind == "reasoning": return projection.decodedText
            default: return nil
            }
        }.joined()
        let tools = projected.parts.compactMap(toolEvent)
        let currentToolCallID = projected.parts.compactMap { part -> String? in
            guard part.partID == surface?.currentActivityPartID,
                  case .tool(let value) = part
            else { return nil }
            return value.toolCallID
        }.first
        let ownsActivePart = activeRun?.activePartID.map { activeID in
            projected.parts.contains { $0.partID == activeID }
        } ?? false
        var metadata = ["nola.canonicalMessageID": projected.messageID]
        metadata[ChatConversationSurfaceMeta.stateKey] = projected.state.rawValue
        if let surface {
            metadata[ChatConversationSurfaceMeta.liveKey] = String(surface.liveMessageID == projected.messageID)
            metadata[ChatConversationSurfaceMeta.placeholderKey] = String(
                surface.liveMessageID == projected.messageID && surface.showAssistantPlaceholder
            )
            metadata[ChatConversationSurfaceMeta.archivedActivityIDsKey] =
                surface.archivedActivityPartIDs.joined(separator: "\u{1F}")
            if let currentToolCallID {
                metadata[ChatConversationSurfaceMeta.currentToolCallIDKey] = currentToolCallID
            }
        }
        return ChatMessage(
            id: UUID(uuidString: projected.messageID)
                ?? IOSCanonicalMessageIdentity.stableUUID(for: projected.messageID),
            role: role,
            content: content,
            includeInContext: true,
            thinking: reasoning,
            thinkingFinished: projected.state != .running && projected.state != .pending,
            hostToolEvents: tools,
            tracePhase: ownsActivePart ? activeRun?.displayKey : nil,
            canonicalParts: projected.parts,
            sourceReferences: sourceReferences(from: projected.parts),
            canonicalMessageRevision: projected.revision,
            metadata: metadata
        )
    }

    nonisolated static func toolEvent(
        _ part: IOSConversationProjectionV1.Part
    ) -> HostAgentToolEvent? {
        guard case .tool(let value) = part else { return nil }
        return toolEvent(from: value)
    }

    nonisolated static func toolEvent(
        from value: IOSConversationProjectionV1.ToolPart,
        preservingPresentationFrom previous: HostAgentToolEvent? = nil
    ) -> HostAgentToolEvent {
        let presentation = previous.flatMap { $0.id == value.toolCallID ? $0 : nil }
        let status: HostAgentToolEvent.Status
        switch value.state {
        case .pending, .running, .waitingForInput: status = .running
        case .waitingForPermission: status = .permissionRequired
        case .completed, .sealed: status = .succeeded
        case .cancelled: status = .cancelled
        case .failed: status = .failed
        }
        return HostAgentToolEvent(
            id: value.toolCallID,
            name: value.capabilityID,
            argsPreview: value.input.displayText,
            status: status,
            display: value.title,
            text: value.output?.displayText ?? value.outputPreview ?? "",
            screenshotURL: presentation?.screenshotURL, finalURL: presentation?.finalURL,
            pageTitle: presentation?.pageTitle, downloadURL: presentation?.downloadURL,
            downloadFilename: presentation?.downloadFilename, screenshotWarning: presentation?.screenshotWarning,
            sensitive: presentation?.containsSensitiveData ?? false
        )
    }

    /// Canonical facts always win for an owned call, including after a late
    /// legacy draft update. Uncommitted trace is not a source of these facts.
    nonisolated static func overlayCanonicalTools(
        on events: [HostAgentToolEvent], parts: [IOSConversationProjectionV1.Part]
    ) -> [HostAgentToolEvent] {
        let tools = parts.compactMap { part -> IOSConversationProjectionV1.ToolPart? in
            guard case .tool(let tool) = part else { return nil }
            return tool
        }
        var result = events.map { event in
            guard let tool = tools.first(where: { $0.toolCallID == event.id }) else { return event }
            return toolEvent(from: tool, preservingPresentationFrom: event)
        }
        for tool in tools where !result.contains(where: { $0.id == tool.toolCallID }) {
            result.append(toolEvent(from: tool))
        }
        return result
    }

    static func sourceReferences(from parts: [IOSConversationProjectionV1.Part]) -> [ChatSourceReference] {
        var sources = [String: IOSConversationProjectionV1.SourcePart]()
        for case .source(let source) in parts { sources[source.sourceID] = source }
        var cited = Set<String>()
        var references = [ChatSourceReference]()
        for case .citation(let citation) in parts {
            cited.insert(citation.sourceID)
            let source = sources[citation.sourceID]
            references.append(ChatSourceReference(id: citation.citationID,
                sourceID: citation.sourceID, citationID: citation.citationID,
                title: citation.title, uri: citation.uri, mimeType: citation.mimeType ?? source?.mimeType,
                location: citation.location, originToolCallID: citation.originToolCallID,
                memoryUsage: source?.evidence?.usage,
                rangeStart: citation.rangeStart, rangeEnd: citation.rangeEnd))
        }
        for case .source(let source) in parts where !cited.contains(source.sourceID) {
            references.append(ChatSourceReference(id: source.sourceID, sourceID: source.sourceID,
                title: source.title, uri: source.uri, mimeType: source.mimeType, memoryUsage: source.evidence?.usage))
        }
        return references
    }
}
