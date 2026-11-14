// SPDX-License-Identifier: Apache-2.0
// Today is a read-only arrangement of existing local records and conversation work.
import SwiftUI

struct IOSPersonalWorkspaceView: View {
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.scenePhase) private var scenePhase
    @State private var pageWidth: CGFloat = 390
    @State private var conversations: [IOSWorkspaceConversationSummary] = []
    @State private var notifications: [NotificationEntity] = []
    @State private var persistedWaitingIDs: Set<UUID> = []
    @State private var persistedRecentIDs: [UUID] = []
    @State private var persistedRunningIDs: Set<UUID> = []
    @State private var hasMoreFacts = false
    @State private var unrepresentedExpertNames: [String] = []
    @State private var hasLoadedFacts = false
    @State private var queuedPlans: [IOSWorkspaceQueuedPlan] = []
    @State private var refreshTask: Task<Void, Never>?
    @State private var error: String?
    @Binding var destination: IOSWorkspaceDestination?
    @Binding var notificationKey: String?

    private var waiting: [IOSWorkspaceConversationSummary] {
        conversations.filter {
            if persistedWaitingIDs.contains($0.id) { return true }
            guard let execution = container.chatManager.executionForConversation($0.id) else { return false }
            return execution.pendingHostPermissionRequest != nil || execution.localAgentPermissionRequest != nil
                || execution.remoteRunNeedsResume || execution.unknownEffectRecovery != nil
        }
    }
    private var running: [IOSWorkspaceConversationSummary] {
        let waitingIDs = Set(waiting.map(\.id))
        return conversations.filter { !waitingIDs.contains($0.id) &&
            (persistedRunningIDs.contains($0.id) || container.chatManager.executionForConversation($0.id)?.processingState.blocksInput == true) }
    }
    private var recent: [IOSWorkspaceConversationSummary] {
        let priorityIDs = Set((waiting + running).map(\.id))
        return persistedRecentIDs.filter { !priorityIDs.contains($0) }.compactMap { id in conversations.first { $0.id == id } }
    }
    private var updates: [NotificationEntity] {
        notifications.filter { !$0.isRead && $0.status != .cancelled }.prefix(8).map { $0 }
    }

    var body: some View {
        NavigationStack {
            List {
                Group {
                IOSWorkspacePageHeading(title: String(localized: "workspace.today.title"),
                    subtitle: NSLocalizedString("workspace.today.subtitle", value: "Your conversations, updates, and upcoming plans.", comment: "Today page subtitle"))
                    .padding(.bottom, 12)
                if let error {
                    Section { Text(error).foregroundStyle(.secondary); Button("common.retry") { refresh() } }
                }
                if !unrepresentedExpertNames.isEmpty {
                    Section("workspace.today.attention") {
                        Text("workspace.today.expertDestinationUnconfirmed").font(.subheadline).foregroundStyle(.secondary)
                        ForEach(Array(unrepresentedExpertNames.enumerated()), id: \.offset) { _, name in
                            Text(name).font(.body)
                        }
                    }
                }
                if !waiting.isEmpty {
                    Section("workspace.today.waiting") {
                        ForEach(waiting, id: \.id) { conversation in conversationRow(conversation) }
                    }
                }
                if !running.isEmpty {
                    Section("workspace.today.underway") {
                        ForEach(running, id: \.id) { conversation in conversationRow(conversation) }
                    }
                }
                if !updates.isEmpty {
                    Section("workspace.today.updates") {
                        ForEach(updates) { item in
                            VStack(alignment: .leading, spacing: 6) {
                                Button {
                                    notificationKey = item.dedupeKey ?? item.id
                                    destination = .records
                                } label: {
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text(item.title ?? L10n.notificationsTitle).font(.body.weight(.medium)).foregroundStyle(.primary)
                                        if let body = item.body, !body.isEmpty {
                                            Text(body).font(.subheadline).foregroundStyle(.secondary).lineLimit(3)
                                        }
                                        HStack {
                                            if let source = item.source, !source.isEmpty { Text(source) }
                                            Spacer()
                                            Text(item.createdAt, style: .relative)
                                        }.font(.caption).foregroundStyle(.secondary)
                                    }.frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).contentShape(Rectangle())
                                }.buttonStyle(.plain)
                                Button("workspace.today.markRead") { markRead(item) }
                                    .font(.caption).frame(minHeight: 44)
                            }.padding(.vertical, 4)
                        }
                    }
                }
                if !queuedPlans.isEmpty || (!hasLoadedFacts && error == nil) {
                    Section("workspace.today.upNext") {
                        if !hasLoadedFacts {
                            ProgressView().frame(minHeight: 44)
                        } else {
                            ForEach(queuedPlans) { plan in
                                Button { destination = .plans; notificationKey = nil } label: {
                                    VStack(alignment: .leading, spacing: 5) {
                                        Text(plan.title).foregroundStyle(.primary)
                                        Text(plan.date, format: .dateTime.month().day().hour().minute())
                                            .font(.subheadline).foregroundStyle(.secondary)
                                        Text("workspace.today.queuedInstance").font(.caption).foregroundStyle(.secondary)
                                    }.frame(maxWidth: .infinity, minHeight: 52, alignment: .leading).contentShape(Rectangle())
                                }.buttonStyle(.plain)
                            }
                        }
                    }
                }
                if hasLoadedFacts && waiting.isEmpty && running.isEmpty && updates.isEmpty && queuedPlans.isEmpty && unrepresentedExpertNames.isEmpty && error == nil {
                    Section {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("workspace.today.empty").font(.headline)
                            Text("workspace.today.emptyDetail").font(.subheadline).foregroundStyle(.secondary)
                        }.padding(.vertical, 24)
                    }.listRowSeparator(.hidden)
                }
                if hasMoreFacts {
                    Section {
                        Text("workspace.today.moreFacts").font(.caption).foregroundStyle(.secondary)
                        HStack {
                            Button(L10n.chatTitle) {
                                NotificationCenter.default.post(name: .nolaOpenChatTab, object: nil)
                                NotificationCenter.default.post(name: .nolaSearchConversations, object: "")
                            }.frame(minHeight: 44)
                            Spacer()
                            Button(L10n.scheduledActionsTitle) { notificationKey = nil; destination = .plans }.frame(minHeight: 44)
                        }
                    }
                }
                if !recent.isEmpty {
                    Section("workspace.today.recent") {
                        ForEach(recent, id: \.id) { conversation in conversationRow(conversation) }
                    }
                }
                }
                .listRowInsets(EdgeInsets(top: 12, leading: pageWidth < 359 ? 16 : 20, bottom: 12, trailing: pageWidth < 359 ? 16 : 20))
                .listRowBackground(DesignTokens.backgroundColor)
                .listRowSeparator(.hidden)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(DesignTokens.backgroundColor)
            .background(IOSWorkspaceWidthObserver(width: $pageWidth))
            .navigationTitle("")
            .toolbar(.hidden, for: .navigationBar)
            .refreshable { refresh(); await refreshTask?.value }
            .task(id: scenePhase) {
                guard scenePhase == .active else { refreshTask?.cancel(); return }
                refresh()
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .seconds(5)) } catch { return }
                    guard !Task.isCancelled else { return }
                    refresh()
                }
            }
            .onDisappear { refreshTask?.cancel() }
            .accessibilityIdentifier("workspace.today")
        }
    }

    @ViewBuilder private func conversationRow(_ conversation: IOSWorkspaceConversationSummary) -> some View {
        Button {
            NotificationCenter.default.post(name: .nolaOpenChatTab, object: conversation.id)
        } label: {
            VStack(alignment: .leading, spacing: 5) {
                Text(IOSConversationTitlePresentationPolicy.displayTitle(conversation.title,
                    conversationID: conversation.id.uuidString, fallback: L10n.chatConversationDefaultTitle))
                    .font(.body.weight(.medium)).foregroundStyle(.primary)
                    .lineLimit(2)
                if persistedWaitingIDs.contains(conversation.id) {
                    Text("workspace.today.needsResponse").font(.subheadline).foregroundStyle(.secondary)
                } else if let execution = container.chatManager.executionForConversation(conversation.id) {
                    IOSWorkspaceExecutionStatus(execution: execution)
                }
                Text(conversation.lastActiveAt, style: .relative).font(.caption).foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity, minHeight: 52, alignment: .leading).contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityIdentifier("workspace.conversation.\(conversation.id.uuidString.lowercased())")
    }
    private func refresh() {
        refreshTask?.cancel()
        let runtime = container.applicationRuntime
        let notificationStore = container.notificationStore
        refreshTask = Task { @MainActor in
            let worker = Task.detached(priority: .utility) {
                let facts = try IOSWorkspaceReadFacts.snapshot(runtime: runtime)
                try Task.checkCancellation()
                let notifications = normalizedNotificationOrder(try notificationStore?.fetchAll() ?? [])
                var seen = Set<UUID>()
                let conversations = (facts.conversations + facts.recent_conversations).compactMap { fact -> IOSWorkspaceConversationSummary? in
                    guard let id = UUID(uuidString: fact.conversation_id), seen.insert(id).inserted else { return nil }
                    return IOSWorkspaceConversationSummary(id: id, title: fact.title,
                        lastActiveAt: Date(timeIntervalSince1970: Double(fact.updated_at_ms) / 1000))
                }
                let waiting = Set(facts.conversations.filter(\.needs_response).compactMap { UUID(uuidString: $0.conversation_id) })
                let running = Set(facts.conversations.filter { $0.active_run != nil || $0.expert_contexts.contains { $0.fact.active_run != nil } }.compactMap { UUID(uuidString: $0.conversation_id) })
                let plans = facts.queued_instances.map { run in
                    IOSWorkspaceQueuedPlan(id: run.occurrence_id, title: run.title ?? String(localized: "scheduled_actions.title"),
                        date: Date(timeIntervalSince1970: Double(run.scheduled_for_ms) / 1000))
                }
                let recent = facts.recent_conversations.compactMap { UUID(uuidString: $0.conversation_id) }
                let unrepresented = facts.unrepresented_expert_contexts.map(\.display_name)
                return (waiting, running, plans, facts.has_more_conversations || facts.has_more_queued_instances, recent, unrepresented, conversations, notifications)
            }
            do {
                let facts = try await withTaskCancellationHandler { try await worker.value }
                    onCancel: { worker.cancel() }
                guard !Task.isCancelled else { return }
                persistedWaitingIDs = facts.0
                persistedRunningIDs = facts.1
                queuedPlans = facts.2
                hasMoreFacts = facts.3
                persistedRecentIDs = facts.4
                unrepresentedExpertNames = facts.5
                conversations = facts.6
                notifications = facts.7
                hasLoadedFacts = true
                error = nil
            } catch is CancellationError { }
            catch { if !Task.isCancelled { self.error = error.localizedDescription } }
        }
    }

    private func markRead(_ item: NotificationEntity) {
        do {
            guard let store = container.notificationStore else { return }
            try store.markRead(id: item.id)
            refresh()
        } catch { self.error = error.localizedDescription }
    }
}

