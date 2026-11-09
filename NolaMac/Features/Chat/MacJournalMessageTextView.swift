// SPDX-License-Identifier: Apache-2.0
// Message text reads committed Part references. Large bodies stay paged; tool
// and permission controls continue to use their separate canonical models.
import SwiftUI
import AppKit
import Observation

/// Tool diagnostics belong in a bounded transcript preview. The explicit
/// reader retains the full body; no nested vertical scroller steals gestures.
nonisolated enum MacJournalPlainPreview {
    static func text(_ value: String) -> String {
        String(value.prefix(4096)).split(separator: "\n", omittingEmptySubsequences: false)
            .prefix(24).joined(separator: "\n")
    }
}

/// The transcript owns this bounded residency, not a row's temporary SwiftUI parent.
@MainActor
final class MacJournalPresentationResidency {
    private final class WeakOwner {
        weak var value: MacJournalPresentationOwner?
        init(_ value: MacJournalPresentationOwner) { self.value = value }
    }
    private var entries: [MacJournalOwnerKey: MacJournalPresentationOwner] = [:]
    private var mounted: [MacJournalOwnerKey: WeakOwner] = [:]
    private var order: [MacJournalOwnerKey] = []
    let capacity: Int
    init(capacity: Int = 16) { self.capacity = max(1, capacity) }
    var count: Int { entries.count }

    func owner(for reference: MacJournalMessageReference) -> MacJournalPresentationOwner {
        let key = reference.ownerKey
        order.removeAll { $0 == key }
        order.append(key)
        let owner = entries[key] ?? mounted[key]?.value ?? MacJournalPresentationOwner()
        entries[key] = owner
        mounted[key] = WeakOwner(owner)
        while order.count > capacity {
            let evicted = order.removeFirst()
            // A mounted row retains its owner. LRU only bounds offscreen reuse;
            // evicting a cache entry must not cancel visible UI's in-flight read.
            entries.removeValue(forKey: evicted)
        }
        mounted = mounted.filter { $0.value.value != nil }
        return owner
    }

    func retain(_ keys: Set<MacJournalOwnerKey>) {
        for (key, owner) in mounted where !keys.contains(key) { owner.value?.cancel() }
        entries = entries.filter { keys.contains($0.key) }
        mounted = mounted.filter { keys.contains($0.key) && $0.value.value != nil }
        order.removeAll { !keys.contains($0) }
    }

    func removeAll() {
        mounted.values.forEach { $0.value?.cancel() }
        entries.removeAll()
        mounted.removeAll()
        order.removeAll()
    }
}

struct MacJournalReaderSelection: Identifiable {
    let id = UUID()
    let reference: MacJournalMessageReference
    let partID: String
    let content: MacAssistantJournalContentReference
    let presentation: MacJournalBodyPresentation
}

struct MacHistoricalReaderSelection: Identifiable {
    let id = UUID()
    let selections: [MacHistoricalPartSelection]
    let reader: MacHistoricalPartDetailModel.Reader?
}

enum MacMessageReaderSelection: Identifiable {
    case body(MacJournalReaderSelection)
    case historical(MacHistoricalReaderSelection)
    var id: UUID { switch self { case .body(let value): value.id; case .historical(let value): value.id } }
}

/// Modal residency belongs to the message, never a regroupable Part child.
@MainActor
struct MacMessageReaderPresenter: ViewModifier {
    let owner: MacJournalPresentationOwner
    func body(content: Content) -> some View {
        let epoch = owner.readerEpoch
        let presentedID = owner.readerSelection?.id
        return content
            .sheet(item: Binding(get: { owner.readerEpoch == epoch ? owner.readerSelection : nil },
                set: { owner.updateReader($0, replacing: presentedID, epoch: epoch) })) { selection in
                switch selection {
                case .body(let value):
                    MacJournalTextReader(reference: value.reference, partID: value.partID,
                        content: value.content, presentation: value.presentation)
                case .historical(let value):
                    MacHistoricalPartDetailsView(selections: value.selections, reader: value.reader)
                }
            }
            .onDisappear { owner.invalidateReaders(epoch: epoch) }
    }
}

@MainActor @Observable
final class MacJournalPresentationOwner {
    struct Projection {
        let reference: MacJournalMessageReference
        let parts: [MacAssistantJournalPart]
    }
    typealias PartsReader = @MainActor (MacJournalMessageReference, MacJournalMessageReference?, [MacAssistantJournalPart]) async throws -> [MacAssistantJournalPart]
    private final class WeakBody {
        weak var value: MacJournalBodyState?
        init(_ value: MacJournalBodyState) { self.value = value }
    }
    // Content references are capabilities bound to one exact journal cursor.
    // Publish the pair atomically, never a new Part with the previous cursor.
    private(set) var projection: Projection?
    func projection(matching reference: MacJournalMessageReference) -> Projection? {
        guard let projection, projection.reference.ownerKey == reference.ownerKey else { return nil }
        return projection
    }
    var parts: [MacAssistantJournalPart] { projection?.parts ?? [] }
    var loadedReference: MacJournalMessageReference? { projection?.reference }
    private(set) var error: String?
    var expansions: [String: MacJournalActivityExpansion] = [:]
    var toolTechnicalExpansions: [String: Bool] = [:]
    var completedToolExpansions: [String: Bool] = [:]
    var processChoices: [String: Bool] = [:]
    private(set) var readerEpoch = UUID()
    private(set) var readerSelection: MacMessageReaderSelection?
    func openReader(_ selection: MacMessageReaderSelection, epoch: UUID) {
        guard epoch == readerEpoch else { return }
        readerSelection = selection
    }
    func updateReader(_ selection: MacMessageReaderSelection?, replacing expectedID: UUID?, epoch: UUID) {
        guard epoch == readerEpoch, readerSelection?.id == expectedID else { return }
        readerSelection = selection
    }
    func invalidateReaders(epoch expectedEpoch: UUID? = nil) {
        guard expectedEpoch == nil || expectedEpoch == readerEpoch else { return }
        readerEpoch = UUID(); readerSelection = nil
    }

    var bodyReader: MacJournalReaderSelection? {
        get { if case .body(let value)? = readerSelection { value } else { nil } }
    }
    var historicalReader: MacHistoricalReaderSelection? {
        get { if case .historical(let value)? = readerSelection { value } else { nil } }
    }
    func retainProcessExpansion(partIDs: [String], in segmentPartIDs: [String]) {
        guard !segmentPartIDs.isEmpty, partIDs.allSatisfy({ segmentPartIDs.contains($0) }),
              !segmentPartIDs.contains(where: { processChoices[$0] == false }) else { return }
        for partID in partIDs { processChoices[partID] = true }
    }
    var activityCoverage = MacJournalActivityCoverage()
    var referenceCoverage = MacJournalReferenceCoverage()
    @ObservationIgnored private var desired: MacJournalMessageReference?
    @ObservationIgnored private var worker: Task<Void, Never>?
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var bodies: [String: MacJournalBodyState] = [:]
    @ObservationIgnored private var mountedBodies: [String: WeakBody] = [:]
    @ObservationIgnored private var bodyOrder: [String] = []
    @ObservationIgnored private let readParts: PartsReader

    init(readParts: @escaping PartsReader = { reference, base, parts in
        if let base, base.ownerKey == reference.ownerKey {
            return try await MacJournalTextRead.updatedParts(reference, from: base, existing: parts)
        }
        return try await MacJournalTextRead.parts(reference)
    }) { self.readParts = readParts }

    func request(_ reference: MacJournalMessageReference, retry: Bool = false) {
        if let existing = desired ?? loadedReference, existing.ownerKey != reference.ownerKey {
            cancel()
            projection = nil
            expansions = [:]
            toolTechnicalExpansions = [:]
            completedToolExpansions = [:]
            processChoices = [:]
            invalidateReaders()
            activityCoverage = MacJournalActivityCoverage()
            referenceCoverage = MacJournalReferenceCoverage()
            bodies = [:]
            mountedBodies = [:]
            bodyOrder = []
            error = nil
        }
        if let existing = desired ?? loadedReference, existing.ownerKey == reference.ownerKey,
           let old = UInt64(existing.cursor.sequence), let new = UInt64(reference.cursor.sequence), old > new { return }
        if desired != reference || retry { error = nil }
        desired = reference
        guard worker == nil, error == nil, loadedReference != reference || retry else { return }
        let ticket = generation
        worker = Task { [weak self] in
            guard let self else { return }
            defer { if self.generation == ticket { self.worker = nil } }
            while !Task.isCancelled, self.generation == ticket, let next = self.desired {
                do {
                    let values = try await self.readParts(next, self.loadedReference, self.parts)
                    try Task.checkCancellation()
                    guard self.generation == ticket else { return }
                    self.projection = Projection(reference: next, parts: values)
                    var activityCoverage = self.activityCoverage
                    activityCoverage.adopt(identity: next.cursor.identity,
                        partIDs: Set(values.compactMap { part in
                            switch part.metadata {
                            case .reasoning, .step, .tool, .error, .skill, .modelAttempt, .task: part.part_id
                            default: nil
                            }
                        }), taskIDs: Set(values.compactMap { part in
                            if case .task(let task) = part.metadata { return task.task_id }; return nil
                        }))
                    if activityCoverage != self.activityCoverage { self.activityCoverage = activityCoverage }
                    var referenceCoverage = self.referenceCoverage
                    referenceCoverage.adopt(identity: next.cursor.identity,
                        sourceIDs: Set(values.compactMap { part in
                            if case .source(let source) = part.metadata { return source.source_id }; return nil
                        }), citationIDs: Set(values.compactMap { part in
                            if case .citation(let citation) = part.metadata { return citation.citation_id }; return nil
                        }), attachmentIDs: Set(values.compactMap { part in
                            if case .attachment(let attachment) = part.metadata { return attachment.attachment_id }; return nil
                        }))
                    if referenceCoverage != self.referenceCoverage { self.referenceCoverage = referenceCoverage }
                    self.error = nil
                    if self.desired == next { return }
                } catch {
                    if !Task.isCancelled, self.generation == ticket, self.desired != next { continue }
                    if !Task.isCancelled, self.generation == ticket { self.error = error.localizedDescription }
                    return
                }
            }
        }
    }

    func body(partID: String, field: String) -> MacJournalBodyState {
        let key = "\(partID.utf8.count):\(partID)\(field)"
        bodyOrder.removeAll { $0 == key }
        bodyOrder.append(key)
        let state = bodies[key] ?? mountedBodies[key]?.value ?? MacJournalBodyState()
        bodies[key] = state
        mountedBodies[key] = WeakBody(state)
        // At most eight offscreen prefixes; mounted previews retain their own
        // state and are never cancelled by another visible field being read.
        while bodyOrder.count > 8 { bodies.removeValue(forKey: bodyOrder.removeFirst()) }
        mountedBodies = mountedBodies.filter { $0.value.value != nil }
        return state
    }

