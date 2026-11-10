// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Models persistent assistant traces and assembles raw model deltas into stable semantic reveal nodes.

import Foundation
import CryptoKit

nonisolated struct MacAssistantCompactionReference: Codable, Hashable, Identifiable, Sendable {
    let id: String
    let summary: String
    let sourceItemIDs: [String]
    let sourceFingerprint: String
}

/// Public canonical display facts, not an executable permission challenge.
/// Summary, preview and structured result remain separate even when empty/null.
nonisolated struct MacAssistantToolMetadata: Codable, Hashable, Sendable {
    let toolCallID: String
    let capabilityID: String
    let origin: MacCoreCapabilityOrigin
    let permission: MacCoreToolPermission?
    let summary: String?
    let outputPreview: String?
    let output: MacCanonicalJSONValue?
    let fullOutput: MacCanonicalToolFullOutput?
    let artifacts: [MacCorePartArtifact]
    let startedAtMs: UInt64?
    let finishedAtMs: UInt64?

    var resultText: String? { output?.displayString ?? outputPreview }
    var sourceLabel: String { origin.providerTitle ?? origin.providerID ?? origin.kind }
}

nonisolated struct MacAssistantPresentation: Codable, Hashable, Sendable {
    var textNodes: [MacAssistantTextNode] = []
    var mutableTextNode: MacAssistantTextNode?
    var traceItems: [MacAssistantTrace] = []
    var sourceReferences: [MacAssistantSourceReference]?
    var taskHandles: [MacCoreTaskHandle]?
    var taskGuidance: [String: MacCoreTaskGuidanceQueue]?
    var compactionReferences: [MacAssistantCompactionReference]?
    var lastCommittedID: UUID?
    var isComplete = false

    var renderedTextNodes: [MacAssistantTextNode] {
        guard let mutableTextNode,
              !textNodes.contains(where: { $0.id == mutableTextNode.id }) else {
            return textNodes
        }
        // Keep the unfinished block visible under its stable identity. The
        // renderer repairs incomplete inline Markdown and only animates new
        // semantic nodes, so a growing prose tail no longer disappears until
        // punctuation arrives or rebuilds the already-painted prefix.
        return textNodes + [mutableTextNode]
    }

    var renderedSourceReferences: [MacAssistantSourceReference] {
        sourceReferences ?? []
    }

    var renderedTaskHandles: [MacCoreTaskHandle] { taskHandles ?? [] }
    var renderedTaskGuidance: [String: MacCoreTaskGuidanceQueue] { taskGuidance ?? [:] }
    var renderedCompactionReferences: [MacAssistantCompactionReference] {
        compactionReferences ?? []
    }

    var hasVisibleContent: Bool {
        !renderedTextNodes.isEmpty
            || !traceItems.isEmpty
            || !renderedSourceReferences.isEmpty
            || !renderedTaskHandles.isEmpty
            || !renderedCompactionReferences.isEmpty
    }

    mutating func replaceTaskProjection(
        _ tasks: [MacCoreTaskHandle],
        guidance: [String: MacCoreTaskGuidanceQueue]
    ) {
        guard taskHandles != tasks || taskGuidance != guidance else { return }
        taskHandles = tasks
        taskGuidance = guidance
    }

    mutating func replaceSourceReferences(_ references: [MacAssistantSourceReference]) {
        guard sourceReferences != references else { return }
        sourceReferences = references
    }

    mutating func upsertCompactionReference(_ reference: MacAssistantCompactionReference) {
        var references = compactionReferences ?? []
        if let index = references.firstIndex(where: { $0.id == reference.id }) {
            guard references[index] != reference else { return }
            references[index] = reference
        } else {
            references.append(reference)
        }
        compactionReferences = references
    }

    var lastStableIdentity: String {
        (lastCommittedID ?? textNodes.last?.id ?? traceItems.last?.id)?.uuidString ?? "empty"
    }

    /// Drives transcript following. Trace/tool mutations deliberately do not
    /// participate because a collapsed activity card has not moved the answer.
    var lastTextStableIdentity: String {
        guard let node = renderedTextNodes.last else { return "pending" }
        return "\(node.id.uuidString):\(node.text.utf8.count)"
    }

    static func processing() -> MacAssistantPresentation {
        MacAssistantPresentation()
    }

    private static func stableTraceIdentity(_ externalID: String?) -> UUID {
        guard let externalID, !externalID.isEmpty else { return UUID() }
        return MacCanonicalMessageIdentity.stableID("trace:\(externalID)") ?? UUID()
    }

    private static func stableTraceDate(_ externalID: String?) -> Date {
        externalID?.isEmpty == false ? .distantPast : .now
    }

    mutating func append(_ nodes: [MacAssistantTextNode]) {
        guard !nodes.isEmpty else { return }
        if let mutableTextNode,
           nodes.contains(where: { $0.id == mutableTextNode.id }) {
            self.mutableTextNode = nil
        }
        textNodes.append(contentsOf: nodes)
        lastCommittedID = nodes.last?.id
    }

    mutating func updateMutableTextNode(_ node: MacAssistantTextNode?) {
        guard mutableTextNode != node else { return }
        mutableTextNode = node
    }

    mutating func replaceText(with nodes: [MacAssistantTextNode]) {
        textNodes = nodes
        mutableTextNode = nil
        if let id = nodes.last?.id { lastCommittedID = id }
    }

    mutating func appendThinking(_ value: String, replacing: Bool) {
        guard !value.isEmpty else { return }
        if let index = traceItems.lastIndex(where: {
            $0.kind == .thinking && $0.state == .running
        }) {
            if replacing {
                traceItems[index].detail = value
            } else if !traceItems[index].detail.hasSuffix(value) {
                traceItems[index].detail += value
            }
            return
        }
        let item = MacAssistantTrace(
            kind: .thinking,
            title: String(localized: "Thinking"),
            detail: value,
            state: .running
        )
        traceItems.append(item)
        lastCommittedID = item.id
    }

    /// Uses transport thinking only until the first canonical Reasoning Part
    /// arrives. Core snapshots can mark a reasoning part complete before
    /// already-buffered diagnostic deltas are delivered; accepting those late
    /// deltas would create a second, temporary process row during execution.
    mutating func appendFallbackThinking(_ value: String, replacing: Bool) {
        guard !traceItems.contains(where: {
            $0.kind == .thinking && $0.externalID != nil
        }) else { return }
        appendThinking(value, replacing: replacing)
    }

    mutating func upsertCoreThinking(
        partID: String,
        title: String,
        detail: String,
        isComplete: Bool
    ) {
        let state: MacAssistantTrace.State = isComplete ? .completed : .running
        if let index = traceItems.firstIndex(where: {
            $0.kind == .thinking && $0.externalID == partID
        }) {
            guard traceItems[index].title != title
                    || traceItems[index].detail != detail
                    || traceItems[index].state != state else { return }
            traceItems[index].title = title
            traceItems[index].detail = detail
            traceItems[index].state = state
            traceItems[index].finishedAt = state.isTerminal ? .now : nil
            return
        }
        if let index = traceItems.lastIndex(where: {
            $0.kind == .thinking && $0.externalID == nil && $0.state == .running
        }) {
            traceItems[index].externalID = partID
            traceItems[index].title = title
            traceItems[index].detail = detail
            traceItems[index].state = state
            traceItems[index].finishedAt = state.isTerminal ? .now : nil
            return
        }
        let item = MacAssistantTrace(
            id: Self.stableTraceIdentity(partID),
            externalID: partID,
            kind: .thinking,
            title: title,
            detail: detail,
            state: state,
            startedAt: Self.stableTraceDate(partID),
            finishedAt: state.isTerminal ? Self.stableTraceDate(partID) : nil
        )
        traceItems.append(item)
        lastCommittedID = item.id
    }

    mutating func upsertTool(
        externalID: String?,
        title: String,
        detail: String,
        state: MacAssistantTrace.State,
        arguments: String? = nil,
        result: String? = nil,
        affectedFiles: [String] = []
    ) {
        // Tool discovery and paged-output reads are transport plumbing. They
        // may be frequent and useful to the agent, but admitting them into
        // the presentation model invalidates the whole process card without
        // adding user-visible work. Real tools remain fully auditable.
        guard !MacToolTraceDisplayPolicy.isProtocolPlumbingTool(title) else {
            // A canonical Part may identify a provisional generic tool as
            // protocol plumbing. Reconcile that same identity instead of
            // leaving its old running row visible indefinitely.
            if let externalID, !externalID.isEmpty {
                traceItems.removeAll { $0.kind == .tool && $0.externalID == externalID }
            }
            return
        }
        let result = MacToolTraceDisplayPolicy.persistedResult(
            toolName: title,
            raw: result
        )
        let statusDetail = MacAssistantTrace.statusDetail(
            toolName: title,
            rawDetail: detail,
            state: state
        )
        let index = traceItems.lastIndex { item in
            guard item.kind == .tool else { return false }
            if let externalID, !externalID.isEmpty { return item.externalID == externalID }
            return item.title == title && item.state == .running
        }
        if let index {
            let existing = traceItems[index]
            let hasSamePayload: Bool
            if state == .running || state == .waiting {
                hasSamePayload = arguments == nil || existing.arguments == arguments
            } else {
                hasSamePayload = result == nil || existing.result == result
            }
            if existing.state == state,
               existing.title == title,
               existing.detail == statusDetail,
               hasSamePayload,
               (arguments == nil || existing.arguments == arguments),
               (result == nil || existing.result == result),
               affectedFiles.allSatisfy({ existing.affectedFiles?.contains($0) == true }) {
                return
            }
            traceItems[index].title = title
            if !detail.isEmpty {
                traceItems[index].detail = statusDetail
                if state == .running {
                    if let arguments, !arguments.isEmpty {
                        traceItems[index].arguments = arguments
                    } else if traceItems[index].arguments == nil {
                        traceItems[index].arguments = detail
                    }
                } else if state.isTerminal {
                    traceItems[index].result = result ?? detail
                }
                // Core owns the affected-resource list. Parsing arbitrary
                // output text made every file merely mentioned by a tool look
                // like an operated artifact and produced rows of duplicate
                // file chips below the answer.
                let discoveredFiles = affectedFiles
                if !discoveredFiles.isEmpty {
                    let currentFiles = traceItems[index].affectedFiles ?? []
                    traceItems[index].affectedFiles = Array(NSOrderedSet(array: currentFiles + discoveredFiles))
                        .compactMap { $0 as? String }
                }
            }
            if let arguments, !arguments.isEmpty {
                traceItems[index].arguments = arguments
            }
            if let result, !result.isEmpty {
                traceItems[index].result = result
            }
            traceItems[index].state = state
            traceItems[index].finishedAt = state.isTerminal ? .now : nil
            return
        }
        let item = MacAssistantTrace(
            id: Self.stableTraceIdentity(externalID),
            externalID: externalID,
            kind: .tool,
            title: title,
            detail: statusDetail,
            arguments: arguments ?? (state == .running ? detail : nil),
            result: result ?? (state.isTerminal ? detail : nil),
            affectedFiles: Array(NSOrderedSet(array: affectedFiles))
                .compactMap { $0 as? String },
            state: state,
            startedAt: Self.stableTraceDate(externalID),
            finishedAt: state.isTerminal ? Self.stableTraceDate(externalID) : nil
        )
        traceItems.append(item)
        lastCommittedID = item.id
    }

    /// Canonical rich Parts bypass the legacy string-envelope result filter.
    /// A human title must never change capability-based visibility or authority.
    mutating func upsertCanonicalTool(partID: String, title: String, input: String,
                                     state: MacAssistantTrace.State, metadata: MacAssistantToolMetadata,
                                     affectedFiles: [String]) {
        guard !MacToolTraceDisplayPolicy.isProtocolPlumbingTool(metadata.capabilityID) else { return }
        let item = MacAssistantTrace(
            id: Self.stableTraceIdentity(partID), externalID: partID, kind: .tool,
            title: title, detail: metadata.summary ?? "", arguments: input,
            result: metadata.resultText, affectedFiles: affectedFiles, state: state,
            startedAt: metadata.startedAtMs.map { Date(timeIntervalSince1970: Double($0) / 1000) }
                ?? Self.stableTraceDate(partID),
            finishedAt: metadata.finishedAtMs.map { Date(timeIntervalSince1970: Double($0) / 1000) },
            tool: metadata
        )
        if let index = traceItems.firstIndex(where: { $0.kind == .tool && $0.externalID == partID }) {
            traceItems[index] = item
        } else {
            traceItems.append(item)
        }
        lastCommittedID = item.id
    }

    mutating func finishActiveThinking() {
        guard let index = traceItems.lastIndex(where: {
            $0.kind == .thinking && $0.state == .running
        }) else { return }
        traceItems[index].state = .completed
        traceItems[index].finishedAt = .now
    }

    mutating func upsertStatus(
        externalID: String,
        title: String,
        detail: String,
        state: MacAssistantTrace.State
    ) {
        if let index = traceItems.firstIndex(where: {
            $0.kind == .status && $0.externalID == externalID
        }) {
            guard traceItems[index].title != title
                    || traceItems[index].detail != detail
                    || traceItems[index].state != state else {
                return
            }
            traceItems[index].title = title
            traceItems[index].detail = detail
            traceItems[index].state = state
            traceItems[index].finishedAt = state.isTerminal ? .now : nil
            return
        }
        let item = MacAssistantTrace(
            id: Self.stableTraceIdentity(externalID),
            externalID: externalID,
            kind: .status,
            title: title,
            detail: detail,
            state: state,
            startedAt: Self.stableTraceDate(externalID),
            finishedAt: state.isTerminal ? Self.stableTraceDate(externalID) : nil
        )
        traceItems.append(item)
        lastCommittedID = item.id
    }

    /// Some execution facts are deliberately client-visible even though Core
    /// Parts remain the canonical owner of answer, reasoning, and tool rows.
    /// Keep those disclosures when a newer canonical projection arrives.
    static func preservesAcrossCanonicalProjection(_ trace: MacAssistantTrace) -> Bool {
        guard trace.kind == .status, let externalID = trace.externalID else { return false }
        // The committed Attempt ledger owns model failure and retry facts.
        // Discard both obsolete transport previews and protocol-recovery
        // overlays instead of duplicating their canonical activities.
        if externalID.hasPrefix("model-route:") || externalID.hasPrefix("model-protocol:") {
            return false
        }
        if externalID == "validation-diagnostic" { return true }
        return externalID.hasPrefix("agent-host-status:")
            && (trace.state == .failed || trace.state == .cancelled)
    }

    mutating func reconcileCanonicalTraceItems(_ canonical: [MacAssistantTrace]) {
        let canonicalIDs = Set(canonical.map(\.id))
        let disclosures = traceItems.filter {
            Self.preservesAcrossCanonicalProjection($0) && !canonicalIDs.contains($0.id)
        }
        traceItems = canonical + disclosures
    }

    /// Converts a transport/startup failure into an explicit run status.
    /// The initial processing presentation contains a provisional thinking
    /// row so the assistant bubble appears immediately. If the Host fails
    /// before Core publishes reasoning, tools, or text, keeping that row
    /// misrepresents a launch failure as a failed thinking process.
    mutating func recordRunFailure(
        externalID: String,
        title: String,
        detail: String
    ) {
        traceItems.removeAll {
            $0.kind == .thinking && $0.state == .running && $0.externalID == nil
        }
        upsertStatus(
            externalID: externalID,
            title: title,
            detail: detail,
            state: .failed
        )
    }

    mutating func removeStatus(externalID: String) {
        traceItems.removeAll {
            $0.kind == .status && $0.externalID == externalID
        }
    }

    mutating func updateWaitingTool(
        title: String,
        detail: String,
        state: MacAssistantTrace.State
    ) {
        guard let index = traceItems.lastIndex(where: {
            $0.kind == .tool
                && $0.title == title
                && ($0.state == .waiting || $0.state == .running)
        }) else {
            upsertTool(externalID: nil, title: title, detail: detail, state: state)
            return
        }
        traceItems[index].detail = detail
        traceItems[index].state = state
        if state.isTerminal {
            traceItems[index].result = detail
            traceItems[index].finishedAt = .now
        } else {
            traceItems[index].finishedAt = nil
        }
    }

    mutating func appendPlan(_ detail: String, externalID: String? = nil) {
        guard !detail.isEmpty else { return }
        finishActiveThinking()
        if let externalID,
           let index = traceItems.lastIndex(where: {
               $0.kind == .plan && $0.externalID == externalID
           }) {
            guard traceItems[index].detail != detail else { return }
            traceItems[index].detail = detail
            traceItems[index].state = .running
            traceItems[index].finishedAt = nil
            return
        }
        let item = MacAssistantTrace(
            id: Self.stableTraceIdentity(externalID),
            externalID: externalID,
            kind: .plan,
            title: String(localized: "Plan"),
            detail: detail,
            state: .running,
            startedAt: Self.stableTraceDate(externalID)
        )
        traceItems.append(item)
        lastCommittedID = item.id
    }

    mutating func finish(finishedAt: Date? = .now) {
        mutableTextNode = nil
        isComplete = true
        for index in traceItems.indices
        where traceItems[index].state == .running || traceItems[index].state == .waiting {
            switch traceItems[index].kind {
            case .tool:
                // A tool is complete only after its matching terminal event arrives.
                // Treat an unmatched start as interrupted instead of presenting its
                // arguments as a successful result.
                traceItems[index].state = .failed
                traceItems[index].detail = String(localized: "Stopped")
                traceItems[index].result = nil
            case .thinking, .plan, .status:
                traceItems[index].state = .completed
            }
            traceItems[index].finishedAt = finishedAt
        }
    }
}

