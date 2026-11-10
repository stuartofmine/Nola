// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct IOSRoomConversationView: View {
    @ObservedObject var roomWorkspace: IOSAgentRoomWorkspace
    @StateObject private var submission: IOSRoomSubmissionWorkspace
    @FocusState private var focused: Bool
    @State private var highlightedMention: String?
    @State private var dismissedMentionText: String?

    init(workspace: IOSAgentRoomWorkspace, submission: IOSRoomSubmissionWorkspace) {
        roomWorkspace = workspace
        _submission = StateObject(wrappedValue: submission)
    }

    init(workspace: IOSAgentRoomWorkspace, runtime: IOSApplicationRuntime, chatManager: ChatManager,
         routingProvider: @escaping () -> LLMClient) {
        roomWorkspace = workspace
        let execution = IOSRoomExecutionService(runtime: runtime, manager: chatManager)
        let submission = IOSRoomSubmissionWorkspace(
            service: IOSAgentRoomService(runtime: runtime),
            executeOwner: { try await execution.execute($0, intent: $1) })
        submission.prepareOwner = { try await execution.prepare($0) }
        submission.resumePreparedExpertOwner = { try await chatManager.resumePreparedExpertConversation($0) }
        submission.recoverOwner = { try await execution.recover($0, intent: $1) }
        submission.cancelPreparedOwner = { try await execution.cancelPrepared($0) }
        let routing = IOSRoomRoutingService(runtime: runtime, provider: routingProvider)
        submission.routeSubmission = { try await routing.route($0, room: $1) }
        _submission = StateObject(wrappedValue: submission)
    }

    var body: some View {
        VStack(spacing: 0) {
            IOSAgentRoomTimeline(workspace: roomWorkspace,
                submissions: roomWorkspace.room.flatMap { submission.submissions[$0.room.id] } ?? [],
                pending: roomWorkspace.room.flatMap { submission.pending[$0.room.id] } ?? [],
                onRetry: submission.retry, onCancelPending: submission.cancelPending)
            if let room = submission.selectedRoom, room.room.id == roomWorkspace.room?.room.id {
                VStack(alignment: .leading, spacing: 8) {
                    if let error = submission.errors[room.room.id] {
                        Text(error).font(.footnote).foregroundStyle(DesignTokens.dangerColor).textSelection(.enabled)
                    }
                    if let id = submission.draft.participantID {
                        let member = room.room.participants.first { $0.id == id }
                        Button { submission.removeMention() } label: {
                            Label("@" + (member?.displayName ?? String(localized: "room.member_unavailable")), systemImage: "xmark.circle")
                                .frame(minHeight: 44)
                        }.buttonStyle(.plain)
                            .accessibilityHint(Text("room.remove_mention"))
                    }
                    if mentionsVisible {
                        ScrollView {
                            VStack(alignment: .leading, spacing: 4) {
                                ForEach(submission.candidates) { member in
                                    Button { submission.mention(member); focused = true } label: {
                                        HStack(spacing: 8) {
                                            IOSAgentRoomAvatar(participant: member)
                                            Text(member.displayName)
                                            Spacer()
                                            if highlightedID == member.id { Image(systemName: "checkmark") }
                                        }.frame(minHeight: 44)
                                    }.buttonStyle(.plain)
                                        .accessibilityLabel("@" + member.displayName)
                                        .accessibilityHint(member.description)
                                        .accessibilityAddTraits(highlightedID == member.id ? .isSelected : [])
                                        .accessibilityIdentifier("room.mention." + member.id)
                                }
                            }
                        }.frame(maxHeight: 192)
                    }
                    HStack(alignment: .bottom, spacing: 8) {
                        IOSRoomComposerField(text: Binding(get: { submission.draft.text }, set: submission.edit),
                            focused: Binding(get: { focused }, set: { focused = $0 }),
                            label: String(localized: "room.message_placeholder"),
                            mentionsVisible: mentionsVisible && !submission.candidates.isEmpty,
                            command: mentionCommand, send: { if submission.canSend { submission.send() } })
                            .frame(minHeight: 64, maxHeight: 96)
                            .overlay(alignment: .topLeading) {
                                if submission.draft.text.isEmpty {
                                    Text("room.message_placeholder").foregroundStyle(.secondary).padding(8)
                                        .allowsHitTesting(false).accessibilityHidden(true)
                                }
                            }
                        Button { submission.send() } label: {
                            Image(systemName: "arrow.up")
                                .font(.body.weight(.semibold))
                                .foregroundStyle(submission.canSend ? DesignTokens.onAccentColor : Color.secondary)
                                .frame(width: 44, height: 44)
                                .background(submission.canSend ? DesignTokens.primaryColor : DesignTokens.rowFillColor, in: Circle())
                        }.disabled(!submission.canSend)
                            .accessibilityLabel(Text("chat.send"))
                    }
                    .padding(12)
                    .background(DesignTokens.surfaceColor, in: RoundedRectangle(cornerRadius: 16))
                    .overlay(RoundedRectangle(cornerRadius: 16)
                        .stroke(focused ? DesignTokens.primaryColor : DesignTokens.strongBorderColor, lineWidth: focused ? 2 : 1))
                }.padding(16)
            }
        }
        .task(id: roomWorkspace.room?.room.id) {
            highlightedMention = nil; dismissedMentionText = nil; focused = false
            if let room = roomWorkspace.room { await submission.select(room) }
        }
        .onChange(of: roomWorkspace.room?.revision) { _, _ in
            if let room = roomWorkspace.room { Task { await submission.select(room) } }
        }
        .onChange(of: submission.draft.text) { _, _ in highlightedMention = nil; dismissedMentionText = nil }
    }
    private var mentionsVisible: Bool { submission.draft.mentionQuery != nil && dismissedMentionText != submission.draft.text }
    private var highlightedID: String? {
        submission.candidates.first(where: { $0.id == highlightedMention })?.id ?? submission.candidates.first?.id
    }
    private func mentionCommand(_ command: String) -> Bool {
        guard mentionsVisible else { return false }
        if command == "dismiss" { dismissedMentionText = submission.draft.text; return true }
        let candidates = submission.candidates
        guard !candidates.isEmpty else { return false }
        let current = candidates.firstIndex(where: { $0.id == highlightedID }) ?? 0
        if command == "accept" { submission.mention(candidates[current]); focused = true }
        else if command == "up" || command == "down" {
            highlightedMention = candidates[(current + (command == "up" ? -1 : 1) + candidates.count) % candidates.count].id
        } else { return false }
        return true
    }
}