    func completedToolsExpanded(partIDs: [String]) -> Bool {
        guard let first = partIDs.first else { return false }
        if let choice = completedToolExpansions[first] { return choice }
        // Forming a compact group must not hide a tool the user already opened.
        // Without a user choice the existing compact default remains unchanged.
        return partIDs.contains {
            let choice = expansions[$0]?.userChoice
            return choice == true || choice == nil && toolTechnicalExpansions[$0] == true
        }
    }

    func cancel() {
        generation &+= 1
        worker?.cancel()
        worker = nil
        mountedBodies.values.forEach { $0.value?.cancel() }
    }
}

private struct MacJournalPresentationOwnerKey: EnvironmentKey {
    static let defaultValue: MacJournalPresentationOwner? = nil
}
private struct MacJournalProcessPartIDsKey: EnvironmentKey {
    static let defaultValue: [String] = []
}
extension EnvironmentValues {
    var journalProcessPartIDs: [String] {
        get { self[MacJournalProcessPartIDsKey.self] }
        set { self[MacJournalProcessPartIDsKey.self] = newValue }
    }
    var journalPresentationOwner: MacJournalPresentationOwner? {
        get { self[MacJournalPresentationOwnerKey.self] }
        set { self[MacJournalPresentationOwnerKey.self] = newValue }
    }
}

@MainActor @Observable
final class MacJournalBodyState {
    struct Segment: Equatable {
        let partID: String
        let content: MacAssistantJournalContentReference
        var hasReadableContent: Bool { content.utf8_bytes != "0" }
    }
    struct Request: Equatable {
        let reference: MacJournalMessageReference
        let partID: String
        let content: MacAssistantJournalContentReference
        let isFinal: Bool
        let presentation: MacJournalBodyPresentation
        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.reference == rhs.reference && lhs.partID == rhs.partID
                && lhs.content == rhs.content && lhs.isFinal == rhs.isFinal && lhs.presentation == rhs.presentation
        }
        func hasSameBody(as other: Self) -> Bool {
            reference.ownerKey == other.reference.ownerKey && partID == other.partID
                && content == other.content && isFinal == other.isFinal && presentation == other.presentation
        }
    }
    private struct Snapshot {
        var text = ""
        var markdownProjection: MacJournalMarkdownProjection?
        var artifacts: [MacCorePartArtifact]?
        var loadState: MacJournalPreviewLoadState = .loading
    }
    private var snapshot = Snapshot()
    var text: String { snapshot.text }
    var markdownProjection: MacJournalMarkdownProjection? { snapshot.markdownProjection }
    var artifacts: [MacCorePartArtifact]? { snapshot.artifacts }
    var loadState: MacJournalPreviewLoadState { snapshot.loadState }
    var error: String?
    var revealedNodeIDs: Set<UUID> = []
    @ObservationIgnored private var desired: Request?
    @ObservationIgnored private var loaded: Request?
    @ObservationIgnored private var worker: Task<Void, Never>?
    @ObservationIgnored private var generation = 0

    func request(_ request: Request, retry: Bool = false) {
        if let existing = desired ?? loaded, existing.reference.ownerKey != request.reference.ownerKey {
            cancel()
            snapshot = Snapshot()
            revealedNodeIDs = []
            loaded = nil
            error = nil
        }
        // A failure belongs to its read request, not to this field forever.
        // In particular a newer cursor must be able to repair a stale read.
        if desired != request || retry { error = nil }
        desired = request
        guard worker == nil, error == nil, loaded?.hasSameBody(as: request) != true || retry else { return }
        let ticket = generation
        worker = Task { [weak self] in
            guard let self else { return }
            defer { if self.generation == ticket { self.worker = nil } }
            while !Task.isCancelled, self.generation == ticket, let next = self.desired {
                do {
                    var prefix = "", offset = "0", segmentIndex = 0
                    // Empty journal fields are real metadata, not unread text.
                    // They must not consume a range-read slot or leave a fully
                    // displayed empty Part labeled as a truncated preview.
                    let segments = [Segment(partID: next.partID, content: next.content)].filter(\.hasReadableContent)
                    var finished = segments.isEmpty
                    for _ in 0..<2 {
                        if finished { break }
                        let segment = segments[segmentIndex]
                        let range = try await MacJournalTextRead.range(next.reference, partID: segment.partID,
                            content: segment.content, offset: offset)
                        try Task.checkCancellation()
                        prefix += range.text
                        offset = range.next_offset_bytes
                        if range.end_of_content {
                            segmentIndex += 1
                            offset = "0"
                        }
                        finished = segmentIndex == segments.count
                        if finished { break }
                    }
                    #if DEBUG
                    let preparationStart = SuspendingClock.now
                    #endif
                    let displayed = try next.presentation.display(prefix, complete: finished)
                    let artifacts = finished && next.content.field == "tool_artifacts"
                        ? try JSONDecoder().decode([MacCorePartArtifact].self, from: Data(displayed.utf8)) : nil
                    guard self.generation == ticket else { return }
                    var snapshot = Snapshot(text: displayed, artifacts: artifacts,
                        loadState: .loaded(complete: finished))
                    if next.presentation == .markdown {
                        // Prepare before publishing. An observable empty projection
                        // followed by an in-place update invalidates the layout twice
                        // and exposes a transient blank tree during body hydration.
                        let previousProjection = self.markdownProjection
                            ?? MacJournalMarkdownProjection(identitySeed: "\(next.partID):\(next.content.field)")
                        let projection = try await MacConversationReadTask.run {
                            var projection = previousProjection
                            projection.update(displayed, isFinal: next.isFinal)
                            return projection
                        }
                        snapshot.markdownProjection = projection
                    }
                    try Task.checkCancellation()
                    guard self.generation == ticket else { return }
                    // Text, semantic nodes and read completeness belong to one
                    // body version. Never render the new text with old/nil nodes.
                    #if DEBUG
                    let publicationStart = SuspendingClock.now
                    #endif
                    self.snapshot = snapshot
                    self.error = nil
                    self.loaded = next
                    #if DEBUG
                    if ProcessInfo.processInfo.environment["NOLA_DIAGNOSE_BODY_HYDRATION"] == "1" {
                        let preparation = preparationStart.duration(to: publicationStart)
                        let publication = publicationStart.duration(to: .now)
                        print("journal_body_timing uptime=\(ProcessInfo.processInfo.systemUptime) bytes=\(displayed.utf8.count) prepare=\(preparation) publish=\(publication)")
                    }
                    #endif
                    if self.desired?.hasSameBody(as: next) == true { return }
                } catch {
                    if !Task.isCancelled, self.generation == ticket, self.desired != next { continue }
                    if !Task.isCancelled, self.generation == ticket { self.error = error.localizedDescription }
                    return
                }
            }
        }
    }

    func cancel() { generation &+= 1; worker?.cancel(); worker = nil }
}

nonisolated struct MacJournalActivityExpansion {
    private(set) var userChoice: Bool?
    private(set) var observedRunning = false

    func isExpanded(isRunning: Bool) -> Bool { userChoice ?? (isRunning || observedRunning) }

    mutating func observe(isRunning: Bool) { observedRunning = observedRunning || isRunning }

    mutating func toggle(isRunning: Bool) {
        userChoice = !isExpanded(isRunning: isRunning)
    }
    mutating func setExpanded(_ value: Bool) { userChoice = value }
}

/// Journal hydration feeds the same stable semantic nodes as live presentation.
/// Completion seals the existing tail; it never replaces the rendered tree.
nonisolated struct MacJournalMarkdownProjection: Sendable {
    private(set) var presentation = MacAssistantPresentation()
    private(set) var text = ""
    private(set) var hasStreamed = false
    private var assembler: MacSemanticStreamAssembler
    private let identitySeed: String

    var renderedSource: String {
        presentation.isComplete ? text : String(text.dropLast(assembler.unconfirmedTableSuffixCount))
    }

    init(identitySeed: String) {
        self.identitySeed = identitySeed
        assembler = MacSemanticStreamAssembler(identitySeed: identitySeed)
    }

    mutating func update(_ value: String, isFinal: Bool) {
        hasStreamed = hasStreamed || !isFinal
        if !value.hasPrefix(text) || (presentation.isComplete && value != text) {
            assembler = MacSemanticStreamAssembler(identitySeed: identitySeed)
            presentation = MacAssistantPresentation()
            text = ""
        }
        let delta = String(value.dropFirst(text.count))
        presentation.append(assembler.append(delta))
        text = value
        if isFinal {
            presentation.append(assembler.finish())
            presentation.updateMutableTextNode(nil)
        } else {
            presentation.updateMutableTextNode(assembler.mutableTail)
        }
        presentation.isComplete = isFinal
    }
}

nonisolated enum MacJournalPreviewLoadState: Equatable {
    case loading
    case loaded(complete: Bool)

    var isComplete: Bool { self == .loaded(complete: true) }
    var showsFullReader: Bool { self == .loaded(complete: false) }
}

nonisolated enum MacJournalActivityTitle {
    static func symbol(state: String) -> String {
        switch state {
        case "completed": "checkmark.circle"
        case "failed": "exclamationmark.circle"
        case "cancelled": "xmark.circle"
        case "pending", "queued": "clock"
        case "waiting", "waiting_for_input", "waiting_for_permission", "suspended", "retry_waiting": "pause.circle"
        case "sealed": "circle.dotted"
        default: "questionmark.circle"
        }
    }

    static func hasDetails(title: MacJournalText, bodies: [MacAssistantJournalContentReference]) -> Bool {
        bodies.contains { body in
            guard body.utf8_bytes != "0" else { return false }
            if body.field == "reasoning_summary" { return true }
            if case .field(let field) = title { return body.field == field }
            return false
        }
    }

    static func showsReasoning(state: String, hasDetails: Bool) -> Bool {
        // Unknown/failed states remain inspectable. Only settled empty
        // placeholders disappear; the underlying journal is untouched.
        hasDetails || !["completed", "sealed", "cancelled"].contains(state)
    }

    static func label(isReasoning: Bool, isRunning: Bool = false, inlineTitle: String?) -> String {
        // Reasoning is a typed protocol activity. Its wire title is not UI copy.
        if isReasoning { return isRunning ? String(localized: "Thinking") : String(localized: "Thinking Process") }
        return inlineTitle.flatMap { $0.isEmpty ? nil : $0 } ?? String(localized: "Step")
    }
}