nonisolated struct MacAssistantTextNode: Identifiable, Codable, Hashable, Sendable {
    enum Kind: Codable, Hashable, Sendable {
        case phrase
        case markdownBlock
        case code(language: String)
        case mermaid
    }

    let id: UUID
    let blockID: UUID
    let kind: Kind
    let text: String

    init(id: UUID = UUID(), blockID: UUID, kind: Kind, text: String) {
        self.id = id
        self.blockID = blockID
        self.kind = kind
        self.text = text
    }
}

nonisolated enum MacStreamingPresentationTiming {
    // Streamdown's useful constraint is a short reveal with bounded visual
    // debt: presentation should stay close to transport instead of building a
    // long typewriter queue. Keep the same contract in native SwiftUI.
    static let semanticStepMilliseconds = 48
    static let burstStepMilliseconds = 20
    static let revealDuration = AppleStreamingRevealTiming.duration
    // Non-text diagram entrance only; glyph ink uses the shared wave renderer.
    static let initialRevealOpacity = 0.7
    static let revealStagger = 0.02
    static let maximumRevealBacklog = 0.32

    static func revealStarts(
        for ids: [UUID],
        at now: Date,
        after existingHorizon: Date? = nil
    ) -> [UUID: Date] {
        let uniqueIDs = ids.reduce(into: [UUID]()) { result, id in
            if !result.contains(id) { result.append(id) }
        }
        guard !uniqueIDs.isEmpty else { return [:] }
        let backlogEnd = now.addingTimeInterval(maximumRevealBacklog)
        let requestedStart = max(existingHorizon ?? now, now)
        let firstStart = min(requestedStart, backlogEnd)
        let available = max(0, backlogEnd.timeIntervalSince(firstStart))
        let step = uniqueIDs.count > 1
            ? min(revealStagger, max(0.004, available / Double(uniqueIDs.count - 1)))
            : 0
        var starts: [UUID: Date] = [:]
        for (index, id) in uniqueIDs.enumerated() {
            starts[id] = firstStart.addingTimeInterval(Double(index) * step)
        }
        return starts
    }

    static func stepMilliseconds(queuedSegments: Int) -> Int {
        if queuedSegments >= 12 { return burstStepMilliseconds }
        if queuedSegments >= 4 { return 32 }
        return semanticStepMilliseconds
    }
}

nonisolated enum MacAssistantTraceTimelinePolicy {
    static func initiallyExpandsDetails(for state: MacAssistantTrace.State) -> Bool {
        _ = state
        return false
    }

    /// Mirrors Codex's active-item/history-item contract: every semantic
    /// reasoning or tool item remains in order, while only that item's detail
    /// disclosure is collapsible. A later tool must never hide an earlier one.
    static func visibleDetailTraces(_ traces: [MacAssistantTrace]) -> [MacAssistantTrace] {
        MacAssistantArtifactPresentationPolicy.visibleProcessTraces(traces)
    }

    /// Only adjacent tools share a disclosure. Appending an event must never
    /// move an earlier group across reasoning or another tool operation.
    static func groupedDetailTraces(_ traces: [MacAssistantTrace]) -> [MacAssistantTraceGroup] {
        var groups: [MacAssistantTraceGroup] = []
        for trace in visibleDetailTraces(traces) {
            guard trace.kind == .tool else {
                groups.append(MacAssistantTraceGroup(traces: [trace]))
                continue
            }
            let key = trace.title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if let group = groups.last, group.isToolGroup,
                group.traces[0].title
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                    .lowercased() == key {
                groups[groups.count - 1].traces.append(trace)
            } else {
                groups.append(MacAssistantTraceGroup(traces: [trace]))
            }
        }
        return groups
    }

    struct Page {
        let traces: [MacAssistantTrace]
        let earlierEndID: UUID?
        let newerEndID: UUID?
        let isHistory: Bool
    }

    /// A history page is anchored by identity, not an offset from the growing
    /// tail. Only an explicit navigation action switches back to live activity.
    static func page(_ traces: [MacAssistantTrace], endingAt endID: UUID?, limit: Int = 24) -> Page {
        let end: Int
        if let endID {
            guard let index = traces.firstIndex(where: { $0.id == endID }) else {
                return Page(traces: [], earlierEndID: nil, newerEndID: nil, isHistory: true)
            }
            end = index + 1
        } else { end = traces.count }
        let start = max(0, end - max(1, limit))
        return Page(traces: Array(traces[start..<end]),
                    earlierEndID: start > 0 ? traces[start - 1].id : nil,
                    newerEndID: end < traces.count ? traces[min(traces.count, end + max(1, limit)) - 1].id : nil,
                    isHistory: endID != nil)
    }
}

nonisolated struct MacAssistantTraceGroup: Identifiable, Hashable, Sendable {
    var traces: [MacAssistantTrace]

    var id: UUID { traces[0].id }
    var isToolGroup: Bool { traces[0].kind == .tool }

    var representative: MacAssistantTrace {
        traces.last(where: { $0.state == .running })
            ?? traces.last(where: { $0.state == .waiting })
            ?? traces.last(where: { $0.state == .failed })
            ?? traces[traces.count - 1]
    }

    var state: MacAssistantTrace.State {
        if traces.contains(where: { $0.state == .running }) { return .running }
        if traces.contains(where: { $0.state == .waiting }) { return .waiting }
        if traces.contains(where: { $0.state == .failed }) { return .failed }
        if traces.contains(where: { $0.state == .cancelled }) { return .cancelled }
        if traces.contains(where: { $0.state == .incomplete }) { return .incomplete }
        return .completed
    }
}

/// Keeps workflow scratch files in the audit trail instead of presenting them
/// as user deliverables. The policy is deliberately resource-based: it never
/// infers intent from user wording or a localized Skill name.
nonisolated enum MacAssistantArtifactPresentationPolicy {
    static func isDeliverableOperation(_ operation: String) -> Bool {
        switch operation.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "create", "write", "edit", "export", "download", "generate", "save", "output":
            return true
        default:
            return false
        }
    }

    static func isIntermediate(_ url: URL) -> Bool {
        let normalized = url.standardizedFileURL
        let path = normalized.path.lowercased()
        guard path.contains("/workspaces/sessions/") else { return false }

        let name = normalized.lastPathComponent.lowercased()
        let stem = normalized.deletingPathExtension().lastPathComponent.lowercased()
        let ext = normalized.pathExtension.lowercased()
        if name.hasPrefix("_") || ["tmp", "temp", "bak", "log"].contains(ext) {
            return true
        }
        if ["spec", "manifest", "request", "payload"].contains(stem)
            || stem.hasSuffix("-spec") || stem.hasSuffix("_spec") {
            return true
        }
        if ["py", "sh", "js"].contains(ext)
            && (stem.contains("helper")
                || stem.contains("patch")
                || stem.contains("inject")
                || stem.contains("add_link")) {
            return true
        }
        return false
    }

    static func shouldAttach(_ url: URL) -> Bool {
        !isIntermediate(url)
    }

    static func shouldShowInProcess(_ trace: MacAssistantTrace) -> Bool {
        // Empty thinking is a live placeholder, not a completed process step.
        // Retain failures and genuine reasoning; never mutate the audit data.
        if trace.kind == .thinking,
           (trace.state == .completed || trace.state == .cancelled),
           trace.detail.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return false
        }
        // Attachment promotion and process transparency are separate
        // concerns. Scratch resources should not become user deliverables,
        // but the operation that produced or consumed them remains part of
        // the chronological audit trail, matching Codex TurnItem behavior.
        if trace.kind == .tool,
           MacToolTraceDisplayPolicy.isProtocolPlumbingTool(trace.capabilityID) {
            return false
        }
        if trace.kind == .status,
           trace.externalID?.hasPrefix("agent-host-status:") == true,
           trace.state != .failed,
           trace.state != .cancelled {
            return false
        }
        return true
    }

    static func visibleProcessTraces(_ traces: [MacAssistantTrace]) -> [MacAssistantTrace] {
        traces.filter(shouldShowInProcess)
    }
}