nonisolated private struct IOSWorkspaceConversationSummary: Identifiable, Sendable {
    let id: UUID
    let title: String
    let lastActiveAt: Date
}

private struct IOSWorkspaceExecutionStatus: View {
    @ObservedObject var execution: ChatManager
    var body: some View {
        Text(execution.getStateDescription()).font(.subheadline).foregroundStyle(.secondary)
    }
}

enum IOSWorkspaceDestination: String, CaseIterable, Identifiable, Hashable {
    case knowledge, sources, memory, plans, records, models, settings
    var id: String { rawValue }
    init?(page: Int) {
        switch page { case 0: self = .sources; case 1: self = .memory; case 2: self = .plans; case 3: self = .records; default: return nil }
    }
    var title: String {
        if self == .knowledge { return NSLocalizedString("information.knowledge", value: "Knowledge points", comment: "Information destination") }
        return NSLocalizedString("workspace.\(rawValue)", comment: "Workspace destination")
    }
    var informationTitle: String {
        switch self {
        case .memory: NSLocalizedString("information.memory", value: "Long-term memory", comment: "Information destination")
        case .records: NSLocalizedString("information.activity", value: "Activity", comment: "Information destination")
        default: title
        }
    }
    var informationDetail: String {
        switch self {
        case .knowledge: NSLocalizedString("information.knowledge.detail", value: "Conversation insights, with their sources preserved", comment: "Information entry detail")
        case .sources: NSLocalizedString("information.sources.detail", value: "Documents, connections, and access policies", comment: "Information entry detail")
        case .memory: NSLocalizedString("information.memory.detail", value: "Categories, evidence, and confidence", comment: "Information entry detail")
        case .plans: NSLocalizedString("information.plans.detail", value: "Schedules, status, and run history", comment: "Information entry detail")
        case .records: NSLocalizedString("information.activity.detail", value: "Items needing a response and execution records", comment: "Information entry detail")
        default: ""
        }
    }
    var iconAsset: String {
        switch self { case .knowledge: "nola-file"; case .sources: "nola-folder"; case .memory: "nola-person"; case .plans: "nola-calendar"; case .records: "nola-clock"; case .models: "nola-models"; case .settings: "nola-settings" }
    }
    var symbol: String {
        switch self { case .knowledge: "lightbulb"; case .sources: "folder"; case .memory: "brain"; case .plans: "calendar"; case .records: "bell"; case .models: "cpu"; case .settings: "gearshape" }
    }
}