nonisolated enum MacJournalFailurePresentation {
    static func canOpenModelSettings(for code: String) -> Bool {
        ["provider_failed", "authentication_failed", "chatgpt_reauthorization_required",
         "model_account_changed", "invalid_model_request", "model_unavailable",
         "quota_exhausted", "responses_protocol_error"].contains(code)
    }

    // Presentation only: exact committed codes, never provider-text matching.
    // Unknown failures retain their original journal body.
    static func message(for code: String, bundle: Bundle = .main) -> String? {
        let key: String
        switch code {
        case "responses_incomplete": key = "The model did not finish this response. You can ask it to continue."
        case "responses_timeout": key = "The model stopped sending output. Check the connection before trying again."
        case "responses_transport_error": key = "The model connection was interrupted before usable output arrived."
        case "responses_tool_transaction_incomplete": key = "The model did not finish its tool request. No tools from this response were executed."
        case "responses_provider_failed": key = "The model service reported a failure. Check its status or choose another model."
        case "provider_failed": key = "The model request failed. Check the model connection and account status in model settings before retrying."
        case "chatgpt_reauthorization_required": key = "Open model settings and sign in to or reauthorize your ChatGPT account, then retry."
        case "model_account_changed": key = "The selected model account changed. Select its current model in model settings, then retry."
        case "invalid_model_request": key = "The selected model cannot accept these request settings. Review the model protocol and conversation overrides."
        case "model_repetitive_reasoning": key = "The model repeated its reasoning without making progress. Generation was stopped; retry or select another model."
        case "responses_protocol_error": key = "The model returned an invalid response. Check the connection's API protocol."
        case "content_filtered": key = "The model service stopped this response because of its content policy."
        case "authentication_failed": key = "The model service rejected its credentials. Check the API key in model settings."
        case "quota_exhausted": key = "The model service quota is exhausted. Check your account or choose another model."
        case "rate_limited": key = "The model service is receiving too many requests. Try again later."
        case "model_unavailable": key = "The requested model or endpoint is unavailable. Check the model connection settings."
        case "context_length_exceeded": key = "The conversation exceeds the model's context limit. Compact it or choose a model with a larger context window."
        case "model_resampled": key = "The model request was restarted."
        case "invalid_compaction_summary": key = "The model returned an unusable conversation summary."
        default: return nil
        }
        return bundle.localizedString(forKey: key, value: key, table: "Chat")
    }
}

nonisolated enum MacJournalPartFlow {
    static func segments(_ parts: [MacAssistantJournalPart]) -> [AppleMessagePartFlowSegment<MacAssistantJournalPart>] {
        AppleMessagePartFlow.segments(parts, id: \.part_id) { part in
            switch part.metadata {
            case .reasoning: .reasoning
            case .tool: .tool
            default: .content
            }
        }
    }
    static func liveReasoning(_ parts: [MacAssistantJournalPart], delivery: MacChatMessage.DeliveryState?) -> [MacAssistantJournalPart] {
        guard delivery == .running else { return [] }
        return parts.filter { part in
            if case .reasoning(let activity) = part.metadata { return activity.isRunning }
            return false
        }
    }
    static func hasLiveReasoning(_ parts: [MacAssistantJournalPart], delivery: MacChatMessage.DeliveryState?) -> Bool {
        !liveReasoning(parts, delivery: delivery).isEmpty
    }
    static func delivery(_ parts: [MacAssistantJournalPart]) -> MacChatMessage.DeliveryState? {
        let states = parts.compactMap { part -> String? in
            switch part.metadata { case .reasoning(let value): value.state; case .tool(let value): value.state; default: nil }
        }
        if states.contains(where: { !["completed", "sealed", "cancelled", "failed"].contains($0) }) { return .running }
        if states.contains("failed") { return .failed }
        if states.contains("cancelled") { return .cancelled }
        if states.contains("sealed") { return nil }
        return .completed
    }
}

/// A presentation partition, never a rewritten transcript or inferred summary.
/// Activity order is the committed journal order, including tool boundaries.
nonisolated struct MacJournalProcessProjection {
    let activities: [MacAssistantJournalPart]
    var toolCount: Int { activities.reduce(0) { total, part in
        if case .tool = part.metadata { return total + 1 }; return total
    } }
    var attentionState: String? {
        let states = activities.compactMap { part -> String? in
            switch part.metadata {
            case .error: return "failed"
            case .tool(let value): return value.state
            case .reasoning(let value), .step(let value): return value.state
            case .task(let value): return value.state
            case .modelAttempt(let value): return value.state
            default: return nil
            }
        }
        return ["waiting_for_permission", "waiting_for_input", "pending", "failed"]
            .first { states.contains($0) }
    }
    var requiresAttention: Bool {
        attentionState != nil
    }
    init(_ parts: [MacAssistantJournalPart]) {
        var activities: [MacAssistantJournalPart] = []
        for part in parts {
            switch part.metadata {
            case .reasoning, .step, .tool, .error, .skill, .modelAttempt, .task: activities.append(part)
            default: break
            }
        }
        self.activities = activities

    }
}

nonisolated enum MacJournalProcessStatus {
    static func primary(delivery: MacChatMessage.DeliveryState?, attention: String?) -> String? {
        if let delivery, delivery != .running { return delivery.rawValue }
        if attention == "waiting_for_permission" || attention == "waiting_for_input" { return attention }
        return delivery?.rawValue
    }
}

nonisolated enum MacJournalRunStatus {
    static func attentionState(_ outcome: String?) -> String? {
        guard let outcome, ["failed", "blocked", "incomplete"].contains(outcome) else { return nil }
        return outcome
    }
}

nonisolated struct MacJournalCompletedToolGrouping {
    var groups: [String: [MacAssistantJournalPart]] = [:]
    var hiddenIDs: Set<String> = []

    init(_ parts: [MacAssistantJournalPart]) {
        var run: [MacAssistantJournalPart] = []
        func flush() {
            defer { run.removeAll(keepingCapacity: true) }
            guard run.count >= 3, let first = run.first else { return }
            groups[first.part_id] = run
            hiddenIDs.formUnion(run.dropFirst().map(\.part_id))
        }
        for part in parts {
            if case .tool(let tool) = part.metadata, tool.state == "completed" {
                // Contiguous completion is not ownership: never fold a cell's
                // children together with another cell or a model-owned call.
                if let previous = run.last, case .tool(let prior) = previous.metadata,
                   prior.execution_source != tool.execution_source {
                    flush()
                }
                run.append(part)
            } else {
                flush()
            }
        }
        flush()
    }
}

