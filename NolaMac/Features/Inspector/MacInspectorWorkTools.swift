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
                    HStack { Label(tab.title, systemImage: tab.symbol); Spacer(); Image(systemName: "arrow.down").font(.caption) }
                        .frame(minHeight: 36).contentShape(Rectangle())
                }.buttonStyle(MacPressFeedbackButtonStyle())
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

// Navigation only. A web link does not grant local-file or source-Host access.
struct MacInspectorArtifactRow: View {
    let name: String
    let url: URL
    let source: Date
    let type: String
    let canReadLocalFiles: Bool
    let isLocalHost: Bool
    let inspectFile: (MacCodeReference) -> Void
    @Environment(\.openURL) private var openURL

    static func destination(url: URL, canReadLocalFiles: Bool, isLocalHost: Bool) -> MacJournalArtifactDestination? {
        MacJournalArtifactDestination.resolve(url.absoluteString, localJournal: canReadLocalFiles && isLocalHost)
    }

    var body: some View {
        let destination = Self.destination(url: url, canReadLocalFiles: canReadLocalFiles, isLocalHost: isLocalHost)
        VStack(alignment: .leading, spacing: 4) {
            Button {
                switch destination {
                case .web(let url): openURL(url)
                case .localFile(let url): inspectFile(.init(url: url, line: nil, column: nil))
                case nil: break
                }
            } label: {
                Label(name, systemImage: url.isFileURL ? "doc" : "link")
                    .lineLimit(3).fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(MacPressFeedbackButtonStyle())
            .disabled(destination == nil)
            .help(name)
            Text(source, style: .date).font(.caption).foregroundStyle(.secondary)
            if !type.isEmpty { Text(type).font(.caption).foregroundStyle(.secondary) }
            if destination == nil {
                Text("This artifact requires its source Host to provide a preview.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
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
                MacInspectorArtifactRow(name: artifact.name, url: artifact.url, source: artifact.source,
                    type: artifact.type, canReadLocalFiles: canReadLocalFiles,
                    isLocalHost: model.executionHost == .local, inspectFile: inspectFile)
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
    @State private var isNotRepository = false
    @State private var selected: String?
    @State private var diff = ""
    @State private var error: String?
    @State private var diffError: String?
    @State private var isLoadingDiff = false
    @State private var diffRequestID: UUID?
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
                        .buttonStyle(MacPressFeedbackButtonStyle()).help(String(localized: "Refresh")).accessibilityLabel(String(localized: "Refresh"))
                }
                if loading { ProgressView() }
                if isNotRepository {
                    Label(String(localized: "The selected workspace is not a Git repository."), systemImage: "folder")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(String(localized: "Choose Folder…")) { model.chooseWorkspace() }
                        .buttonStyle(MacPressFeedbackButtonStyle())
                }
                if let error { Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
                if let snapshot {
                    MacInspectorGitChangesView(snapshot: snapshot, selected: selected,
                        diff: diff, diffError: diffError, isLoadingDiff: isLoadingDiff,
                        select: { selected = $0 }, openReview: openReview)
                }
            }
        }
        .task(id: "\(owner)/\(retry)/\(model.executionHost)/\(model.effectiveWorkspaceURL.path)") {
            snapshot = nil; selected = nil; diff = ""; error = nil; loading = false; isNotRepository = false
            guard model.executionHost == .local else { return }
            let requestedOwner = owner, url = model.effectiveWorkspaceURL
            let requestID = UUID()
            loadRequestID = requestID; loading = true
            defer { if loadRequestID == requestID { loading = false } }
            do {
                let result = try await MacWorkspaceGitService.shared.snapshot(at: url)
                guard !Task.isCancelled, owner == requestedOwner, loadRequestID == requestID else { return }
                snapshot = result
            } catch MacWorkspaceGitError.notRepository {
                guard !Task.isCancelled, owner == requestedOwner, loadRequestID == requestID else { return }
                isNotRepository = true
            } catch {
                guard !Task.isCancelled, owner == requestedOwner, loadRequestID == requestID else { return }
                self.error = error.localizedDescription
            }
        }
        .task(id: "\(owner)/\(selected ?? "")") {
            diff = ""; diffError = nil; isLoadingDiff = false
            let requestID = UUID()
            diffRequestID = requestID
            guard let snapshot, let change = snapshot.changes.first(where: { $0.id == selected }) else { return }
            let requestedOwner = owner, requestedSelection = selected
            isLoadingDiff = true
            defer { if diffRequestID == requestID { isLoadingDiff = false } }
            do {
                let value = try await MacWorkspaceGitService.shared.diff(for: change, in: snapshot)
                guard !Task.isCancelled, owner == requestedOwner, selected == requestedSelection, diffRequestID == requestID else { return }
                diff = value
            } catch {
                guard !Task.isCancelled, owner == requestedOwner, selected == requestedSelection, diffRequestID == requestID else { return }
                diffError = error.localizedDescription
            }
        }
    }
}

// The same content renders real Git reads and populated visual fixtures.
struct MacInspectorGitChangesView: View {
    let snapshot: MacGitWorkspaceSnapshot
    let selected: String?
    let diff: String
    let diffError: String?
    let isLoadingDiff: Bool
    let select: (String) -> Void
    let openReview: (URL) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if snapshot.changes.isEmpty { Text("Workspace Is Clean").foregroundStyle(.secondary) }
            ForEach(snapshot.changes) { change in
                VStack(alignment: .leading, spacing: 6) {
                    Button { select(change.id) } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(change.path).font(.callout).lineLimit(3)
                                .fixedSize(horizontal: false, vertical: true)
                            Text(change.displayStatus).font(.caption).foregroundStyle(.secondary)
                        }
                        .padding(8)
                        .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                    }
                    .buttonStyle(MacSidebarNavigationButtonStyle(isSelected: selected == change.id))
                    .accessibilityAddTraits(selected == change.id ? .isSelected : [])
                    .help(change.path)
                    if selected == change.id {
                        if isLoadingDiff {
                            ProgressView().controlSize(.small).accessibilityLabel(String(localized: "Loading"))
                        } else if let diffError {
                            Text(diffError).font(.caption).foregroundStyle(.red).textSelection(.enabled)
                        } else if !diff.isEmpty {
                            MacInspectorDiffPreview(diff: diff)
                        } else {
                            Text("No textual diff is available for this file.", tableName: "ConversationInspector")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Button("Open in Tab") { openReview(snapshot.repositoryURL) }
                            .buttonStyle(MacPressFeedbackButtonStyle())
                    }
                }
            }
            if selected == nil {
                Button("Open in Tab") { openReview(snapshot.repositoryURL) }
                    .buttonStyle(MacPressFeedbackButtonStyle())
            }
        }
    }
}

