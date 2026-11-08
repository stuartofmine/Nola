// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides native review, commit, and push interaction for a Git workspace.

import AppKit
import SwiftUI

private func macWorkspaceReviewText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

private func macGitStatusText(_ value: String) -> String {
    switch value.lowercased() {
    case "conflict", "conflicted": return macWorkspaceReviewText("mac.workspace.status.conflict", "Conflict")
    case "modified": return macWorkspaceReviewText("mac.workspace.status.modified", "Modified")
    case "added": return macWorkspaceReviewText("mac.workspace.status.added", "Added")
    case "deleted": return macWorkspaceReviewText("mac.workspace.status.deleted", "Deleted")
    case "renamed": return macWorkspaceReviewText("mac.workspace.status.renamed", "Renamed")
    case "untracked": return macWorkspaceReviewText("mac.workspace.status.untracked", "Untracked")
    case "staged": return macWorkspaceReviewText("mac.workspace.status.staged", "Staged")
    default: return value
    }
}

struct MacWorkspaceReviewRoute: Codable, Hashable {
    let path: String
}

struct MacWorkspaceReviewView: View {
    let route: MacWorkspaceReviewRoute
    var embedsInWorkspace = false

    @State private var snapshot: MacGitWorkspaceSnapshot?
    @State private var selectedChangeID: String?
    @State private var diff = ""
    @State private var isLoadingDiff = false
    @State private var diffRequestID: UUID?
    @State private var isLoading = false
    @State private var isSuggestingCommit = false
    @State private var errorMessage: String?
    @State private var commitMessage = ""
    @State private var lastSuggestedCommitMessage = ""
    @State private var operation: GitOperation?
    @State private var feedback: MacGitOperationReceipt?
    @State private var showsCommitConfirmation = false
    @State private var showsPushConfirmation = false