enum MacJournalTextRead {
    /// Explicit user export only. Keep one snapshot pinned for every metadata
    /// page and body range; clipboard callers publish only the complete result.
    static func markdown(_ reference: MacJournalMessageReference, historicalFallback: String) async throws -> String {
        try Task.checkCancellation()
        let task = Task.detached(priority: .userInitiated) {
            let port = try MacApplicationPort(databaseURL: reference.databaseURL)
            let snapshot = try port.openAssistantJournalSnapshot(at: reference.cursor)
            defer { try? port.releaseAssistantJournalSnapshot(snapshot) }
            var after: String?
            var visited = Set<String>()
            var parts = Set<String>()
            var result = ""
            var historical = false
            repeat {
                try Task.checkCancellation()
                let page = try port.assistantJournalParts(snapshot: snapshot, afterPartID: after)
                guard page.at == reference.cursor else { throw MacRemoteHostError.invalidResponse }
                for part in page.parts {
                    guard part.identity == reference.cursor.identity, parts.insert(part.part_id).inserted else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    if case .historical(let value) = part.metadata, value.canonical_kind == "text" {
                        historical = true
                    }
                    guard case .text(let text) = part.metadata, text.visibility == "public",
                          let content = part.bodies.first(where: { $0.field == "markdown" }) else { continue }
                    var offset = "0"
                    while true {
                        try Task.checkCancellation()
                        let range = try port.assistantJournalRange(snapshot: snapshot, partID: part.part_id,
                            content: content, offsetBytes: offset)
                        guard range.offset_bytes == offset, range.total_utf8_bytes == content.utf8_bytes,
                              let start = UInt64(offset), let next = UInt64(range.next_offset_bytes),
                              let total = UInt64(content.utf8_bytes), start <= next, next <= total,
                              next - start == UInt64(range.text.utf8.count), range.text.utf8.count <= 32768,
                              range.end_of_content == (next == total), range.end_of_content || next > start else {
                            throw MacRemoteHostError.invalidResponse
                        }
                        result += range.text
                        if range.end_of_content { break }
                        offset = range.next_offset_bytes
                    }
                }
                after = page.next_part_id
                if let after, !visited.insert(after).inserted { throw MacRemoteHostError.invalidResponse }
            } while after != nil
            try Task.checkCancellation()
            return historical ? historicalFallback : result
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    static func updatedParts(_ reference: MacJournalMessageReference,
                             from base: MacJournalMessageReference,
                             existing: [MacAssistantJournalPart]) async throws -> [MacAssistantJournalPart] {
        try Task.checkCancellation()
        guard reference.databaseURL == base.databaseURL,
              reference.cursor.identity == base.cursor.identity,
              let before = UInt64(base.cursor.sequence), let after = UInt64(reference.cursor.sequence),
              before <= after else { throw MacRemoteHostError.invalidResponse }
        if reference.cursor == base.cursor { return existing }
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            let port = try MacApplicationPort(databaseURL: reference.databaseURL)
            let snapshot = try port.openAssistantJournalSnapshot(at: reference.cursor)
            defer { try? port.releaseAssistantJournalSnapshot(snapshot) }
            var values: [String: MacAssistantJournalPart] = [:]
            for part in existing {
                guard part.identity == base.cursor.identity, values[part.part_id] == nil else {
                    throw MacRemoteHostError.invalidResponse
                }
                values[part.part_id] = part
            }
            var next: String?
            var seen = Set<String>()
            repeat {
                try Task.checkCancellation()
                let page = try port.assistantJournalPartChanges(snapshot: snapshot, base: base.cursor, afterPartID: next)
                guard page.base == base.cursor, page.at == reference.cursor else {
                    throw MacRemoteHostError.invalidResponse
                }
                var previous = next
                for change in page.changes {
                    guard seen.insert(change.part_id).inserted,
                          previous == nil || previous!.utf8.lexicographicallyPrecedes(change.part_id.utf8) else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    if let head = change.head {
                        guard head.identity == reference.cursor.identity, head.part_id == change.part_id else {
                            throw MacRemoteHostError.invalidResponse
                        }
                    }
                    // A nil head removes the entry. It is not an absent update.
                    values[change.part_id] = change.head
                    previous = change.part_id
                }
                if let continuation = page.next_part_id {
                    guard continuation == page.changes.last?.part_id,
                          next == nil || next!.utf8.lexicographicallyPrecedes(continuation.utf8) else {
                        throw MacRemoteHostError.invalidResponse
                    }
                }
                next = page.next_part_id
            } while next != nil
            let ordered = try values.values.map { part -> (Int, UInt64, MacAssistantJournalPart) in
                let sections = ["transcript": 0, "tasks": 1, "skills": 2, "model_attempts": 3]
                guard let section = sections[part.order_key.section], let position = UInt64(part.order_key.position) else {
                    throw MacRemoteHostError.invalidResponse
                }
                return (section, position, part)
            }.sorted { left, right in
                if left.0 != right.0 { return left.0 < right.0 }
                if left.1 != right.1 { return left.1 < right.1 }
                return left.2.part_id.utf8.lexicographicallyPrecedes(right.2.part_id.utf8)
            }
            try Task.checkCancellation()
            return ordered.map { $0.2 }
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    static func completeSmallText(_ reference: MacJournalMessageReference, partID: String,
                                  content: MacAssistantJournalContentReference) async throws -> String? {
        guard let count = UInt64(content.utf8_bytes) else { throw MacRemoteHostError.invalidResponse }
        // Navigation metadata has a fixed read budget, independent of body
        // size. Larger values remain available through the explicit reader.
        guard count <= 32768 else { return nil }
        let result = try await range(reference, partID: partID, content: content, offset: "0")
        guard result.end_of_content else { throw MacRemoteHostError.invalidResponse }
        return result.text
    }
    static func isLocal(_ reference: MacJournalMessageReference) async throws -> Bool {
        try Task.checkCancellation()
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            let port = try MacApplicationPort(databaseURL: reference.databaseURL)
            let head = try port.localAssistantJournalHead(identity: reference.cursor.identity)
            try Task.checkCancellation()
            return head.identity == reference.cursor.identity
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }
    static func parts(_ reference: MacJournalMessageReference) async throws -> [MacAssistantJournalPart] {
        try Task.checkCancellation()
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            let port = try MacApplicationPort(databaseURL: reference.databaseURL)
            let snapshot = try port.openAssistantJournalSnapshot(at: reference.cursor)
            defer { try? port.releaseAssistantJournalSnapshot(snapshot) }
            var result: [MacAssistantJournalPart] = []
            var after: String?
            var visited = Set<String>()
            var partIDs = Set<String>()
            repeat {
                try Task.checkCancellation()
                let page = try port.assistantJournalParts(snapshot: snapshot, afterPartID: after)
                guard page.at == reference.cursor,
                      page.parts.allSatisfy({ $0.identity == reference.cursor.identity && partIDs.insert($0.part_id).inserted }) else {
                    throw MacRemoteHostError.invalidResponse
                }
                result.append(contentsOf: page.parts)
                after = page.next_part_id
                if let after, !visited.insert(after).inserted { throw MacRemoteHostError.invalidResponse }
            } while after != nil
            return result
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    static func range(_ reference: MacJournalMessageReference, partID: String,
                      content: MacAssistantJournalContentReference, offset: String) async throws -> MacAssistantJournalTextRange {
        try Task.checkCancellation()
        #if DEBUG
        let diagnose = ProcessInfo.processInfo.environment["NOLA_DIAGNOSE_BODY_HYDRATION"] == "1"
        let admitted = ProcessInfo.processInfo.systemUptime
        #endif
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            #if DEBUG
            let started = ProcessInfo.processInfo.systemUptime
            #endif
            let port = try MacApplicationPort(databaseURL: reference.databaseURL)
            #if DEBUG
            let connected = ProcessInfo.processInfo.systemUptime
            #endif
            let snapshot = try port.openAssistantJournalSnapshot(at: reference.cursor)
            defer { try? port.releaseAssistantJournalSnapshot(snapshot) }
            #if DEBUG
            let opened = ProcessInfo.processInfo.systemUptime
            #endif
            let result = try port.assistantJournalRange(snapshot: snapshot, partID: partID, content: content, offsetBytes: offset)
            #if DEBUG
            if diagnose {
                print("journal_range_timing admitted=\(admitted) started=\(started) connected=\(connected) opened=\(opened) returned=\(ProcessInfo.processInfo.systemUptime) bytes=\(result.text.utf8.count)")
            }
            #endif
            try Task.checkCancellation()
            guard result.offset_bytes == offset,
                  result.total_utf8_bytes == content.utf8_bytes,
                  let start = UInt64(offset), let next = UInt64(result.next_offset_bytes),
                  let total = UInt64(content.utf8_bytes), start <= next, next <= total,
                  next - start == UInt64(result.text.utf8.count), result.text.utf8.count <= 32768,
                  result.end_of_content == (next == total),
                  result.end_of_content || next > start else {
                throw MacRemoteHostError.invalidResponse
            }
            return result
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }
}

@MainActor
struct MacJournalMessageTextView: View {
    @Environment(\.uiSurfaceContext) private var uiSurfaceContext
    let reference: MacJournalMessageReference
    var deliveryState: MacChatMessage.DeliveryState? = nil
    var onActivitiesLoaded: (MacAssistantJournalIdentity, Set<String>, Set<String>) -> Void = { _, _, _ in }
    var onSendTaskGuidance: (String, String) -> Void = { _, _ in }
    var onRequestTaskCancellation: (String) -> Void = { _ in }
    var taskGuidance: [String: MacCoreTaskGuidanceQueue] = [:]
    var onReferencesLoaded: (MacAssistantJournalIdentity, Set<String>, Set<String>, Set<String>) -> Void = { _, _, _, _ in }
    var presentationOwner: MacJournalPresentationOwner?
    @State private var localOwner = MacJournalPresentationOwner()
    @State private var terminalOutcome: (owner: MacJournalOwnerKey, value: String)?
    private var outcome: String? {
        terminalOutcome?.owner == reference.ownerKey ? terminalOutcome?.value : nil
    }
    private var owner: MacJournalPresentationOwner { presentationOwner ?? localOwner }
    // SwiftUI may evaluate a reused row before onChange delivers its next
    // reference. Never paint the previous conversation/Run for that one frame.
    private var selectedProjection: MacJournalPresentationOwner.Projection? { owner.projection(matching: reference) }
    private var parts: [MacAssistantJournalPart] { selectedProjection?.parts ?? [] }
    private var loadedReference: MacJournalMessageReference? { selectedProjection?.reference }

    @ViewBuilder private func journalParts(_ visibleParts: [MacAssistantJournalPart],
        allParts: [MacAssistantJournalPart], reference loadedReference: MacJournalMessageReference,
        inlineReasoning: Bool = false) -> some View {
        // Tool compaction sees the untouched canonical sequence. Every other
        // Part remains at its own position; only Copy assembles public prose.
        let grouping = MacJournalCompletedToolGrouping(allParts)
        ForEach(visibleParts, id: \.part_id) { part in
            if let group = grouping.groups[part.part_id] {
                MacJournalCompletedToolsView(reference: loadedReference, parts: group)
            } else if grouping.hiddenIDs.contains(part.part_id) {
                EmptyView()
            } else if case .reasoning(let activity) = part.metadata {
                if MacJournalActivityTitle.showsReasoning(state: activity.state,
                    hasDetails: MacJournalActivityTitle.hasDetails(title: activity.title, bodies: part.bodies)) {
                    MacJournalActivityView(reference: loadedReference, part: part, activity: activity, isReasoning: true,
                        inlineDetails: inlineReasoning)
                }
            } else if case .step(let activity) = part.metadata {
                MacJournalActivityView(reference: loadedReference, part: part, activity: activity, isReasoning: false)
            } else if case .tool(let tool) = part.metadata {
                MacJournalToolView(reference: loadedReference, part: part, tool: tool)
            } else if case .error(let failure) = part.metadata {
                MacJournalFailureView(reference: loadedReference, part: part, failure: failure)
            } else if case .skill(let skill) = part.metadata {
                MacJournalSkillView(reference: loadedReference, part: part, skill: skill)
            } else if case .modelAttempt(let attempt) = part.metadata {
                MacJournalModelAttemptView(reference: loadedReference, part: part, attempt: attempt)
            } else if case .task(let task) = part.metadata {
                MacJournalTaskView(reference: loadedReference, part: part, task: task,
                    guidance: taskGuidance[task.task_id], onSendGuidance: onSendTaskGuidance,
                    onRequestCancellation: onRequestTaskCancellation)
                    .id(loadedReference.ownerKey)
            } else if case .uiSurface(let value) = part.metadata {
                if value.surface.identity.owner.conversation_id == loadedReference.cursor.identity.conversation_id,
                   value.surface.identity.owner.message_id == loadedReference.cursor.identity.assistant_message_id,
                   uiSurfaceContext != nil {
                    MacMessageReferencedSurface(reference: value.surface, summary: value.summary).id(value.surface.identity)
                }
            } else if case .attachment(let attachment) = part.metadata {
                MacJournalSourceView(reference: loadedReference, part: part,
                    title: attachment.name, uri: attachment.uri, mime: attachment.mime_type,
                    operation: attachment.operation, isAttachment: true)
            } else if case .source(let source) = part.metadata {
                MacJournalSourceView(reference: loadedReference, part: part,
                    title: source.title, uri: source.uri, mime: source.mime_type)
            } else if case .citation(let citation) = part.metadata {
                MacJournalSourceView(reference: loadedReference, part: part,
                    title: citation.title, uri: citation.uri, mime: citation.mime_type,
                    location: citation.location)
            } else if case .historical(let historical) = part.metadata {
                MacJournalHistoricalPartView(reference: loadedReference, part: part, historical: historical)
                    .id(MacHistoricalJournalPartViewKey(owner: loadedReference.ownerKey, partID: part.part_id,
                        canonicalRevision: historical.canonical_revision))
            } else if case .text(let text) = part.metadata, text.visibility == "public",
               let content = part.bodies.first(where: { $0.field == "markdown" }) {
                MacJournalTextPreview(reference: loadedReference, partID: part.part_id,
                    content: content, isFinal: text.is_final)
            }
        }
    }

    var body: some View {
        // Derive page-wide rendering decisions once from the same snapshot.
        // ForEach must not rescan all Parts (and reread the observable owner)
        // for every row when a long journal projection is published.
        let projection = selectedProjection
        return VStack(alignment: .leading, spacing: 12) {
            if let projection {
                let loadedReference = projection.reference
                let ordered = projection.parts
                ForEach(MacJournalPartFlow.segments(ordered)) { segment in
                    if segment.kind == .process {
                        let process = MacJournalProcessProjection(segment.parts)
                        MacJournalProcessView(partIDs: segment.parts.map(\.part_id), delivery: MacJournalPartFlow.delivery(segment.parts),
                            requiresAttention: process.requiresAttention,
                            toolCount: process.toolCount, attentionState: process.attentionState,
                            hasLiveReasoning: MacJournalPartFlow.hasLiveReasoning(segment.parts, delivery: deliveryState),
                            reasoningOnly: process.toolCount == 0,
                            settledStatus: MacJournalPartFlow.delivery(segment.parts) == nil ? "sealed" : nil) {
                            journalParts(segment.parts, allParts: ordered,
                                reference: loadedReference, inlineReasoning: true)
                        }
                    } else {
                        journalParts(segment.parts, allParts: ordered, reference: loadedReference)
                    }
                }
            }
            if let state = MacJournalRunStatus.attentionState(outcome) {
                MacJournalStatusText(state: state)
                    .accessibilityIdentifier("chat.journal.runOutcome")
            }
            if let error = owner.error {
                Text(error).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                Button { owner.request(reference, retry: true) } label: { Image(systemName: "arrow.clockwise") }
                    .accessibilityLabel(Text("Retry"))
            }
        }
        .animation(nil, value: parts.map(\.part_id))
        .environment(\.journalPresentationOwner, owner)
        .modifier(MacMessageReaderPresenter(owner: owner))
        .onChange(of: reference, initial: true) { _, value in owner.request(value) }
        .task(id: "\(reference.ownerKey)|\(deliveryState?.rawValue ?? "")|\(reference.cursor.sequence)") {
            guard deliveryState != .running else { return }
            let current = reference
            let read = Task.detached(priority: .utility) {
                let port = try MacApplicationPort(databaseURL: current.databaseURL)
                return try port.canonicalRunTerminal(conversationID: current.cursor.identity.conversation_id,
                    runID: current.cursor.identity.run_id)
            }
            do {
                let terminal = try await withTaskCancellationHandler { try await read.value } onCancel: { read.cancel() }
                try Task.checkCancellation()
                if let terminal, terminal.runID == current.cursor.identity.run_id,
                   terminal.conversationID == current.cursor.identity.conversation_id {
                    terminalOutcome = (current.ownerKey, terminal.result.value.kind.rawValue)
                }
            } catch { /* Never invent success when the canonical result cannot be read. */ }
        }
        .onChange(of: loadedReference, initial: true) { _, loaded in
            if let loaded {
                onActivitiesLoaded(loaded.cursor.identity, Set(parts.compactMap { part in
                    switch part.metadata {
                    case .reasoning, .step, .tool, .error, .skill, .modelAttempt, .task: return part.part_id
                    default: return nil
                    }
                }), Set(parts.compactMap { part in
                    if case .task(let task) = part.metadata { return task.task_id }
                    return nil
                }))
                onReferencesLoaded(loaded.cursor.identity, Set(parts.compactMap { part in
                    if case .source(let source) = part.metadata { return source.source_id }
                    return nil
                }), Set(parts.compactMap { part in
                    if case .citation(let citation) = part.metadata { return citation.citation_id }
                    return nil
                }), Set(parts.compactMap { part in
                    if case .attachment(let attachment) = part.metadata { return attachment.attachment_id }
                    return nil
                }))
            }
        }
        .onDisappear { if presentationOwner == nil { localOwner.cancel() } }
    }
}

@MainActor
private struct MacJournalSourceView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let title: MacJournalText
    let uri: MacJournalText
    let mime: MacJournalText?
    var location: MacAssistantJournalMetadata.Location?
    var operation: MacJournalText?
    var isAttachment = false
    @State private var resolvedURI: String?
    @State private var resolvedKey: MacJournalFieldReadKey?
    @State private var technicalExpanded = false

    private var uriContent: MacAssistantJournalContentReference? {
        guard case .field(let field) = uri else { return nil }
        return part.bodies.first { $0.field == field }
    }

    private var readKey: MacJournalFieldReadKey {
        .init(databaseURL: reference.databaseURL, identity: reference.cursor.identity,
            partID: part.part_id, content: uriContent)
    }

    private var address: String? {
        inline(uri) ?? (resolvedKey == readKey ? resolvedURI : nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let address {
                MacJournalArtifactList(reference: reference, artifacts: [
                    .init(uri: address, name: inline(title) ?? address,
                        mimeType: mime.flatMap(inline), operation: operation.flatMap(inline))
                ])
            } else if let name = inline(title) {
                Label { Text(verbatim: name) } icon: { Image(systemName: isAttachment ? "doc" : "link") }
                if let operation = operation.flatMap(inline) {
                    Text(verbatim: operation).font(.caption).foregroundStyle(.secondary)
                }
            }
            if let location {
                HStack {
                    if let page = location.page { Text("Page"); Text(verbatim: page) }
                    if let line = location.line_start { Text("Line"); Text(verbatim: line) }
                    if let end = location.line_end { Text(verbatim: "– " + end) }
                }.font(.caption).foregroundStyle(.secondary)
                if let section = location.section.flatMap(inline) {
                    Text(verbatim: section).font(.caption)
                }
            }
            // Field-backed titles/URIs/evidence remain bounded reads of this
            // exact Part. Never invent a destination from a truncated URI.
            ForEach(part.bodies.filter { MacJournalSourceFieldPresentation.isPrimary($0.field) }, id: \.field) { content in
                MacJournalTextPreview(reference: reference, partID: part.part_id,
                    content: content, isFinal: true, presentation: .plain, usesMonospacedText: false)
            }
            DisclosureGroup(isExpanded: $technicalExpanded) {
                if technicalExpanded {
                    if let address { Text(verbatim: address).font(.caption).textSelection(.enabled) }
                    switch part.metadata {
                    case .source(let source):
                        Text(verbatim: source.source_id).font(.caption).textSelection(.enabled)
                    case .citation(let citation):
                        Text(verbatim: citation.source_id + " · " + citation.citation_id)
                            .font(.caption).textSelection(.enabled)
                        Text(verbatim: citation.origin_tool_call_id).font(.caption).textSelection(.enabled)
                        if let range = citation.byte_range {
                            Text(verbatim: "[\(range.start), \(range.end))").font(.caption.monospaced()).textSelection(.enabled)
                        }
                    case .attachment(let attachment):
                        Text(verbatim: attachment.attachment_id).font(.caption).textSelection(.enabled)
                    default: EmptyView()
                    }
                    ForEach(part.bodies.filter { !MacJournalSourceFieldPresentation.isPrimary($0.field) }, id: \.field) { content in
                        MacJournalTextPreview(reference: reference, partID: part.part_id,
                            content: content, isFinal: true,
                            presentation: content.field == "source_evidence" ? .structured("evidence") : .plain)
                    }
                }
            } label: { Text("Technical details", tableName: "Chat").font(.caption).foregroundStyle(.secondary) }
            .disclosureGroupStyle(MacRowDisclosureStyle())
        }
        .accessibilityIdentifier(isAttachment ? "chat.journal.attachment" : "chat.journal.source")
        .task(id: readKey) {
            let key = readKey
            resolvedURI = nil
            resolvedKey = nil
            guard let content = key.content else { return }
            do {
                let value = try await MacJournalTextRead.completeSmallText(reference,
                    partID: part.part_id, content: content)
                try Task.checkCancellation()
                resolvedURI = value
                resolvedKey = key
            } catch {
                // The existing bounded body reader exposes read/retry errors.
                // Failure to resolve navigation must never enable a stale URL.
            }
        }
    }

    private func inline(_ value: MacJournalText) -> String? {
        if case .inline(let text) = value { return text }
        return nil
    }
}

nonisolated enum MacJournalSourceFieldPresentation {
    static func isPrimary(_ field: String) -> Bool {
        ["title", "name", "citation_section", "source_snippet"].contains(field)
    }
}

@MainActor
private struct MacJournalTaskView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let task: MacAssistantJournalMetadata.Task
    let guidance: MacCoreTaskGuidanceQueue?
    let onSendGuidance: (String, String) -> Void
    let onRequestCancellation: (String) -> Void
    @State private var draft = ""
    private var terminal: Bool { task.isTerminal }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label { Text("Task") } icon: { Image(systemName: "list.bullet.rectangle") }
                Spacer(minLength: 8)
                MacJournalStatusText(state: task.state)
            }
            if let fraction = task.progress?.fraction { ProgressView(value: fraction) }
            ForEach(part.bodies, id: \.field) { content in
                MacJournalTextPreview(reference: reference, partID: part.part_id, content: content,
                    isFinal: terminal, presentation: .plain)
            }
            if let guidance, !guidance.pending.isEmpty {
                ForEach(guidance.pending) { message in
                    Text(verbatim: message.content).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                }
            }
            if task.acceptsInteraction {
                HStack {
                    if task.supportsGuidance {
                    TextField(String(localized: "Task guidance", table: "Chat"), text: $draft)
                    Button {
                        let message = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard task.acceptsInteraction, task.supportsGuidance, !message.isEmpty else { return }
                        onSendGuidance(task.task_id, message)
                        draft = ""
                    } label: { Image(systemName: "arrow.up") }
                        .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityLabel(Text("Task guidance", tableName: "Chat"))
                    }
                    if task.supportsCancellation {
                    Button { onRequestCancellation(task.task_id) } label: {
                        Text("Cancel task", tableName: "Chat")
                    }.disabled(task.cancel_requested)
                    }
                }
            }
            if task.supportsCancellation {
                MacOwnedTaskRecoveryView(reference: reference, taskID: task.task_id)
            }
            // Controls send exact Task IDs to the existing admission port;
            // display state never synthesizes an execution lease or owner.
        }.accessibilityIdentifier("chat.journal.task")
    }
}