struct IOSRoomSubmissionRows: View {
    let room: IOSAgentRoomSnapshot?
    let submissions: [IOSRoomSubmission]
    let pending: [IOSRoomSubmissionWorkspace.Pending]
    let onRetry: (IOSRoomSubmissionWorkspace.Pending) -> Void
    let onCancelPending: (String) -> Void

    var body: some View {
        ForEach(IOSRoomSubmissionWorkspace.timeline(receipts: submissions, pending: pending)) { entry in
            VStack(alignment: .leading, spacing: 8) {
                row(entry.input, state: entry.state, selectedID: entry.selectedParticipantID)
                if ["accepted", "selected"].contains(entry.state) {
                    Button("room.cancel_pending") { onCancelPending(entry.id) }.frame(minHeight: 44)
                }
                if let item = entry.pending, let failure = item.failure {
                    Text(failure).font(.footnote).foregroundStyle(DesignTokens.dangerColor).textSelection(.enabled)
                    Button("room.retry") { onRetry(item) }.frame(minHeight: 44)
                }
            }
        }
    }

    private func row(_ input: IOSRoomSubmissionInput, state: String, selectedID: String?) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Spacer(minLength: 32)
                Text(input.text).font(.body).textSelection(.enabled).padding(12)
                    .background(DesignTokens.accentSubtleColor, in: RoundedRectangle(cornerRadius: 16))
            }
            HStack(spacing: 8) {
                Image(systemName: ["failed", "send_failed"].contains(state) ? "exclamationmark.circle" : "bubble.left")
                if let id = selectedID, let member = room?.room.participants.first(where: { $0.id == id }) {
                    Text(member.displayName).fontWeight(.medium)
                }
                Text(LocalizedStringKey(statusKey(state))).foregroundStyle(.secondary)
            }.font(.footnote)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("room.submission." + input.submissionID)
    }
    private func statusKey(_ state: String) -> String {
        switch state {
        case "sending": "room.sending"
        case "accepted": "room.accepted"
        case "selected": "room.selected"
        case "executing": "room.executing"
        case "completed": "room.completed"
        case "incomplete": "room.incomplete"
        case "blocked": "room.blocked"
        case "send_failed": "room.status_unknown"
        case "failed": "room.failed"
        case "cancelled": "room.cancelled"
        default: "room.status_unknown"
        }
    }
}
