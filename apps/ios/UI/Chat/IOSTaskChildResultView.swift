// SPDX-License-Identifier: Apache-2.0
// Read-only inspection of a Core-verified task child; the parent remains selected.
import SwiftUI

struct IOSTaskChildResultView: View {
    @EnvironmentObject private var container: ServiceContainer
    let destination: AppleTaskChildDestination
    @State private var messages: [ChatMessage] = []
    @State private var title = ""
    @State private var beforeOrdinal: Int64?
    @State private var hasMore = false
    @State private var loading = false
    @State private var failure: String?

    @State private var recovery: IOSUnknownEffectRecoveryInspection?
    @State private var pendingRecovery: IOSPendingUnknownEffectRecovery?
    @State private var recoveryReason = ""
    @State private var recoveryStatus: String?
    @State private var confirmingRecovery = false

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 20) {
                Text(String(localized: "Task conversation · Read only"))
                    .font(.caption).foregroundStyle(.secondary)
                recoveryActions
                if hasMore {
                    Button(String(localized: "Load earlier messages")) {
                        Task { await load(earlier: true) }
                    }.disabled(loading).frame(minHeight: 44)
                }
                ForEach(messages, id: \.id) { message in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(message.role == .user ? String(localized: "Task instructions") : String(localized: "Assistant"))
                            .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                        if !message.thinking.isEmpty {
                            DisclosureGroup(String(localized: "Reasoning")) {
                                ChatMarkdownView(content: message.thinking)
                            }.font(.callout).foregroundStyle(.secondary)
                        }
                        if !message.content.isEmpty { ChatMarkdownView(content: message.content) }
                        ForEach(message.hostToolEvents, id: \.id) { event in
                            Text(event.display.isEmpty ? event.name : event.display)
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("chat.task.result.\(message.role == .assistant ? "assistant" : "instructions").\(message.id.uuidString)")
                }
                if loading { ProgressView().frame(maxWidth: .infinity) }
                if let failure {
                    Text(failure).font(.callout).foregroundStyle(.secondary)
                        .accessibilityIdentifier("chat.task.result.error")
                    Button(String(localized: "common.retry")) { Task { await load(earlier: false) } }
                        .frame(minHeight: 44)
                }
            }.padding(20).textSelection(.enabled)
        }
        .navigationTitle(title.isEmpty ? String(localized: "Task result") : title)
        .navigationBarTitleDisplayMode(.inline)
        .accessibilityIdentifier("chat.task.result.readOnly")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { Task { await load(earlier: false) } } label: {
                    Image(systemName: "arrow.clockwise").frame(minWidth: 44, minHeight: 44)
                }.disabled(loading).accessibilityLabel(String(localized: "Refresh"))
            }
        }
        .task(id: destination) { await load(earlier: false) }
    }

    private var recoveryActions: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button(String(localized: "chat.recovery.check_status")) {
                do {
                    let result = try container.chatManager.inspectTaskRecovery(destination)
                    recovery = result.0; pendingRecovery = result.1
                    recoveryStatus = result.0 == nil && result.1 == nil ? String(localized: "task.recovery.none") : nil
                } catch { recoveryStatus = error.localizedDescription }
            }.frame(minHeight: 44)
            if pendingRecovery != nil {
                Text(String(localized: "chat.recovery.pending"))
                Button(String(localized: "chat.recovery.check_receipt")) { resolveRecovery() }
                    .frame(minHeight: 44)
            } else if let recovery {
                Text(String(localized: "chat.recovery.confirm_block"))
                TextField(String(localized: "chat.recovery.reason"), text: $recoveryReason, axis: .vertical)
                    .textFieldStyle(.roundedBorder).lineLimit(2...4)
                Button(String(localized: "chat.recovery.keep_blocked")) { confirmingRecovery = true }
                    .frame(minHeight: 44)
                    .disabled(recovery.independent_pending_effects != 0 || recovery.targets.isEmpty
                              || recoveryReason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                if recovery.independent_pending_effects != 0 { Text(String(localized: "chat.recovery.independent_work")) }
            }
            if let recoveryStatus { Text(recoveryStatus).font(.callout).foregroundStyle(.secondary) }
        }
        .accessibilityIdentifier("chat.task.recovery")
        .confirmationDialog(String(localized: "chat.recovery.confirm_title"), isPresented: $confirmingRecovery,
                            titleVisibility: .visible) {
            Button(String(localized: "chat.recovery.keep_blocked"), role: .destructive) { resolveRecovery() }
            Button(String(localized: "common.cancel"), role: .cancel) {}
        } message: { Text(String(localized: "chat.recovery.confirm_block")) }
    }

    private func resolveRecovery() {
        do {
            let reply = try container.chatManager.resolveTaskRecovery(destination, inspection: recovery,
                pending: pendingRecovery, reason: recoveryReason)
            recoveryStatus = reply.outcome == .unknownPreservedBlocked
                ? String(localized: "chat.recovery.blocked") : String(localized: "chat.recovery.changed")
            recovery = nil; pendingRecovery = nil
        } catch {
            recoveryStatus = error.localizedDescription
            // Transport uncertainty retains the exact command in Application's existing journal.
            if let result = try? container.chatManager.inspectTaskRecovery(destination) {
                recovery = result.0; pendingRecovery = result.1
            }
        }
    }

    @MainActor private func load(earlier: Bool) async {
        guard !loading else { return }
        loading = true
        defer { loading = false }
        do {
            let result = try await container.chatManager.loadTaskChildWindow(destination,
                beforeOrdinal: earlier ? beforeOrdinal : nil)
            try Task.checkCancellation()
            if earlier {
                let existing = Set(messages.map(\.id))
                messages = result.messages.filter { !existing.contains($0.id) } + messages
            } else { messages = result.messages }
            title = result.window.projection.title
            beforeOrdinal = result.window.beforeOrdinal
            hasMore = result.window.hasMore
            failure = nil
        } catch is CancellationError {
            // Navigation away leaves the parent draft and execution unchanged.
        } catch { failure = error.localizedDescription }
    }
}