struct MacInspectorDiffPreview: View {
    let diff: String
    static let lineLimit = 24
    static func lines(_ diff: String) -> [Substring] {
        // Only discover one extra line to establish truncation. Do not build a
        // view or an array of every line in an arbitrarily large workspace diff.
        var result: [Substring] = []
        var start = diff.startIndex
        while result.count <= lineLimit {
            guard let end = diff[start...].firstIndex(of: "\n") else {
                result.append(diff[start...]); break
            }
            result.append(diff[start..<end])
            start = diff.index(after: end)
        }
        return result
    }

    var body: some View {
        let lines = Self.lines(diff)
        let preview = lines.prefix(Self.lineLimit).joined(separator: "\n")
        VStack(alignment: .leading, spacing: 6) {
            ScrollView(.horizontal) {
                Text(preview).font(.system(size: 11, design: .monospaced))
                    .lineSpacing(2).fixedSize(horizontal: true, vertical: true)
                    .textSelection(.enabled).padding(8)
            }
            .frame(height: CGFloat(min(lines.count, Self.lineLimit)) * 17 + 16)
            .frame(maxWidth: .infinity, alignment: .leading)
            if lines.count > Self.lineLimit {
                Text("Diff preview is shortened. Open in a tab to review the full changes.", tableName: "ConversationInspector")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