    private enum GitOperation: Equatable { case commit, push }

    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                if let snapshot {
                    HStack(spacing: 7) {
                        Image(systemName: "arrow.triangle.branch")
                        Text(snapshot.branch)
                            .fontWeight(.medium)
                            .lineLimit(1)
                        Spacer()
                        Label(publishStatus(snapshot), systemImage: publishStatusSymbol(snapshot))
                            .foregroundStyle(publishStatusColor(snapshot))
                            .lineLimit(1)
                    }
                    .font(.system(size: 11))
                    .padding(12)
                    .background(MacDesignColor.panel)

                    List(selection: $selectedChangeID) {
                        ForEach(MacGitChange.Group.allCases, id: \.rawValue) { group in
                            let changes = snapshot.changes.filter { $0.group == group }
                            if !changes.isEmpty {
                                Section(group.title) {
                                    ForEach(changes) { change in
                                        MacGitChangeRow(change: change)
                                            .tag(change.id)
                                            .contextMenu { fileActions(change, snapshot: snapshot) }
                                    }
                                }
                            }
                        }
                    }
                    .listStyle(.sidebar)
                    .scrollContentBackground(.hidden)
                    .background(MacDesignColor.canvas)
                    .overlay {
                        if snapshot.changes.isEmpty {
                            ContentUnavailableView(
                                macWorkspaceReviewText("mac.workspace.clean.title", "Workspace Is Clean"),
                                systemImage: "checkmark.circle",
                                description: Text(macWorkspaceReviewText("mac.workspace.clean.detail", "Git reports no staged, modified, or untracked files."))
                            )
                        }
                    }

                    Divider()
                    if let feedback {
                        operationFeedback(feedback)
                        Divider()
                    }
                    commitComposer(snapshot)
                } else if isLoading {
                    ProgressView(macWorkspaceReviewText("mac.workspace.loading", "Reading workspace…"))
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView(
                        macWorkspaceReviewText("mac.workspace.no_repository.title", "No Repository"),
                        systemImage: "folder.badge.questionmark",
                        description: Text(errorMessage ?? macWorkspaceReviewText("mac.workspace.no_repository.detail", "Choose a Git workspace from the chat inspector."))
                    )
                }
            }
            .background(MacDesignColor.canvas)
            .frame(minWidth: 250, idealWidth: 310, maxWidth: 420, maxHeight: .infinity)

            Group {
                if let change = selectedChange, let snapshot {
                    MacGitDiffDetail(change: change, snapshot: snapshot, diff: diff, isLoading: isLoadingDiff)
                } else {
                    ContentUnavailableView(
                        macWorkspaceReviewText("mac.workspace.review.title", "Review Changes"),
                        systemImage: "doc.text.magnifyingglass",
                        description: Text(macWorkspaceReviewText("mac.workspace.review.detail", "Select a changed file to inspect its diff."))
                    )
                }
            }
            .frame(minWidth: embedsInWorkspace ? 240 : 520, maxWidth: .infinity, maxHeight: .infinity)
            .background(MacDesignColor.canvas)
        }
        .background(MacDesignColor.canvas)
        .navigationTitle(macWorkspaceReviewText("mac.workspace.title", "Workspace Changes"))
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
              if !embedsInWorkspace {
                if let snapshot {
                    Button {
                        showsPushConfirmation = true
                    } label: {
                        Label(macWorkspaceReviewText("mac.workspace.push", "Push"), systemImage: "arrow.up.circle")
                    }
                    .disabled(operation != nil || !snapshot.canPush)
                    .help(macWorkspaceReviewText("mac.workspace.push.help", "Push the current branch"))
                    Button {
                        NSWorkspace.shared.open(snapshot.repositoryURL)
                    } label: {
                        Image(systemName: "folder")
                    }
                    .help(macWorkspaceReviewText("mac.workspace.open_repository", "Open Repository in Finder"))
                    .accessibilityLabel(macWorkspaceReviewText("mac.workspace.open_repository", "Open Repository in Finder"))
                }
                Button(action: refresh) {
                    Image(systemName: "arrow.clockwise")
                }
                .disabled(isLoading || operation != nil)
                .help(macWorkspaceReviewText("mac.workspace.refresh", "Refresh Changes"))
                .accessibilityLabel(macWorkspaceReviewText("mac.workspace.refresh", "Refresh Changes"))
              }
            }
        }
        .toolbar(embedsInWorkspace ? .hidden : .automatic, for: .windowToolbar)
        .task { await loadSnapshot() }
        .onChange(of: selectedChangeID) { _, _ in
            Task { await loadDiff() }
        }
        .confirmationDialog(
            macWorkspaceReviewText("mac.workspace.commit.confirm_title", "Commit all workspace changes?"),
            isPresented: $showsCommitConfirmation,
            titleVisibility: .visible
        ) {
            Button(macWorkspaceReviewText("mac.workspace.commit.all", "Commit All Changes")) { Task { await commitAll() } }
            Button(macWorkspaceReviewText("mac.workspace.cancel", "Cancel"), role: .cancel) {}
        } message: {
            Text(commitMessage)
        }
        .confirmationDialog(
            macWorkspaceReviewText("mac.workspace.push.confirm_title", "Push the current branch?"),
            isPresented: $showsPushConfirmation,
            titleVisibility: .visible
        ) {
            Button(macWorkspaceReviewText("mac.workspace.push", "Push")) { Task { await push() } }
            Button(macWorkspaceReviewText("mac.workspace.cancel", "Cancel"), role: .cancel) {}
        } message: {
            Text(macWorkspaceReviewText("mac.workspace.push.confirm_detail", "The current branch will be published to its upstream remote."))
        }
    }

    private var selectedChange: MacGitChange? {
        guard let selectedChangeID else { return nil }
        return snapshot?.changes.first { $0.id == selectedChangeID }
    }

    private func refresh() {
        Task { await loadSnapshot() }
    }

    private func loadSnapshot() async {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }
        do {
            let value = try await MacWorkspaceGitService.shared.snapshot(
                at: URL(fileURLWithPath: route.path, isDirectory: true)
            )
            snapshot = value
            let suggestion = await MacWorkspaceGitService.shared.suggestedCommitMessage(for: value)
            if commitMessage.isEmpty || commitMessage == lastSuggestedCommitMessage {
                commitMessage = suggestion
            }
            lastSuggestedCommitMessage = suggestion
            if let selectedChangeID,
               value.changes.contains(where: { $0.id == selectedChangeID }) {
                await loadDiff()
            } else {
                self.selectedChangeID = value.changes.first?.id
            }
        } catch {
            snapshot = nil
            diff = ""
            errorMessage = error.localizedDescription
        }
    }

    @ViewBuilder
    private func commitComposer(_ snapshot: MacGitWorkspaceSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            if snapshot.changes.isEmpty {
                HStack(spacing: 8) {
                    Image(systemName: publishStatusSymbol(snapshot))
                        .foregroundStyle(publishStatusColor(snapshot))
                    Text(publishStatus(snapshot))
                        .font(.system(size: 11.5, weight: .medium))
                    Spacer()
                    if operation == .push {
                        ProgressView().controlSize(.small)
                    } else if snapshot.canPush {
                        Button(macWorkspaceReviewText("mac.workspace.push", "Push")) { showsPushConfirmation = true }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                    }
                }
            } else {
                HStack {
                    Label(macWorkspaceReviewText("mac.workspace.commit.summary", "Commit summary"), systemImage: "text.badge.checkmark")
                        .font(.system(size: 11, weight: .semibold))
                    Spacer()
                    Button {
                        Task {
                            guard !isSuggestingCommit else { return }
                            isSuggestingCommit = true
                            defer { isSuggestingCommit = false }
                            let suggestion = await MacWorkspaceGitService.shared.suggestedCommitMessage(for: snapshot)
                            commitMessage = suggestion
                            lastSuggestedCommitMessage = suggestion
                        }
                    } label: {
                        if isSuggestingCommit {
                            ProgressView()
                                .controlSize(.mini)
                        } else {
                            Image(systemName: "wand.and.stars")
                        }
                    }
                    .macToolbarIcon()
                    .disabled(isSuggestingCommit || operation != nil)
                    .help(macWorkspaceReviewText("mac.workspace.commit.regenerate", "Regenerate commit summary"))
                    .accessibilityLabel(macWorkspaceReviewText("mac.workspace.commit.regenerate", "Regenerate commit summary"))
                }
                TextField(macWorkspaceReviewText("mac.workspace.commit.placeholder", "Describe these changes"), text: $commitMessage, axis: .vertical)
                    .textFieldStyle(.roundedBorder)
                    .lineLimit(2...4)
                    .accessibilityIdentifier("mac-git-commit-summary")
                    .onSubmit { if canCommit(snapshot) { showsCommitConfirmation = true } }
                HStack {
                    if operation == .commit {
                        ProgressView()
                            .controlSize(.small)
                        Text(macWorkspaceReviewText("mac.workspace.commit.creating", "Creating commit…"))
                            .font(.system(size: 10.5))
                            .foregroundStyle(.secondary)
                    } else {
                        Label("\(snapshot.changes.count)", systemImage: "doc.on.doc")
                            .font(.system(size: 10.5, weight: .medium))
                            .foregroundStyle(.secondary)
                            .help(macWorkspaceReviewText("mac.workspace.files", "Files"))
                    }
                    Spacer()
                    Button(macWorkspaceReviewText("mac.workspace.commit.all", "Commit All Changes")) { showsCommitConfirmation = true }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                        .disabled(!canCommit(snapshot))
                        .accessibilityIdentifier("mac-git-commit-all")
                }
            }
        }
        .padding(12)
        .background(MacDesignColor.panel)
    }

    private func canCommit(_ snapshot: MacGitWorkspaceSnapshot) -> Bool {
        operation == nil
            && !snapshot.changes.isEmpty
            && !commitMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func commitAll() async {
        guard let snapshot else { return }
        operation = .commit
        defer { operation = nil }
        do {
            feedback = try await MacWorkspaceGitService.shared.commitAll(message: commitMessage, in: snapshot)
            commitMessage = ""
            lastSuggestedCommitMessage = ""
            await loadSnapshot()
        } catch {
            feedback = MacGitOperationReceipt(
                title: macWorkspaceReviewText("mac.workspace.commit.failed", "Commit failed"),
                detail: error.localizedDescription,
                isError: true
            )
        }
    }

    private func push() async {
        guard let snapshot else { return }
        operation = .push
        defer { operation = nil }
        do {
            feedback = try await MacWorkspaceGitService.shared.push(snapshot)
            await loadSnapshot()
        } catch {
            feedback = MacGitOperationReceipt(
                title: macWorkspaceReviewText("mac.workspace.push.failed", "Push failed"),
                detail: error.localizedDescription,
                isError: true
            )
        }
    }

    private func operationFeedback(_ receipt: MacGitOperationReceipt) -> some View {
        HStack(alignment: .top, spacing: 9) {
            Image(systemName: receipt.isError ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                .foregroundStyle(receipt.isError ? Color.orange : Color.green)
                .font(.system(size: 13, weight: .semibold))
            VStack(alignment: .leading, spacing: 3) {
                Text(receipt.title)
                    .font(.system(size: 11.5, weight: .semibold))
                if !receipt.detail.isEmpty {
                    Text(receipt.detail)
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .textSelection(.enabled)
                }
            }
            Spacer(minLength: 8)
            Button {
                feedback = nil
            } label: {
                Image(systemName: "xmark")
            }
            .macToolbarIcon()
            .help(macWorkspaceReviewText("mac.workspace.dismiss", "Dismiss"))
            .accessibilityLabel(macWorkspaceReviewText("mac.workspace.dismiss", "Dismiss"))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background((receipt.isError ? Color.orange : Color.green).opacity(0.07))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("mac-git-operation-feedback")
    }

    private func publishStatus(_ snapshot: MacGitWorkspaceSnapshot) -> String {
        if !snapshot.hasCommits { return macWorkspaceReviewText("mac.workspace.commit.all", "Commit All Changes") }
        if snapshot.upstreamName == nil, !snapshot.hasOriginRemote {
            return macWorkspaceReviewText("mac.workspace.remote.missing", "Add an origin remote before pushing.")
        }
        if snapshot.upstreamName == nil { return macWorkspaceReviewText("mac.workspace.push", "Push") }
        if snapshot.aheadCount > 0 {
            return "\(macWorkspaceReviewText("mac.workspace.push", "Push")) · \(snapshot.aheadCount)"
        }
        if snapshot.behindCount > 0 {
            return "↓ \(snapshot.behindCount)"
        }
        return macWorkspaceReviewText("mac.workspace.remote.current", "The remote is up to date.")
    }

    private func publishStatusSymbol(_ snapshot: MacGitWorkspaceSnapshot) -> String {
        if snapshot.canPush { return "arrow.up.circle.fill" }
        if snapshot.upstreamName != nil, snapshot.aheadCount == 0, snapshot.behindCount == 0 {
            return "checkmark.icloud.fill"
        }
        if snapshot.behindCount > 0 { return "arrow.down.circle.fill" }
        return "circle.dashed"
    }

    private func publishStatusColor(_ snapshot: MacGitWorkspaceSnapshot) -> Color {
        if snapshot.canPush { return .blue }
        if snapshot.upstreamName != nil, snapshot.aheadCount == 0, snapshot.behindCount == 0 {
            return .green
        }
        if snapshot.behindCount > 0 { return .orange }
        return .secondary
    }

    private func loadDiff() async {
        guard let snapshot, let change = selectedChange else {
            diffRequestID = nil
            diff = ""
            isLoadingDiff = false
            return
        }
        let requestID = UUID()
        diffRequestID = requestID
        isLoadingDiff = true
        do {
            let loadedDiff = try await MacWorkspaceGitService.shared.diff(for: change, in: snapshot)
            guard diffRequestID == requestID, selectedChange?.id == change.id else { return }
            diff = loadedDiff
            isLoadingDiff = false
        } catch {
            guard diffRequestID == requestID, selectedChange?.id == change.id else { return }
            diff = String(format: macWorkspaceReviewText("mac.workspace.diff.failed", "Unable to load diff: %@"), error.localizedDescription)
            isLoadingDiff = false
        }
    }

    @ViewBuilder
    private func fileActions(_ change: MacGitChange, snapshot: MacGitWorkspaceSnapshot) -> some View {
        let fileURL = snapshot.repositoryURL.appendingPathComponent(change.path)
        Button(macWorkspaceReviewText("mac.workspace.open", "Open")) { NSWorkspace.shared.open(fileURL) }
        Button(macWorkspaceReviewText("mac.workspace.quick_look", "Quick Look")) { MacQuickLookController.shared.preview([fileURL]) }
        Button(macWorkspaceReviewText("mac.workspace.reveal", "Reveal in Finder")) { NSWorkspace.shared.activateFileViewerSelecting([fileURL]) }
        Divider()
        Button(macWorkspaceReviewText("mac.workspace.copy_path", "Copy Relative Path")) {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(change.path, forType: .string)
        }
    }
}

private struct MacGitChangeRow: View {
    let change: MacGitChange

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: symbol)
                .foregroundStyle(color)
                .frame(width: 15)
            VStack(alignment: .leading, spacing: 2) {
                Text(URL(fileURLWithPath: change.path).lastPathComponent)
                    .font(.system(size: 11.5, weight: .medium))
                    .lineLimit(1)
                let directory = (change.path as NSString).deletingLastPathComponent
                if !directory.isEmpty {
                    Text(directory)
                        .font(.system(size: 9.5))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 6)
            Text(change.statusCode)
                .font(.system(size: 9.5, weight: .semibold, design: .monospaced))
                .foregroundStyle(color)
                .padding(.horizontal, 6)
                .frame(height: 20)
                .background(color.opacity(0.09), in: Capsule(style: .continuous))
        }
        .padding(.vertical, 2)
    }

    private var symbol: String {
        switch change.group {
        case .staged: return "checkmark.circle"
        case .workingTree: return "pencil.circle"
        case .untracked: return "plus.circle"
        }
    }

    private var color: Color {
        if change.displayStatus.lowercased().contains("conflict") { return .red }
        switch change.group {
        case .staged: return .green
        case .workingTree: return .orange
        case .untracked: return .blue
        }
    }
}

