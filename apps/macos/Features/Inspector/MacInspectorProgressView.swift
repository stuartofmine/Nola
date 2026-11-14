// SPDX-License-Identifier: Apache-2.0
// Current Core facts only; work reports never imply verified completion.
import SwiftUI
import Observation

// Refreshes are not navigations: retain visible facts until their replacement arrives.
nonisolated struct MacInspectorRefreshState {
    private(set) var owner: String?
    private(set) var loaded = false
    private(set) var failed = false
    private(set) var controlsConfirmed = false

    mutating func begin(owner next: String?) -> Bool {
        let changed = owner != next
        if changed { owner = next; loaded = false; controlsConfirmed = false }
        failed = false
        return changed
    }

    mutating func finish(succeeded: Bool) {
        if succeeded { loaded = true }
        failed = !succeeded
        controlsConfirmed = succeeded
    }
}

nonisolated struct MacInspectorProgressSnapshot: Decodable, Equatable, Sendable {
    struct Todo: Decodable, Equatable, Identifiable, Sendable {
        let id: String
        let content: String
        let status: String
    }
    struct Tasks: Decodable, Equatable, Sendable {
        struct Item: Decodable, Equatable, Identifiable, Sendable {
            let id: String
            let owner_session_id: String
            let state: String
            let current_step: String?
        }
        let order: [String]
        let tasks: [String: Item]
    }
    struct Requirement: Decodable, Equatable, Identifiable, Sendable {
        let id: String
        let description: String
        let status: String
    }
    struct Work: Decodable, Equatable, Sendable {
        struct Entry: Decodable, Equatable, Identifiable, Sendable {
            let id: String
            let content: String
            let progress: String?
            let supersedes: String?
        }
        let entries: [Entry]
        var current: [Entry] {
            let replaced = Set(entries.compactMap(\.supersedes))
            return entries.filter { !replaced.contains($0.id) }
        }
    }
    struct Goal: Decodable, Equatable, Sendable {
        let id: String
        let objective: String
        let status: String
        let requirements: [String: Requirement]
        let work: Work
    }
    let active: Goal?
    let can_submit: Bool?
    let can_pause: Bool
    let can_resume: Bool
    let can_cancel: Bool
    let control_token: String?
    let applied_operations: [String: AppleGoalOperation]?
}

// Drafts survive disclosure removal, and belong to a conversation rather than
// whichever conversation happens to be selected when an asynchronous action ends.
@MainActor @Observable
final class MacInspectorGoalDraft {
    var objective = ""
    var constraints = ""
    var wakeAt = Date().addingTimeInterval(3600)
    var isSubmitting = false
    var error: String?
}

@MainActor
final class MacInspectorGoalDraftStore {
    private var values: [String: MacInspectorGoalDraft] = [:]
    func draft(for owner: String) -> MacInspectorGoalDraft {
        if let value = values[owner] { return value }
        let value = MacInspectorGoalDraft()
        values[owner] = value
        return value
    }
}

nonisolated enum MacInspectorGoalControlPolicy {
    static func canStart(_ snapshot: MacInspectorProgressSnapshot?) -> Bool {
        guard snapshot?.can_submit != false else { return false }
        guard let active = snapshot?.active else { return true }
        return ["completed", "cancelled"].contains(active.status)
    }

    static func canControl(_ snapshot: MacInspectorProgressSnapshot?) -> Bool {
        guard let active = snapshot?.active, !["completed", "cancelled"].contains(active.status),
              let token = snapshot?.control_token, !token.isEmpty else { return false }
        return true
    }
}

enum MacInspectorProgressMode { case progress, goal, execution }

