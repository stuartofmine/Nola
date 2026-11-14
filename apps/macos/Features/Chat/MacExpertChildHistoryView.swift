// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import Observation

struct MacExpertChildPresentation: Identifiable {
    let parentID: String
    let child: AppleExpertChildConversation
    let sourceID: ObjectIdentifier
    var id: String { parentID + ":" + child.id + ":" + String(describing: sourceID) }
}

/// The existing canonical transcript reader, without a composer or execution actions.
struct MacExpertChildHistoryView: View {
    let presentation: MacExpertChildPresentation
    let execution: MacRoomExecutionService?
    let continueInParent: () async throws -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var reader: MacExpertChildHistoryReader?
    @State private var error: String?
    @State private var continuing = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(presentation.child.title).font(.headline).lineLimit(2)
                    Text(presentation.child.isCurrent && presentation.child.active
                         ? "Expert conversation · read only" : "Previous expert configuration · read only")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction).buttonStyle(MacActionButtonStyle())
            }.padding(16)
            Divider()
            if let reader {
                MacTranscriptView(conversationID: presentation.child.conversationID,
                    messages: reader.messages, isRunning: false, liveActivities: [], searchText: "", bottomClearance: 0,
                    onSendTaskGuidance: { _, _, _ in }, onRequestTaskCancellation: { _, _ in },
                    hasEarlierStoredMessages: reader.hasMore, isLoadingHistoryPage: reader.isLoading,
                    onLoadEarlierHistory: { Task { await reader.load(earlier: true) } },
                    readUIChanges: { identity, revision in
                        guard let source = reader.sourceOwner, let execution else { throw MacApplicationPortError.invalidResponse }
                        return try await execution.readHistoryUIChanges(childID: presentation.child.id, source: source, identity: identity, baseRevision: revision)
                    },
                    readUIRevision: { reference in
                        guard let source = reader.sourceOwner, let execution else { throw MacApplicationPortError.invalidResponse }
                        return try await execution.readHistoryUIRevision(childID: presentation.child.id, source: source, reference: reference)
                    },
                    readLocalSource: { reader.sourceOwner == .local }, permitsFollowupUIActions: false,
                    sourceAuthors: reader.sourceAuthors)
                    .environment(\.macTranscriptReadOnly, true)
                    .overlay {
                        if reader.messages.isEmpty && !reader.isLoading && reader.error == nil {
                            ContentUnavailableView { Label("No expert messages yet", systemImage: "bubble.left") }
                        }
                    }
                    .accessibilityIdentifier("expert.child.transcript")
                if reader.isLoading { ProgressView().controlSize(.small).padding() }
                if let problem = reader.error {
                    HStack {
                        Text(problem).font(.caption).foregroundStyle(.red).textSelection(.enabled)
                        Button("Retry") { Task { await reader.load(earlier: !reader.messages.isEmpty) } }
                            .buttonStyle(MacActionButtonStyle()).disabled(reader.isLoading)
                    }.padding()
                }
            } else if error == nil { ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity) }
            if let error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled).padding() }
            if presentation.child.isCurrent && presentation.child.active {
                Divider()
                HStack {
                    Spacer()
                    Button("Continue in main conversation") {
                        continuing = true; error = nil
                        Task { @MainActor in
                            do { try await continueInParent(); dismiss() }
                            catch { self.error = error.localizedDescription }
                            continuing = false
                        }
                    }.buttonStyle(MacActionButtonStyle()).disabled(continuing || reader == nil)
                        .accessibilityIdentifier("expert.child.continue")
                }.padding(12)
            }
        }.background(MacDesignColor.canvas).frame(minWidth: 560, minHeight: 480)
            .task(id: presentation.id) {
                guard let execution else { error = String(localized: "Expert history is unavailable"); return }
                let database = execution.historyDatabaseURL
                let parent = presentation.parentID, child = presentation.child.id
                let value = MacExpertChildHistoryReader(parentID: parent, childID: child, databaseURL: database) { before in
                    try await Task.detached {
                        try MacAgentRoomService(runtime: MacApplicationPort(databaseURL: database))
                            .childHistory(parentID: parent, childID: child, before: before)
                    }.value
                }
                reader = value
                await value.load()
            }
            .onDisappear { reader?.invalidate(); reader = nil }
    }
}