nonisolated indirect enum MacAgentTraceJSONValue: Codable, Hashable, Sendable {
    case null
    case boolean(Bool)
    case number(Double)
    case string(String)
    case array([MacAgentTraceJSONValue])
    case object([String: MacAgentTraceJSONValue])

    var objectValue: [String: MacAgentTraceJSONValue]? {
        guard case .object(let value) = self else { return nil }
        return value
    }

    var stringValue: String? {
        guard case .string(let value) = self else { return nil }
        return value
    }

    var booleanValue: Bool? {
        guard case .boolean(let value) = self else { return nil }
        return value
    }

    var integerValue: Int? {
        guard case .number(let value) = self, value.isFinite else { return nil }
        return Int(exactly: value)
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .boolean(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([MacAgentTraceJSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: MacAgentTraceJSONValue].self))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .boolean(let value):
            try container.encode(value)
        case .number(let value):
            try container.encode(value)
        case .string(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        }
    }

    var displayString: String {
        if case .string(let value) = self { return value }
        let object = foundationObject
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(
                  withJSONObject: object,
                  options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
              ) else {
            return String(describing: object)
        }
        return String(decoding: data, as: UTF8.self)
    }

    private var foundationObject: Any {
        switch self {
        case .null: return NSNull()
        case .boolean(let value): return value
        case .number(let value): return value
        case .string(let value): return value
        case .array(let values): return values.map(\.foundationObject)
        case .object(let values): return values.mapValues { $0.foundationObject }
        }
    }

    init(jsonObject value: Any) throws {
        switch value {
        case is NSNull:
            self = .null
        case let value as NSNumber:
            if CFGetTypeID(value) == CFBooleanGetTypeID() {
                self = .boolean(value.boolValue)
            } else {
                self = .number(value.doubleValue)
            }
        case let value as String:
            self = .string(value)
        case let values as [Any]:
            self = .array(try values.map(Self.init(jsonObject:)))
        case let values as [String: Any]:
            self = .object(try values.mapValues(Self.init(jsonObject:)))
        default:
            throw CocoaError(.coderReadCorrupt)
        }
    }
}

nonisolated struct MacCorePartArtifact: Codable, Hashable, Sendable {
    let uri: String
    let name: String
    let mimeType: String?
    let operation: String?

    private enum CodingKeys: String, CodingKey {
        case uri, name, operation
        case mimeType = "mime_type"
    }
}

nonisolated struct MacCoreToolPermission: Codable, Hashable, Sendable {
    let requestID: String
    let permissionDomain: String
    let rationale: String
    let decision: String?

    private enum CodingKeys: String, CodingKey {
        case rationale, decision
        case requestID = "request_id"
        case permissionDomain = "permission_domain"
    }
}

/// Capability provenance retained by canonical tool presentation.
nonisolated struct MacCoreCapabilityOrigin: Codable, Hashable, Sendable {
    let kind: String
    let providerID: String?
    let providerTitle: String?
    let namespace: String?

    private enum CodingKeys: String, CodingKey {
        case kind, namespace
        case providerID = "provider_id"
        case providerTitle = "provider_title"
    }
}


nonisolated struct MacCoreTurnTerminalEnvelope: Codable, Hashable, Sendable {
    enum Status: String, Codable, Hashable, Sendable {
        case completed
        case failed
        case cancelled
    }

    let sessionID: String
    let turnID: String
    let cursor: UInt64
    let revision: UInt64
    let status: Status
    let reason: String?

    static func validated(
        sessionID: String,
        turnID: String,
        cursor: UInt64,
        revision: UInt64,
        status: Status,
        reason: String?
    ) -> Self? {
        let sessionID = sessionID.trimmingCharacters(in: .whitespacesAndNewlines)
        let turnID = turnID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sessionID.isEmpty,
              !turnID.isEmpty,
              cursor > 0,
              revision > 0 else {
            return nil
        }
        return Self(
            sessionID: sessionID,
            turnID: turnID,
            cursor: cursor,
            revision: revision,
            status: status,
            reason: reason
        )
    }
}




nonisolated struct MacAgentTraceResource: Codable, Hashable, Sendable {
    let uri: String
    let operation: String
    let displayName: String?
    let mimeType: String?

    private enum CodingKeys: String, CodingKey {
        case uri
        case operation
        case displayName = "display_name"
        case mimeType = "mime_type"
    }

    var filePath: String? {
        if uri.hasPrefix("/") { return uri }
        guard let url = URL(string: uri) else { return nil }
        if url.isFileURL {
            return url.standardizedFileURL.path
        }
        if url.scheme == nil, !uri.isEmpty {
            return uri
        }
        return nil
    }
}

nonisolated struct MacAgentTraceUsage: Codable, Hashable, Sendable {
    let usedTokens: Int
    let contextWindowTokens: Int
    let promptTokenLimit: Int
    let cachedInputTokens: Int?
    let cachedInputTokensReported: Bool?
    let cacheWriteInputTokens: Int?
    let cacheWriteInputTokensReported: Bool?

    private enum CodingKeys: String, CodingKey {
        case usedTokens = "used_tokens"
        case contextWindowTokens = "context_window_tokens"
        case promptTokenLimit = "prompt_token_limit"
        case cachedInputTokens = "cached_input_tokens"
        case cachedInputTokensReported = "cached_input_tokens_reported"
        case cacheWriteInputTokens = "cache_write_input_tokens"
        case cacheWriteInputTokensReported = "cache_write_input_tokens_reported"
    }
}

nonisolated struct MacPromptCacheDiagnostic: Codable, Hashable, Sendable {
    var cacheUsage: MacPromptCacheUsage? = nil
    let schemaVersion: Int
    let status: String
    let identityStable: Bool
    let identitySHA256: String?
    let reasons: [String]
    let fragmentChanges: [MacPromptCacheFragmentChange]?
    let cacheReadInputTokens: Int?
    let cacheWriteInputTokens: Int?

    private enum CodingKeys: String, CodingKey {
        case cacheUsage
        case schemaVersion
        case status
        case identityStable
        case identitySHA256 = "identitySha256"
        case reasons
        case fragmentChanges
        case cacheReadInputTokens
        case cacheWriteInputTokens
    }
}

nonisolated struct MacPromptCacheFragmentChange: Codable, Hashable, Sendable {
    let fragmentId: String
    let kind: String
}

nonisolated struct MacAgentTraceValidation: Codable, Hashable, Sendable {
    let status: String
    let level: String?
    let summary: String?
    let nextAction: String?

    private enum CodingKeys: String, CodingKey {
        case status
        case level
        case summary
        case nextAction = "next_action"
    }
}

nonisolated struct MacAgentTraceMemoryUsage: Codable, Hashable, Sendable {
    let recordID: String
    let componentID: String
    let scope: MacAgentTraceJSONValue?
    let namespace: MacAgentTraceJSONValue?
    let provenanceKind: String?
    let source: String?
    let title: String?
    let observedAt: String?
    let requiresCurrentStateVerification: Bool
    let truncated: Bool

    private enum CodingKeys: String, CodingKey {
        case recordID = "record_id"
        case componentID = "component_id"
        case scope
        case namespace
        case provenanceKind = "provenance_kind"
        case source
        case title
        case observedAt = "observed_at"
        case requiresCurrentStateVerification = "requires_current_state_verification"
        case truncated
    }

    var displayLine: String {
        let label = title?.trimmingCharacters(in: .whitespacesAndNewlines)
        let fields = [
            label?.isEmpty == false ? label : recordID,
            scope?.displayString,
            namespace?.displayString,
            source,
            observedAt,
        ].compactMap { value -> String? in
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return trimmed.isEmpty ? nil : trimmed
        }
        return fields.joined(separator: " · ")
    }
}

nonisolated struct MacAgentTracePayload: Codable, Hashable, Sendable {
    let phase: String?
    let summary: String?
    let detail: String?
    let metadata: MacAgentTraceJSONValue?
    let toolName: String?
    let callID: String?
    let arguments: MacAgentTraceJSONValue?
    let result: MacAgentTraceJSONValue?
    let permissionRequestID: String?
    let permissionScope: String?
    let permissionDecision: String?
    let reason: String?
    let code: String?
    let progress: Double?
    let usage: MacAgentTraceUsage?
    let promptCacheDiagnostic: MacPromptCacheDiagnostic?
    let validation: MacAgentTraceValidation?
    let resources: [MacAgentTraceResource]?
    let memories: [MacAgentTraceMemoryUsage]?

    private enum CodingKeys: String, CodingKey {
        case phase
        case summary
        case detail
        case metadata
        case toolName = "tool_name"
        case callID = "call_id"
        case arguments
        case result
        case permissionRequestID = "permission_request_id"
        case permissionScope = "permission_scope"
        case permissionDecision = "permission_decision"
        case reason
        case code
        case progress
        case usage
        case promptCacheDiagnostic = "prompt_cache_diagnostic"
        case validation
        case resources
        case memories
    }
}

nonisolated struct MacAgentTraceEnvelope: Codable, Hashable, Sendable {
    let schemaVersion: Int
    let traceID: String
    let runID: String
    let conversationID: String
    let messageID: String?
    let turnID: String
    let eventID: String
    let sequence: UInt64
    let emittedAt: String
    let kind: String
    let spanID: String?
    let parentSpanID: String?
    let status: String
    let visibility: String
    let payload: MacAgentTracePayload

    private enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case traceID = "trace_id"
        case runID = "run_id"
        case conversationID = "conversation_id"
        case messageID = "message_id"
        case turnID = "turn_id"
        case eventID = "event_id"
        case sequence
        case emittedAt = "emitted_at"
        case kind
        case spanID = "span_id"
        case parentSpanID = "parent_span_id"
        case status
        case visibility
        case payload
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        kind = try values.decode(String.self, forKey: .kind)
        guard !["assistant.delta", "assistant.reset", "thinking.delta"].contains(kind) else {
            throw DecodingError.dataCorruptedError(forKey: .kind, in: values,
                debugDescription: "Assistant content belongs to the committed journal")
        }
        enum RetiredPayloadKeys: String, CodingKey { case delta }
        let fields = try values.nestedContainer(keyedBy: RetiredPayloadKeys.self, forKey: .payload)
        guard !fields.contains(.delta) else {
            throw DecodingError.dataCorruptedError(forKey: .delta, in: fields,
                debugDescription: "Trace cannot carry body deltas")
        }
        schemaVersion = try values.decode(Int.self, forKey: .schemaVersion)
        traceID = try values.decode(String.self, forKey: .traceID)
        runID = try values.decode(String.self, forKey: .runID)
        conversationID = try values.decode(String.self, forKey: .conversationID)
        messageID = try values.decodeIfPresent(String.self, forKey: .messageID)
        turnID = try values.decode(String.self, forKey: .turnID)
        eventID = try values.decode(String.self, forKey: .eventID)
        sequence = try values.decode(UInt64.self, forKey: .sequence)
        emittedAt = try values.decode(String.self, forKey: .emittedAt)
        spanID = try values.decodeIfPresent(String.self, forKey: .spanID)
        parentSpanID = try values.decodeIfPresent(String.self, forKey: .parentSpanID)
        status = try values.decode(String.self, forKey: .status)
        visibility = try values.decode(String.self, forKey: .visibility)
        payload = try values.decode(MacAgentTracePayload.self, forKey: .payload)
    }

    static func decode(_ value: Any?) -> MacAgentTraceEnvelope? {
        guard let value = value as? [String: Any],
              !["assistant.delta", "assistant.reset", "thinking.delta"].contains(value["kind"] as? String ?? ""),
              (value["payload"] as? [String: Any])?["delta"] == nil,
              JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value) else {
            return nil
        }
        return try? JSONDecoder().decode(MacAgentTraceEnvelope.self, from: data)
    }

    var isError: Bool {
        ["failed", "cancelled", "incomplete"].contains(status.lowercased())
    }

    var affectedFiles: [String] {
        Array(NSOrderedSet(array: payload.resources?.compactMap(\.filePath) ?? []))
            .compactMap { $0 as? String }
    }
}