@MainActor
private struct MacJournalStatusText: View {
    let state: String
    private var label: LocalizedStringKey {
        switch state {
        case "pending": "Pending"
        case "waiting_for_permission": "Waiting for permission"
        case "waiting_for_input": "Waiting for your answer"
        case "running": "Running"
        case "completed": "Completed"
        case "delegation_accepted": "Delegated"
        case "incomplete": "Incomplete"
        case "blocked": "Blocked"
        case "finished", "sealed": "Finished"
        case "failed": "Failed"
        case "cancelled": "Cancelled"
        default: "Unknown"
        }
    }
    var body: some View { Text(label).font(.caption).foregroundStyle(.secondary) }
}

@MainActor
private struct MacJournalSkillView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let skill: MacAssistantJournalMetadata.Skill

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            MacJournalSkillHeader(value: .init(skill: skill)).equatable()
            ForEach(part.bodies, id: \.field) { content in
                MacJournalTextPreview(reference: reference, partID: part.part_id, content: content,
                    isFinal: skill.state != "running", presentation: .plain)
            }
            MacJournalSkillDetails(skillID: skill.skill_id, runID: skill.run_id, mode: skill.mode)
                .equatable()
        }.accessibilityIdentifier("chat.journal.skill")
    }
}

/// Cursor/body changes are deliberately absent from static Skill chrome.
nonisolated struct MacJournalSkillHeaderValue: Equatable {
    let title: String
    let state: String
    let modelTurns: String
    let toolCalls: String
    let elapsedMillis: String

    init(skill: MacAssistantJournalMetadata.Skill) {
        if case .inline(let name) = skill.display_name { title = name }
        else { title = skill.skill_id }
        state = skill.state
        modelTurns = skill.model_turns
        toolCalls = skill.tool_calls
        elapsedMillis = skill.elapsed_millis
    }
}