/// Navigation content has no stack of its own. Each entry point supplies exactly
/// one container, while editors retain their existing sheets and cancellation guards.
struct IOSWorkspaceManagementContent: View {
    let destination: IOSWorkspaceDestination
    var notificationKey: String? = nil
    var notificationRequestID: UUID? = nil
    var onModelConfigured: ((ModelInfo) -> Void)? = nil
    var body: some View {
        Group {
            switch destination {
            case .knowledge: IOSKnowledgePointsView()
            case .sources: HomeView(ownsNavigationStack: false)
            case .memory: PersonalIntelligenceDashboardView(embedded: true)
            case .plans: ScheduledActionsView(embedded: true)
            case .records:
                if notificationKey != nil || notificationRequestID != nil {
                    NotificationCenterView(initialDeepLinkKey: notificationKey,
                        ownsNavigationStack: false, deepLinkRequestID: notificationRequestID)
                } else { IOSConversationActivityView() }
            case .models: ModelsView(ownsNavigationStack: false, onConfigured: onModelConfigured)
            case .settings: SettingsView(ownsNavigationStack: false)
            }
        }
        .navigationTitle(destination.informationTitle)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
    }
}

struct IOSWorkspaceManagementSheet: View {
    let destination: IOSWorkspaceDestination
    let notificationKey: String?
    var onModelConfigured: ((ModelInfo) -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        Group {
            if destination == .models {
                // Model configuration owns its cancellation and completion flow.
                ModelsView(onConfigured: onModelConfigured)
            } else {
                NavigationStack {
                    IOSWorkspaceManagementContent(destination: destination, notificationKey: notificationKey,
                        onModelConfigured: onModelConfigured)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button(L10n.close) { dismiss() }.frame(minHeight: 44)
                                    .accessibilityIdentifier("workspace.management.close")
                            }
                        }
                }
            }
        }
        .presentationDragIndicator(.visible)
    }
}