struct MacInspectorProgressView: View {
    @Bindable var model: MacWorkspaceModel
    var mode: MacInspectorProgressMode = .progress
    @State private var refresh = MacInspectorRefreshState()
    @State private var snapshot: MacInspectorProgressSnapshot?
    @State private var todos: [MacInspectorProgressSnapshot.Todo] = []
    @State private var tasks: [MacInspectorProgressSnapshot.Tasks.Item] = []
    @State private var retry = 0
    var draftStore: MacInspectorGoalDraftStore? = nil
    @State private var localDrafts = MacInspectorGoalDraftStore()
    @State private var goalToCancel: AppleGoalControl?
    private var draft: MacInspectorGoalDraft { (draftStore ?? localDrafts).draft(for: model.selectedConversationID ?? "") }
    private var objective: String {
        get { draft.objective }
        nonmutating set { draft.objective = newValue }
    }
    private var constraints: String {
        get { draft.constraints }
        nonmutating set { draft.constraints = newValue }
    }
    private var controlBusy: Bool { draft.isSubmitting }
    @State private var controlPending = false
    private var controlError: String? {
        get { draft.error }
        nonmutating set { draft.error = newValue }
    }
    private var wakeAt: Date {
        get { draft.wakeAt }
        nonmutating set { draft.wakeAt = newValue }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if model.selectedConversationID == nil {
                Text(String(localized: "inspector.conversation_needed", table: "ConversationInspector"))
                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            } else if refresh.owner != model.selectedConversationID || (!refresh.loaded && !refresh.failed) {
                ProgressView().accessibilityIdentifier("mac-inspector-progress-loading")
            } else {
              if refresh.failed {
                Text("inspector.failed", tableName: "ConversationInspector")
                    .font(.callout).foregroundStyle(.secondary)
                Button(String(localized: "Retry")) { retry += 1 }
              }
              if refresh.loaded {
                  if mode == .goal {
                      goalControls.disabled(!refresh.controlsConfirmed || refresh.owner != model.selectedConversationID)
                  }
                  if mode == .execution && tasks.isEmpty {
                      Text("inspector.state.idle", tableName: "ConversationInspector")
                          .font(.callout).foregroundStyle(.secondary)
                  } else if mode != .goal || snapshot?.active != nil {
                      MacInspectorProgressContent(snapshot: mode == .goal ? snapshot : nil,
                          todos: mode == .progress ? todos : [], tasks: mode == .execution ? tasks : [])
                          .equatable()
                  }
              }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .task(id: "\(model.selectedConversationID ?? ""):\(model.inspectorSelectedConversationIsRunning):\(model.selectedMessages.count):\(retry)") {
            let requestedOwner = model.selectedConversationID
            if refresh.begin(owner: requestedOwner) {
                snapshot = nil; todos = []; tasks = []
                controlPending = false
            }
            guard let requestedOwner else { refresh.finish(succeeded: true); return }
            repeat {
            do {
                let projection = try await model.inspectorSessionProjection(owner: requestedOwner)
                let plan = try await model.inspectorConversationPlan(owner: requestedOwner)
                guard projection.map({ $0.conversationID == requestedOwner }) ?? true,
                      plan.map({ $0.conversationID == requestedOwner }) ?? true else {
                    throw MacApplicationPortError.invalidResponse
                }
                let decodedTodos = try plan.map { try JSONDecoder().decode(
                    [MacInspectorProgressSnapshot.Todo].self, from: JSONEncoder().encode($0.todos)) } ?? []
                let taskProjection = try projection.map { try JSONDecoder().decode(
                    MacInspectorProgressSnapshot.Tasks.self, from: $0.taskProjection) }
                let previousPending = try await model.pendingGoalControl(conversationID: requestedOwner)
                let decoded = try await model.inspectorGoalSnapshot(conversationID: requestedOwner)
                let pending = try await model.pendingGoalControl(conversationID: requestedOwner)
                guard !Task.isCancelled, model.selectedConversationID == requestedOwner else { return }
                if let previousPending, pending == nil,
                   let receipt = decoded?.applied_operations?[previousPending.control.operationId], receipt.rejection == nil {
                    // The source receipt resolves a lost response; do not keep
                    // its obsolete transport error beside confirmed controls.
                    controlError = nil
                }
                controlPending = pending != nil
                if let pending, pending.control.action == .start, objective.isEmpty {
                    objective = pending.control.objective ?? ""
                    constraints = pending.control.constraints?.joined(separator: "\n") ?? ""
                }
                guard !Task.isCancelled, model.selectedConversationID == requestedOwner else { return }
                let decodedTasks = taskProjection.map { value in value.order.compactMap { value.tasks[$0] }
                    .filter { $0.owner_session_id == requestedOwner } } ?? []
                // Identical polling results must not invalidate the visible content.
                if snapshot != decoded { snapshot = decoded }
                if todos != decodedTodos { todos = decodedTodos }
                if tasks != decodedTasks { tasks = decodedTasks }
                refresh.finish(succeeded: true)
            } catch {
                guard !Task.isCancelled, model.selectedConversationID == requestedOwner else { return }
                refresh.finish(succeeded: false)
                return
            }
            // Message and execution changes still request an immediate refresh.
            do { try await Task.sleep(for: .seconds(model.inspectorSelectedConversationIsRunning ? 3 : 15)) } catch { return }
            } while !Task.isCancelled && model.selectedConversationID == requestedOwner
        }
    }

    @ViewBuilder private var goalControls: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let error = controlError { Text(error).font(.caption).foregroundStyle(.red) }
            if controlPending {
                Text("Goal action awaiting confirmation").font(.caption).foregroundStyle(.secondary)
                Button("Retry saved action") { submit(nil) }.disabled(controlBusy).accessibilityIdentifier("mac-goal-retry")
            } else if MacInspectorGoalControlPolicy.canControl(snapshot),
                      let goal = snapshot?.active, let token = snapshot?.control_token {
                HStack {
                    if snapshot?.can_pause == true {
                        Button("Pause") { submit(.init(action: .pause, expectedGoalId: goal.id, expectedControlToken: token)) }.accessibilityIdentifier("mac-goal-pause")
                    }
                    if snapshot?.can_resume == true {
                        Button("Resume") { submit(.init(action: .resume, expectedGoalId: goal.id, expectedControlToken: token)) }.accessibilityIdentifier("mac-goal-resume")
                    }
                    if snapshot?.can_cancel == true {
                        Button("Cancel", role: .destructive) { goalToCancel = .init(action: .cancel, expectedGoalId: goal.id, expectedControlToken: token) }.accessibilityIdentifier("mac-goal-cancel")
                    }
                }.disabled(controlBusy)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Resume at").font(.caption).foregroundStyle(.secondary)
                    DatePicker("Resume at", selection: Binding(get: { wakeAt }, set: { wakeAt = $0 }), in: Date()...).labelsHidden()
                }
                Button("Schedule resume") {
                    submit(.init(action: .schedule, expectedGoalId: goal.id, expectedControlToken: token,
                        wakeAtEpochMs: Int64(wakeAt.timeIntervalSince1970 * 1000)))
                }.disabled(controlBusy)
            } else if MacInspectorGoalControlPolicy.canStart(snapshot) {
                Text(String(localized: "goal.guidance", table: "ConversationInspector"))
                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                MacInspectorDocumentEditor(text: Binding(get: { objective }, set: { objective = $0 }), placeholder: String(localized: "goal.example", table: "ConversationInspector"), minHeight: 88)
                    .accessibilityLabel(String(localized: "Goal"))
                    .accessibilityIdentifier("mac-goal-objective")
                DisclosureGroup(String(localized: "goal.optional_constraints", table: "ConversationInspector")) {
                    MacInspectorDocumentEditor(text: Binding(get: { constraints }, set: { constraints = $0 }), placeholder: String(localized: "goal.constraints"), minHeight: 64)
                        .accessibilityLabel(String(localized: "goal.constraints"))
                }
                Button("goal.start") {
                    submit(.init(action: .start, objective: objective,
                        constraints: constraints.split(separator: "\n").map(String.init)))
                }.disabled(controlBusy || model.selectedConversationID == nil || objective.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty).accessibilityIdentifier("mac-goal-start")
            } else {
                Text("Goal controls are unavailable. Refresh to confirm the current goal state.", tableName: "ConversationInspector")
                    .font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button(String(localized: "Refresh")) { retry += 1 }
                    .buttonStyle(MacPressFeedbackButtonStyle())
            }
        }
        .onChange(of: model.selectedConversationID) { _, _ in goalToCancel = nil }
        .confirmationDialog(String(localized: "goal.cancel_title", table: "ConversationInspector"), isPresented: Binding(
            get: { goalToCancel != nil }, set: { if !$0 { goalToCancel = nil } })) {
            Button(String(localized: "Cancel"), role: .destructive) {
                guard let observed = goalToCancel else { return }
                goalToCancel = nil; submit(observed)
            }
            Button(String(localized: "Close"), role: .cancel) { goalToCancel = nil }
        } message: { Text(String(localized: "goal.cancel_explanation", table: "ConversationInspector")) }
    }

    private func submit(_ control: AppleGoalControl?) {
        guard let owner = model.selectedConversationID,
              refresh.owner == owner, refresh.controlsConfirmed else { return }
        let submittedDraft = draft
        guard !submittedDraft.isSubmitting else { return }
        submittedDraft.isSubmitting = true; submittedDraft.error = nil
        Task { @MainActor in
            defer {
                submittedDraft.isSubmitting = false
                if model.selectedConversationID == owner { retry += 1 }
            }
            do {
                let applied = try await model.controlGoal(conversationID: owner, control: control)
                if applied, control?.action == .start {
                    submittedDraft.objective = ""; submittedDraft.constraints = ""
                }
                guard model.selectedConversationID == owner else { return }
                controlPending = !applied
            } catch {
                submittedDraft.error = error.localizedDescription
            }
        }
    }

}

