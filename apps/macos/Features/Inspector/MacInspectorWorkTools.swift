// SPDX-License-Identifier: Apache-2.0
// Conversation tools present existing owner-scoped facts; they never invent work.
import AppKit
import SwiftUI

struct MacInspectorParticipant: Identifiable {
    let id: String
    let name: String
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
            if participants.isEmpty {
                Label("No experts in this conversation", systemImage: "person.2")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
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
    @State private var thumbnail: NSImage?

    static func destination(url: URL, canReadLocalFiles: Bool, isLocalHost: Bool) -> MacJournalArtifactDestination? {
        MacJournalArtifactDestination.resolve(url.absoluteString, localJournal: canReadLocalFiles && isLocalHost)
    }

    static func isImageArtifact(url: URL, type: String) -> Bool {
        if type.lowercased().hasPrefix("image/") { return true }
        return ["png", "jpg", "jpeg", "gif", "heic", "webp", "tiff", "bmp", "svg"]
            .contains(url.pathExtension.lowercased())
    }

    var body: some View {
        let destination = Self.destination(url: url, canReadLocalFiles: canReadLocalFiles, isLocalHost: isLocalHost)
        let isImage = Self.isImageArtifact(url: url, type: type)
        VStack(alignment: .leading, spacing: 4) {
            Button {
                switch destination {
                case .web(let url): openURL(url)
                case .localFile(let url):
                    inspectFile(.init(url: url, line: nil, column: nil))
                case nil: break
                }
            } label: {
                VStack(alignment: .leading, spacing: 6) {
                    if isImage, let thumbnail {
                        Image(nsImage: thumbnail)
                            .resizable()
                            .scaledToFit()
                            .frame(maxWidth: .infinity, maxHeight: 140, alignment: .leading)
                            .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
                            .overlay {
                                RoundedRectangle(cornerRadius: 7, style: .continuous)
                                    .strokeBorder(Color.primary.opacity(0.1), lineWidth: 0.5)
                            }
                    }
                    HStack(alignment: .center, spacing: 9) {
                        Image(systemName: isImage ? "photo" : (url.isFileURL ? "doc" : "link"))
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 24, height: 24)
                            .background(
                                Color.primary.opacity(0.05),
                                in: RoundedRectangle(cornerRadius: 6, style: .continuous)
                            )
                        VStack(alignment: .leading, spacing: 2) {
                            Text(name)
                                .font(.system(size: 13, weight: .medium))
                                .lineLimit(3)
                                .fixedSize(horizontal: false, vertical: true)
                            HStack(spacing: 4) {
                                Text(source, style: .date)
                                if !type.isEmpty {
                                    Text("·")
                                    Text(type)
                                }
                            }
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 8)
                        Image(systemName: isImage ? "eye" : (url.isFileURL ? "doc.text.magnifyingglass" : "arrow.up.right"))
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.tertiary)
                            .accessibilityHidden(true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(MacHorizontalBarButtonStyle(isSubtle: true))
            .disabled(destination == nil)
            .help(name)
            .task(id: destination) {
                guard isImage, case .some(.localFile(let localURL)) = destination else {
                    thumbnail = nil
                    return
                }
                thumbnail = await Task.detached(priority: .utility) {
                    let accessed = localURL.startAccessingSecurityScopedResource()
                    defer { if accessed { localURL.stopAccessingSecurityScopedResource() } }
                    return NSImage(contentsOf: localURL)
                }.value
            }
            .contextMenu {
                switch destination {
                case .localFile(let localURL):
                    Button(String(localized: "Preview")) {
                        inspectFile(.init(url: localURL, line: nil, column: nil))
                    }
                    Button(String(localized: "Reveal in Finder")) {
                        NSWorkspace.shared.activateFileViewerSelecting([localURL])
                    }
                    Button(String(localized: "Open with Default App")) {
                        NSWorkspace.shared.open(localURL)
                    }
                    Divider()
                    Button(String(localized: "Copy Path")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(localURL.path, forType: .string)
                    }
                case .web(let webURL):
                    Button(String(localized: "Open Link")) {
                        openURL(webURL)
                    }
                    Button(String(localized: "Copy Link")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(webURL.absoluteString, forType: .string)
                    }
                case nil:
                    Button(url.isFileURL ? String(localized: "Copy Path") : String(localized: "Copy URI")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(
                            url.isFileURL ? url.path : url.absoluteString,
                            forType: .string
                        )
                    }
                }
            }
            if destination == nil {
                Text("This artifact requires its source Host to provide a preview.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 4)
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
            if artifacts.isEmpty {
                Label("No artifacts in the loaded conversation history", systemImage: "doc.on.doc")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
            ForEach(artifacts) { artifact in
                MacInspectorArtifactRow(name: artifact.name, url: artifact.url, source: artifact.source,
                    type: artifact.type, canReadLocalFiles: canReadLocalFiles,
                    isLocalHost: model.executionHost == .local, inspectFile: inspectFile)
            }
            // Input attachments are a distinct source; show them on both local and remote
            // conversations while gating local file actions on local provenance inside the view.
            MacInspectorWorkingContextView(
                model: model,
                resourcesOnly: true,
                canReadLocalFiles: canReadLocalFiles
            )
        }
    }
}

struct MacWorkspaceFileEntry: Identifiable, Equatable, Sendable {
    let url: URL
    let relativePath: String
    let size: Int64
    var id: String { relativePath }
}

enum MacWorkspaceFileBrowser {
    static let defaultMaxDepth = 3
    static let defaultMaxEntries = 150
    static let skippedDirectoryNames: Set<String> = [".git", "node_modules", "target"]

    nonisolated static func listFiles(
        in rootURL: URL,
        maxDepth: Int = defaultMaxDepth,
        maxEntries: Int = defaultMaxEntries,
        fileManager: FileManager = .default
    ) -> (entries: [MacWorkspaceFileEntry], truncated: Bool) {
        let resolvedRoot = rootURL.standardizedFileURL.resolvingSymlinksInPath()
        var isDir: ObjCBool = false
        guard fileManager.fileExists(atPath: resolvedRoot.path, isDirectory: &isDir), isDir.boolValue else {
            return ([], false)
        }

        let keys: [URLResourceKey] = [.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey, .isHiddenKey]
        var entries: [MacWorkspaceFileEntry] = []
        var truncated = false
        var queue: [(url: URL, prefix: String, depth: Int)] = [(resolvedRoot, "", 0)]
        var head = 0

        while head < queue.count {
            let current = queue[head]
            head += 1
            guard let children = try? fileManager.contentsOfDirectory(
                at: current.url,
                includingPropertiesForKeys: keys,
                options: [.skipsHiddenFiles]
            ) else {
                continue
            }

            let sortedChildren = children.sorted {
                $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending
            }

            for child in sortedChildren {
                let name = child.lastPathComponent
                if name.hasPrefix(".") { continue }
                guard let values = try? child.resourceValues(forKeys: Set(keys)) else { continue }
                if values.isHidden == true || values.isSymbolicLink == true { continue }

                let relativePath = current.prefix.isEmpty ? name : "\(current.prefix)/\(name)"
                if values.isDirectory == true {
                    if skippedDirectoryNames.contains(name) { continue }
                    if current.depth < maxDepth {
                        queue.append((child, relativePath, current.depth + 1))
                    }
                } else if values.isRegularFile == true {
                    if entries.count >= maxEntries {
                        truncated = true
                        return (entries, truncated)
                    }
                    entries.append(
                        MacWorkspaceFileEntry(
                            url: child,
                            relativePath: relativePath,
                            size: Int64(values.fileSize ?? 0)
                        )
                    )
                }
            }
        }

        entries.sort {
            $0.relativePath.localizedStandardCompare($1.relativePath) == .orderedAscending
        }
        return (entries, truncated)
    }
}

struct MacInspectorReviewView: View {
    @Bindable var model: MacWorkspaceModel
    let openReview: (URL) -> Void
    @Environment(\.macOpenFileReference) private var openFileReference
    @State private var snapshot: MacGitWorkspaceSnapshot?
    @State private var isNotRepository = false
    @State private var workspaceFiles: [MacWorkspaceFileEntry] = []
    @State private var workspaceFilesTruncated = false
    @State private var selected: String?
    @State private var diff = ""
    @State private var error: String?
    @State private var diffError: String?
    @State private var isLoadingDiff = false
    @State private var diffRequestID: UUID?
    @State private var loading = false
    @State private var loadRequestID: UUID?
    @State private var loadedOwner = ""
    @State private var retry = 0
    private var owner: String { (model.selectedConversationID ?? "") + "/" + model.effectiveWorkspaceURL.path }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if model.executionHost != .local {
                Text("Workspace review requires the source Host's Git reader.").font(.callout).foregroundStyle(.secondary)
            } else {
                HStack(spacing: 8) {
                    if let snapshot {
                        Label(snapshot.branch, systemImage: "arrow.triangle.branch").lineLimit(1)
                    } else if isNotRepository {
                        Label(String(localized: "Workspace Files"), systemImage: "folder").lineLimit(1)
                    }
                    Spacer()
                    if let snapshot {
                        Button(String(localized: "Open Full Review")) { openReview(snapshot.repositoryURL) }
                            .buttonStyle(MacActionButtonStyle())
                            .controlSize(.small)
                    } else if isNotRepository {
                        Button {
                            NSWorkspace.shared.open(model.effectiveWorkspaceURL)
                        } label: {
                            Image(systemName: "folder").frame(width: 28, height: 28)
                        }
                        .buttonStyle(MacPressFeedbackButtonStyle())
                        .help(String(localized: "Open Workspace in Finder"))
                        .accessibilityLabel(String(localized: "Open Workspace in Finder"))
                    }
                    Button { retry += 1 } label: { Image(systemName: "arrow.clockwise").frame(width: 32, height: 32) }
                        .buttonStyle(MacPressFeedbackButtonStyle()).help(String(localized: "Refresh")).accessibilityLabel(String(localized: "Refresh"))
                }
                if loading { ProgressView() }
                if isNotRepository {
                    HStack(spacing: 8) {
                        Text(String(localized: "Not a Git repository"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button(String(localized: "Choose Folder…")) { Task { await model.chooseWorkspace() } }
                            .buttonStyle(MacActionButtonStyle())
                            .controlSize(.small)
                            .disabled(model.isSelectedConversationVisiblyRunning)
                    }
                    if workspaceFiles.isEmpty && !loading {
                        Label(String(localized: "No files in this workspace yet."), systemImage: "doc")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    } else {
                        VStack(alignment: .leading, spacing: 4) {
                            ForEach(workspaceFiles) { entry in
                                Button {
                                    openFileReference(.init(url: entry.url, line: nil, column: nil))
                                } label: {
                                    HStack(alignment: .center, spacing: 9) {
                                        Image(systemName: "doc")
                                            .font(.system(size: 12, weight: .medium))
                                            .foregroundStyle(.secondary)
                                            .frame(width: 22, height: 22)
                                            .background(
                                                Color.primary.opacity(0.05),
                                                in: RoundedRectangle(cornerRadius: 6, style: .continuous)
                                            )
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(entry.relativePath)
                                                .font(.system(size: 12.5, weight: .medium))
                                                .lineLimit(2)
                                                .truncationMode(.middle)
                                                .fixedSize(horizontal: false, vertical: true)
                                            Text(ByteCountFormatter.string(fromByteCount: entry.size, countStyle: .file))
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                        Spacer(minLength: 8)
                                        Image(systemName: "eye")
                                            .font(.system(size: 11, weight: .medium))
                                            .foregroundStyle(.tertiary)
                                            .accessibilityHidden(true)
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(MacHorizontalBarButtonStyle(isSubtle: true))
                                .help(entry.relativePath)
                                .contextMenu {
                                    Button(String(localized: "Preview")) {
                                        openFileReference(.init(url: entry.url, line: nil, column: nil))
                                    }
                                    Button(String(localized: "Reveal in Finder")) {
                                        NSWorkspace.shared.activateFileViewerSelecting([entry.url])
                                    }
                                    Button(String(localized: "Open with Default App")) {
                                        NSWorkspace.shared.open(entry.url)
                                    }
                                    Divider()
                                    Button(String(localized: "Copy Path")) {
                                        NSPasteboard.general.clearContents()
                                        NSPasteboard.general.setString(entry.url.path, forType: .string)
                                    }
                                }
                            }
                            if workspaceFilesTruncated {
                                Text(String(localized: "Showing first \(MacWorkspaceFileBrowser.defaultMaxEntries) files."))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .padding(.top, 4)
                            }
                        }
                    }
                }
                if let error { Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
                if let snapshot {
                    MacInspectorGitChangesView(snapshot: snapshot, selected: selected,
                        diff: diff, diffError: diffError, isLoadingDiff: isLoadingDiff,
                        select: { selected = selected == $0 ? nil : $0 }, openReview: openReview)
                }
            }
        }
        .onChange(of: model.inspectorSelectedConversationIsRunning) { wasRunning, isRunning in
            if wasRunning && !isRunning { retry += 1 }
        }
        .task(id: "\(owner)/\(retry)/\(model.executionHost)/\(model.effectiveWorkspaceURL.path)") {
            let scopeKey = "\(owner)/\(model.executionHost)/\(model.effectiveWorkspaceURL.path)"
            if loadedOwner != scopeKey {
                snapshot = nil; workspaceFiles = []; workspaceFilesTruncated = false
                selected = nil; diff = ""; loadedOwner = scopeKey
            }
            error = nil; loading = false; isNotRepository = false
            guard model.executionHost == .local else { return }
            let requestedOwner = owner, url = model.effectiveWorkspaceURL
            let requestID = UUID()
            loadRequestID = requestID; loading = true
            defer { if loadRequestID == requestID { loading = false } }
            do {
                let result = try await MacWorkspaceGitService.shared.snapshot(
                    at: url, allowEnclosingRepository: !model.usesSessionSandbox)
                guard !Task.isCancelled, owner == requestedOwner, loadRequestID == requestID else { return }
                snapshot = result
                workspaceFiles = []
                workspaceFilesTruncated = false
            } catch MacWorkspaceGitError.notRepository {
                guard !Task.isCancelled, owner == requestedOwner, loadRequestID == requestID else { return }
                snapshot = nil
                isNotRepository = true
                let listing = await Task.detached(priority: .utility) {
                    MacWorkspaceFileBrowser.listFiles(in: url)
                }.value
                guard !Task.isCancelled, owner == requestedOwner, loadRequestID == requestID else { return }
                workspaceFiles = listing.entries
                workspaceFilesTruncated = listing.truncated
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
                let isSelected = selected == change.id
                VStack(alignment: .leading, spacing: 6) {
                    Button { select(change.id) } label: {
                        HStack(spacing: 8) {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(change.path)
                                    .font(.callout.weight(.medium))
                                    .lineLimit(3)
                                    .fixedSize(horizontal: false, vertical: true)
                                Text(change.displayStatus)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer(minLength: 8)
                            Image(systemName: "chevron.down")
                                .font(.system(size: 10.5, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .rotationEffect(.degrees(isSelected ? 180 : 0))
                                .accessibilityHidden(true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(MacHorizontalBarButtonStyle(isSubtle: true, isExpanded: isSelected))
                    .accessibilityAddTraits(isSelected ? .isSelected : [])
                    .help(change.path)
                    if isSelected {
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
                    }
                }
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

    private func lineForeground(_ line: Substring) -> Color {
        if line.hasPrefix("+") && !line.hasPrefix("+++") { return MacDesignColor.success }
        if line.hasPrefix("-") && !line.hasPrefix("---") { return MacDesignColor.danger }
        if line.hasPrefix("@@") { return MacDesignColor.brand }
        return .primary
    }

    private func lineBackground(_ line: Substring) -> Color {
        if line.hasPrefix("+") && !line.hasPrefix("+++") { return MacDesignColor.success.opacity(0.10) }
        if line.hasPrefix("-") && !line.hasPrefix("---") { return MacDesignColor.danger.opacity(0.10) }
        return .clear
    }

    var body: some View {
        let lines = Self.lines(diff)
        let visibleLines = Array(lines.prefix(Self.lineLimit))
        VStack(alignment: .leading, spacing: 6) {
            ScrollView(.horizontal) {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(Array(visibleLines.enumerated()), id: \.offset) { _, line in
                        Text(String(line))
                            .font(.system(size: MacDesignTokens.Typography.inspectorDetail, design: .monospaced))
                            .foregroundStyle(lineForeground(line))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(lineBackground(line))
                    }
                }
                .textSelection(.enabled)
                .padding(.vertical, 6)
            }
            .frame(height: CGFloat(min(lines.count, Self.lineLimit)) * 18 + 16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(MacDesignColor.subtleFill.opacity(0.55), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            if lines.count > Self.lineLimit {
                Text("Diff preview is shortened. Open full review to inspect all changes.", tableName: "ConversationInspector")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