nonisolated enum MacAgentTraceApplyDisposition: Equatable {
    case applied
    case duplicate
    case gap(expected: UInt64, received: UInt64)
    case outOfOrder(last: UInt64, received: UInt64)
    case unsupportedSchema(Int)
    case unsupportedKind(String)
    case identityMismatch
    case eventConflict
    case invalidEvent
    case retiredTrace
}

nonisolated struct MacAgentTraceApplyOutcome: Equatable {
    let disposition: MacAgentTraceApplyDisposition
    let usage: MacAgentTraceUsage?
    let promptCacheDiagnostic: MacPromptCacheDiagnostic?
    let isTerminal: Bool

    static func ignored(_ disposition: MacAgentTraceApplyDisposition) -> Self {
        Self(
            disposition: disposition,
            usage: nil,
            promptCacheDiagnostic: nil,
            isTerminal: false
        )
    }
}

/// Presentation stream ownership for one admitted conversation execution.
/// Canonical Run/generation authority remains with the caller, not this value.
nonisolated struct MacConversationTraceReducer {
    private var conversationID: String?
    private var activeTraceID: String?
    private var active = MacAgentTraceReducer()
    private var retiredTraceIDs: Set<String> = []

    mutating func apply(_ event: MacAgentTraceEnvelope,
                        to presentation: inout MacAssistantPresentation,
                        projectsContent: Bool = true) -> MacAgentTraceApplyOutcome {
        if let conversationID, conversationID != event.conversationID {
            return .ignored(.identityMismatch)
        }
        guard !retiredTraceIDs.contains(event.traceID) else {
            return .ignored(.retiredTrace)
        }
        let switching = activeTraceID != event.traceID
        var candidate = switching ? MacAgentTraceReducer() : active
        let result = candidate.apply(event, to: &presentation, projectsContent: projectsContent)
        switch result.disposition {
        case .applied, .gap:
            if switching, let activeTraceID { retiredTraceIDs.insert(activeTraceID) }
            conversationID = event.conversationID
            activeTraceID = event.traceID
            active = candidate
        default:
            break
        }
        return result
    }
}

nonisolated struct MacAgentTraceReducer {
    private var traceID: String?
    private var runID: String?
    private var conversationID: String?
    private var lastSequence: UInt64 = 0
    private var seenEventIDs: [String: SHA256.Digest] = [:]
    private var projectedToolProgress: [String: String] = [:]

    mutating func apply(
        _ event: MacAgentTraceEnvelope,
        to presentation: inout MacAssistantPresentation,
        projectsContent: Bool = true
    ) -> MacAgentTraceApplyOutcome {
        guard event.schemaVersion == 1 else {
            return .ignored(.unsupportedSchema(event.schemaVersion))
        }
        guard !["assistant.delta", "assistant.reset", "thinking.delta"].contains(event.kind) else {
            return .ignored(.unsupportedKind(event.kind))
        }
        // One reducer owns one immutable stream identity. The conversation
        // owner creates a separate reducer when it admits a successor stream.
        if let conversationID, conversationID != event.conversationID {
            return .ignored(.identityMismatch)
        }
        if let traceID, traceID != event.traceID {
            return .ignored(.identityMismatch)
        }
        if let runID, runID != event.runID {
            return .ignored(.identityMismatch)
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let bytes = try? encoder.encode(event) else {
            return .ignored(.invalidEvent)
        }
        let digest = SHA256.hash(data: bytes)
        if let previous = seenEventIDs[event.eventID] {
            return .ignored(previous == digest ? .duplicate : .eventConflict)
        }
        guard event.sequence > lastSequence else {
            return .ignored(.outOfOrder(last: lastSequence, received: event.sequence))
        }
        if traceID == nil {
            traceID = event.traceID
            runID = event.runID
            conversationID = event.conversationID
        }

        // Sequence is a cursor in the host's global event journal. A client can
        // legitimately observe holes when events are hidden, but the gap remains
        // useful diagnostic information and must not stop later events.
        let expectedSequence = lastSequence + 1
        let disposition: MacAgentTraceApplyDisposition = event.sequence == expectedSequence
            ? .applied
            : .gap(expected: expectedSequence, received: event.sequence)

        seenEventIDs[event.eventID] = digest
        lastSequence = event.sequence
        // Core Parts are the canonical presentation projection for Agent Host
        // sessions. Trace still advances its cursor and exposes usage and
        // diagnostics, but must not render the same reasoning/tool operation a
        // second time when the caller has delegated content ownership to Core.
        if projectsContent {
            applyProjection(event, to: &presentation)
        } else if ["run.completed", "run.failed", "run.cancelled", "phase.changed", "context.compacted", "validation.updated"]
            .contains(event.kind) {
            // Core owns answer/reasoning/tool Parts, but terminal failure is a
            // Run lifecycle fact rather than duplicated content. Suppressing
            // it leaves an empty green "completed" bubble when the Host has
            // actually failed. Compaction phase feedback is also transport
            // state: Core owns the durable checkpoint, while the trace owns
            // the short-lived "compressing" notice shown before it exists.
            applyProjection(event, to: &presentation)
        }
        return MacAgentTraceApplyOutcome(
            disposition: disposition,
            usage: event.kind == "usage.snapshot" ? event.payload.usage : nil,
            promptCacheDiagnostic: event.kind == "usage.snapshot"
                ? event.payload.promptCacheDiagnostic
                : nil,
            isTerminal: ["run.completed", "run.failed", "run.cancelled"].contains(event.kind)
        )
    }

    private mutating func applyProjection(
        _ event: MacAgentTraceEnvelope,
        to presentation: inout MacAssistantPresentation
    ) {
        switch event.kind {
        case "run.started":
            // Run bookkeeping drives the pending indicator but is not a
            // user-visible operation. Remove stale rows decoded from older hosts.
            presentation.removeStatus(externalID: "run-status")
        case "phase.changed":
            switch event.payload.phase {
            case "compacting":
                presentation.upsertStatus(
                    externalID: "context-compaction",
                    title: String(localized: "Compressing conversation context"),
                    detail: String(localized: "Earlier context is being summarized. The original conversation remains available."),
                    state: .running
                )
            case "compaction_failed":
                presentation.upsertStatus(
                    externalID: "context-compaction",
                    title: String(localized: "Context compression failed"),
                    detail: event.payload.detail
                        ?? event.payload.reason
                        ?? String(localized: "The conversation will continue without replacing the original context."),
                    state: .failed
                )
            default:
                presentation.removeStatus(externalID: "run-status")
            }
        case "context.compacted":
            presentation.removeStatus(externalID: "context-compaction")
        case "tool.started", "tool.progress", "tool.permission.requested",
             "tool.permission.resolved", "tool.completed":
            applyTool(event, to: &presentation)
        case "plan.updated":
            if let detail = event.payload.detail ?? event.payload.summary,
               !detail.isEmpty {
                presentation.appendPlan(
                    detail,
                    externalID: event.spanID ?? event.eventID
                )
            }
        case "validation.updated":
            guard let validation = event.payload.validation else { return }
            let title: String
            let state: MacAssistantTrace.State
            switch validation.status.lowercased() {
            case "evidence_insufficient":
                title = String(localized: "Evidence insufficient")
                state = .incomplete
            case "incomplete":
                title = String(localized: "Incomplete")
                state = .incomplete
            case "blocked":
                title = String(localized: "Blocked")
                state = .incomplete
            case "failed", "error":
                title = String(localized: "Failed")
                state = .failed
            case "required", "not_required", "succeeded", "completed":
                presentation.removeStatus(externalID: "validation-diagnostic")
                return
            default:
                return
            }
            presentation.upsertStatus(
                externalID: "validation-diagnostic",
                title: title,
                detail: validation.summary
                    ?? validation.nextAction
                    ?? title,
                state: state
            )
        case "memory.used":
            // Trace retains the redacted usage ledger for diagnostics. Core
            // projects user-visible memory evidence as SourcePart so it has a
            // single stable lifecycle and cannot duplicate or flicker here.
            return
        case "run.completed":
            presentation.removeStatus(externalID: "context-compaction")
            presentation.finish()
        case "run.failed", "run.cancelled":
            presentation.removeStatus(externalID: "context-compaction")
            presentation.upsertStatus(
                externalID: "run-status",
                title: String(localized: "Nola"),
                detail: event.payload.reason
                    ?? event.payload.summary
                    ?? String(localized: "Stopped"),
                state: .failed
            )
            presentation.finish()
        default:
            break
        }
    }

    private static func localizedToolName(_ rawName: String?) -> String {
        let normalized = rawName?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: "-", with: "_") ?? ""
        switch normalized {
        case "agent", "nola", "local_agent", "nola_agent":
            return "Nola"
        case "skill_context":
            return String(localized: "Skill context")
        case "read_file":
            return String(localized: "Read file")
        case "write_file", "append_file", "edit_file", "apply_patch":
            return String(localized: "Edit file")
        case "list_dir", "file_tree":
            return String(localized: "Browse files")
        case "find_files", "search_files", "search_text":
            return String(localized: "Search files")
        case "project_overview":
            return String(localized: "Inspect project")
        case "codebase_search", "codebase_text_search", "codebase_trace", "codebase_read_symbol":
            return String(localized: "Search files")
        case "codebase_index", "codebase_index_status", "codebase_projects", "codebase_architecture",
             "codebase_query", "codebase_schema", "codebase_check_coverage", "codebase_changed_impact",
             "codebase_delete_index", "codebase_adr", "codebase_ingest_traces":
            return String(localized: "Inspect project")
        case "web_search":
            return String(localized: "Search the web")
        case "web_fetch":
            return String(localized: "Read webpage")
        case "bash", "shell", "run_command":
            return String(localized: "Run command")
        case "memory_search":
            return String(localized: "Search memory")
        case "memory_save", "memory_write":
            return String(localized: "Save memory")
        case "search_calendar_events":
            return String(localized: "Search calendar")
        case "create_calendar_event":
            return String(localized: "Create calendar event")
        case "search_reminders":
            return String(localized: "Search reminders")
        case "create_reminder":
            return String(localized: "Create reminder")
        case "update_reminder":
            return String(localized: "Update reminder")
        case "browser", "chrome", "chrome_control":
            return String(localized: "Control Chrome")
        case let name where name.hasPrefix("chrome_native_") || name.hasPrefix("browser_"):
            return String(localized: "Control Chrome")
        case let name where name.hasPrefix("computer_"):
            return String(localized: "Control this Mac")
        case "nola_orchestrate":
            return String(localized: "Create automation")
        case "":
            return String(localized: "Tool")
        default:
            return rawName ?? String(localized: "Tool")
        }
    }

    private mutating func applyTool(
        _ event: MacAgentTraceEnvelope,
        to presentation: inout MacAssistantPresentation
    ) {
        if event.kind == "tool.progress", event.payload.toolName == nil, event.payload.callID == nil {
            return
        }
        let toolName = event.payload.toolName?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased() ?? ""
        guard !Self.internalControlTools.contains(toolName) else { return }
        let projectionID = event.spanID ?? event.payload.callID ?? event.eventID
        if event.kind == "tool.progress" {
            guard let progress = meaningfulToolProgress(event, projectionID: projectionID) else {
                return
            }
            projectedToolProgress[projectionID] = progress
        } else if event.kind == "tool.completed" {
            projectedToolProgress[projectionID] = nil
        }
        let arguments = event.payload.arguments?.displayString
        let result = event.payload.result?.displayString
        let detail = event.payload.summary
            ?? event.payload.detail
            ?? (event.kind == "tool.completed" ? result : arguments)
            ?? ""
        let recoverableProtocolCorrection = event.isError
            && [event.payload.summary, event.payload.detail, result]
                .compactMap { $0 }
                .contains {
                    $0.contains("tool_execution_skipped")
                }
        let state: MacAssistantTrace.State
        if event.kind == "tool.permission.requested" {
            state = .waiting
        } else if event.kind == "tool.completed" {
            state = event.isError && !recoverableProtocolCorrection ? .failed : .completed
        } else if event.kind == "tool.permission.resolved",
                  event.isError || event.payload.permissionDecision == "deny" {
            state = .failed
        } else {
            state = .running
        }
        presentation.upsertTool(
            externalID: projectionID,
            // Keep the protocol identity stable in persisted traces. Views own
            // localization and can therefore render restored history in the
            // user's current language instead of freezing the launch language.
            title: event.payload.toolName ?? String(localized: "Tool"),
            detail: detail,
            state: state,
            arguments: arguments,
            result: result,
            affectedFiles: event.affectedFiles
        )
    }

    /// A provider can stream a tool's JSON arguments one character at a time.
    /// Those transport fragments are useful to the protocol journal, but they
    /// are not presentation events. Project only an actual human-readable
    /// progress sentence, at semantic boundaries, and keep the canonical tool
    /// row stable for the rest of the run.
    private func meaningfulToolProgress(
        _ event: MacAgentTraceEnvelope,
        projectionID: String
    ) -> String? {
        let candidate = (event.payload.summary ?? event.payload.detail ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if candidate == "model_retry" {
            return Bundle.main.localizedString(
                forKey: "mac.memory.retry",
                value: "Retry",
                table: nil
            )
        }
        guard candidate.count >= 12 else { return nil }

        let arguments = event.payload.arguments?.displayString
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let result = event.payload.result?.displayString
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard candidate != arguments, candidate != result else { return nil }

        if (candidate.hasPrefix("{") && !candidate.hasSuffix("}"))
            || (candidate.hasPrefix("[") && !candidate.hasSuffix("]")) {
            return nil
        }
        if let previous = projectedToolProgress[projectionID] {
            guard previous != candidate else { return nil }
            let growth = abs(candidate.count - previous.count)
            let endsAtBoundary = candidate.last.map {
                "\n。！？；：.!?;:".contains($0)
            } ?? false
            guard growth >= 48 || endsAtBoundary else { return nil }
        }
        return candidate
    }

    private static let internalControlTools: Set<String> = [
        "agent_task_status",
        "goal_complete",
        "set_thinking_mode",
    ]
}

nonisolated struct MacAssistantTrace: Identifiable, Codable, Hashable, Sendable {
    enum Kind: String, Codable, Hashable, Sendable {
        case thinking
        case tool
        case plan
        case status
    }

    enum State: String, Codable, Hashable, Sendable {
        case running
        case waiting
        case completed
        case failed
        case cancelled
        // Settled diagnostic, not a failed execution or permission wait.
        case incomplete

        var isTerminal: Bool { self == .completed || self == .failed || self == .cancelled || self == .incomplete }
    }

    let id: UUID
    var externalID: String?
    let kind: Kind
    var title: String
    var detail: String
    var arguments: String?
    var result: String?
    var affectedFiles: [String]?
    var state: State
    let startedAt: Date
    var finishedAt: Date?
    var tool: MacAssistantToolMetadata?

    var capabilityID: String { tool?.capabilityID ?? title }

    init(
        id: UUID = UUID(),
        externalID: String? = nil,
        kind: Kind,
        title: String,
        detail: String,
        arguments: String? = nil,
        result: String? = nil,
        affectedFiles: [String]? = nil,
        state: State,
        startedAt: Date = .now,
        finishedAt: Date? = nil,
        tool: MacAssistantToolMetadata? = nil
    ) {
        self.id = id
        self.externalID = externalID
        self.kind = kind
        self.title = title
        self.detail = detail
        self.arguments = arguments
        self.result = result
        self.affectedFiles = affectedFiles
        self.state = state
        self.startedAt = startedAt
        self.finishedAt = finishedAt
        self.tool = tool
    }

    var toolSymbol: String {
        let value = "\(title) \(detail)".lowercased()
        if value.contains("search") || value.contains("find") || value.contains("检索") || value.contains("搜索") {
            return "magnifyingglass"
        }
        if value.contains("read") || value.contains("file") || value.contains("读取") || value.contains("文件") {
            return "doc.text"
        }
        if value.contains("shell") || value.contains("terminal") || value.contains("command") || value.contains("bash") {
            return "terminal"
        }
        if value.contains("edit") || value.contains("write") || value.contains("patch")
            || value.contains("编辑") || value.contains("写入") {
            return "pencil.and.outline"
        }
        if value.contains("screenshot") || value.contains("computer") || value.contains("截图") {
            return "camera.viewfinder"
        }
        if value.contains("browser") || value.contains("web") || value.contains("网页") {
            return "globe"
        }
        return "wrench.and.screwdriver"
    }

    static func statusDetail(toolName: String, rawDetail: String, state: State) -> String {
        let trimmed = rawDetail.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }

        if let data = trimmed.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            let preferredKeys = [
                "command", "query", "path", "file_path", "target_path",
                "url", "pattern", "description"
            ]
            for key in preferredKeys {
                guard let value = object[key] as? String else { continue }
                let summary = firstDisplayLine(in: value)
                if !summary.isEmpty { return summary }
            }
        }

        return firstDisplayLine(in: trimmed)
    }

    private static func firstDisplayLine(in value: String) -> String {
        let line = value
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first(where: { !$0.isEmpty }) ?? ""
        if line == "{" || line == "[" || line == "}" || line == "]" {
            return ""
        }
        let maximumLength = 160
        guard line.count > maximumLength else { return line }
        return String(line.prefix(maximumLength - 1)) + "…"
    }

    static func filePaths(in value: String) -> [String] {
        guard !value.isEmpty else { return [] }
        let pattern = #"(?i)(?:\"|\b)(?:path|file|file_path|notebook_path|target_path)(?:\"|\b)\s*[:=]\s*[\"']?([^,}\n\"']+)"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return [] }
        let range = NSRange(value.startIndex..<value.endIndex, in: value)
        let paths = regex.matches(in: value, range: range).compactMap { match -> String? in
            guard match.numberOfRanges > 1,
                  let pathRange = Range(match.range(at: 1), in: value) else { return nil }
            return String(value[pathRange]).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return Array(NSOrderedSet(array: paths).compactMap { $0 as? String }).prefix(12).map { $0 }
    }
}

