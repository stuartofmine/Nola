// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import Combine
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
private func appleUIHasPresentedSheet() -> Bool {
    #if os(iOS)
    return UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState == .foregroundActive }.flatMap(\.windows)
        .contains { $0.rootViewController?.presentedViewController != nil }
    #elseif os(macOS)
    return NSApp.modalWindow != nil || NSApp.windows.contains { $0.attachedSheet != nil || $0.sheetParent != nil }
    #else
    return false
    #endif
}

@MainActor
private func appleUIHasMarkedText() -> Bool {
    #if os(iOS)
    func marked(_ view: UIView) -> Bool {
        if view.isFirstResponder, let input = view as? UITextInput { return input.markedTextRange != nil }
        return view.subviews.contains(where: marked)
    }
    return UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState == .foregroundActive }.flatMap(\.windows).contains(where: marked)
    #elseif os(macOS)
    return NSApp.windows.contains { ($0.firstResponder as? NSTextView)?.hasMarkedText() == true }
    #else
    return false
    #endif
}

/// Native composition is transient renderer state. One observer is shared by
/// mounted cards; it never evaluates form rules or persists user input.
@MainActor
private final class AppleUICompositionMonitor: ObservableObject {
    static let shared = AppleUICompositionMonitor()
    @Published private(set) var generation: UInt64 = 0
    private(set) var isComposing = false
    private var readers = 0
    private var timer: AnyCancellable?
    func acquire() {
        readers += 1
        refresh()
        guard timer == nil else { return }
        timer = Timer.publish(every: 0.08, on: .main, in: .common).autoconnect().sink { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }
    func release() {
        readers = max(0, readers - 1)
        if readers == 0 { timer = nil }
    }
    private func refresh() {
        let current = appleUIHasMarkedText()
        guard current != isComposing else { return }
        isComposing = current
        generation &+= 1
    }
}

nonisolated enum AppleUISubmissionOutcome<JSON: JournalJSONValue>: Sendable {
    case committed(AppleUIInteractionReceipt<JSON>)
    case rejected(String)
    case unknown(String)
}

/// Process-local editing state is keyed by the complete Core owner tuple. It is
/// retained when SwiftUI virtualizes a row or navigation destroys its view.
/// It contains only uncommitted edits and receipt identity, never UI authority.
@MainActor
private final class AppleUIEditingRegistry {
    static let shared = AppleUIEditingRegistry()
    private var entries: [AppleUISurfaceIdentity: AnyObject] = [:]
    func state<JSON: JournalJSONValue>(for identity: AppleUISurfaceIdentity) -> AppleUIEditingState<JSON> {
        if let state = entries[identity] as? AppleUIEditingState<JSON> { return state }
        let state = AppleUIEditingState<JSON>()
        entries[identity] = state
        return state
    }
}

@MainActor
final class AppleUIEditingState<JSON: JournalJSONValue>: ObservableObject {
    @Published var drafts: [String: JSON] = [:]
    @Published var draftRevision: UInt64?
    @Published var draftGeneration: UInt64 = 0
    @Published var preview: AppleUIPreviewResult<JSON>?
    @Published var pending = false
    @Published var unresolved: AppleUISubmission<JSON>?
    @Published var failure: String?
    @Published var receipt: AppleUIInteractionReceipt<JSON>?

    static func key(_ node: String, _ binding: String) -> String { "\(node.utf8.count):\(node)\(binding)" }