@MainActor
private struct MacJournalSkillHeader: View, Equatable {
    let value: MacJournalSkillHeaderValue

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: "sparkles")
                Text(verbatim: value.title)
                Spacer(minLength: 8)
                MacJournalStatusText(state: value.state)
            }
            HStack(spacing: 16) {
                LabeledContent("Model turns", value: value.modelTurns)
                LabeledContent("Tool calls", value: value.toolCalls)
                Text("\(value.elapsedMillis) ms")
            }.font(.caption).foregroundStyle(.secondary)
        }.transaction { $0.animation = nil }
    }
}

@MainActor
struct MacJournalSkillDetails: View, Equatable {
    let skillID: String
    let runID: String
    let mode: String

    var body: some View {
        DisclosureGroup("Details") {
            Text(verbatim: "\(skillID) · \(runID) · \(mode)")
                .font(.caption).textSelection(.enabled)
        }
        .disclosureGroupStyle(MacRowDisclosureStyle()).transaction { $0.animation = nil }
    }
}

@MainActor
private struct MacJournalModelAttemptView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let attempt: MacAssistantJournalMetadata.ModelAttempt

    private var presentation: AppleModelAttemptPresentation {
        .init(state: attempt.state, provider: attempt.provider, model: attempt.model,
            successorAttemptID: attempt.fallback_attempt_id, successorProvider: attempt.fallback_provider,
            successorModel: attempt.fallback_model)
    }

    private var failureSummary: String? {
        MacJournalFailurePresentation.message(for: attempt.failure_code)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Image(systemName: "cpu")
                Text(verbatim: presentation.modelName)
                    .font(.callout.weight(.medium))
                    .lineLimit(2)
                Spacer(minLength: 8)
                if !presentation.hasSuccessor {
                    Text(presentation.title()).font(.caption)
                        .foregroundStyle(presentation.isFailure ? Color.red : Color.secondary)
                }
            }
            if let failureSummary {
                Text(verbatim: failureSummary).textSelection(.enabled)
            } else {
                attemptBodies
            }
            if presentation.hasSuccessor {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(presentation.title())
                    if let nextModel = presentation.successorModelName { Text(verbatim: nextModel) }
                }.font(.callout).foregroundStyle(.secondary)
            }
            DisclosureGroup {
                VStack(alignment: .leading, spacing: 4) {
                    if let provider = attempt.provider { Text(verbatim: provider) }
                    Text(verbatim: attempt.attempt_id)
                    Text(verbatim: attempt.failure_code)
                    if let status = attempt.http_status { Text("HTTP \(status)") }
                    if let fallback = attempt.fallback_attempt_id { Text(verbatim: fallback) }
                    if let provider = attempt.fallback_provider { Text(verbatim: provider) }
                    if failureSummary != nil { attemptBodies }
                }.font(.caption).textSelection(.enabled)
            } label: {
                Text("Diagnostic details", tableName: "Chat")
            }
            .disclosureGroupStyle(MacRowDisclosureStyle())
            // A recorded retryable failure does not authorize an automatic
            // retry; the next committed attempt owns that transition.
        }.accessibilityIdentifier("chat.journal.modelAttempt")
    }

    private var attemptBodies: some View {
        ForEach(part.bodies, id: \.field) { content in
            MacJournalTextPreview(reference: reference, partID: part.part_id, content: content,
                isFinal: attempt.state != "running", presentation: .plain)
        }
    }
}

@MainActor
private struct MacJournalFailureView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let failure: MacAssistantJournalMetadata.Failure
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label {
                Text("Error")
            } icon: {
                Image(systemName: "exclamationmark.circle")
            }.foregroundStyle(.red)
            if let summary = MacJournalFailurePresentation.message(for: failure.code) {
                Text(verbatim: summary).textSelection(.enabled)
                if MacJournalFailurePresentation.canOpenModelSettings(for: failure.code) {
                    Button("mac.window.model_center") { openWindow(id: "model-center") }
                        .buttonStyle(MacPressFeedbackButtonStyle())
                        .accessibilityIdentifier("chat.journal.failure.open-model-settings")
                }
                DisclosureGroup {
                    Text(verbatim: failure.code).font(.caption).textSelection(.enabled)
                    failureBody
                } label: {
                    Text("Diagnostic details", tableName: "Chat")
                }
                .disclosureGroupStyle(MacRowDisclosureStyle())
            } else {
                failureBody
                Text(verbatim: failure.code).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            }
            // Recoverability is a display fact, not permission to resubmit a
            // tool or retry an effect whose settlement may still be unknown.
        }
        .accessibilityIdentifier("chat.journal.failure")
    }

    @ViewBuilder
    private var failureBody: some View {
        if let content = part.bodies.first(where: { $0.field == "failure_message" }) {
            MacJournalTextPreview(reference: reference, partID: part.part_id,
                content: content, isFinal: true, presentation: .plain)
        }
    }
}

@MainActor
private struct MacJournalCompletedToolsView: View {
    let reference: MacJournalMessageReference
    let parts: [MacAssistantJournalPart]
    @Environment(\.journalPresentationOwner) private var owner
    @Environment(\.journalProcessPartIDs) private var processPartIDs
    @State private var localExpanded = false
    private var expanded: Bool {
        get { owner?.completedToolsExpanded(partIDs: parts.map(\.part_id)) ?? localExpanded }
        nonmutating set {
            if newValue { owner?.retainProcessExpansion(partIDs: parts.map(\.part_id), in: processPartIDs) }
            if let owner, let first = parts.first { owner.completedToolExpansions[first.part_id] = newValue }
            else { localExpanded = newValue }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button { expanded.toggle() } label: {
                HStack(spacing: 8) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                    Label {
                        Text("\(parts.count) completed tools", tableName: "Chat")
                    } icon: {
                        Image(systemName: "checkmark.circle")
                    }
                    Spacer(minLength: 0)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(expanded ? Text("Expanded", tableName: "Chat") : Text("Collapsed", tableName: "Chat"))
            .accessibilityIdentifier("chat.journal.completedTools")
            if expanded {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(parts, id: \.part_id) { part in
                        if case .tool(let tool) = part.metadata {
                            MacJournalToolView(reference: reference, part: part, tool: tool)
                        }
                    }
                }.padding(.top, 6)
            }
        }
    }
}

nonisolated enum MacJournalToolFieldPresentation {
    static func isResult(_ field: String) -> Bool {
        field == "tool_summary" || field == "tool_artifacts"
    }
}