nonisolated enum MacToolTraceDisplayPolicy {
    private static let payloadKeys: Set<String> = [
        "body", "content", "contents", "data", "insert", "new_text",
        "old_text", "patch", "replacement", "source", "text",
    ]
    private static let preferredKeys = [
        "path", "file_path", "target_path", "notebook_path", "cwd",
        "command", "query", "url", "pattern", "start_line", "line_start",
        "end_line", "line_end", "offset", "limit", "timeout_sec",
    ]

    static func parameters(for trace: MacAssistantTrace) -> String? {
        guard let raw = trace.arguments?.trimmingCharacters(in: .whitespacesAndNewlines),
              !raw.isEmpty else {
            return nil
        }
        guard let object = jsonObject(from: raw) as? [String: Any] else {
            return normalized(raw)
        }

        let keys = object.keys
            .filter { !$0.hasPrefix("_") }
            .sorted { lhs, rhs in
                let leftRank = preferredKeys.firstIndex(of: lhs) ?? preferredKeys.count
                let rightRank = preferredKeys.firstIndex(of: rhs) ?? preferredKeys.count
                return leftRank == rightRank ? lhs < rhs : leftRank < rightRank
            }
        let lines = keys.compactMap { key -> String? in
            guard let value = object[key] else { return nil }
            if payloadKeys.contains(key.lowercased()) {
                guard let byteCount = payloadByteCount(value), byteCount > 0 else { return nil }
                let size = ByteCountFormatter.string(
                    fromByteCount: Int64(byteCount),
                    countStyle: .file
                )
                return "\(displayLabel(for: key)) · \(size)"
            }
            guard let value = compactValue(value), !value.isEmpty else { return nil }
            return "\(displayLabel(for: key)) · \(value)"
        }
        guard !lines.isEmpty else { return nil }
        return lines.joined(separator: "\n")
    }

    static func operationTitle(for trace: MacAssistantTrace) -> String? {
        guard !isProtocolPlumbingTool(trace.capabilityID) else { return nil }
        // Preserve the actual canonical title instead of inferring a new one.
        if trace.tool != nil { return trace.title }
        let normalizedTitle = trace.capabilityID
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "-", with: "_")
            .replacingOccurrences(of: " ", with: "_")
            .lowercased()
        let arguments = trace.arguments.flatMap(jsonObject(from:)) as? [String: Any]
        let target = [
            "query", "pattern", "command", "url", "path", "file_path",
            "target_path",
        ]
            .compactMap { arguments?[$0] as? String }
            .first?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let fileTarget = target.map {
            let name = URL(fileURLWithPath: $0).lastPathComponent
            return name.isEmpty ? $0 : name
        }

        if ["list_dir", "list_directory", "file_tree"].contains(normalizedTitle) {
            return titled(String(localized: "List folder"), target: fileTarget)
        }
        if ["read_file", "read_text_file", "read_many_files", "notebook_read", "file_outline", "read_symbol"]
            .contains(normalizedTitle) {
            return titled(String(localized: "Read file"), target: fileTarget)
        }
        if ["write_file", "create_file", "append_file"].contains(normalizedTitle) {
            return titled(String(localized: "Write file"), target: fileTarget)
        }
        if [
            "edit_file", "edit_file_lines", "multi_edit_file", "notebook_edit",
            "apply_patch", "replace_text", "replace_symbol_body", "replace_in_files",
            "rename_symbol", "format_files", "move_file",
        ].contains(normalizedTitle) {
            return titled(String(localized: "Edit file"), target: fileTarget)
        }
        if ["delete_file", "remove_file"].contains(normalizedTitle) {
            return titled(String(localized: "Delete file"), target: fileTarget)
        }
        if normalizedTitle == "bash"
            || normalizedTitle.contains("shell")
            || normalizedTitle.contains("command") {
            return titled(String(localized: "Run command"), target: target)
        }
        if normalizedTitle == "workspace_status" {
            return String(localized: "Inspect workspace")
        }
        if normalizedTitle == "tool_search" {
            return String(localized: "Search")
        }
        if normalizedTitle.hasPrefix("tool_output_") {
            return String(localized: "Read")
        }
        if [
            "file_search", "find_files", "find_up", "grep", "glob", "search_files",
            "ripgrep", "code_search", "workspace_symbols", "symbol_search",
            "implementation_search", "document_search",
        ].contains(normalizedTitle) {
            return titled(String(localized: "Search"), target: target)
        }
        return nil
    }

    private static func titled(_ action: String, target: String?) -> String {
        guard let target, !target.isEmpty else { return action }
        let firstLine = target.split(whereSeparator: \.isNewline).first.map(String.init)
            ?? target
        let conciseTarget = firstLine.count > 72
            ? String(firstLine.prefix(71)) + "…"
            : firstLine
        return "\(action) \(conciseTarget)"
    }

    static func headerDetail(for trace: MacAssistantTrace) -> String? {
        guard !isProtocolPlumbingTool(trace.capabilityID) else { return nil }
        let detail = trace.detail.trimmingCharacters(in: .whitespacesAndNewlines)
        let protocolStates: Set<String> = [
            "running", "completed", "succeeded", "failed", "waiting", "cancelled", "canceled"
        ]
        guard !detail.isEmpty,
              !protocolStates.contains(detail.lowercased()),
              detail != trace.title,
              detail != "{",
              detail != "[",
              !detail.hasPrefix("{"),
              !detail.hasPrefix("["),
              jsonObject(from: detail) == nil else {
            return nil
        }
        if let operationTitle = operationTitle(for: trace),
           operationTitle.localizedCaseInsensitiveContains(detail) {
            return nil
        }
        return concise(detail, maximumCharacters: 220, maximumLines: 2)
    }

    static func result(for trace: MacAssistantTrace) -> String? {
        guard let full = fullResult(for: trace) else { return nil }
        return concise(full, maximumCharacters: 640, maximumLines: 8)
    }

    static func fullResult(for trace: MacAssistantTrace) -> String? {
        guard !isProtocolPlumbingTool(trace.capabilityID) else { return nil }
        if let tool = trace.tool { return tool.resultText }
        guard let raw = trace.result?.trimmingCharacters(in: .whitespacesAndNewlines),
              !raw.isEmpty else {
            return nil
        }

        if let object = jsonObject(from: raw) as? [String: Any] {
            if let display = (object["display"] ?? object["_nola_display"]) as? String,
               let summary = normalized(display),
               !summary.isEmpty {
                return summary
            }
            if isFileRead(trace) {
                return safeFileOperationSummary(trace)
            }
            if let preview = object["output_preview"] as? String,
               let summary = humanReadablePayload(preview),
               !summary.isEmpty {
                return summary
            }
            if let text = object["text"] as? String {
                return humanReadablePayload(text)
            }
            if let error = object["error"] as? String {
                return normalized(error)
            }
            // JSON envelopes are protocol data. If none of the explicitly
            // user-facing fields above are present, do not leak transport
            // metadata such as `_nola_control` or continuation hints.
            return nil
        }

        if isFileRead(trace) {
            return safeFileOperationSummary(trace)
        }
        return normalized(raw)
    }

    static func persistedResult(toolName: String, raw: String?) -> String? {
        guard !isProtocolPlumbingTool(toolName) else { return nil }
        guard let raw = raw?.trimmingCharacters(in: .whitespacesAndNewlines),
              !raw.isEmpty else {
            return nil
        }
        let maximumCharacters = 8_000
        guard let object = jsonObject(from: raw) as? [String: Any] else {
            return concise(raw, maximumCharacters: maximumCharacters, maximumLines: 120)
        }

        let retainedKeys = [
            "display", "error", "output_preview", "full_output_ref", "artifacts",
            "path", "file_path", "target_path", "start_line", "line_start",
            "end_line", "line_end", "line_count", "lines",
        ]
        var retained = retainedKeys.reduce(into: [String: Any]()) { partial, key in
            if let value = object[key] {
                partial[key] = value
            }
        }
        if retained["display"] == nil, let display = object["_nola_display"] {
            retained["display"] = display
        }
        if retained["display"] == nil,
           retained["output_preview"] == nil,
           let text = object["text"] as? String {
            retained["output_preview"] = concise(
                text,
                maximumCharacters: 4_000,
                maximumLines: 80
            )
        }
        if retained.isEmpty {
            if object.keys.contains(where: { $0.hasPrefix("_nola_") })
                || object["continuation_hint"] != nil {
                return nil
            }
            return concise(raw, maximumCharacters: maximumCharacters, maximumLines: 120)
        }
        guard JSONSerialization.isValidJSONObject(retained),
              let data = try? JSONSerialization.data(
                withJSONObject: retained,
                options: [.sortedKeys, .withoutEscapingSlashes]
              ),
              let value = String(data: data, encoding: .utf8) else {
            return concise(raw, maximumCharacters: maximumCharacters, maximumLines: 120)
        }
        return value
    }

    static func isProtocolPlumbingTool(_ rawTitle: String) -> Bool {
        let title = rawTitle
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "-", with: "_")
            .replacingOccurrences(of: " ", with: "_")
            .lowercased()
        return title == "tool_search" || title.hasPrefix("tool_output_")
    }

    static func fileReadOperation(for trace: MacAssistantTrace) -> MacToolFileOperation? {
        // Rich canonical rows own their exact title and artifacts. The legacy
        // file-read header must not replace that title with inferred text.
        guard trace.tool == nil else { return nil }
        guard isFileRead(trace) else { return nil }
        let arguments = trace.arguments.flatMap(jsonObject(from:)) as? [String: Any]
        let result = trace.result.flatMap(jsonObject(from:)) as? [String: Any]
        let pathKeys = ["path", "file_path", "target_path"]
        let path = pathKeys.compactMap { arguments?[$0] as? String }.first
            ?? pathKeys.compactMap { result?[$0] as? String }.first
            ?? trace.affectedFiles?.first
        guard let path, !path.isEmpty else { return nil }

        let start = integer(in: arguments, keys: ["start_line", "line_start"])
            ?? integer(in: result, keys: ["start_line", "line_start"])
        var end = integer(in: arguments, keys: ["end_line", "line_end"])
            ?? integer(in: result, keys: ["end_line", "line_end"])
        let count = integer(in: arguments, keys: ["line_count", "limit"])
            ?? integer(in: result, keys: ["line_count"])
        let normalizedStart = start ?? (count == nil ? nil : 1)
        if end == nil, let normalizedStart, let count, count > 0 {
            end = normalizedStart + count - 1
        }
        if (normalizedStart == nil || end == nil),
           let range = lineRange(in: trace.result ?? trace.detail) {
            return MacToolFileOperation(path: path, startLine: range.0, endLine: range.1)
        }
        return MacToolFileOperation(path: path, startLine: normalizedStart, endLine: end)
    }

    private static func integer(in object: [String: Any]?, keys: [String]) -> Int? {
        for key in keys {
            if let value = object?[key] as? Int { return value }
            if let value = object?[key] as? NSNumber { return value.intValue }
            if let value = object?[key] as? String, let number = Int(value) { return number }
        }
        return nil
    }

    private static func lineRange(in value: String) -> (Int, Int)? {
        guard let expression = try? NSRegularExpression(
            pattern: #"(?:lines?|行)\s*[:：]?\s*(\d+)\s*(?:-|–|—|to|到)\s*(\d+)"#,
            options: [.caseInsensitive]
        ) else {
            return nil
        }
        let range = NSRange(value.startIndex..<value.endIndex, in: value)
        guard let match = expression.firstMatch(in: value, range: range),
              let startRange = Range(match.range(at: 1), in: value),
              let endRange = Range(match.range(at: 2), in: value),
              let start = Int(value[startRange]),
              let end = Int(value[endRange]) else {
            return nil
        }
        return (start, end)
    }

    private static func isFileRead(_ trace: MacAssistantTrace) -> Bool {
        let title = trace.title.lowercased()
        return title.contains("read")
            || title.contains("cat")
            || title.contains("读取")
            || title.contains("阅读文件")
    }

    private static func safeFileOperationSummary(_ trace: MacAssistantTrace) -> String? {
        let arguments = trace.arguments.flatMap(jsonObject(from:)) as? [String: Any]
        let start = integer(in: arguments, keys: ["start_line", "line_start"])
        let end = integer(in: arguments, keys: ["end_line", "line_end"])
        if let start, let end {
            return String(
                format: String(localized: "Lines %lld–%lld", table: "Chat"),
                Int64(start),
                Int64(end)
            )
        }
        if let start,
           let count = integer(in: arguments, keys: ["line_count", "limit"]),
           count > 0 {
            return String(
                format: String(localized: "Lines %lld–%lld", table: "Chat"),
                Int64(start),
                Int64(start + count - 1)
            )
        }
        let detail = trace.detail.trimmingCharacters(in: .whitespacesAndNewlines)
        if !detail.isEmpty,
           detail != "{",
           detail != "[",
           jsonObject(from: detail) == nil,
           !looksLikeFileBody(detail) {
            return concise(detail, maximumCharacters: 360, maximumLines: 3)
        }
        return trace.affectedFiles?.first.map { URL(fileURLWithPath: $0).lastPathComponent }
    }

    private static func humanReadablePayload(_ value: String) -> String? {
        let normalizedValue = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedValue.isEmpty else { return nil }
        guard let object = jsonObject(from: normalizedValue) as? [String: Any] else {
            return normalized(normalizedValue)
        }
        for key in ["display", "summary", "text", "error"] {
            if let candidate = object[key] as? String,
               let result = normalized(candidate),
               !result.isEmpty {
                return result
            }
        }
        return nil
    }

    private static func looksLikeFileBody(_ value: String) -> Bool {
        value.contains("\n   1 |")
            || value.contains("\n  1 |")
            || value.contains("\nlines:")
            || value.split(whereSeparator: \.isNewline).count > 8
    }

    private static func payloadByteCount(_ value: Any) -> Int? {
        if let value = value as? String { return value.utf8.count }
        guard JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value) else {
            return nil
        }
        return data.count
    }

    private static func compactValue(_ value: Any) -> String? {
        if let value = value as? String {
            return normalized(value)
        }
        if let value = value as? NSNumber {
            return value.stringValue
        }
        guard JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(
                withJSONObject: value,
                options: [.sortedKeys, .withoutEscapingSlashes]
              ) else {
            return nil
        }
        return normalized(String(decoding: data, as: UTF8.self))
    }

    private static func jsonObject(from value: String) -> Any? {
        guard let data = value.data(using: .utf8) else { return nil }
        return try? JSONSerialization.jsonObject(with: data)
    }

    private static func displayLabel(for key: String) -> String {
        switch key.replacingOccurrences(of: "-", with: "_").lowercased() {
        case "path", "file_path", "target_path", "root", "directory":
            return String(localized: "Path")
        case "offset", "start", "start_line", "line_start":
            return String(localized: "Offset")
        case "limit", "count", "max_results", "end_line", "line_end":
            return String(localized: "Limit")
        case "pattern", "regex":
            return String(localized: "Pattern")
        case "query", "search_query":
            return String(localized: "Query")
        case "command", "cmd":
            return String(localized: "Command")
        case "url", "uri":
            return String(localized: "URL")
        case "timeout", "timeout_ms", "timeout_seconds":
            return String(localized: "Timeout")
        default:
            return key.replacingOccurrences(of: "_", with: " ")
                .replacingOccurrences(of: "-", with: " ")
                .capitalized
        }
    }

    private static func concise(
        _ value: String,
        maximumCharacters: Int,
        maximumLines: Int
    ) -> String? {
        let normalized = value
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return nil }
        let lines = normalized.split(separator: "\n", omittingEmptySubsequences: false)
        var preview = lines.prefix(maximumLines).joined(separator: "\n")
        var wasTruncated = lines.count > maximumLines
        if preview.count > maximumCharacters {
            preview = String(preview.prefix(maximumCharacters))
            wasTruncated = true
        }
        return wasTruncated ? "\(preview)\n…" : preview
    }

    private static func normalized(_ value: String) -> String? {
        let normalized = value
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.isEmpty ? nil : normalized
    }
}