// Presentation is independent of loading so populated, long, and error-adjacent
// states can be inspected without launching a model or touching a user database.
struct MacInspectorProgressContent: View, Equatable {
    let snapshot: MacInspectorProgressSnapshot?
    let todos: [MacInspectorProgressSnapshot.Todo]
    let tasks: [MacInspectorProgressSnapshot.Tasks.Item]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let goal = snapshot?.active {
                VStack(alignment: .leading, spacing: 6) {
                    heading("inspector.section.goal")
                    Text(goal.objective).font(.system(size: 15, weight: .semibold)).textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                    status(goal.status)
                }
            }
            if !tasks.isEmpty {
                section("inspector.section.execution") {
                    ForEach(tasks) { task in
                        row(task.current_step ?? task.id, state: task.state)
                    }
                }
            }
            if !todos.isEmpty {
                section("inspector.section.plan") {
                    ForEach(todos) { todo in
                        row(todo.content, state: todo.status)
                    }
                }
            }
            if let goal = snapshot?.active {
                if !goal.requirements.isEmpty {
                    section("inspector.section.requirements") {
                        ForEach(goal.requirements.values.sorted { $0.id < $1.id }) { requirement in
                            row(requirement.description, state: requirement.status)
                        }
                    }
                }
                if !goal.work.current.isEmpty {
                    section("inspector.section.work") {
                        ForEach(goal.work.current) { entry in
                            row(entry.content, state: entry.progress)
                        }
                    }
                }
            }
            if snapshot?.active == nil && todos.isEmpty && tasks.isEmpty {
                Label(String(localized: "inspector.no_plan", table: "ConversationInspector"),
                      systemImage: "checklist")
                    .font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.vertical, 12)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func heading(_ key: String) -> some View {
        Text(NSLocalizedString(key, tableName: "ConversationInspector", comment: "Progress section"))
            .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .medium))
            .foregroundStyle(.secondary)
            .accessibilityAddTraits(.isHeader)
    }

    private func section<Content: View>(_ key: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            heading(key)
            content()
        }
    }

    private func row(_ text: String, state: String?) -> some View {
        let active = state == "in_progress" || state == "running"
        return HStack(alignment: .top, spacing: 10) {
            Image(systemName: Self.symbol(state))
                .foregroundStyle(active ? MacDesignColor.brand : Color.secondary)
                .frame(width: 16).padding(.top, 2).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(text).font(.system(size: 13)).textSelection(.enabled)
                    .fontWeight(active ? .medium : .regular)
                    .fixedSize(horizontal: false, vertical: true)
                if let state { status(state) }
            }
        }
    }

    // Reported work is not verified completion. Unknown states remain neutral.
    static func symbol(_ state: String?) -> String {
        switch state {
        case "completed": return "checkmark.circle"
        case "in_progress", "running": return "circle.inset.filled"
        case "blocked", "failed": return "exclamationmark.circle"
        case "cancelled": return "minus.circle"
        default: return "circle"
        }
    }

    private func status(_ value: String) -> some View {
        let key = "inspector.state." + value
        let localized = NSLocalizedString(key, tableName: "ConversationInspector", comment: "Canonical progress state")
        return Text(localized == key
            ? String(localized: "inspector.state.unknown", table: "ConversationInspector") + " · " + value
            : localized)
            .font(.caption).foregroundStyle(.secondary)
    }
}