@MainActor
struct MacJournalToolView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let tool: MacAssistantJournalMetadata.Tool
    @State private var localTechnicalExpanded = false
    @Environment(\.journalPresentationOwner) private var owner
    @Environment(\.journalProcessPartIDs) private var processPartIDs
    private var technicalExpanded: Bool {
        get { owner?.toolTechnicalExpansions[part.part_id] ?? localTechnicalExpanded }
        nonmutating set {
            if newValue { owner?.retainProcessExpansion(partIDs: [part.part_id], in: processPartIDs) }
            if let owner { owner.toolTechnicalExpansions[part.part_id] = newValue }
            else { localTechnicalExpanded = newValue }
            if newValue { expanded = true }
        }
    }
    @State private var localExpanded: Bool?
    private var expanded: Bool {
        get { owner?.expansions[part.part_id]?.userChoice ?? localExpanded ?? (tool.state == "waiting_for_permission") }
        nonmutating set {
            if newValue { owner?.retainProcessExpansion(partIDs: [part.part_id], in: processPartIDs) }
            if let owner {
                var expansion = owner.expansions[part.part_id] ?? MacJournalActivityExpansion()
                expansion.setExpanded(newValue)
                owner.expansions[part.part_id] = expansion
            } else { localExpanded = newValue }
        }
    }

    init(reference: MacJournalMessageReference, part: MacAssistantJournalPart,
         tool: MacAssistantJournalMetadata.Tool) {
        self.reference = reference
        self.part = part
        self.tool = tool
        // A failed call must remain visible, but must not open its potentially
        // huge raw input/result or move the user's reading position.
    }

    private func fieldLabel(_ field: String) -> LocalizedStringKey {
        switch field {
        case "tool_input": "Input"
        case "tool_output", "tool_output_text": "Output"
        case "tool_artifacts": "Artifacts"
        case "tool_resources": "Resources"
        case "title": "Title"
        case "tool_summary": "Summary"
        case "permission_domain": "Permission"
        case "permission_rationale": "Reason"
        case "permission_decision": "Decision"
        case "origin_provider_id", "origin_provider_title", "origin_namespace": "Source"
        default: "Details"
        }
    }

    var body: some View {
        DisclosureGroup(isExpanded: Binding(get: { expanded }, set: { expanded = $0 })) {
            if expanded {
                VStack(alignment: .leading, spacing: 10) {
                    if !part.bodies.contains(where: { $0.field == "tool_summary" }) {
                        inlineValue(tool.summary)
                    }
                    ForEach(part.bodies.filter { MacJournalToolFieldPresentation.isResult($0.field) }, id: \.field) { content in
                        MacJournalTextPreview(reference: reference, partID: part.part_id, content: content,
                            isFinal: tool.state != "running" && tool.state != "pending",
                            presentation: .toolField(content.field), usesMonospacedText: false)
                    }
                    DisclosureGroup(isExpanded: Binding(get: { technicalExpanded }, set: { technicalExpanded = $0 })) {
                        if technicalExpanded { technicalDetails }
                    } label: {
                        Text("Technical details", tableName: "Chat")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .disclosureGroupStyle(MacRowDisclosureStyle())
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("chat.journal.tool.technicalDetails")
                }.padding(.top, 6)
            }
        } label: {
            HStack {
                Image(systemName: "wrench.and.screwdriver")
                VStack(alignment: .leading, spacing: 4) {
                    if case .inline(let title) = tool.title {
                        Text(verbatim: MacToolPresentationPolicy.displayTitle(for: title)).lineLimit(2)
                    } else {
                        Text(verbatim: MacToolPresentationPolicy.displayTitle(for: tool.capability_id)).lineLimit(2)
                    }
                    if !expanded, tool.state == "failed", let summary = tool.summary,
                       case .inline(let text) = summary, !text.isEmpty {
                        Text(verbatim: text).lineLimit(2).foregroundStyle(.red)
                    }
                }
                Spacer(minLength: 8)
                MacJournalStatusText(state: tool.state)
            }
            .font(.body)
            .frame(minHeight: 28)
        }
        .disclosureGroupStyle(MacRowDisclosureStyle())
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.journal.tool")
    }

    private var technicalDetails: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(verbatim: tool.capability_id + " · " + tool.tool_call_id)
                .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            GroupBox("Source") {
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: tool.origin.kind)
                    inlineValue(tool.origin.provider_title)
                    inlineValue(tool.origin.provider_id)
                    inlineValue(tool.origin.namespace)
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            if let permission = tool.permission {
                // Display facts only: all decision controls remain on
                // the independently bound live permission challenge.
                GroupBox("Permission") {
                    VStack(alignment: .leading, spacing: 4) {
                        inlineValue(permission.permission_domain)
                        inlineValue(permission.rationale)
                        inlineValue(permission.decision)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            if case .inline(let resources) = tool.affected_resources, !resources.isEmpty {
                GroupBox("Resources") {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(Array(resources.enumerated()), id: \.offset) { _, resource in
                            Text(verbatim: "\(resource.resource_type) · \(resource.operation) · \(resource.uri)")
                                .textSelection(.enabled)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            ForEach(part.bodies.filter { !MacJournalToolFieldPresentation.isResult($0.field) }, id: \.field) { content in
                VStack(alignment: .leading, spacing: 4) {
                    Text(fieldLabel(content.field)).font(.caption).foregroundStyle(.secondary)
                    MacJournalTextPreview(reference: reference, partID: part.part_id, content: content,
                        isFinal: tool.state != "running" && tool.state != "pending",
                        presentation: .toolField(content.field))
                }
            }
            if let fullOutput = tool.full_output {
                GroupBox("Full output") {
                    Text(verbatim: "\(fullOutput.message_id) / \(fullOutput.part_id) / \(fullOutput.tool_call_id)")
                        .font(.caption).textSelection(.enabled)
                }
            }
        }.padding(.top, 6)
    }

    @ViewBuilder private func inlineValue(_ value: MacJournalText?) -> some View {
        if let value, case .inline(let text) = value {
            Text(verbatim: text).textSelection(.enabled)
        }
    }
}

@MainActor
struct MacJournalProcessView<Content: View>: View {
    let partIDs: [String]
    let delivery: MacChatMessage.DeliveryState?
    let requiresAttention: Bool
    let toolCount: Int
    let attentionState: String?
    let hasLiveReasoning: Bool
    let reasoningOnly: Bool
    var settledStatus: String? = nil
    @ViewBuilder var content: () -> Content
    @Environment(\.journalPresentationOwner) private var owner
    @Environment(\.colorSchemeContrast) private var contrast
    @State private var localChoices: [String: Bool] = [:]
    @State private var hovering = false
    private var expanded: Bool {
        AppleMessagePartFlow.isExpanded(choices: partIDs.compactMap { (owner?.processChoices ?? localChoices)[$0] },
            defaultExpanded: hasLiveReasoning)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                let next = !expanded
                if let owner { for id in partIDs { owner.processChoices[id] = next } }
                else { for id in partIDs { localChoices[id] = next } }
            } label: {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.body.weight(.semibold))
                    Group {
                        if reasoningOnly { Text(hasLiveReasoning ? "Thinking" : "Thinking Process") }
                        else { Text("Execution process", tableName: "Chat") }
                    }.fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    if expanded, toolCount > 0 {
                        Label { Text(verbatim: String(toolCount)) } icon: { Image(systemName: "wrench.and.screwdriver") }
                            .accessibilityLabel(Text("Tool calls", tableName: "Chat"))
                            .accessibilityValue(Text(verbatim: String(toolCount)))
                    }
                    if let state = settledStatus ?? MacJournalProcessStatus.primary(delivery: delivery, attention: attentionState) {
                        MacJournalStatusText(state: state)
                    }
                    }
                    if attentionState == "failed", delivery != .failed {
                        Text("Some operations failed", tableName: "Chat")
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.leading, 22)
                    }
                }
                .font(.body)
                .frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .contentShape(Rectangle())
                .background(hovering ? MacDesignColor.hover : Color.clear)
            }
            .buttonStyle(.plain)
            .onHover { hovering = $0 }
            .accessibilityValue(expanded ? Text("Expanded", tableName: "Chat") : Text("Collapsed", tableName: "Chat"))
            .accessibilityIdentifier("chat.journal.process.toggle")
            if expanded {
                VStack(alignment: .leading, spacing: 6) { content() }
                    .padding(.horizontal, 12)
                    .padding(.bottom, 10)
            }
        }
        .background(expanded ? MacDesignColor.surfaceRaised : Color.clear, in: RoundedRectangle(cornerRadius: 12))
        .overlay {
            if requiresAttention || contrast == .increased {
                RoundedRectangle(cornerRadius: 10)
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
            }
        }
        .environment(\.journalProcessPartIDs, partIDs)
        .animation(nil, value: expanded)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.journal.process")
    }
}

@MainActor
private struct MacJournalActivityView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let activity: MacAssistantJournalMetadata.Activity
    let isReasoning: Bool
    var inlineDetails = false
    @Environment(\.journalPresentationOwner) private var owner
    @State private var localExpansion = MacJournalActivityExpansion()
    private var expansion: MacJournalActivityExpansion {
        get { owner?.expansions[part.part_id] ?? localExpansion }
        nonmutating set {
            if let owner { owner.expansions[part.part_id] = newValue }
            else { localExpansion = newValue }
        }
    }
    // The enclosing process owns automatic disclosure. Once opened, its
    // reasoning remains readable without another click for every model round.
    private var expanded: Bool { expansion.userChoice ?? true }

    private var hasDetails: Bool {
        MacJournalActivityTitle.hasDetails(title: activity.title, bodies: part.bodies)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if inlineDetails {
                details
            } else if isReasoning || hasDetails {
                Button {
                    var next = expansion
                    next.setExpanded(!expanded)
                    expansion = next
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: expanded ? "chevron.down" : "chevron.right")
                            .font(.caption.weight(.semibold))
                        activityLabel
                        Spacer(minLength: 0)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 8)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityValue(expanded ? Text("Expanded", tableName: "Chat") : Text("Collapsed", tableName: "Chat"))
                .accessibilityIdentifier(isReasoning ? "chat.journal.reasoning.toggle" : "chat.journal.step.toggle")
                if expanded { details }
            } else {
                activityLabel
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier(isReasoning ? "chat.journal.reasoning" : "chat.journal.step")
        .onChange(of: activity.isRunning, initial: true) { _, running in
            expansion.observe(isRunning: running)
        }
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 8) {
            if case .field(let field) = activity.title,
               let title = part.bodies.first(where: { $0.field == field }) {
                MacJournalTextPreview(reference: reference, partID: part.part_id,
                    content: title, isFinal: !activity.isRunning)
            }
            if let summary = part.bodies.first(where: { $0.field == "reasoning_summary" }) {
                MacJournalTextPreview(reference: reference, partID: part.part_id,
                    content: summary, isFinal: !activity.isRunning)
            }
            if !hasDetails {
                Text(activity.isRunning
                    ? String(localized: "Waiting for thinking content…", table: "Chat")
                    : String(localized: "No thinking details were returned.", table: "Chat"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .padding(.leading, 18)
        .padding(.top, 4)
        .padding(.bottom, 4)
    }

    private var activityLabel: some View {
        HStack(spacing: 6) {
            if activity.isRunning {
                MacBreathingStatusLight(diameter: 10)
            } else {
                Image(systemName: MacJournalActivityTitle.symbol(state: activity.state))
            }
            Text(MacJournalActivityTitle.label(isReasoning: isReasoning, isRunning: activity.isRunning,
                inlineTitle: { if case .inline(let title) = activity.title { return title }; return nil }()))
                .lineLimit(2)
            if ["pending", "waiting_for_permission", "waiting_for_input", "failed", "cancelled"].contains(activity.state) {
                MacJournalStatusText(state: activity.state)
            }
        }
        .font(.body)
        .foregroundStyle(.secondary)
    }
}

@MainActor
private struct MacJournalTextPreview: View {
    let reference: MacJournalMessageReference
    let partID: String
    let content: MacAssistantJournalContentReference
    let isFinal: Bool
    var presentation: MacJournalBodyPresentation = .markdown
    var usesMonospacedText = true
    @Environment(\.journalPresentationOwner) private var owner
    @Environment(\.journalProcessPartIDs) private var processPartIDs
    @State private var localState = MacJournalBodyState()
    @State private var attachedState: MacJournalBodyState?
    @State private var attachedOwnerKey: MacJournalOwnerKey?
    private var state: MacJournalBodyState {
        if attachedOwnerKey == reference.ownerKey, let attachedState { return attachedState }
        return owner?.body(partID: partID, field: content.field) ?? localState
    }
    private var request: MacJournalBodyState.Request {
        .init(reference: reference, partID: partID, content: content, isFinal: isFinal,
              presentation: presentation)
    }

    var body: some View {
        let epoch = owner?.readerEpoch
        return VStack(alignment: .leading, spacing: 8) {
            if let artifacts = state.artifacts {
                MacJournalArtifactList(reference: reference, artifacts: artifacts)
            } else if presentation == .markdown, let projection = state.markdownProjection {
                MacAssistantPresentationView(
                    nodes: projection.presentation.renderedTextNodes,
                    animateNewNodes: projection.hasStreamed,
                    contentIsFinal: projection.presentation.isComplete,
                    previouslyRevealedNodeIDs: state.revealedNodeIDs
                )
            } else if presentation == .markdown {
                MacMarkdownView(
                    markdown: state.text,
                    isStreaming: !isFinal,
                    animatesAppends: false
                )
            } else {
                Text(verbatim: MacJournalPlainPreview.text(state.text))
                    .font(usesMonospacedText ? .system(.caption, design: .monospaced) : .body)
                    .textSelection(.enabled)
            }
            if state.loadState.showsFullReader
                || (presentation != .markdown && MacJournalPlainPreview.text(state.text) != state.text) {
                if !state.text.isEmpty { Text("history.preview.label", tableName: "Chat").font(.caption).foregroundStyle(.secondary) }
                Button {
                    owner?.retainProcessExpansion(partIDs: [partID], in: processPartIDs)
                    if let epoch {
                        owner?.openReader(.body(.init(reference: reference, partID: partID,
                            content: content, presentation: presentation)), epoch: epoch)
                    }
                }
                    label: { Text("history.reader.open", tableName: "Chat") }
                    .buttonStyle(.link)
                    .accessibilityIdentifier("journal-reader-open")
            }
            if let error = state.error {
                Text(error).font(.caption).foregroundStyle(.secondary)
                Button { state.request(request, retry: true) } label: { Image(systemName: "arrow.clockwise") }
                    .accessibilityLabel(Text("Retry"))
            }
        }
        .simultaneousGesture(TapGesture().onEnded { owner?.retainProcessExpansion(partIDs: [partID], in: processPartIDs) })
        .onChange(of: request, initial: true) { _, value in
            if attachedState == nil || attachedOwnerKey != reference.ownerKey {
                attachedState = state
                attachedOwnerKey = reference.ownerKey
            }
            state.request(value)
        }
        .onChange(of: state.markdownProjection?.presentation.renderedTextNodes.map(\.id) ?? [], initial: true) { _, ids in
            state.revealedNodeIDs.formUnion(ids)
        }
        .onDisappear { if owner == nil { localState.cancel() } }
    }
}

@MainActor
private struct MacJournalArtifactList: View {
    private struct ProofReadKey: Hashable {
        let owner: MacJournalOwnerKey
        let retry: Int
    }
    let reference: MacJournalMessageReference
    let artifacts: [MacCorePartArtifact]
    @State private var localOwner: MacJournalOwnerKey?
    @State private var proofFailed = false
    @State private var proofRetry = 0
    @Environment(\.openURL) private var openURL

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(artifacts.enumerated()), id: \.offset) { _, artifact in
                let destination = MacJournalArtifactDestination.resolve(artifact.uri, localJournal: localOwner == reference.ownerKey)
                Button {
                    switch destination {
                    case .web(let url): openURL(url)
                    case .localFile(let url): MacQuickLookController.shared.preview([url])
                    case nil: break
                    }
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Label(artifact.name, systemImage: "doc")
                        if let mime = artifact.mimeType { Text(verbatim: mime).font(.caption).foregroundStyle(.secondary) }
                        if let operation = artifact.operation { Text(verbatim: operation).font(.caption).foregroundStyle(.secondary) }
                    }
                }
                .buttonStyle(.link)
                .disabled(destination == nil)
                .help(artifact.uri)
                .contextMenu {
                    if case .some(.localFile(let url)) = destination {
                        Button("Quick Look") { MacQuickLookController.shared.preview([url]) }
                        Button("Reveal in Finder") { NSWorkspace.shared.activateFileViewerSelecting([url]) }
                    } else if case .some(.web(let url)) = destination {
                        Button("Open Link") { openURL(url) }
                    }
                }
            }
            if proofFailed {
                Button { proofRetry += 1 } label: { Text("Retry") }
                    .buttonStyle(.link)
                    .accessibilityIdentifier("chat.journal.artifact.retry")
            }
        }
        .task(id: ProofReadKey(owner: reference.ownerKey, retry: proofRetry)) {
            let key = reference.ownerKey
            localOwner = nil
            proofFailed = false
            do {
                let local = try await MacJournalTextRead.isLocal(reference)
                try Task.checkCancellation()
                localOwner = local ? key : nil
            } catch {
                // Unproven provenance never opens a local path. Retry is an
                // explicit bounded read, not a query on every stream cursor.
                if !Task.isCancelled { proofFailed = true }
            }
        }
    }
}

