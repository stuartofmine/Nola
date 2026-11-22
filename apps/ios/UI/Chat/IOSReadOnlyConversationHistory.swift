// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// The writer uses Foundation UUIDs. Reading uses the exact case-sensitive Core key.
struct IOSReadOnlyConversationHistory: View {
    struct Selection: Identifiable { let id: String }
    let selection: Selection
    let runtime: IOSApplicationRuntime
    @Environment(\.dismiss) private var dismiss
    @State private var messages: [ChatMessage] = []
    @State private var title = ""
    @State private var beforeOrdinal: Int64?
    @State private var loaded = false
    @State private var loading = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 18) {
                    if let error {
                        Text(error).foregroundStyle(.secondary)
                        Button("common.retry") { Task { await load() } }
                    } else if loading || !loaded {
                        ProgressView().frame(maxWidth: .infinity)
                    } else if beforeOrdinal != nil {
                        Button("Load earlier messages") { Task { await load() } }
                            .frame(maxWidth: .infinity, minHeight: 44)
                    } else if messages.isEmpty {
                        Text("chat.history.no_messages").foregroundStyle(.secondary)
                    }
                    ForEach(messages) { message in
                        ChatBubble(conversationID: nil, message: message, isLatestAssistant: false,
                            isStreaming: false, isRetrieving: false, pendingHostPermissionRequest: nil,
                            onHostPermissionDecision: { _, _ in }, surfaceInteractionsEnabled: false,
                            readOnlyHistoryOwner: selection.id)
                    }
                }
                .padding(20)
            }
            .background(DesignTokens.chatBackgroundColor)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button(L10n.done) { dismiss() } } }
            .safeAreaInset(edge: .bottom) {
                Text("chat.history.otherClientReadOnly").font(.footnote)
                    .foregroundStyle(DesignTokens.secondaryContentColor)
                    .frame(maxWidth: .infinity).padding(12).background(DesignTokens.backgroundColor)
            }
            .accessibilityIdentifier("chat.history.exactReader")
            .task { await load() }
        }
    }

    private func load() async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        let owner = selection.id, before = beforeOrdinal, runtime = runtime
        do {
            let window = try await Task.detached {
                try runtime.loadConversationProjectionWindowV1(id: owner, beforeOrdinal: before,
                    limit: 40, contentByteLimit: 512_000)
            }.value
            try Task.checkCancellation()
            guard window.projection.conversationID == owner else { throw IOSApplicationRuntimeError.invalidResponse }
            if window.hasMore {
                guard let next = window.beforeOrdinal, before.map({ next < $0 }) ?? true else {
                    throw IOSApplicationRuntimeError.invalidResponse
                }
            }
            let known = Set(messages.map(\.id))
            let earlier = IOSConversationProjectionPresenter.messages(from: window.projection).filter { !known.contains($0.id) }
            messages.insert(contentsOf: earlier, at: 0)
            beforeOrdinal = window.hasMore ? window.beforeOrdinal : nil
            title = IOSConversationTitlePresentationPolicy.displayTitle(window.projection.title,
                conversationID: owner, fallback: L10n.chatConversationDefaultTitle)
            loaded = true
        } catch is CancellationError {} catch { self.error = error.localizedDescription }
    }
}
