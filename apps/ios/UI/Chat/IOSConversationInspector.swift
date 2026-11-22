// SPDX-License-Identifier: Apache-2.0
import SwiftUI

enum IOSConversationInspectorPane: String, Identifiable {
    case progress, goals, resources
    var id: String { rawValue }
}

nonisolated struct IOSInspectorProgressFacts: Decodable, Sendable {
    struct GoalProjection: Decodable, Sendable {
        struct Goal: Decodable, Sendable {
            struct Requirement: Decodable, Identifiable, Sendable {
                let id: String
                let description: String
                let status: String
            }
            struct Work: Decodable, Sendable {
                struct Entry: Decodable, Identifiable, Sendable {
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
            let id: String
            let objective: String
            let constraints: [String]?
            let status: String
            let next_wake_at_epoch_ms: Int64?
            let requirements: [String: Requirement]
            let work: Work

            var unlistedConstraints: [String] {
                var shown = Set(requirements.values.map { $0.description.trimmingCharacters(in: .whitespacesAndNewlines) })
                return (constraints ?? []).filter { constraint in
                    let text = constraint.trimmingCharacters(in: .whitespacesAndNewlines)
                    return !text.isEmpty && shown.insert(text).inserted
                }
            }
        }
        let active: Goal?
        let control_token: String?
        let can_pause: Bool
        let can_resume: Bool
        let can_cancel: Bool
        let applied_operations: [String: AppleGoalOperation]
    }
    struct Plan: Decodable, Sendable {
        struct Todo: Decodable, Identifiable, Sendable {
            let id: String
            let content: String
            let status: String
        }
        let conversation_id: String
        let todos: [Todo]
    }
    let conversation_id: String
    let goal_projection: GoalProjection?
    let task_projection: AgentCoreTaskProjection?
}

nonisolated struct IOSInspectorProgressSnapshot: Sendable {
    var supportsGoalControl = false
    var pendingGoalControl: AppleGoalControlIntent? = nil
    var goalControlNotice: String? = nil
    var goalDraft: AppleGoalDraft? = nil
    let facts: IOSInspectorProgressFacts?
    let plan: IOSInspectorProgressFacts.Plan?

    static func applyingGoalObservation(_ observation: AgentCoreJSONValue,
                                        to session: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        guard case .object(var fields) = session,
              case .object(var goal) = fields["goal_projection"] else { return session }
        let controls: [String: AgentCoreJSONValue]
        if case .object(let value) = observation {
            guard case .object(let active) = goal["active"],
                  active["id"] == value["goal_id"] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            controls = value
        } else if observation == .null, goal["active"] == nil {
            controls = [:]
        } else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        for key in ["can_pause", "can_resume", "can_cancel"] {
            if controls.isEmpty { goal[key] = .bool(false) }
            else {
                guard let value = controls[key], case .bool = value else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                goal[key] = value
            }
        }
        goal["control_token"] = controls["control_token"] ?? .null
        fields["goal_projection"] = .object(goal)
        return .object(fields)
    }

    static func remote(_ value: AgentCoreJSONValue, sourceHostID: String, conversationID: String) throws -> Self {
        guard !sourceHostID.isEmpty, !conversationID.isEmpty,
              case .object(let fields) = value,
              Set(fields.keys) == ["source_host_id", "conversation_id", "goal_projection", "plan"],
              fields["source_host_id"] == .string(sourceHostID),
              fields["conversation_id"] == .string(conversationID),
              let goal = fields["goal_projection"], let planValue = fields["plan"] else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let decoder = JSONDecoder()
        let projection = try decoder.decode(IOSInspectorProgressFacts.GoalProjection?.self,
            from: JSONEncoder().encode(goal))
        let plan = try decoder.decode(IOSInspectorProgressFacts.Plan?.self,
            from: JSONEncoder().encode(planValue))
        guard plan.map({ $0.conversation_id == conversationID }) ?? true else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        // This source endpoint does not publish task facts. Never fill them from
        // a local mirror or mistake absent source data for a local authority.
        return Self(facts: .init(conversation_id: conversationID, goal_projection: projection,
            task_projection: nil), plan: plan)
    }
}

struct IOSConversationInspector: View {
    let pane: IOSConversationInspectorPane
    let conversationID: String?
    let loadProgress: @MainActor (String) async throws -> IOSInspectorProgressSnapshot
    var submitGoalControl: (@MainActor (String, AppleGoalControl?) async throws -> Bool)? = nil
    var discardGoalControl: (@MainActor (String) async throws -> Void)? = nil
    let documents: [UserUploadedContextItem]
    let processingState: ChatProcessingState
    var initialObjective: String = ""
    var embeddedInNavigationStack: Bool = false
    @Environment(\.dismiss) private var dismiss
    @State private var facts: IOSInspectorProgressFacts?
    @State private var plan: IOSInspectorProgressFacts.Plan?
    @State private var loadedOwner: String?
    @State private var appliedInitialObjective = false
    @State private var loading = true
    @State private var failed = false
    @State private var retry = 0
    @State private var supportsGoalControl = false
    @State private var pendingGoalControl: AppleGoalControlIntent?
    @State private var goalControlNotice: String?
    @State private var sendingGoalControl = false
    @State private var showGoalStart = false
    @State private var goalObjective = ""
    @State private var goalConstraints = ""
    @FocusState private var goalObjectiveFocused: Bool
    @State private var goalToCancel: AppleGoalControl?
    @State private var showSchedule = false
    @State private var goalWake = Date().addingTimeInterval(3600)

    private var inspectorContent: some View {
        List {
            Group {
            if pane == .resources {
                if documents.isEmpty {
                    ContentUnavailableView(String(localized: "inspector.no_resources", table: "ConversationInspector"), systemImage: "folder")
                }
                ForEach(documents) { item in
                    DisclosureGroup {
                        Text(item.content).textSelection(.enabled)
                            .font(.body).padding(.vertical, 8)
                    } label: { Label(item.name, systemImage: "doc") }
                    .disclosureGroupStyle(NolaRowDisclosureStyle(isPlain: true))
                }
            } else if conversationID == nil {
                ContentUnavailableView {
                    Label(String(localized: "inspector.progress", table: "ConversationInspector"), systemImage: "bubble.left.and.bubble.right")
                } description: {
                    Text(String(localized: "inspector.conversation_needed", table: "ConversationInspector"))
                }
            } else if (loading && facts == nil && plan == nil) || loadedOwner != conversationID {
                ProgressView()
            } else if failed && facts == nil && plan == nil {
                Text("inspector.failed", tableName: "ConversationInspector")
                Button(String(localized: "inspector.retry", table: "ConversationInspector")) { retry += 1 }
            } else {
                if failed {
                    Text("inspector.failed", tableName: "ConversationInspector")
                    Button(String(localized: "inspector.retry", table: "ConversationInspector")) { retry += 1 }
                }
                Section {
                    HStack {
                        Text(goalText("inspector.section.status", "Session status"))
                            .font(.subheadline.weight(.medium))
                        Spacer()
                        status(processingState.blocksInput ? "running" : "idle")
                    }
                }
                if pane == .goals && supportsGoalControl && submitGoalControl != nil { goalControls }
                if pane == .goals, let goal = facts?.goal_projection?.active {
                    Section(goalText("goal.section.goal", "Goal")) {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(goal.objective).font(.headline)
                            status(goal.status)
                        }
                        if let wake = goal.next_wake_at_epoch_ms, wake > 0 {
                            LabeledContent(goalText("goal.wake", "Continue at")) {
                                Text(Date(timeIntervalSince1970: TimeInterval(wake) / 1_000.0), format: .dateTime.month().day().hour().minute())
                            }
                            .font(.caption).foregroundStyle(.secondary)
                        }
                        if ["paused", "needs_user"].contains(goal.status) {
                            Text(goalText("goal.paused_reply_hint", "Paused for your input. Send a reply in chat or tap Continue."))
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    let constraints = goal.unlistedConstraints
                    if !constraints.isEmpty {
                        Section(goalText("goal.constraints", "Constraints")) {
                            ForEach(constraints.indices, id: \.self) { index in
                                Text(verbatim: constraints[index])
                                    .accessibilityIdentifier("goal.constraint.\(index)")
                            }
                        }
                    }
                    let sortedRequirements = goal.requirements.values.sorted { $0.id < $1.id }
                    if !sortedRequirements.isEmpty {
                        Section(goalText("goal.section.requirements", "Requirements")) {
                            ForEach(sortedRequirements) { requirement in
                                inspectorStatusRow(title: requirement.description, state: requirement.status)
                            }
                        }
                    }
                    let workEntries = goal.work.current
                    if !workEntries.isEmpty {
                        Section(goalText("goal.section.work", "Progress")) {
                            ForEach(workEntries) { entry in
                                if let value = entry.progress {
                                    inspectorStatusRow(title: entry.content, state: value)
                                } else {
                                    Text(entry.content)
                                }
                            }
                        }
                    }
                }
                if pane != .goals {
                    let todos = plan?.todos ?? []
                    if !todos.isEmpty {
                        Section(goalText("inspector.section.checklist", "Checklist")) {
                            ForEach(todos) { todo in
                                inspectorStatusRow(title: todo.content, state: todo.status)
                            }
                        }
                    }
                    if !currentTasks.isEmpty {
                        Section(goalText("inspector.section.tasks", "Tasks")) {
                            ForEach(currentTasks) { task in
                                inspectorStatusRow(title: taskDisplayTitle(task), state: task.state.rawValue)
                            }
                        }
                    }
                }
                if facts?.goal_projection?.active == nil && (pane == .goals || ((plan?.todos.isEmpty ?? true) && currentTasks.isEmpty)) {
                    if pane == .goals {
                        ContentUnavailableView(String(localized: "work.tools.noGoal"), systemImage: "target")
                    } else {
                        ContentUnavailableView(String(localized: "inspector.no_plan", table: "ConversationInspector"), systemImage: "checklist")
                    }
                }
            }
            }
            .listRowBackground(DesignTokens.surfaceColor)
            .listRowSeparatorTint(DesignTokens.borderColor)
        }
        .nolaGroupedSurface()
        .navigationTitle(pane == .goals ? String(localized: "work.tools.goals") : pane == .progress
            ? String(localized: "inspector.progress", table: "ConversationInspector")
            : String(localized: "inspector.resources", table: "ConversationInspector"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if !embeddedInNavigationStack {
                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.close) { dismiss() }.accessibilityIdentifier("chat.inspector.close")
                }
            }
        }
    }

    var body: some View {
        Group {
            if embeddedInNavigationStack {
                inspectorContent
            } else {
                NavigationStack {
                    inspectorContent
                }
            }
        }
        .task(id: "\(conversationID ?? ""):\(retry)") {
            if loadedOwner != conversationID {
                facts = nil; plan = nil; supportsGoalControl = false; pendingGoalControl = nil; goalControlNotice = nil
                sendingGoalControl = false; showGoalStart = false; showSchedule = false; goalObjective = ""; goalConstraints = ""; goalToCancel = nil
            }
            loadedOwner = conversationID; loading = true; failed = false
            guard pane != .resources, let owner = conversationID else { loading = false; return }
            repeat {
            do {
                let result = try await loadProgress(owner)
                guard !Task.isCancelled, conversationID == owner else { return }
                guard result.facts.map({ $0.conversation_id == owner }) ?? true,
                      result.plan.map({ $0.conversation_id == owner }) ?? true else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
                facts = result.facts; plan = result.plan; loading = false; failed = false
                supportsGoalControl = result.supportsGoalControl
                if pendingGoalControl?.control.action == .start, result.pendingGoalControl == nil,
                   result.goalControlNotice == nil { goalObjective = "" }
                pendingGoalControl = result.pendingGoalControl
                if !appliedInitialObjective {
                    appliedInitialObjective = true
                    let prefill = initialObjective.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !prefill.isEmpty, facts?.goal_projection?.active == nil, pendingGoalControl == nil {
                        goalObjective = prefill
                        if canStartNewGoal { showGoalStart = true }
                    }
                }
                if goalObjective.isEmpty, let draft = result.goalDraft {
                    goalObjective = draft.objective
                    if goalConstraints.isEmpty { goalConstraints = draft.constraints.joined(separator: "\n") }
                }
                if let notice = result.goalControlNotice { goalControlNotice = notice }
                else if result.pendingGoalControl == nil { goalControlNotice = nil }
            } catch {
                guard !Task.isCancelled, conversationID == owner else { return }
                failed = true; loading = false
                return
            }
            do { try await Task.sleep(for: .seconds(3)) } catch { return }
            } while !Task.isCancelled
        }
    }

    @ViewBuilder private var goalControls: some View {
        Section {
            if let notice = goalControlNotice { Text(notice).font(.caption).foregroundStyle(.secondary) }
            if pendingGoalControl != nil {
                Text(goalText("goal.pending", "Waiting for Host confirmation"))
                HStack {
                    Button(goalText("goal.retry", "Retry saved action")) { sendGoalControl(nil) }
                        .buttonStyle(.borderless)
                        .frame(minHeight: 44)
                        .disabled(sendingGoalControl)
                    if discardGoalControl != nil {
                        Spacer()
                        Button(goalText("goal.discard", "Discard"), role: .destructive) {
                            discardPendingControl()
                        }
                        .buttonStyle(.borderless)
                        .frame(minHeight: 44)
                        .disabled(sendingGoalControl)
                    }
                }
            } else if let projection = facts?.goal_projection,
                      projection.can_pause || projection.can_resume || projection.can_cancel {
                HStack {
                    if projection.can_pause {
                        Button(goalText("goal.pause", "Pause")) { sendObservedGoalControl(.pause) }
                            .buttonStyle(.borderless)
                            .frame(minHeight: 44)
                    }
                    if projection.can_resume {
                        Button(goalText("goal.resume", "Continue")) { sendObservedGoalControl(.resume) }
                            .buttonStyle(.borderless)
                            .frame(minHeight: 44)
                    }
                    Spacer()
                    Menu {
                        Button(goalText("goal.schedule", "Schedule continuation")) { showSchedule = true }
                        if projection.can_cancel {
                            Button(goalText("goal.cancel", "Cancel Goal"), role: .destructive) {
                                if let goal = projection.active, let token = projection.control_token {
                                    goalToCancel = AppleGoalControl(action: .cancel, expectedGoalId: goal.id, expectedControlToken: token)
                                }
                            }
                        }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                            .frame(width: 44, height: 44)
                            .contentShape(Rectangle())
                            .accessibilityLabel(goalText("goal.actions", "Goal actions"))
                    }
                    .buttonStyle(.borderless)
                }
                .disabled(sendingGoalControl || facts?.goal_projection?.control_token == nil)
            } else if canStartNewGoal {
                Button(goalText("goal.start", "Start a Goal")) { showGoalStart = true }
                    .disabled(sendingGoalControl)
            }
            if sendingGoalControl { ProgressView().accessibilityLabel(goalText("goal.sending", "Sending Goal action")) }
        }
        .confirmationDialog(goalText("goal.cancel_title", "End this goal?"), isPresented: Binding(
            get: { goalToCancel != nil }, set: { if !$0 { goalToCancel = nil } })) {
            Button(goalText("goal.cancel", "Cancel Goal"), role: .destructive) {
                guard let observed = goalToCancel else { return }
                goalToCancel = nil; sendGoalControl(observed)
            }
            Button(L10n.close, role: .cancel) { goalToCancel = nil }
        } message: { Text(goalText("goal.cancel_explanation", "Automatic work on this goal will stop. Your conversation and completed results remain.")) }
        .sheet(isPresented: $showGoalStart) {
            NavigationStack {
                Form {
                    Group {
                    Text(goalText("goal.guidance", "Describe a result. Nola keeps working and pauses when it needs your input or permission."))
                        .foregroundStyle(.secondary)
                    TextField(goalText("goal.objective", "What should this Goal achieve?"), text: $goalObjective, axis: .vertical)
                        .lineLimit(3...8).focused($goalObjectiveFocused)
                    TextField(goalText("goal.optional_constraints", "Constraints (optional)"), text: $goalConstraints, axis: .vertical)
                        .lineLimit(2...5)
                    Text(goalText("goal.example", "Example: review the CLI, fix interaction issues, and run tests."))
                        .font(.callout).foregroundStyle(.secondary)
                    Text(goalText("goal.model_note", "Uses this conversation’s selected model and current tool permissions."))
                        .font(.caption).foregroundStyle(.secondary)
                    }
                    .listRowBackground(DesignTokens.surfaceColor)
                }
                .nolaGroupedSurface()
                .navigationTitle(goalText("goal.start", "Start a Goal"))
                .onAppear { goalObjectiveFocused = true }
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button(L10n.close) { showGoalStart = false } }
                    ToolbarItem(placement: .confirmationAction) {
                        Button(goalText("goal.start", "Start")) {
                            let objective = goalObjective.trimmingCharacters(in: .whitespacesAndNewlines)
                            let parsedConstraints = goalConstraints
                                .components(separatedBy: .newlines)
                                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                                .filter { !$0.isEmpty }
                            showGoalStart = false
                            sendGoalControl(AppleGoalControl(action: .start, objective: objective, constraints: parsedConstraints))
                        }.disabled(goalObjective.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .toggleStyle(NolaSwitchToggleStyle())
        }
        .sheet(isPresented: $showSchedule) {
            NavigationStack {
                Form {
                    DatePicker(goalText("goal.wake", "Continue at"), selection: $goalWake, in: Date()...)
                        .listRowBackground(DesignTokens.cardBackgroundColor)
                }
                    .nolaGroupedSurface()
                    .navigationTitle(goalText("goal.schedule", "Schedule continuation"))
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) { Button(L10n.close) { showSchedule = false } }
                        ToolbarItem(placement: .confirmationAction) {
                            Button(goalText("goal.schedule", "Schedule")) {
                                showSchedule = false
                                sendObservedGoalControl(.schedule, wake: Int64(goalWake.timeIntervalSince1970 * 1_000))
                            }
                        }
                    }
            }
            .toggleStyle(NolaSwitchToggleStyle())
        }
    }

    private func sendObservedGoalControl(_ action: AppleGoalControl.Action, wake: Int64? = nil) {
        guard let projection = facts?.goal_projection, let goal = projection.active,
              let token = projection.control_token else { return }
        sendGoalControl(AppleGoalControl(action: action, expectedGoalId: goal.id, expectedControlToken: token, wakeAtEpochMs: wake))
    }

    private func sendGoalControl(_ control: AppleGoalControl?) {
        guard !sendingGoalControl, let owner = conversationID, let submitGoalControl else { return }
        sendingGoalControl = true; goalControlNotice = nil
        Task { @MainActor in
            do {
                let confirmed = try await submitGoalControl(owner, control)
                guard loadedOwner == owner, conversationID == owner else { return }
                if !confirmed { goalControlNotice = goalText("goal.pending", "Waiting for Host confirmation") }
                else if control?.action == .start || pendingGoalControl?.control.action == .start {
                    goalObjective = ""
                    goalConstraints = ""
                }
            } catch {
                guard loadedOwner == owner, conversationID == owner else { return }
                goalControlNotice = error.localizedDescription
            }
            sendingGoalControl = false
            retry += 1
        }
    }

    private func discardPendingControl() {
        guard !sendingGoalControl, let owner = conversationID, let discardGoalControl else { return }
        sendingGoalControl = true; goalControlNotice = nil
        Task { @MainActor in
            defer {
                if loadedOwner == owner && conversationID == owner {
                    sendingGoalControl = false
                    retry += 1
                }
            }
            do {
                try await discardGoalControl(owner)
                guard loadedOwner == owner, conversationID == owner else { return }
                pendingGoalControl = nil
            } catch {
                guard loadedOwner == owner, conversationID == owner else { return }
                goalControlNotice = error.localizedDescription
            }
        }
    }

    private var canStartNewGoal: Bool {
        guard let goal = facts?.goal_projection?.active else { return true }
        return ["completed", "failed", "cancelled"].contains(goal.status)
    }

    private func taskDisplayTitle(_ task: AgentCoreTaskHandle) -> String {
        if let step = task.currentStep?.trimmingCharacters(in: .whitespacesAndNewlines), !step.isEmpty {
            return step
        }
        if let custom = task.lineage?.title?.trimmingCharacters(in: .whitespacesAndNewlines), !custom.isEmpty {
            return custom
        }
        switch task.kind {
        case .agent: return NSLocalizedString("chat.task.agent", value: "Background agent", comment: "Core task kind")
        case .tool: return NSLocalizedString("chat.task.tool", value: "Background tool", comment: "Core task kind")
        case .skill: return NSLocalizedString("chat.task.skill", value: "Skill task", comment: "Core task kind")
        case .dataRefresh: return NSLocalizedString("chat.task.data_refresh", value: "Data refresh", comment: "Core task kind")
        }
    }

    private func goalText(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(key, tableName: "ConversationInspector", value: fallback, comment: "Goal control")
    }

    private var currentTasks: [AgentCoreTaskHandle] {
        guard let projection = facts?.task_projection else { return [] }
        return projection.order.compactMap { projection.tasks[$0] }
            .filter { $0.ownerSessionId == conversationID }
    }

    private func inspectorStatusRow(title: String, state: String) -> some View {
        let normalized = state.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let isCompleted = ["completed", "done", "succeeded", "satisfied", "verified"].contains(normalized)
        return HStack(alignment: .top, spacing: 10) {
            Image(systemName: statusSymbol(for: normalized))
                .font(.body)
                .foregroundStyle(statusTint(for: normalized))
                .padding(.top, 1)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 5) {
                Text(title)
                    .font(.body)
                    .foregroundStyle(isCompleted ? .secondary : .primary)
                    .strikethrough(isCompleted)
                status(state)
            }
        }
        .padding(.vertical, 2)
    }

    private func statusSymbol(for normalized: String) -> String {
        switch normalized {
        case "completed", "done", "succeeded", "satisfied", "verified":
            return "checkmark.circle.fill"
        case "running", "in_progress", "active":
            return "clock.arrow.circlepath"
        case "failed", "error", "cancelled", "canceled":
            return "exclamationmark.circle.fill"
        case "paused", "needs_user", "waiting":
            return "pause.circle.fill"
        default:
            return "circle"
        }
    }

    private func statusTint(for normalized: String) -> Color {
        switch normalized {
        case "completed", "done", "succeeded", "satisfied", "verified":
            return DesignTokens.successColor
        case "running", "in_progress", "active":
            return DesignTokens.actionColor
        case "failed", "error", "cancelled", "canceled":
            return DesignTokens.dangerColor
        case "paused", "needs_user", "waiting":
            return DesignTokens.warningColor
        default:
            return DesignTokens.secondaryContentColor
        }
    }

    private func status(_ value: String) -> some View {
        let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let tint = statusTint(for: normalized)
        let key = "inspector.state." + value
        let localized = NSLocalizedString(key, tableName: "ConversationInspector", comment: "Canonical progress state")
        let label = localized == key
            ? String(localized: "inspector.state.unknown", table: "ConversationInspector") + " · " + value
            : localized
        return Text(label)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(tint.opacity(0.12), in: Capsule())
    }
}