nonisolated struct MacToolFileOperation: Hashable, Sendable {
    let path: String
    let startLine: Int?
    let endLine: Int?
}

nonisolated struct MacSemanticStreamAssembler: Sendable {
    private(set) var rawText: String
    private(set) var mutableTail: MacAssistantTextNode?
    private var pending = ""
    private let identitySeed: String?
    private var nextNodeOrdinal = 0
    private var nextBlockOrdinal = 0
    private var currentBlockID: UUID
    private var mutableTailID: UUID
    private var tailScanner: MacMarkdownTailBoundaryScanner?
    private var cachedFence: (marker: Character, count: Int, language: String, contentStartBytes: Int)?
    private var establishedTable = false
    // Conservative grammar hint: a table requires a pipe. Scan incoming bytes
    // once, not the entire remaining paragraph for every emitted phrase.
    private var pendingMayContainTable = false
    private(set) var boundaryScannedByteCount = 0

    /// Hide incomplete table syntax without fragmenting the native document.
    var unconfirmedTableSuffixCount: Int {
        isPendingTablePrefix() && !hasRecognizableTablePrefix() ? pending.count : 0
    }

    init(existingText: String = "", identitySeed: String? = nil) {
        rawText = ""
        self.identitySeed = identitySeed
        currentBlockID = Self.identity(
            seed: identitySeed,
            component: "block:0"
        )
        mutableTailID = Self.identity(
            seed: identitySeed,
            component: "node:0"
        )
        // Restore parser state and ordinal ownership, not just the raw string.
        // Already committed nodes belong to the existing presentation; only
        // the unfinished tail may be emitted again when more content arrives.
        _ = append(existingText)
    }

    mutating func append(_ delta: String) -> [MacAssistantTextNode] {
        guard !delta.isEmpty else { return [] }
        rawText += delta
        pending += delta
        pendingMayContainTable = pendingMayContainTable || delta.utf8.contains(124)
        scanTail(delta.utf8)
        let nodes = drain(force: false)
        refreshMutableTail()
        return nodes
    }

    mutating func finish() -> [MacAssistantTextNode] {
        let nodes = drain(force: true)
        mutableTail = nil
        return nodes
    }

    private mutating func drain(force: Bool) -> [MacAssistantTextNode] {
        var output: [MacAssistantTextNode] = []
        while !pending.isEmpty {
            prepareTailScanner()
            if pending.hasPrefix("```") {
                guard let node = consumeCodeFence(force: force) else { break }
                output.append(node)
                advanceBlock()
                continue
            }

            if let table = consumeTableIfComplete(force: force) {
                output.append(table)
                advanceBlock()
                continue
            }
            if !force, isPendingTablePrefix() {
                break
            }

            if let markdownBlock = consumeStructuredMarkdownBlock(force: force) {
                output.append(markdownBlock)
                advanceBlock()
                continue
            }
            if !force, beginsStructuredMarkdownBlock() { break }

            if !force, shouldAwaitMarkdownLineCompletion() {
                break
            }

            guard let boundary = phraseBoundary(force: force) else { break }
            let phrase = String(pending[..<boundary])
            removePending(through: boundary)
            if !phrase.isEmpty {
                output.append(makeNode(kind: .phrase, text: phrase))
            }
        }
        return output
    }

    private mutating func consumeCodeFence(force: Bool) -> MacAssistantTextNode? {
        guard let opening = codeFenceOpening() else {
            if force {
                let value = pending
                removePending(through: pending.endIndex)
                return makeNode(kind: .markdownBlock, text: value)
            }
            return nil
        }
        guard let close = tailScanner?.closingBoundary(allowEndOfInput: force).map(stringRange) else {
            guard force else { return nil }
            let body = visibleCodeBody(
                String(pending[opening.contentStart...]),
                marker: opening.marker,
                minimumCount: opening.count
            )
            removePending(through: pending.endIndex)
            return makeNode(kind: .code(language: opening.language), text: body)
        }
        let body = String(pending[opening.contentStart..<close.lowerBound])
            .trimmingCharacters(in: .newlines)
        removePending(through: close.upperBound)
        return makeNode(
            kind: opening.language.lowercased() == "mermaid"
                ? .mermaid
                : .code(language: opening.language),
            text: body
        )
    }

    private mutating func consumeTableIfComplete(force: Bool) -> MacAssistantTextNode? {
        guard pendingMayContainTable else { return nil }
        if !establishedTable {
            let lines = pending.split(separator: "\n", maxSplits: 2, omittingEmptySubsequences: false)
            guard lines.count >= 2,
                  MacMarkdownTableSyntax.hasHeader(String(lines[0]), divider: String(lines[1])) else { return nil }
        }
        let paragraphEnd = tailScanner?.boundary.map(stringRange) ?? (establishedTable ? nil : pending.range(of: "\n\n"))
        guard let paragraphEnd else {
            guard force else { return nil }
            let value = pending
            removePending(through: pending.endIndex)
            return makeNode(kind: .markdownBlock, text: value)
        }
        let value = String(pending[..<paragraphEnd.lowerBound])
        removePending(through: paragraphEnd.upperBound)
        return makeNode(kind: .markdownBlock, text: value)
    }

    private mutating func scanTail<S: Sequence>(_ bytes: S) where S.Element == UInt8 {
        guard var scanner = tailScanner else { return }
        let before = scanner.scannedByteCount
        scanner.append(bytes)
        boundaryScannedByteCount += scanner.scannedByteCount - before
        tailScanner = scanner
    }

    private mutating func prepareTailScanner() {
        guard tailScanner == nil else { return }
        if let opening = codeFenceOpening() {
            let offset = pending.utf8.distance(from: pending.utf8.startIndex, to: opening.contentStart)
            cachedFence = (opening.marker, opening.count, opening.language, offset)
            tailScanner = MacMarkdownTailBoundaryScanner(kind: .fence(
                marker: opening.marker == "`" ? 96 : 126, minimumCount: opening.count), startingAt: offset)
            scanTail(pending.utf8.dropFirst(offset))
        } else if pendingMayContainTable {
            let lines = pending.split(separator: "\n", maxSplits: 2, omittingEmptySubsequences: false)
            if lines.count > 2, MacMarkdownTableSyntax.hasHeader(String(lines[0]), divider: String(lines[1])) {
                establishedTable = true
                tailScanner = MacMarkdownTailBoundaryScanner(kind: .table)
                scanTail(pending.utf8)
            }
        }
    }

    private func stringRange(_ bytes: Range<Int>) -> Range<String.Index> {
        let lower = pending.utf8.index(pending.utf8.startIndex, offsetBy: bytes.lowerBound)
        let upper = pending.utf8.index(lower, offsetBy: bytes.count)
        return lower..<upper
    }

    private mutating func removePending(through end: String.Index) {
        pending.removeSubrange(pending.startIndex..<end)
        if pending.isEmpty { pendingMayContainTable = false }
        tailScanner = nil
        cachedFence = nil
        establishedTable = false
    }

    private mutating func makeNode(
        kind: MacAssistantTextNode.Kind,
        text: String
    ) -> MacAssistantTextNode {
        let node = MacAssistantTextNode(
            id: mutableTailID,
            blockID: currentBlockID,
            kind: kind,
            text: text
        )
        nextNodeOrdinal += 1
        mutableTailID = Self.identity(
            seed: identitySeed,
            component: "node:\(nextNodeOrdinal)"
        )
        return node
    }

    private mutating func advanceBlock() {
        nextBlockOrdinal += 1
        currentBlockID = Self.identity(
            seed: identitySeed,
            component: "block:\(nextBlockOrdinal)"
        )
    }

    private static func identity(seed: String?, component: String) -> UUID {
        guard let seed,
              let stable = MacCanonicalMessageIdentity.stableID("\(seed):\(component)") else {
            return UUID()
        }
        return stable
    }

    private mutating func refreshMutableTail() {
        if isPartialCodeFenceHeader() {
            mutableTail = nil
            return
        }
        if let opening = codeFenceOpening() {
            let body = visibleCodeBody(
                String(pending[opening.contentStart...]),
                marker: opening.marker,
                minimumCount: opening.count
            )
            mutableTail = MacAssistantTextNode(
                id: mutableTailID,
                blockID: currentBlockID,
                kind: opening.language.lowercased() == "mermaid"
                    ? .mermaid
                    : .code(language: opening.language),
                text: body
            )
            return
        }

        let visibleText = pending.trimmingCharacters(in: .newlines)
        guard !visibleText.isEmpty else {
            mutableTail = nil
            return
        }
        if isPendingTablePrefix() {
            // A header row is not a table until its divider is complete.
            // Exposing the mutable prefix sooner makes `| Header |` and the
            // growing `| ---` control syntax flash as ordinary prose. Keep
            // that transport tail hidden, then reveal the native table as
            // soon as the divider establishes the block grammar.
            guard hasRecognizableTablePrefix() else {
                mutableTail = nil
                return
            }
            mutableTail = MacAssistantTextNode(
                id: mutableTailID,
                blockID: currentBlockID,
                kind: .markdownBlock,
                text: visibleText
            )
            return
        }
        if beginsStructuredMarkdownBlock() {
            mutableTail = MacAssistantTextNode(
                id: mutableTailID,
                blockID: currentBlockID,
                kind: .markdownBlock,
                text: visibleText
            )
            return
        }
        mutableTail = MacAssistantTextNode(
            id: mutableTailID,
            blockID: currentBlockID,
            kind: .phrase,
            text: visibleText
        )
    }

    private struct CodeFenceOpening {
        let marker: Character
        let count: Int
        let language: String
        let contentStart: String.Index
    }

    private func codeFenceOpening() -> CodeFenceOpening? {
        if let cachedFence {
            return CodeFenceOpening(marker: cachedFence.marker, count: cachedFence.count,
                language: cachedFence.language, contentStart: pending.utf8.index(
                    pending.utf8.startIndex, offsetBy: cachedFence.contentStartBytes))
        }
        let prefix = pending.prefix(4).drop { $0 == " " }
        guard prefix.first == "`" || prefix.first == "~" else { return nil }
        guard let lineEnd = pending.utf8.firstIndex(of: 10) else { return nil }
        let line = pending[..<lineEnd]
        let indentation = line.prefix { $0 == " " }
        guard indentation.count <= 3 else { return nil }
        let content = line.dropFirst(indentation.count)
        guard let marker = content.first, marker == "`" || marker == "~" else { return nil }
        let run = content.prefix { $0 == marker }
        guard run.count >= 3 else { return nil }
        let language = content.dropFirst(run.count)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return CodeFenceOpening(
            marker: marker,
            count: run.count,
            language: language,
            contentStart: pending.utf8.index(after: lineEnd)
        )
    }

    private func isPartialCodeFenceHeader() -> Bool {
        if cachedFence != nil || establishedTable { return false }
        guard !pending.contains("\n") else { return false }
        let line = pending.drop { $0 == " " }
        guard pending.distance(from: pending.startIndex, to: line.startIndex) <= 3,
              let marker = line.first,
              marker == "`" || marker == "~" else {
            return false
        }
        let run = line.prefix { $0 == marker }
        return run.count >= 3 || run.count == line.count
    }

    private func visibleCodeBody(
        _ value: String,
        marker: Character,
        minimumCount: Int
    ) -> String {
        if let cachedFence, let tailScanner {
            guard let partialStart = tailScanner.partialFenceLineStart else { return value }
            return String(pending[stringRange(cachedFence.contentStartBytes..<partialStart)])
        }
        let lastLineStart: String.Index
        if let newline = value.lastIndex(of: "\n") {
            lastLineStart = value.index(after: newline)
        } else {
            lastLineStart = value.startIndex
        }
        let lastLine = value[lastLineStart...]
        let trimmed = lastLine.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty,
              trimmed.count < minimumCount,
              trimmed.allSatisfy({ $0 == marker }) else {
            return value
        }
        return String(value[..<lastLineStart])
    }

    private func isPendingTablePrefix() -> Bool {
        guard pendingMayContainTable else { return false }
        if establishedTable { return true }
        let lines = pending.split(separator: "\n", maxSplits: 2, omittingEmptySubsequences: false).lazy.map(String.init)
        guard let header = lines.first, looksLikeTableRow(header) else { return false }
        guard lines.count > 1 else { return true }
        if lines.count > 2 {
            // Once the divider line ends, a width mismatch is ordinary prose,
            // not a partial table that may hide all subsequent streamed text.
            return MacMarkdownTableSyntax.hasHeader(header, divider: lines[1])
        }
        let divider = lines[1].trimmingCharacters(in: .whitespaces)
        if divider.isEmpty { return true }
        return isPotentialTableDivider(divider) || MacMarkdownTableSyntax.isDivider(divider)
    }

    private func hasRecognizableTablePrefix() -> Bool {
        guard pendingMayContainTable else { return false }
        if establishedTable { return true }
        let lines = pending.split(separator: "\n", maxSplits: 2, omittingEmptySubsequences: false).lazy.map(String.init)
        // A syntactically valid prefix can still gain another column. Wait for
        // the delimiter line's newline before publishing a live table.
        return lines.count > 2 && MacMarkdownTableSyntax.hasHeader(lines[0], divider: lines[1])
    }

    private func looksLikeTableRow(_ line: String) -> Bool {
        MacMarkdownTableSyntax.isRow(line)
    }

    private func isPotentialTableDivider(_ line: String) -> Bool {
        guard !line.isEmpty else { return true }
        return line.allSatisfy { character in
            character == "|" || character == ":" || character == "-" || character.isWhitespace
        }
    }

    private func phraseBoundary(force: Bool) -> String.Index? {
        var count = 0
        var softBoundary: String.Index?
        for index in pending.indices {
            count += 1
            let character = pending[index]
            let end = pending.index(after: index)
            let candidate = String(pending[..<end])
            if character == "\n", inlineMarkdownIsBalanced(candidate) {
                return end
            }
            if "。！？；：.!?;:".contains(character), count >= 20, inlineMarkdownIsBalanced(candidate) {
                return end
            }
            if "，、,".contains(character), count >= 30, inlineMarkdownIsBalanced(candidate) {
                return end
            }
            if character.isWhitespace, count >= 36, inlineMarkdownIsBalanced(candidate) {
                softBoundary = end
            }
            if count >= 52, let softBoundary { return softBoundary }
            if count >= 72, inlineMarkdownIsBalanced(candidate) { return end }
        }
        if force { return pending.endIndex }
        return nil
    }

    private func inlineMarkdownIsBalanced(_ value: String) -> Bool {
        let unescaped = removingEscapedMarkdownCharacters(in: value)
        guard delimiterCount("`", in: unescaped).isMultiple(of: 2),
              delimiterCount("**", in: unescaped).isMultiple(of: 2),
              delimiterCount("__", in: unescaped).isMultiple(of: 2),
              delimiterCount("~~", in: unescaped).isMultiple(of: 2),
              singleAsteriskCount(in: unescaped).isMultiple(of: 2),
              unescaped.filter({ $0 == "[" }).count == unescaped.filter({ $0 == "]" }).count else {
            return false
        }

        // Parentheses only affect streaming safety when they form a Markdown
        // link destination. Ordinary prose and function calls must not hold an
        // otherwise complete semantic phrase until the end of the response.
        var searchStart = unescaped.startIndex
        while let destination = unescaped.range(of: "](", range: searchStart..<unescaped.endIndex) {
            var depth = 1
            var index = destination.upperBound
            while index < unescaped.endIndex, depth > 0 {
                switch unescaped[index] {
                case "(": depth += 1
                case ")": depth -= 1
                default: break
                }
                index = unescaped.index(after: index)
            }
            guard depth == 0 else { return false }
            searchStart = index
        }
        return true
    }

    private func removingEscapedMarkdownCharacters(in value: String) -> String {
        var result = ""
        var escaping = false
        for character in value {
            if escaping {
                escaping = false
                continue
            }
            if character == "\\" {
                escaping = true
            } else {
                result.append(character)
            }
        }
        return result
    }

    private func delimiterCount(_ delimiter: String, in value: String) -> Int {
        var count = 0
        var searchStart = value.startIndex
        while let range = value.range(of: delimiter, range: searchStart..<value.endIndex) {
            count += 1
            searchStart = range.upperBound
        }
        return count
    }

    private func singleAsteriskCount(in value: String) -> Int {
        var count = 0
        var index = value.startIndex
        while index < value.endIndex {
            guard value[index] == "*" else {
                index = value.index(after: index)
                continue
            }
            let next = value.index(after: index)
            if next < value.endIndex, value[next] == "*" {
                index = value.index(after: next)
            } else {
                count += 1
                index = next
            }
        }
        return count
    }

    private enum StructuredMarkdownLineKind: Equatable {
        case heading
        case quote
        case list
        case thematicBreak
    }

    private mutating func consumeStructuredMarkdownBlock(
        force: Bool
    ) -> MacAssistantTextNode? {
        guard mayBeginStructuredMarkdown else { return nil }
        let firstLineEnd = pending.firstIndex(of: "\n")
        let firstLine = firstLineEnd.map { String(pending[..<$0]) } ?? pending
        guard let kind = structuredMarkdownLineKind(firstLine) else { return nil }

        switch kind {
        case .heading, .thematicBreak:
            guard let firstLineEnd else {
                guard force else { return nil }
                let value = pending
                removePending(through: pending.endIndex)
                return makeNode(kind: .markdownBlock, text: value)
            }
            let consumedEnd = pending.index(after: firstLineEnd)
            removePending(through: consumedEnd)
            return makeNode(kind: .markdownBlock, text: firstLine)
        case .quote, .list:
            if let paragraphEnd = containerBoundary(kind: kind, force: force) {
                let value = String(pending[..<paragraphEnd.lowerBound])
                removePending(through: paragraphEnd.upperBound)
                return makeNode(kind: .markdownBlock, text: value)
            }
            guard force else { return nil }
            let value = pending.trimmingCharacters(in: .newlines)
            removePending(through: pending.endIndex)
            return makeNode(kind: .markdownBlock, text: value)
        }
    }

    /// Blank lines separate paragraphs, not necessarily block containers.
    /// Keep a list item's continuation/code or a quoted paragraph with its
    /// parent until a complete next line establishes an outside block.
    private func containerBoundary(kind: StructuredMarkdownLineKind, force: Bool) -> Range<String.Index>? {
        var cursor = pending.startIndex
        while let boundary = pending.range(of: "\n\n", range: cursor..<pending.endIndex) {
            var next = boundary.upperBound
            while next < pending.endIndex, pending[next] == "\n" { next = pending.index(after: next) }
            guard next < pending.endIndex else { return force ? boundary : nil }
            let lineEnd = pending[next...].firstIndex(of: "\n")
            guard lineEnd != nil || force else { return nil }
            let line = String(pending[next..<(lineEnd ?? pending.endIndex)])
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            let continues: Bool
            switch kind {
            case .quote: continues = trimmed.hasPrefix(">")
            case .list:
                continues = line.first == " " || line.first == "\t"
                    || structuredMarkdownLineKind(line) == .list
            default: continues = false
            }
            if !continues { return boundary }
            cursor = boundary.upperBound
        }
        return nil
    }

    private func beginsStructuredMarkdownBlock() -> Bool {
        guard mayBeginStructuredMarkdown else { return false }
        let firstLine = pending.firstIndex(of: "\n")
            .map { String(pending[..<$0]) } ?? pending
        return structuredMarkdownLineKind(firstLine) != nil
    }

    private func structuredMarkdownLineKind(
        _ line: String
    ) -> StructuredMarkdownLineKind? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        let headingPrefix = trimmed.prefix { $0 == "#" }
        if (1...6).contains(headingPrefix.count),
           trimmed.dropFirst(headingPrefix.count).first == " " {
            return .heading
        }
        if trimmed.hasPrefix("> ") {
            return .quote
        }
        if trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") || trimmed.hasPrefix("+ ") {
            return .list
        }
        if let first = trimmed.first, first.isNumber,
           trimmed.range(of: #"^\d+[.)]\s"#, options: .regularExpression) != nil {
            return .list
        }
        let compact = trimmed.filter { !$0.isWhitespace }
        if compact.count >= 3,
           compact.allSatisfy({ $0 == "-" || $0 == "*" || $0 == "_" }) {
            return .thematicBreak
        }
        return nil
    }

    private func shouldAwaitMarkdownLineCompletion() -> Bool {
        guard mayBeginStructuredMarkdown || pending.drop(while: { $0.isWhitespace && $0 != "\n" }).first == "|" else { return false }
        guard !pending.contains("\n") else { return false }
        let trimmed = pending.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return false }

        if trimmed.hasPrefix("|") || trimmed.hasPrefix(">") {
            return true
        }

        let headingPrefix = trimmed.prefix { $0 == "#" }
        if !headingPrefix.isEmpty, headingPrefix.count <= 6 {
            return trimmed.count == headingPrefix.count
                || trimmed.dropFirst(headingPrefix.count).first == " "
        }

        if let marker = trimmed.first, "-*+".contains(marker) {
            if trimmed.count == 1 { return true }
            let next = trimmed[trimmed.index(after: trimmed.startIndex)]
            if next.isWhitespace { return true }
            if trimmed.allSatisfy({ $0 == marker }) { return true }
        }

        return trimmed.range(of: #"^\d+[.)](?:\s|$)"#, options: .regularExpression) != nil
    }

    private var mayBeginStructuredMarkdown: Bool {
        guard let first = pending.drop(while: { $0.isWhitespace && $0 != "\n" }).first else { return false }
        return "#>-*+_".contains(first) || first.isNumber
    }

}