    /// Only an explicit user retry may rebase retained drafts. Removed fields
    /// must remain visible as a conflict rather than being silently discarded.
    func revalidate(against snapshot: AppleUISnapshot<JSON>) {
        guard !pending, unresolved == nil, snapshot.phase == .ready else { return }
        preview = nil
        let available = Set(snapshot.nodes.values.flatMap { node in
            node.bindings.keys.map { Self.key(node.instance_id, $0) }
        })
        guard Set(drafts.keys).isSubset(of: available) else {
            failure = "表单已更新，部分编辑项不再存在。请放弃编辑后重新填写。"
            return
        }
        draftRevision = snapshot.revision
        failure = nil
        draftGeneration &+= 1
    }
}

/// One generated modal at a time across all cards/windows. Permission and host
/// system sheets remain owned by their existing presentation coordinators.
@MainActor
private final class AppleUIModalCoordinator {
    static let shared = AppleUIModalCoordinator()
    var owner: UUID?
    func claim(_ id: UUID) -> Bool {
        guard !appleUIHasPresentedSheet(), owner == nil || owner == id else { return false }
        owner = id
        return true
    }
    func release(_ id: UUID) { if owner == id { owner = nil } }
}

/// Mount with `.id(snapshot.identity)`; never use revision as the view identity.
struct AppleUISurfaceView<JSON: JournalJSONValue>: View {
    let snapshot: AppleUISnapshot<JSON>
    let preview: ((AppleUIPreviewInput<JSON>) async throws -> AppleUIPreviewResult<JSON>)?
    let submit: (AppleUISubmission<JSON>) async -> AppleUISubmissionOutcome<JSON>
    let accepted: (AppleUIInteractionReceipt<JSON>) -> Void
    let pendingSubmission: () throws -> AppleUISubmission<JSON>?
    @StateObject private var editing: AppleUIEditingState<JSON>
    @State private var presentationID = UUID()
    @ObservedObject private var composition = AppleUICompositionMonitor.shared

    init(snapshot: AppleUISnapshot<JSON>,
         preview: ((AppleUIPreviewInput<JSON>) async throws -> AppleUIPreviewResult<JSON>)? = nil,
         submit: @escaping (AppleUISubmission<JSON>) async -> AppleUISubmissionOutcome<JSON>,
         pendingSubmission: @escaping () throws -> AppleUISubmission<JSON>? = { nil },
         accepted: @escaping (AppleUIInteractionReceipt<JSON>) -> Void) {
        self.snapshot = snapshot
        self.preview = preview
        self.submit = submit
        self.accepted = accepted
        self.pendingSubmission = pendingSubmission
        _editing = StateObject(wrappedValue: snapshot.phase == .readOnly || snapshot.phase == .deleted
            ? AppleUIEditingState<JSON>() : AppleUIEditingRegistry.shared.state(for: snapshot.identity))
    }
    @State private var modal: ModalIdentity?

    private struct ModalIdentity: Identifiable { let id: String }
    private var disabled: Bool { editing.pending || editing.unresolved != nil || snapshot.phase != .ready }

