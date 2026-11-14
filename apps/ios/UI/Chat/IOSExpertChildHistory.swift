// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import Combine

nonisolated struct IOSExpertHistoryContinuation: Equatable, Sendable {
    let parentID: UUID
    let roomID: String
    let child: AppleExpertChildConversation

    func participant(in association: IOSConversationRoomAssociation) throws -> IOSRoomParticipant {
        guard association.conversationID == parentID.uuidString,
              association.roomID == roomID, child.isCurrent, child.active,
              let member = association.room.room.participants.first(where: {
                  $0.id == child.participantID && $0.agentID == child.agentID && $0.active &&
                  $0.contextConversationID == child.conversationID && $0.agentRevision == child.agentRevision
              }) else { throw IOSApplicationRuntimeError.rejected(String(localized: "expert.history.changed")) }
        return member
    }
}

nonisolated struct IOSExpertPeerInputAuthor: Equatable, Sendable {
    let participant: AppleExpertTaskSource
    let name: String

    static func map(_ exchanges: [AppleExpertPeerView], ownerID: String, parentID: String,
                    roomID: String) throws -> [String: Self] {
        var authors: [String: Self] = [:]
        func record(_ messageID: String, source: AppleExpertTaskSource, view: AppleExpertPeerView) throws {
            guard !messageID.isEmpty, source.isValid && (!source.isMainAgent || source.context_conversation_id == parentID) else { throw IOSApplicationRuntimeError.invalidResponse }
            let author = Self(participant: source, name: view.displayName(for: source) ?? (source.isMainAgent ? "Nola" : String(localized: "expert.history.peerName")))
            guard authors[messageID] == nil || authors[messageID] == author else { throw IOSApplicationRuntimeError.invalidResponse }
            authors[messageID] = author
        }
        for exchange in exchanges {
            guard exchange.version == 1, exchange.kind == "request", exchange.parentConversationID == parentID,
                  exchange.roomID == roomID else { throw IOSApplicationRuntimeError.invalidResponse }
            if exchange.target.context_conversation_id == ownerID {
                try record(exchange.exchangeID + ":user", source: exchange.source, view: exchange)
            }
            for clarification in exchange.clarifications where clarification.target.context_conversation_id == ownerID {
                try record(clarification.messageID, source: .legacyExpert(clarification.source), view: exchange)
            }
        }
        return authors
    }
}

/// One parent and one source runtime for the entire expanded branch. No message
/// projection is fetched until a specific child is opened.
@MainActor final class IOSExpertChildrenState: ObservableObject {
    let parentID: String
    @Published private(set) var children: [AppleExpertChildConversation] = []
    @Published private(set) var roomID: String?
    @Published private(set) var roomRevision: Int64?
    @Published private(set) var nextOffset: Int?
    @Published private(set) var isLoading = false
    @Published private(set) var loaded = false
    @Published private(set) var error: String?
    private let read: @Sendable (String, Int) throws -> AppleExpertChildConversations
    private var generation = UUID()

    init(parentID: String, read: @escaping @Sendable (String, Int) throws -> AppleExpertChildConversations) {
        self.parentID = parentID; self.read = read
    }
    convenience init(parentID: String, runtime: IOSApplicationRuntime) {
        let service = IOSAgentRoomService(runtime: runtime)
        self.init(parentID: parentID) { try service.childConversations(conversationID: $0, offset: $1) }
    }
    func load(reload: Bool = false) async {
        guard (!isLoading || reload), reload || !loaded || nextOffset != nil else { return }
        let token = UUID(); generation = token; isLoading = true; error = nil
        let offset = reload ? 0 : (loaded ? nextOffset ?? 0 : 0)
        let read = read, parentID = parentID
        defer { if generation == token { isLoading = false } }
        do {
            let page = try await Task.detached { try read(parentID, offset) }.value
            guard !Task.isCancelled, generation == token else { return }
            try page.validate(parentID: parentID, offset: offset)
            if offset > 0 {
                guard page.roomID == roomID, page.roomRevision == roomRevision,
                      Set(children.map(\.id)).isDisjoint(with: page.children.map(\.id)) else {
                    throw IOSApplicationRuntimeError.rejected(String(localized: "expert.history.changed"))
                }
                children += page.children
            } else { children = page.children }
            roomID = page.roomID; roomRevision = page.roomRevision
            nextOffset = page.nextOffset; loaded = true
        } catch { if generation == token { self.error = error.localizedDescription } }
    }
}