@MainActor
struct MacJournalTextReader: View {
    @Environment(\.dismiss) private var dismiss
    let reference: MacJournalMessageReference
    let partID: String
    let content: MacAssistantJournalContentReference
    let presentation: MacJournalBodyPresentation
    @State private var text = ""
    @State private var displayedText = ""
    @State private var artifacts: [MacCorePartArtifact]?
    @State private var offset = "0"
    @State private var complete = false
    @State private var loading = false
    @State private var error: String?
    @State private var pageTask: Task<Void, Never>?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let artifacts {
                    MacJournalArtifactList(reference: reference, artifacts: artifacts)
                } else if presentation == .markdown {
                    MacMarkdownView(markdown: displayedText, isStreaming: !complete, animatesAppends: !complete)
                } else {
                    Text(verbatim: displayedText).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                }
                if !complete, presentation != .markdown {
                    Text("\(offset) / \(content.utf8_bytes) bytes").font(.caption).foregroundStyle(.secondary)
                }
                if !complete {
                    Button { pageTask = Task { await loadNext() } } label: {
                        Label { Text("history.reader.next", tableName: "Chat") }
                            icon: { Image(systemName: "chevron.down") }
                    }
                        .disabled(loading)
                        .accessibilityIdentifier("journal-reader-next")
                }
                if let error {
                    Text(error).font(.caption).foregroundStyle(.secondary)
                        .accessibilityIdentifier("journal-reader-error")
                }
            }.padding(24)
        }
        .safeAreaInset(edge: .bottom) {
            HStack {
                Spacer()
                Button("Close") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .accessibilityIdentifier("journal-reader-close")
            }
            .padding(12)
            .background(.bar)
        }
        .frame(minWidth: 600, minHeight: 400)
        .task { await loadNext() }
        .onDisappear { pageTask?.cancel(); pageTask = nil }
    }

    private func loadNext() async {
        guard !loading, !complete else { return }
        loading = true
        defer { loading = false }
        do {
            guard content.utf8_bytes != "0" else {
                displayedText = try presentation.display("", complete: true)
                complete = true
                error = nil
                return
            }
            let range = try await MacJournalTextRead.range(reference, partID: partID, content: content, offset: offset)
            try Task.checkCancellation()
            let next = text + range.text
            let finished = range.end_of_content
            let display = try presentation.display(next, complete: finished)
            let decodedArtifacts = finished && content.field == "tool_artifacts"
                ? try JSONDecoder().decode([MacCorePartArtifact].self, from: Data(display.utf8)) : nil
            text = next
            displayedText = display
            artifacts = decodedArtifacts
            offset = range.next_offset_bytes
            complete = finished
            error = nil
        } catch {
            if !Task.isCancelled { self.error = error.localizedDescription }
        }
    }
}

/// Explicit parent control; the child transcript stays read-only and Core owns every transition.
private struct MacOwnedTaskRecoveryView: View {
    let reference: MacJournalMessageReference
    let taskID: String
    @State private var inspection: MacUnknownEffectRecoveryInspection?
    @State private var pending: MacUnknownEffectRecoveryDecision?
    @State private var reason = ""
    @State private var status: String?
    @State private var busy = false
    @State private var confirming = false
    private var owner: AppleOwnedTaskRecoveryOwner {
        .init(parent_conversation_id: reference.cursor.identity.conversation_id,
              parent_run_id: reference.cursor.identity.run_id, task_id: taskID)
    }
    private func control() throws -> MacUnknownEffectRecoveryControl {
        let port = try MacApplicationPort(databaseURL: reference.databaseURL)
        switch try port.conversationSourceOwner(conversationID: owner.parent_conversation_id) {
        case .local: break
        case .remote: throw MacApplicationServiceError.rejected(String(localized: "task.recovery.source_host", table: "Chat"))
        case .unavailable: throw MacApplicationServiceError.invalidResponse
        }
        return try MacUnknownEffectRecoveryControl(client: MacApplicationServiceClient(databaseURL: reference.databaseURL))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button(String(localized: "Check execution status", table: "Chat")) { check() }
            if pending != nil {
                Text(String(localized: "A recovery decision is awaiting its receipt. Only that original decision can be recovered.", table: "Chat"))
                Button(String(localized: "Recover decision receipt", table: "Chat")) { resolve() }
            } else if let inspection {
                Text(String(localized: "These operations may already have taken effect. Checking status does not run them again.", table: "Chat"))
                TextField(String(localized: "Reason for this recovery decision", table: "Chat"), text: $reason, axis: .vertical)
                    .textFieldStyle(.roundedBorder).lineLimit(2...4)
                Button(String(localized: "Keep unknown and close as blocked", table: "Chat")) { confirming = true }
                    .disabled(inspection.targets.isEmpty || inspection.independentPendingEffects != 0
                              || reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                if inspection.independentPendingEffects != 0 {
                    Text(String(localized: "Other work is still pending. This task cannot be closed as blocked yet.", table: "Chat"))
                }
            }
            if let status { Text(status).font(.callout).foregroundStyle(.secondary) }
            if busy { ProgressView().controlSize(.small) }
        }.disabled(busy).accessibilityIdentifier("chat.task.recovery")
        .confirmationDialog(String(localized: "Keep unknown and close as blocked", table: "Chat"),
                            isPresented: $confirming, titleVisibility: .visible) {
            Button(String(localized: "Keep unknown and close as blocked", table: "Chat"), role: .destructive) { resolve() }
            Button(String(localized: "Cancel", table: "Chat"), role: .cancel) {}
        } message: {
            Text(String(localized: "Close this task as blocked without retrying. External execution may still be running; this does not undo or confirm the operation.", table: "Chat"))
        }
    }
    private func check() {
        busy = true
        Task { @MainActor in
            defer { busy = false }
            do {
                let controller = try control()
                inspection = try controller.inspectOwnedTask(owner)
                pending = try controller.pendingOwnedDecision(owner)
                status = inspection == nil && pending == nil ? String(localized: "task.recovery.none", table: "Chat") : nil
            } catch { status = error.localizedDescription }
        }
    }
    private func resolve() {
        busy = true
        Task { @MainActor in
            defer { busy = false }
            do {
                let controller = try control()
                let reply: MacUnknownEffectRecoveryReply?
                if let pending {
                    guard try controller.pendingOwnedDecision(owner) == pending else { throw MacApplicationServiceError.invalidResponse }
                    reply = try controller.recoverPending(runID: pending.runID, taskOwner: owner)
                } else {
                    guard let frozen = inspection, try controller.inspectOwnedTask(owner) == frozen else { throw MacApplicationServiceError.invalidResponse }
                    let decision = MacUnknownEffectRecoveryDecision(conversationID: frozen.conversationID,
                        runID: frozen.runID, invocationConfigDigest: frozen.invocationConfigDigest,
                        resolutionID: UUID().uuidString.lowercased(), targets: frozen.targets,
                        choice: .init(kind: .keepUnknownBlocked, reason: reason),
                        resolvedAtMs: Int64(Date().timeIntervalSince1970 * 1000))
                    reply = try controller.submitConfirmed(decision, assistantMessageID: frozen.assistantMessageID, taskOwner: owner)
                }
                guard reply?.outcome == .unknownPreservedBlocked else { throw MacApplicationServiceError.invalidResponse }
                inspection = nil; pending = nil
                status = String(localized: "The task is closed as blocked. Its earlier operations still have unknown results.", table: "Chat")
            } catch {
                status = error.localizedDescription
                pending = try? control().pendingOwnedDecision(owner)
            }
        }
    }
}
