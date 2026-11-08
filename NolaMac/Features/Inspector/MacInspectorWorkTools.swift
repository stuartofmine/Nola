// SPDX-License-Identifier: Apache-2.0
// Conversation tools present existing owner-scoped facts; they never invent work.
import SwiftUI

struct MacInspectorParticipant: Identifiable {
    let id: String
    let name: String
}

struct MacInspectorDecisionOverview: View {
    @Bindable var model: MacWorkspaceModel
    let select: (MacChatInspector.InspectorTab) -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            let status = model.selectedConversationDisplayStatus
            Label(status.title, systemImage: "circle.dotted").font(.callout)
            if status == .waitingApproval || status == .waitingInput || status == .uncertain {
                Text("Review the pending request in this conversation before continuing.")
                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            ForEach([MacChatInspector.InspectorTab.review, .artifacts, .progress, .goal]) { tab in
                Button { select(tab) } label: {
                    HStack { Label(tab.title, systemImage: tab.symbol); Spacer(); Image(systemName: "chevron.right").font(.caption) }
                        .frame(minHeight: 36).contentShape(Rectangle())
                }.buttonStyle(.plain)
            }
        }
    }
}

struct MacInspectorParticipantsView: View {
    let participants: [MacInspectorParticipant]
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("Nola", systemImage: "sparkles")
            ForEach(participants) { participant in
                HStack(alignment: .top, spacing: 12) {
                    AppleExpertAvatar(name: participant.name, size: 32)
                    Text(participant.name).fixedSize(horizontal: false, vertical: true)
                }
            }
            if participants.isEmpty { Text("No experts in this conversation").font(.callout).foregroundStyle(.secondary) }
        }
    }
}

struct MacInspectorArtifactsView: View {
    @Bindable var model: MacWorkspaceModel
    let canReadLocalFiles: Bool
    let inspectFile: (MacCodeReference) -> Void
    private struct Artifact: Identifiable {
        let id: String
        let name: String
        let url: URL
        let source: Date
        let type: String
    }
    private var artifacts: [Artifact] {
        model.selectedMessages.filter { $0.role == .assistant }.flatMap { message in
            message.attachmentURLs.enumerated().map { index, url in
                Artifact(id: message.id.uuidString + "/" + String(index),
                    name: index < message.attachmentNames.count ? message.attachmentNames[index] : url.lastPathComponent,
                    url: url, source: message.createdAt,
                    type: message.attachmentMetadata?.first(where: { $0.uri == url.absoluteString || $0.uri == url.path })?.mimeType ?? url.pathExtension)
            }
        }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if artifacts.isEmpty { Text("No artifacts in the loaded conversation history").font(.callout).foregroundStyle(.secondary) }
            ForEach(artifacts) { artifact in
                VStack(alignment: .leading, spacing: 4) {
                    Button {
                        if canReadLocalFiles && model.executionHost == .local && MacFilePreviewPolicy.isLocalFileURL(artifact.url) { inspectFile(.init(url: artifact.url, line: nil, column: nil)) }
                    } label: {
                        Label(artifact.name, systemImage: "doc").frame(maxWidth: .infinity, minHeight: 36, alignment: .leading).contentShape(Rectangle())
                    }.buttonStyle(.plain).disabled(!canReadLocalFiles || model.executionHost != .local || !MacFilePreviewPolicy.isLocalFileURL(artifact.url))
                    Text(artifact.source, style: .date).font(.caption).foregroundStyle(.secondary)
                    if !artifact.type.isEmpty { Text(artifact.type).font(.caption).foregroundStyle(.secondary) }
                    if !canReadLocalFiles || model.executionHost != .local || !MacFilePreviewPolicy.isLocalFileURL(artifact.url) { Text("This artifact requires its source Host to provide a preview.").font(.caption).foregroundStyle(.secondary) }
                }
            }
            // Input attachments are a distinct source, retained through the existing reader.
            if canReadLocalFiles && model.executionHost == .local {
                MacInspectorWorkingContextView(model: model, resourcesOnly: true)
            }
        }
    }
}

struct MacInspectorReviewView: View {
    @Bindable var model: MacWorkspaceModel
    let openReview: (URL) -> Void
    @State private var snapshot: MacGitWorkspaceSnapshot?
    @State private var selected: String?
    @State private var diff = ""
    @State private var error: String?
    @State private var loading = false
    @State private var loadRequestID: UUID?
    @State private var retry = 0
    private var owner: String { (model.selectedConversationID ?? "") + "/" + model.effectiveWorkspaceURL.path }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if model.executionHost != .local {
                Text("Workspace review requires the source Host's Git reader.").font(.callout).foregroundStyle(.secondary)
            } else {
                HStack {
                    if let snapshot { Label(snapshot.branch, systemImage: "arrow.triangle.branch").lineLimit(1) }
                    Spacer()
                    Button { retry += 1 } label: { Image(systemName: "arrow.clockwise").frame(width: 32, height: 32) }
                        .buttonStyle(.plain).help(String(localized: "Refresh")).accessibilityLabel(String(localized: "Refresh"))
                }
                if loading { ProgressView() }
                if let error { Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
                if let snapshot {
                    if snapshot.changes.isEmpty { Text("Workspace Is Clean").foregroundStyle(.secondary) }
                    ForEach(snapshot.changes) { change in
                        Button { selected = change.id } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(change.path).font(.callout).lineLimit(2)
                                Text(change.displayStatus).font(.caption).foregroundStyle(.secondary)
                            }.frame(maxWidth: .infinity, minHeight: 36, alignment: .leading).contentShape(Rectangle())
                        }.buttonStyle(.plain).accessibilityAddTraits(selected == change.id ? .isSelected : [])
                    }
                    if !diff.isEmpty {
                        ScrollView([.horizontal, .vertical]) { Text(diff).font(.system(size: 11, design: .monospaced)).textSelection(.enabled).padding(8) }
                            .frame(maxHeight: 300)
                    }
                    Button("Open in Tab") { openReview(snapshot.repositoryURL) }.frame(minHeight: 32)
                }
            }
        }
        .task(id: "\(owner)/\(retry)/\(model.executionHost)") {
            snapshot = nil; selected = nil; diff = ""; error = nil
            guard model.executionHost == .local else { return }
            let requestedOwner = owner, url = model.effectiveWorkspaceURL
            let requestID = UUID()
            loadRequestID = requestID; loading = true
            defer { if loadRequestID == requestID { loading = false } }
            do {
                let result = try await MacWorkspaceGitService.shared.snapshot(at: url)
                guard !Task.isCancelled, owner == requestedOwner else { return }
                snapshot = result
            } catch {
                guard !Task.isCancelled, owner == requestedOwner else { return }
                self.error = error.localizedDescription
            }
        }
        .task(id: "\(owner)/\(selected ?? "")") {
            diff = ""
            guard let snapshot, let change = snapshot.changes.first(where: { $0.id == selected }) else { return }
            let requestedOwner = owner, requestedSelection = selected
            do {
                let value = try await MacWorkspaceGitService.shared.diff(for: change, in: snapshot)
                guard !Task.isCancelled, owner == requestedOwner, selected == requestedSelection else { return }
                diff = value
            } catch {
                guard !Task.isCancelled, owner == requestedOwner, selected == requestedSelection else { return }
                self.error = error.localizedDescription
            }
        }
    }
}