nonisolated struct MacThinkingStreamAccumulator {
    struct Update {
        let response: String
        let responseDelta: String
        let thinking: String
        let resetsResponse: Bool
    }

    private var rawText = ""
    private var response = ""

    mutating func append(_ delta: String) -> Update {
        guard !delta.isEmpty else {
            return parse(stableText: textDroppingPartialControlTagSuffix(rawText))
        }
        if delta.hasPrefix(rawText) {
            // Some hosts send cumulative snapshots rather than token deltas.
            rawText = delta
        } else if delta.count >= 64, rawText.hasSuffix(delta) {
            // Terminal snapshots can repeat the already streamed response.
            // Ignore them instead of presenting the complete answer twice.
        } else {
            rawText += delta
        }
        return parse(stableText: textDroppingPartialControlTagSuffix(rawText))
    }

    mutating func finish() -> Update {
        parse(stableText: rawText)
    }

    private mutating func parse(stableText: String) -> Update {
        let parsed = ChatThinkingParser.splitThinkingBlocks(from: stableText)
        guard parsed.response.hasPrefix(response) else {
            // Rendering is append-only once a semantic node is visible. A
            // transient cumulative snapshot may shrink or rewrite its suffix,
            // but it must not tear down the committed Markdown tree.
            return Update(
                response: response,
                responseDelta: "",
                thinking: parsed.thinking,
                resetsResponse: false
            )
        }
        let delta = String(parsed.response.dropFirst(response.count))
        response = parsed.response
        return Update(
            response: parsed.response,
            responseDelta: delta,
            thinking: parsed.thinking,
            resetsResponse: false
        )
    }

    private func textDroppingPartialControlTagSuffix(_ value: String) -> String {
        let markers = ["<think>", "</think>"]
        var longest = 0
        for marker in markers {
            for length in 1..<marker.count where value.hasSuffix(String(marker.prefix(length))) {
                longest = max(longest, length)
            }
        }
        guard longest > 0 else { return value }
        return String(value.dropLast(longest))
    }
}