    private struct PreviewKey: Hashable {
        let identity: AppleUISurfaceIdentity
        let revision: UInt64
        let generation: UInt64
        let compositionGeneration: UInt64
        let phase: String
        let pending: Bool
        let unresolvedID: String?
    }
    private var previewKey: PreviewKey { .init(identity: snapshot.identity, revision: snapshot.revision,
        generation: editing.draftGeneration, compositionGeneration: composition.generation, phase: snapshot.phase.rawValue,
        pending: editing.pending, unresolvedID: editing.unresolved?.submission_id) }
    private var previewReady: Bool {
        guard snapshot.phase == .ready, let value = editing.preview else { return false }
        return value.matches(identity: snapshot.identity, revision: snapshot.revision, generation: editing.draftGeneration)
            && value.snapshot.catalog_id == snapshot.catalog_id && value.snapshot.catalog_digest == snapshot.catalog_digest
    }
    private var displaySnapshot: AppleUISnapshot<JSON> { previewReady ? editing.preview!.snapshot : snapshot }
    private var actionsDisabled: Bool { composition.isComposing || (!editing.drafts.isEmpty && !previewReady) }
    private var draftEdits: [AppleUIEdit<JSON>] {
        snapshot.nodes.values.flatMap { node in node.bindings.keys.compactMap { binding in
            editing.drafts[key(node.instance_id, binding)].map { .set(instanceID: node.instance_id, binding: binding, value: $0) }
        } }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let root = snapshot.root_instance_id { node(root).disabled(disabled) }
            else if !snapshot.summary.isEmpty { Text(snapshot.summary) }
            if editing.pending { ProgressView().controlSize(.small) }
            if let failure = editing.failure { Label(failure, systemImage: "exclamationmark.circle").foregroundStyle(.secondary) }
            if !editing.pending && editing.unresolved == nil && !editing.drafts.isEmpty && (editing.failure != nil || editing.draftRevision != snapshot.revision) {
                Button("重新校验") {
                    editing.revalidate(against: snapshot)
                }.disabled(composition.isComposing)
                Button("放弃编辑，使用当前值") {
                    editing.drafts = [:]
                    editing.draftRevision = nil
                    editing.draftGeneration &+= 1
                    editing.preview = nil
                    editing.failure = nil
                }
            }
            if let unresolved = editing.unresolved {
                Button("查询提交结果") { Task { await deliver(unresolved) } }.disabled(editing.pending)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(snapshot.identity.accessibilityIdentifier)
        .onReceive(editing.$receipt) { receipt in
            if let receipt, receipt.snapshot.identity == snapshot.identity,
               receipt.snapshot.revision > snapshot.revision { accepted(receipt) }
        }
        .onAppear { composition.acquire() }
        .onChange(of: composition.generation) { _, _ in
            guard !editing.drafts.isEmpty, !editing.pending else { return }
            editing.preview = nil
            editing.draftGeneration &+= 1
        }
        .task(id: previewKey) {
            let captured = previewKey
            guard snapshot.phase == .ready, !editing.drafts.isEmpty, editing.unresolved == nil, !editing.pending else { return }
            do {
                try await Task.sleep(for: .milliseconds(180))
                while appleUIHasMarkedText() { try await Task.sleep(for: .milliseconds(80)) }
                try Task.checkCancellation()
                guard editing.draftRevision == snapshot.revision, let preview else {
                    editing.failure = "表单已更新或暂时无法校验，请重新读取后编辑。"
                    return
                }
                let result = try await preview(.init(identity: captured.identity,
                    expected_surface_revision: captured.revision, edits: draftEdits, draft_generation: captured.generation))
                try Task.checkCancellation()
                guard captured == previewKey else { return }
                guard !appleUIHasMarkedText() else {
                    editing.preview = nil
                    editing.draftGeneration &+= 1
                    return
                }
                guard result.matches(identity: captured.identity, revision: captured.revision, generation: captured.generation),
                      result.snapshot.catalog_id == snapshot.catalog_id,
                      result.snapshot.catalog_digest == snapshot.catalog_digest else {
                    editing.preview = nil
                    editing.failure = "表单校验结果已过期或归属不匹配，请重新编辑后重试。"
                    return
                }
                editing.preview = result
                editing.failure = nil
            } catch is CancellationError { }
            catch {
                guard captured == previewKey else { return }
                editing.preview = nil
                editing.failure = error.localizedDescription
            }
        }
        .onDisappear { modal = nil; AppleUIModalCoordinator.shared.release(presentationID); composition.release() }
        .sheet(item: $modal, onDismiss: { AppleUIModalCoordinator.shared.release(presentationID) }) { selection in
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Spacer()
                    Button("关闭") { modal = nil }.keyboardShortcut(.cancelAction)
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        ForEach(snapshot.nodes[selection.id]?.slots["content"] ?? [], id: \.self) { child in
                            node(child).disabled(disabled)
                        }
                    }
                }
            }.padding(20)
        }
    }

    private func node(_ id: String) -> some View {
        AppleUINodeView(id: id, snapshot: displaySnapshot,
            value: { node, binding, committed in editing.drafts[key(node, binding)] ?? committed },
            edit: { node, binding, value in
                guard !disabled, snapshot.nodes[node]?.bindings[binding] != nil else { return }
                editing.draftRevision = editing.draftRevision ?? snapshot.revision
                editing.drafts[key(node, binding)] = value
                editing.draftGeneration &+= 1
                editing.preview = nil
            }, dispatch: { action in
                guard !disabled, !actionsDisabled else { return }
                guard !appleUIHasMarkedText() else {
                    editing.failure = "请先确认正在输入的文字，再提交表单。"
                    return
                }
                let available = Set(snapshot.nodes.values.flatMap { node in
                    node.bindings.keys.map { key(node.instance_id, $0) }
                })
                guard Set(editing.drafts.keys).isSubset(of: available) else {
                    editing.failure = "表单已更新，部分编辑项不再存在。请重新确认后提交。"
                    return
                }
                // Only the advertised binding names are submitted, never paths.
                let edits: [AppleUIEdit<JSON>] = snapshot.nodes.values.flatMap { node in
                    node.bindings.keys.compactMap { binding in
                        editing.drafts[key(node.instance_id, binding)].map {
                            .set(instanceID: node.instance_id, binding: binding, value: $0)
                        }
                    }
                }
                let submission = AppleUISubmission(interaction: AppleUIInteraction(identity: snapshot.identity,
                    expected_surface_revision: editing.draftRevision ?? snapshot.revision, edits: edits, action: action),
                    submission_id: UUID().uuidString, submitted_at_ms: Int64(Date().timeIntervalSince1970 * 1000))
                Task { await deliver(submission) }
            }, present: { id in
                if modal == nil, AppleUIModalCoordinator.shared.claim(presentationID) { modal = ModalIdentity(id: id) }
            }, actionsDisabled: actionsDisabled)
    }

    private func key(_ node: String, _ binding: String) -> String { AppleUIEditingState<JSON>.key(node, binding) }
    @MainActor private func deliver(_ submission: AppleUISubmission<JSON>) async {
        guard !editing.pending, submission.interaction.identity == snapshot.identity else { return }
        editing.pending = true
        editing.failure = nil
        defer { editing.pending = false }
        let outcome = await submit(submission)
        guard submission.interaction.identity == snapshot.identity else { return }
        switch outcome {
        case let .committed(receipt):
            guard receipt.snapshot.identity == snapshot.identity else {
                editing.unresolved = submission
                editing.failure = "提交结果归属不匹配，请重查。"
                return
            }
            editing.drafts = [:]
            editing.draftRevision = nil
            editing.draftGeneration &+= 1
            editing.preview = nil
            editing.unresolved = nil
            editing.receipt = receipt
            accepted(receipt)
        case let .rejected(message):
            editing.unresolved = nil
            editing.preview = nil
            editing.failure = message
        case let .unknown(message):
            editing.unresolved = (try? pendingSubmission()) ?? submission
            editing.failure = message
        }
    }
}