/// Owns one scoped read snapshot. It never constructs a workspace/execution owner.
@Observable @MainActor final class MacExpertChildHistoryReader {
    typealias Page = AppleExpertChildHistory<MacCanonicalConversationWindow>
    private let parentID: String
    private let childID: String
    private let databaseURL: URL?
    private let read: (Int64?) async throws -> Page
    private var generation = UUID()
    private var room: (id: String, revision: Int64)?
    private var before: Int64?
    private(set) var messages: [MacChatMessage] = []
    private(set) var sourceAuthors: [String: String] = [:]
    private(set) var sourceOwner: AppleConversationSourceOwner?
    private(set) var hasMore = false
    private(set) var isLoading = false
    private(set) var error: String?
    init(parentID: String, childID: String, databaseURL: URL? = nil, read: @escaping (Int64?) async throws -> Page) {
        self.parentID = parentID; self.childID = childID; self.databaseURL = databaseURL; self.read = read
    }
    func invalidate() { generation = UUID(); isLoading = false }
    func load(earlier: Bool = false) async {
        guard !isLoading, !earlier || hasMore else { return }
        let token = generation, cursor = earlier ? before : nil
        isLoading = true; error = nil
        defer { if generation == token { isLoading = false } }
        do {
            let page = try await read(cursor)
            guard generation == token, !Task.isCancelled else { return }
            try page.validate(parentID: parentID, childID: childID)
            guard page.window.projection.conversationID == childID,
                  sourceOwner == nil || sourceOwner == page.sourceOwner,
                  !earlier || (room?.id == page.roomID && room?.revision == page.roomRevision),
                  !page.window.hasMore || (page.window.beforeOrdinal != nil && !page.window.projection.messages.isEmpty),
                  cursor == nil || !page.window.hasMore || page.window.beforeOrdinal! < cursor! else { throw MacApplicationPortError.invalidResponse }
            let database = databaseURL
            let projected = try await Task.detached {
                try MacCanonicalConversationRepository.projectMessages(page.window, databaseURL: database, sourceOwner: page.sourceOwner)
            }.value
            guard generation == token, !Task.isCancelled else { return }
            let authors = try Self.authors(page.expertExchanges, parentID: parentID, childID: childID,
                userMessageIDs: Set(page.window.projection.messages.filter { $0.role == .user }.map(\.messageID)))
            if earlier {
                guard Set(messages.map(\.id)).isDisjoint(with: projected.map(\.id)) else { throw MacApplicationPortError.invalidResponse }
                messages = projected + messages; sourceAuthors.merge(authors) { old, _ in old }
            } else { messages = projected; sourceAuthors = authors }
            sourceOwner = page.sourceOwner; room = (page.roomID, page.roomRevision)
            before = page.window.beforeOrdinal; hasMore = page.window.hasMore
        } catch { if generation == token && !Task.isCancelled { self.error = error.localizedDescription } }
    }
    static func authors(_ exchanges: [AppleExpertPeerView], parentID: String, childID: String, userMessageIDs: Set<String>) throws -> [String: String] {
        var result: [String: String] = [:]
        for exchange in exchanges {
            let messageID = exchange.exchangeID + ":user"
            guard exchange.version == 1, exchange.parentConversationID == parentID,
                  exchange.target.context_conversation_id == childID, userMessageIDs.contains(messageID),
                  result[messageID] == nil else { throw MacApplicationPortError.invalidResponse }
            result[messageID] = exchange.displayName(for: exchange.source) ?? (exchange.source.isMainAgent ? "Nola" : String(localized: "Expert"))
        }
        return result
    }
}