private struct MacGitDiffDetail: View {
    let change: MacGitChange
    let snapshot: MacGitWorkspaceSnapshot
    let diff: String
    let isLoading: Bool

    var body: some View {
        VStack(spacing: 0) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 14) {
                    diffIdentity
                    Spacer(minLength: 12)
                    diffActions
                }
                VStack(alignment: .leading, spacing: 10) {
                    diffIdentity
                    diffActions
                }
            }
            .padding(14)
            .background(MacDesignColor.panel)

            Divider()

            GeometryReader { viewport in
                ScrollView([.horizontal, .vertical]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(diffLines.enumerated()), id: \.offset) { _, line in
                            let text = String(line)
                            HStack(spacing: 0) {
                                Rectangle()
                                    .fill(lineAccent(text))
                                    .frame(width: 3)
                                    .padding(.trailing, 10)
                                Text(text)
                                    .foregroundStyle(lineColor(text))
                                    .textSelection(.enabled)
                            }
                            .font(.system(size: 11.5, design: .monospaced))
                            .frame(maxWidth: .infinity, minHeight: 19, alignment: .leading)
                            .background(lineBackground(text))
                        }
                    }
                    .padding(.vertical, 10)
                    .frame(
                        minWidth: max(700, viewport.size.width),
                        minHeight: viewport.size.height,
                        alignment: .topLeading
                    )
                }
                .defaultScrollAnchor(.topLeading)
            }
            .background(Color(nsColor: .textBackgroundColor))
            .overlay {
                if isLoading {
                    Rectangle()
                        .fill(.background.opacity(0.88))
                        .overlay {
                            MacLoadingState(macWorkspaceReviewText("mac.workspace.diff.loading", "Loading diff…"))
                        }
                } else if diff.isEmpty {
                    MacEmptyState(
                        LocalizedStringKey(macWorkspaceReviewText("mac.workspace.diff.empty.title", "No Text Diff")),
                        description: LocalizedStringKey(macWorkspaceReviewText("mac.workspace.diff.empty.detail", "This file has no textual changes to display.")),
                        symbol: "doc"
                    ) {}
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var diffIdentity: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(change.path)
                .font(.system(size: 12.5, weight: .semibold))
                .textSelection(.enabled)
                .lineLimit(2)
                .truncationMode(.middle)
            Text(macGitStatusText(change.displayStatus))
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
        }
    }

    private var diffActions: some View {
        HStack(spacing: 8) {
            Button {
                NSWorkspace.shared.open(fileURL)
            } label: {
                Label(macWorkspaceReviewText("mac.workspace.open", "Open"), systemImage: "arrow.up.forward.app")
            }
            .buttonStyle(.borderedProminent)
            Button {
                MacQuickLookController.shared.preview([fileURL])
            } label: {
                Label(macWorkspaceReviewText("mac.workspace.quick_look", "Quick Look"), systemImage: "eye")
            }
            Button {
                NSWorkspace.shared.activateFileViewerSelecting([fileURL])
            } label: {
                Label(macWorkspaceReviewText("mac.workspace.reveal", "Reveal in Finder"), systemImage: "folder")
            }
        }
        .controlSize(.small)
    }

    private var diffLines: [Substring] {
        diff.split(separator: "\n", omittingEmptySubsequences: false)
    }

    private var fileURL: URL {
        snapshot.repositoryURL.appendingPathComponent(change.path)
    }

    private func lineColor(_ line: String) -> Color {
        if line.hasPrefix("+") && !line.hasPrefix("+++") { return .green }
        if line.hasPrefix("-") && !line.hasPrefix("---") { return .red }
        if line.hasPrefix("@@") { return .blue }
        return .primary
    }

    private func lineBackground(_ line: String) -> Color {
        if line.hasPrefix("+") && !line.hasPrefix("+++") { return .green.opacity(0.055) }
        if line.hasPrefix("-") && !line.hasPrefix("---") { return .red.opacity(0.055) }
        if line.hasPrefix("@@") { return .blue.opacity(0.045) }
        return .clear
    }

    private func lineAccent(_ line: String) -> Color {
        if line.hasPrefix("+") && !line.hasPrefix("+++") { return .green.opacity(0.8) }
        if line.hasPrefix("-") && !line.hasPrefix("---") { return .red.opacity(0.8) }
        if line.hasPrefix("@@") { return .blue.opacity(0.75) }
        return .clear
    }
}

private extension MacGitChange.Group {
    static var allCases: [MacGitChange.Group] { [.staged, .workingTree, .untracked] }
}