nonisolated enum MacPacedStreamSegmenter {
    fileprivate static let semanticBoundaries: Set<Character> = [
        "。", "！", "？", "；", "：", "，",
        ".", "!", "?", ";", ":", ",", "\n"
    ]

    static func segments(
        _ text: String,
        minimumLength: Int = 28,
        maximumLength: Int = 72
    ) -> [String] {
        var buffer = MacPacedStreamBuffer(
            minimumLength: minimumLength,
            maximumLength: maximumLength
        )
        return buffer.append(text) + buffer.finish()
    }
}

/// Buffers the many tiny deltas produced by SSE until a readable phrase is ready.
/// Without cross-event state, SwiftUI coalesces token-sized mutations into a large
/// visual jump even though the transport itself is streaming correctly.
nonisolated struct MacPacedStreamBuffer {
    private let minimumLength: Int
    private let maximumLength: Int
    private var pending = ""

    /// Text accepted from the transport but not yet revealed by the paced UI.
    /// Authoritative cumulative snapshots must include this suffix when
    /// calculating their next delta or the suffix will be enqueued twice.
    var bufferedText: String { pending }

    init(minimumLength: Int = 8, maximumLength: Int = 24) {
        self.minimumLength = max(1, minimumLength)
        self.maximumLength = max(self.minimumLength, maximumLength)
    }

    mutating func append(_ delta: String) -> [String] {
        guard !delta.isEmpty else { return [] }
        pending += delta
        return drain(force: false)
    }

    mutating func finish() -> [String] {
        drain(force: true)
    }

    private mutating func drain(force: Bool) -> [String] {
        var result: [String] = []
        var buffer = ""
        var bufferedCount = 0

        // Traverse once without repeatedly mutating the source string or
        // recounting extended grapheme clusters in the current phrase.
        for character in pending {
            buffer.append(character)
            bufferedCount += 1
            if character == "\n"
                || bufferedCount >= maximumLength
                || (character.isWhitespace && bufferedCount >= minimumLength)
                || (bufferedCount >= minimumLength && MacPacedStreamSegmenter.semanticBoundaries.contains(character)) {
                result.append(buffer)
                buffer.removeAll(keepingCapacity: true)
                bufferedCount = 0
            }
        }

        if force && !buffer.isEmpty {
            result.append(buffer)
        }
        pending = force ? "" : buffer
        return result
    }
}

nonisolated enum MacThinkingActivityFormatter {
    static func currentStep(from value: String, maximumLength: Int = 220) -> String {
        let sanitized = value
            .replacingOccurrences(of: "<think>", with: "")
            .replacingOccurrences(of: "</think>", with: "")
        func normalize(_ value: Substring) -> String {
            value.components(separatedBy: .whitespacesAndNewlines)
                .filter { !$0.isEmpty }.joined(separator: " ")
        }
        // Only the latest nonempty segment is displayed. Walking backwards
        // avoids splitting and normalizing every sentence of a long trace on
        // every SwiftUI evaluation, while retaining the punctuation-only fallback.
        var end = sanitized.endIndex
        var normalized = ""
        for index in sanitized.indices.reversed() {
            switch sanitized[index] {
            case "\n", "\r", "。", "！", "？", "!", "?", "；", ";":
                normalized = normalize(sanitized[sanitized.index(after: index)..<end])
                if !normalized.isEmpty { break }
                end = index
            default: continue
            }
            if !normalized.isEmpty { break }
        }
        if normalized.isEmpty { normalized = normalize(sanitized[..<end]) }
        if normalized.isEmpty { normalized = normalize(sanitized[...]) }
        guard !normalized.isEmpty else { return String(localized: "Working…") }
        guard normalized.count > maximumLength else { return normalized }
        return "…" + normalized.suffix(maximumLength)
    }
}