struct IOSExpertChildrenBranch: View {
    @StateObject private var state: IOSExpertChildrenState
    let onContinue: (IOSExpertHistoryContinuation) -> Void
    @State private var selected: AppleExpertChildConversation?
    let runtime: IOSApplicationRuntime
    let refreshRevision: Int
    init(parentID: String, runtime: IOSApplicationRuntime, refreshRevision: Int = 0, onContinue: @escaping (IOSExpertHistoryContinuation) -> Void) {
        self.runtime = runtime; self.onContinue = onContinue; self.refreshRevision = refreshRevision
        _state = StateObject(wrappedValue: IOSExpertChildrenState(parentID: parentID, runtime: runtime))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(state.children) { child in
                Button { selected = child } label: {
                    HStack(spacing: 10) {
                        Image(systemName: "bubble.left").font(.caption).foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(child.title).font(.subheadline).foregroundStyle(.primary)
                            Text(String(localized: child.isCurrent && child.active ? "expert.history.current" : "expert.history.previous"))
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 4)
                        Text(child.updatedAtMs > 0 ? Date(timeIntervalSince1970: Double(child.updatedAtMs) / 1000).formatted(.dateTime.month().day()) : "")
                            .font(.caption).foregroundStyle(.secondary)
                    }.padding(.vertical, 8).frame(maxWidth: .infinity, minHeight: 52, alignment: .leading).contentShape(Rectangle())
                }.buttonStyle(NolaPressableButtonStyle()).accessibilityIdentifier("chat.child." + child.id)
            }
            if state.isLoading { ProgressView().frame(minHeight: 44).accessibilityIdentifier("chat.children.loading") }
            if let error = state.error {
                Text(error).font(.caption).foregroundStyle(.secondary)
                Button("common.retry") { Task { await state.load(reload: true) } }.frame(minHeight: 44)
                    .accessibilityIdentifier("chat.children.retry")
            } else if state.loaded && state.children.isEmpty {
                Text("expert.history.empty").font(.caption).foregroundStyle(.secondary).padding(.vertical, 12)
            }
            if state.nextOffset != nil && !state.isLoading && state.error == nil {
                Button("expert.history.more") { Task { await state.load() } }.frame(minHeight: 44)
                    .accessibilityIdentifier("chat.children.more")
            }
        }
        .task(id: refreshRevision) { await state.load(reload: true) }
        .sheet(item: $selected) { child in
            if let roomID = state.roomID {
                IOSExpertChildHistory(parentID: state.parentID, roomID: roomID, child: child, runtime: runtime) { request in
                    selected = nil; onContinue(request)
                }
            }
        }
    }
}