struct IOSInformationNavigationRequest: Identifiable, Equatable {
    let id = UUID()
    let destination: IOSWorkspaceDestination
    var notificationKey: String? = nil
}


struct IOSInformationView: View {
    @EnvironmentObject private var container: ServiceContainer
    @State private var pageWidth: CGFloat = 390
    @State private var directory: [IOSConversationDirectoryEntry] = []
    @State private var directoryLoadedAt: Date?
    @State private var directoryError: String?
    private var pageInset: CGFloat { pageWidth < 359 ? 16 : 20 }
    @Binding var path: [IOSWorkspaceDestination]
    @Binding var pendingRequests: [IOSInformationNavigationRequest]
    @State private var activeRequest: IOSInformationNavigationRequest?
    var body: some View {
        NavigationStack(path: $path) {
            List {
                IOSWorkspacePageHeading(title: String(localized: "navigation.library"),
                    subtitle: String(localized: "information.subtitle"))
                    .accessibilityIdentifier("information.heading")
                    .listRowInsets(EdgeInsets(top: 8, leading: pageInset, bottom: 16, trailing: pageInset))
                    .listRowSeparator(.hidden).listRowBackground(Color.clear)
                Section {
                ForEach([IOSWorkspaceDestination.knowledge, .memory, .sources]) { item in
                    Button {
                        activeRequest = nil
                        path.append(item)
                    } label: {
                        HStack(spacing: 12) {
                            Image(item.iconAsset).resizable().renderingMode(.template).scaledToFit()
                                .frame(width: 22, height: 22).foregroundStyle(DesignTokens.actionColor).accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 4) {
                                Text(item.informationTitle).font(.body.weight(.medium))
                                Text(item.informationDetail).font(.subheadline).foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            Spacer(minLength: 8)
                            Image("nola-chevron").resizable().renderingMode(.template).scaledToFit().frame(width: 16, height: 16).foregroundStyle(.secondary)
                        }.foregroundStyle(.primary).frame(minHeight: 44).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .listRowSeparator(.visible)
                    .listRowInsets(EdgeInsets(top: 14, leading: pageInset, bottom: 14, trailing: pageInset))
                    .listRowBackground(DesignTokens.surfaceColor)
                    .accessibilityIdentifier("information.\(item.rawValue)")
                }
                }
                .listSectionSeparator(.hidden)
                if let directoryError {
                    Section {
                        Text(directoryError).font(.footnote).foregroundStyle(.secondary)
                        Button("common.retry") { Task { await loadDirectory(force: true) } }.frame(minHeight: 44)
                    }
                }
                if !directory.isEmpty {
                    Section {
                        ForEach(Array(recentKnowledge.prefix(3))) { item in
                            Button { NotificationCenter.default.post(name: .nolaOpenChatTab, object: item.id) } label: {
                                VStack(alignment: .leading, spacing: 6) {
                                    Text(IOSConversationTitlePresentationPolicy.displayTitle(item.title,
                                        conversationID: item.id.uuidString, fallback: L10n.chatConversationDefaultTitle))
                                        .font(.body.weight(.medium)).foregroundStyle(.primary).lineLimit(2)
                                    if !item.knowledgePoints.isEmpty {
                                        Text(item.knowledgePoints).font(.subheadline).foregroundStyle(.secondary).lineLimit(3)
                                    }
                                    Text(item.lastActiveAt, format: .dateTime.month().day()).font(.caption).foregroundStyle(.secondary)
                                }.frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).contentShape(Rectangle())
                            }.buttonStyle(.plain)
                                .listRowInsets(EdgeInsets(top: 14, leading: pageInset, bottom: 14, trailing: pageInset))
                                .listRowBackground(DesignTokens.surfaceColor)
                        }
                    } header: {
                        Text(recentKnowledge.contains(where: { !$0.knowledgePoints.isEmpty })
                            ? String(localized: "information.recentKnowledge") : String(localized: "information.recentConversations"))
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(DesignTokens.backgroundColor)
            .background(IOSWorkspaceWidthObserver(width: $pageWidth))
            .navigationTitle(NSLocalizedString("navigation.information", value: "Information", comment: "Information tab"))
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(for: IOSWorkspaceDestination.self) { item in
                IOSWorkspaceManagementContent(destination: item,
                    notificationKey: item == .records ? activeRequest?.notificationKey : nil,
                    notificationRequestID: item == .records ? activeRequest?.id : nil)
            }
        }
        .task { await loadDirectory(force: false) }
        .refreshable { await loadDirectory(force: true) }
        .onAppear { consumePendingRequest() }
        .onChange(of: pendingRequests) { _, _ in consumePendingRequest() }
        .onChange(of: path) { _, next in
            if next.isEmpty { activeRequest = nil; consumePendingRequest() }
        }
    }

    private var recentKnowledge: [IOSConversationDirectoryEntry] {
        let saved = directory.filter { !$0.knowledgePoints.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        return saved.isEmpty ? directory : saved
    }

    private func loadDirectory(force: Bool) async {
        guard force || directoryLoadedAt.map({ Date().timeIntervalSince($0) >= 30 }) ?? true else { return }
        let runtime = container.applicationRuntime
        do {
            let loaded = try await Task.detached { try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime) }.value
            try Task.checkCancellation()
            directory = loaded; directoryLoadedAt = Date(); directoryError = nil
        } catch is CancellationError { }
        catch { directoryError = error.localizedDescription }
    }

    private func consumePendingRequest() {
        // Never replace a pushed page: it may own an editor or permission sheet.
        // External requests wait until the user returns to the Information root.
        guard path.isEmpty, let request = pendingRequests.first else { return }
        activeRequest = request
        path.append(request.destination)
        pendingRequests.removeFirst()
    }
}


/// A reading surface over existing conversation knowledge, with the conversation
/// as the editing destination. No synthetic global knowledge store is introduced.
private struct IOSKnowledgePointsView: View {
    @State private var pageWidth: CGFloat = 390
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.dismiss) private var dismiss
    @State private var conversations: [IOSConversationDirectoryEntry] = []
    @State private var loadError: String?

    var body: some View {
        List {
            if let loadError {
                Text(loadError).foregroundStyle(.secondary)
                Button(String(localized: "common.retry")) { Task { await reload() } }.frame(minHeight: 44)
            } else if conversations.isEmpty {
                Text(NSLocalizedString("information.knowledge.empty", value: "No conversation knowledge points yet.", comment: "Empty knowledge list"))
                    .foregroundStyle(.secondary).padding(.vertical, 20).listRowSeparator(.hidden)
                    .listRowBackground(DesignTokens.backgroundColor)
            }
            ForEach(conversations) { conversation in
                VStack(alignment: .leading, spacing: 10) {
                    Text(IOSConversationTitlePresentationPolicy.displayTitle(conversation.title,
                        conversationID: conversation.id.uuidString, fallback: L10n.chatConversationDefaultTitle))
                        .font(.body.weight(.semibold))
                    ChatMarkdownView(content: conversation.knowledgePoints).textSelection(.enabled)
                    Button {
                        dismiss()
                        NotificationCenter.default.post(name: .nolaOpenChatTab, object: conversation.id)
                    } label: {
                        Label(L10n.chatTitle, systemImage: "bubble.left").frame(minHeight: 44)
                    }.buttonStyle(.borderless)
                }
                .listRowSeparator(.hidden)
                .listRowBackground(DesignTokens.backgroundColor)
                .listRowInsets(EdgeInsets(top: 12, leading: pageWidth < 359 ? 16 : 20, bottom: 12, trailing: pageWidth < 359 ? 16 : 20))
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(DesignTokens.backgroundColor)
        .background(IOSWorkspaceWidthObserver(width: $pageWidth))
        .task { await reload() }
        .refreshable { await reload() }
        .accessibilityIdentifier("information.knowledge.list")
    }

    private func reload() async {
        let runtime = container.applicationRuntime
        do {
            let loaded = try await Task.detached { try IOSWorkspaceReadFacts.conversationDirectory(runtime: runtime) }.value
            try Task.checkCancellation()
            conversations = loaded.filter { !$0.knowledgePoints.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            loadError = nil
        } catch is CancellationError { }
        catch { loadError = error.localizedDescription }
    }
}

struct IOSWorkspaceWidthObserver: View {
    @Binding var width: CGFloat
    var body: some View {
        GeometryReader { geometry in
            Color.clear.onChange(of: geometry.size.width, initial: true) { _, value in
                if value > 0 { width = value }
            }
        }
    }
}

struct IOSWorkspacePageHeading: View {
    let title: String
    var subtitle: String? = nil
    @ScaledMetric(relativeTo: .title) private var titleSize: CGFloat = 28
    @ScaledMetric(relativeTo: .title) private var titleLineSpacing: CGFloat = 3
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.system(size: titleSize, weight: .semibold))
                .lineSpacing(titleLineSpacing).foregroundStyle(.primary)
                .accessibilityAddTraits(.isHeader)
            if let subtitle {
                Text(subtitle).font(.subheadline).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}


struct IOSConversationActivityView: View {
    @EnvironmentObject private var container: ServiceContainer
    @State private var nextPlan: AppleTodayQueuedInstance?
    @State private var plansLoadedAt: Date?
    @State private var plansError: String?
    var body: some View {
        let runtime = container.applicationRuntime
        VStack(spacing: 0) {
        NavigationLink { ScheduledActionsView(embedded: true) } label: {
            HStack(spacing: 12) {
                Image(systemName: "calendar").font(.title3).foregroundStyle(DesignTokens.actionColor)
                VStack(alignment: .leading, spacing: 4) {
                    Text("workspace.plans").font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                    if let nextPlan {
                        Text(nextPlan.title ?? String(localized: "workspace.plans")).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                        Text(Date(timeIntervalSince1970: Double(nextPlan.scheduled_for_ms) / 1000),
                            format: .dateTime.month().day().hour().minute()).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 8)
            }.frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).padding(14)
                .background(DesignTokens.surfaceColor, in: RoundedRectangle(cornerRadius: 14))
                .contentShape(Rectangle())
        }.buttonStyle(.plain).padding(.horizontal, 16).padding(.top, 8)
            .accessibilityIdentifier("activity.plans")
        if let plansError { Text(plansError).font(.footnote).foregroundStyle(.secondary).padding(.horizontal) }
        AppleActivityView(load: { query in
            try await Task.detached { try IOSWorkspaceReadFacts.activity(runtime: runtime, query: query) }.value
        }, detail: { entry in
            try await Task.detached { try IOSWorkspaceReadFacts.activityDetail(runtime: runtime, entry: entry) }.value
        }, openSource: { source in
            if let id = UUID(uuidString: source) {
                NotificationCenter.default.post(name: .nolaOpenChatTab, object: id)
            }
        }, canOpenSource: { UUID(uuidString: $0) != nil })
        }
        .task {
            guard plansLoadedAt.map({ Date().timeIntervalSince($0) >= 30 }) ?? true else { return }
            do {
                let facts = try await Task.detached { try IOSWorkspaceReadFacts.snapshot(runtime: runtime) }.value
                try Task.checkCancellation()
                nextPlan = facts.queued_instances.min { $0.scheduled_for_ms < $1.scheduled_for_ms }
                plansLoadedAt = Date(); plansError = nil
            } catch is CancellationError { }
            catch { plansError = error.localizedDescription }
        }
    }
}