/// Historical content is loaded only after an explicit user request. It is never
/// substituted with the latest mutable Surface when a revision is unavailable.
struct AppleUIHistoryView<JSON: JournalJSONValue>: View {
    let reference: AppleUISurfaceReference
    let load: (AppleUISurfaceReference) async throws -> AppleUISnapshot<JSON>?
    @State private var expanded = false
    @State private var snapshot: AppleUISnapshot<JSON>?
    @State private var failure: String?
    @State private var loading = false

    var body: some View {
        DisclosureGroup(String(localized: "a2ui.history.version", defaultValue: "View this form version"), isExpanded: $expanded) {
            if loading { ProgressView() }
            if let failure { Text(failure).foregroundStyle(.secondary) }
            if let snapshot {
                AppleUISurfaceView(snapshot: snapshot,
                    submit: { _ in .rejected("历史表单不可提交。") }, accepted: { _ in })
            }
        }
        .task(id: expanded ? reference : nil) {
            guard expanded else { return }
            loading = true
            failure = nil
            defer { loading = false }
            do {
                let result = try await load(reference)
                try Task.checkCancellation()
                guard let result else { failure = "此版本不可用。"; return }
                guard result.identity == reference.identity, result.revision == reference.revision,
                      result.phase == .readOnly || result.phase == .deleted,
                      result.nodes.values.allSatisfy({ $0.actions.isEmpty && $0.bindings.isEmpty }) else {
                    failure = "历史表单版本不匹配。"
                    return
                }
                snapshot = result
            } catch is CancellationError { }
            catch { failure = error.localizedDescription }
        }
        .id(reference)
    }
}