/// Uses the existing canonical expert timeline with execution observation,
/// permission controls, edits and surface submissions disabled.
struct IOSExpertChildHistory: View {
    @EnvironmentObject private var container: ServiceContainer
    @Environment(\.dismiss) private var dismiss
    let parentID: String
    let roomID: String
    let child: AppleExpertChildConversation
    let runtime: IOSApplicationRuntime
    let onContinue: (IOSExpertHistoryContinuation) -> Void
    @StateObject private var workspace = IOSAgentRoomWorkspace()
    @State private var error: String?
    @State private var isLoading = true
    @State private var canContinue = false
    @State private var peerAuthors: [String: IOSExpertPeerInputAuthor] = [:]
    @State private var historyBeforeOrdinal: Int64?
    @State private var historySourceOwner: AppleConversationSourceOwner = .unavailable
    @State private var openRequest = UUID()
    @State private var activePeerRead: Task<AppleExpertChildHistory<IOSConversationProjectionWindowV1>, Error>?
    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let error {
                    ContentUnavailableView { Label("expert.history.unavailable", systemImage: "exclamationmark.bubble") }
                    description: { Text(error) }
                    actions: { Button("common.retry") { Task { await open() } }.frame(minHeight: 44) }
                } else if isLoading { ProgressView().frame(maxHeight: .infinity) }
                else {
                    if workspace.rows.isEmpty { ContentUnavailableView("expert.history.empty", systemImage: "bubble.left") }
                    else { IOSAgentRoomTimeline(workspace: workspace, showsOwnerControls: false, allowsLiveExecution: false, peerInputAuthors: peerAuthors, loadOlderHistory: loadOlder) }
                    if canContinue {
                        Button("expert.history.continue") {
                            if let id = UUID(uuidString: parentID), id.uuidString == parentID {
                                onContinue(.init(parentID: id, roomID: roomID, child: child))
                            }
                        }
                            .frame(maxWidth: .infinity, minHeight: 44).padding(.horizontal, 20)
                            .accessibilityIdentifier("chat.child.continue")
                    } else {
                        Text(String(localized: child.isCurrent && child.active && UUID(uuidString: parentID)?.uuidString != parentID
                            ? "expert.history.unsupportedParent" : "expert.history.readOnly"))
                            .font(.footnote).foregroundStyle(.secondary).padding(12)
                    }
                }
            }
            .background(DesignTokens.backgroundColor)
            .navigationTitle(child.title).navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button(L10n.done) { dismiss() }.accessibilityIdentifier("chat.child.done") } }
            .task { await open() }
            .onDisappear { openRequest = UUID(); activePeerRead?.cancel(); activePeerRead = nil; workspace.close() }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("chat.child.history")
        }
    }
    @MainActor private func open() async {
        let request = UUID(); openRequest = request
        activePeerRead?.cancel(); activePeerRead = nil
        isLoading = true; error = nil; canContinue = false
        defer { if openRequest == request { isLoading = false; activePeerRead = nil } }
        do {
            guard runtime === container.applicationRuntime,
                  let association = try IOSAgentRoomService(runtime: runtime).forConversation(parentID),
                  association.roomID == roomID else { throw IOSApplicationRuntimeError.invalidResponse }
            let parentSource = try runtime.conversationSourceOwner(conversationID: parentID)
            let childSource = try runtime.conversationSourceOwner(conversationID: child.conversationID)
            guard parentSource != .unavailable, parentSource == childSource else {
                throw IOSApplicationRuntimeError.rejected(String(localized: "expert.history.unavailable"))
            }
            let value = try runtime.executeAgentCommand(.object(["type": .string("load_conversation_metadata"),
                "conversation_id": .string(child.conversationID)]))
            guard case .object(let fields) = value else { throw IOSApplicationRuntimeError.invalidResponse }
            let settings = try IOSConversationSettingsObject.decode(fields["settings_json"]?.stringScalar)
            guard let storedOwner = settings["agentRoomOwner"] else { throw IOSApplicationRuntimeError.invalidResponse }
            let owner = try ConversationAgentRoomOwner.read(storedOwner)
            guard owner.roomID == roomID, owner.participantID == child.participantID, owner.agentID == child.agentID else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            guard UUID(uuidString: child.conversationID)?.uuidString == child.conversationID else {
                throw IOSApplicationRuntimeError.rejected(String(localized: "expert.history.unsupportedParent"))
            }
            let participant = IOSRoomParticipant(id: child.participantID, agentID: child.agentID,
                agentRevision: child.agentRevision ?? 0, displayName: child.title, description: "",
                contextConversationID: child.conversationID, active: child.active)
            let response = try await readScopedWindow(beforeOrdinal: nil)
            guard !Task.isCancelled, openRequest == request else { return }
            guard response.roomID == roomID, response.sourceOwner == parentSource else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            peerAuthors = try IOSExpertPeerInputAuthor.map(response.expertExchanges, ownerID: child.conversationID, parentID: parentID, roomID: roomID)
            let snapshot = IOSAgentRoomSnapshot(revision: response.roomRevision, room: .init(id: roomID, title: child.title, participants: [participant]))
            try workspace.installReadOnlyHistory(snapshot, window: response.window)
            historyBeforeOrdinal = response.window.hasMore ? response.window.beforeOrdinal : nil
            historySourceOwner = response.sourceOwner
            if parentSource == .local, let id = UUID(uuidString: parentID), id.uuidString == parentID,
               let fresh = try? IOSAgentRoomService(runtime: runtime).forConversation(parentID) {
                canContinue = (try? IOSExpertHistoryContinuation(parentID: id, roomID: roomID, child: child).participant(in: fresh)) != nil
            }
        } catch { if openRequest == request { self.error = error.localizedDescription; workspace.close() } }
    }

    @MainActor private func readScopedWindow(beforeOrdinal: Int64?) async throws -> AppleExpertChildHistory<IOSConversationProjectionWindowV1> {
        let service = IOSAgentRoomService(runtime: runtime), parentID = parentID, childID = child.conversationID
        let read = Task.detached { try service.childHistory(conversationID: parentID, childID: childID, beforeOrdinal: beforeOrdinal) }
        activePeerRead = read
        return try await withTaskCancellationHandler(operation: { try await read.value }, onCancel: { read.cancel() })
    }

    @MainActor private func loadOlder() async throws {
        guard let cursor = historyBeforeOrdinal else { return }
        let request = openRequest
        defer { if openRequest == request { activePeerRead = nil } }
        let response = try await readScopedWindow(beforeOrdinal: cursor)
        guard !Task.isCancelled, openRequest == request else { throw CancellationError() }
        guard response.roomID == roomID, response.sourceOwner == historySourceOwner else { throw IOSApplicationRuntimeError.invalidResponse }
        let authors = try IOSExpertPeerInputAuthor.map(response.expertExchanges, ownerID: child.conversationID, parentID: parentID, roomID: roomID)
        for (id, author) in authors where peerAuthors[id] != nil && peerAuthors[id] != author { throw IOSApplicationRuntimeError.invalidResponse }
        peerAuthors.merge(authors) { first, _ in first }
        try workspace.appendReadOnlyHistory(response.window)
        historyBeforeOrdinal = response.window.hasMore ? response.window.beforeOrdinal : nil
    }

}

struct IOSExpertPeerInputMessage: View {
    let message: ChatMessage
    let messageID: String
    let author: IOSExpertPeerInputAuthor
    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(spacing: 6) {
                AppleExpertAvatar(name: author.name, size: 30)
                AppleExpertName(name: author.name, alignment: .center)
            }.frame(width: 38)
            ChatMarkdownView(content: message.content)
                .textSelection(.enabled)
                .padding(12)
                .background(DesignTokens.rowFillColor, in: RoundedRectangle(cornerRadius: 14))
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("room.expert.peer." + messageID)
    }
}