/// Journal references trigger bounded per-Surface reads. The client only applies
/// resolved node replacements; expressions and data bindings stay in Core.
struct AppleUIReferencedSurfaceView<JSON: JournalJSONValue>: View {
    let reference: AppleUISurfaceReference
    let summary: String
    let readChanges: (AppleUISurfaceIdentity, UInt64?) async throws -> AppleUISurfaceUpdate<JSON>
    let preview: (AppleUIPreviewInput<JSON>) async throws -> AppleUIPreviewResult<JSON>
    let readRevision: (AppleUISurfaceReference) async throws -> AppleUISnapshot<JSON>?
    let pendingSubmission: (AppleUISurfaceIdentity) throws -> AppleUISubmission<JSON>?
    let submit: (AppleUISubmission<JSON>) async -> AppleUISubmissionOutcome<JSON>
    @State private var snapshot: AppleUISnapshot<JSON>?
    @State private var failure: String?
    @State private var retry = 0
    @State private var intentReady = false
    @State private var pending: AppleUISubmission<JSON>?
    private struct ReadKey: Hashable { let reference: AppleUISurfaceReference; let retry: Int }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let snapshot {
                AppleUISurfaceView(snapshot: snapshot, preview: preview, submit: submit, pendingSubmission: { try pendingSubmission(reference.identity) }) { receipt in
                    guard receipt.snapshot.identity == reference.identity,
                          receipt.snapshot.revision >= (self.snapshot?.revision ?? 0) else { return }
                    self.snapshot = receipt.snapshot
                }.id(snapshot.identity).disabled(!intentReady)
            } else { Text(summary).foregroundStyle(.secondary) }
            if snapshot == nil, let pending, intentReady {
                Button("查询上次表单提交结果") {
                    Task {
                        let result = await submit(pending)
                        if case .committed(let receipt) = result { snapshot = receipt.snapshot; self.pending = nil }
                        else if case .rejected(let message) = result { self.pending = nil; failure = message }
                        else if case .unknown(let message) = result { failure = message }
                    }
                }
            }
            if let failure {
                Text(failure).font(.caption).foregroundStyle(.secondary)
                Button("重新读取表单") { retry &+= 1 }
            }
            AppleUIHistoryView(reference: reference, load: readRevision)
        }
        .task(id: ReadKey(reference: reference, retry: retry)) {
            intentReady = false
            do {
                let state: AppleUIEditingState<JSON> = AppleUIEditingRegistry.shared.state(for: reference.identity)
                pending = try pendingSubmission(reference.identity)
                if let pending {
                    state.unresolved = pending
                    state.draftRevision = pending.interaction.expected_surface_revision
                    for edit in pending.interaction.edits {
                        if case let .set(instanceID, binding, value) = edit {
                            state.drafts["\(instanceID.utf8.count):\(instanceID)\(binding)"] = value
                        }
                    }
                    state.failure = "上次表单提交尚未确认，请查询原提交结果。"
                }
                intentReady = true
                let base = snapshot
                let update = try await readChanges(reference.identity, base?.revision)
                try Task.checkCancellation()
                // An interaction receipt may have advanced the cache while a
                // journal query was in flight. Never apply its stale base.
                guard snapshot?.revision == base?.revision else { return }
                do {
                    snapshot = try update.applying(to: base, identity: reference.identity)
                } catch {
                    // An unusable delta must not mutate the last committed
                    // projection. Re-read a complete snapshot once.
                    let complete = try await readChanges(reference.identity, nil)
                    try Task.checkCancellation()
                    guard snapshot?.revision == base?.revision else { return }
                    let replacement = try complete.applying(to: nil, identity: reference.identity)
                    guard replacement == nil || (replacement?.revision ?? 0) >= (base?.revision ?? 0) else { throw error }
                    snapshot = replacement
                }
                failure = nil
            } catch is CancellationError { }
            catch { failure = error.localizedDescription }
        }
    }
}
