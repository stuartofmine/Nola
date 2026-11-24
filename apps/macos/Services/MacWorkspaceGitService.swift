// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Reads, reviews, commits, and publishes a native Git workspace.

import Foundation

extension Notification.Name {
    static let nolaMacTurnCheckpointsDidChange = Notification.Name("nola.mac.turnCheckpointsDidChange")
    static let nolaMacWorkspaceDirectoryDidSwitch = Notification.Name("nola.mac.workspaceDirectoryDidSwitch")
}

nonisolated struct MacTurnCheckpoint: Identifiable, Hashable, Sendable {
    let turnId: String
    let timestamp: Date
    let label: String
    let treeSHA: String
    let repositoryURL: URL

    var id: String { turnId }
}

nonisolated struct MacTurnRestoreSummary: Equatable, Sendable {
    let checkpoint: MacTurnCheckpoint
    let restoredFiles: [String]
    let removedFiles: [String]
    let remainingUndo: Int
    let remainingRedo: Int
}

nonisolated struct MacGitWorktree: Identifiable, Hashable, Sendable {
    let path: URL
    let head: String
    let branch: String?
    let isCurrent: Bool
    let isMain: Bool

    var id: String { path.standardizedFileURL.resolvingSymlinksInPath().path }

    var displayBranch: String {
        if let branch, !branch.isEmpty { return branch }
        return head.isEmpty ? "Detached HEAD" : String(head.prefix(7))
    }
}

nonisolated struct MacGitChange: Identifiable, Hashable, Sendable {
    enum Group: String, Sendable {
        case staged
        case workingTree
        case untracked

        var title: String {
            switch self {
            case .staged: return String(localized: "Staged")
            case .workingTree: return String(localized: "Changes")
            case .untracked: return String(localized: "Untracked")
            }
        }
    }

    let indexStatus: Character
    let workTreeStatus: Character
    let path: String
    let originalPath: String?

    var id: String { "\(indexStatus)\(workTreeStatus):\(path)" }
    var isUntracked: Bool { indexStatus == "?" && workTreeStatus == "?" }
    var group: Group {
        if isUntracked { return .untracked }
        if indexStatus != " " { return .staged }
        return .workingTree
    }
    var statusCode: String { String([indexStatus, workTreeStatus]) }
    var displayStatus: String {
        if isUntracked { return String(localized: "New") }
        if indexStatus == "R" || workTreeStatus == "R" { return String(localized: "Renamed") }
        if indexStatus == "D" || workTreeStatus == "D" { return String(localized: "Deleted") }
        if indexStatus == "A" { return String(localized: "Added") }
        if indexStatus == "M" || workTreeStatus == "M" { return String(localized: "Modified") }
        if indexStatus == "U" || workTreeStatus == "U" { return String(localized: "Conflict") }
        return statusCode.trimmingCharacters(in: .whitespaces)
    }
}

nonisolated struct MacGitWorkspaceSnapshot: Sendable {
    let requestedURL: URL
    let repositoryURL: URL
    let branch: String
    let changes: [MacGitChange]
    let hasCommits: Bool
    let hasOriginRemote: Bool
    let upstreamName: String?
    let aheadCount: Int
    let behindCount: Int

    var canPush: Bool {
        guard hasCommits, branch != "Detached HEAD" else { return false }
        if upstreamName == nil { return hasOriginRemote }
        return aheadCount > 0
    }
}

nonisolated struct MacGitOperationReceipt: Sendable {
    let title: String
    let detail: String
    let isError: Bool

    init(title: String, detail: String, isError: Bool = false) {
        self.title = title
        self.detail = detail
        self.isError = isError
    }
}

nonisolated enum MacWorkspaceGitError: LocalizedError {
    case notRepository
    case commandFailed(String)

    var errorDescription: String? {
        switch self {
        case .notRepository:
            return String(localized: "The selected workspace is not a Git repository.")
        case .commandFailed(let message): return message
        }
    }
}

actor MacWorkspaceGitService {
    static let shared = MacWorkspaceGitService()

    private struct CheckpointHistory {
        var undoStack: [MacTurnCheckpoint] = []
        var redoStack: [MacTurnCheckpoint] = []
    }

    private static let maximumUndoDepth = 50
    private var checkpointHistories: [String: CheckpointHistory] = [:]

    init() {}

    func checkpointCounts(at workspaceURL: URL) async -> (undo: Int, redo: Int) {
        guard let repositoryURL = try? await resolveRepositoryURL(at: workspaceURL) else {
            return (0, 0)
        }
        let history = checkpointHistories[repositoryKey(for: repositoryURL)]
        return (history?.undoStack.count ?? 0, history?.redoStack.count ?? 0)
    }

    func checkpoints(at workspaceURL: URL) async -> (undo: [MacTurnCheckpoint], redo: [MacTurnCheckpoint]) {
        guard let repositoryURL = try? await resolveRepositoryURL(at: workspaceURL) else {
            return ([], [])
        }
        let history = checkpointHistories[repositoryKey(for: repositoryURL)]
        return (history?.undoStack ?? [], history?.redoStack ?? [])
    }

    /// Captures a shadow-index snapshot of the working tree without touching
    /// `.git/index`, staged files, or commit history.
    @discardableResult
    func captureTurnCheckpoint(at workspaceURL: URL, label: String) async throws -> MacTurnCheckpoint {
        let repositoryURL = try await resolveRepositoryURL(at: workspaceURL)
        let treeSHA = try await captureShadowTreeSHA(at: repositoryURL)
        let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        let checkpoint = MacTurnCheckpoint(
            turnId: UUID().uuidString,
            timestamp: Date(),
            label: trimmedLabel.isEmpty ? "Turn" : trimmedLabel,
            treeSHA: treeSHA,
            repositoryURL: repositoryURL
        )
        let key = repositoryKey(for: repositoryURL)
        var history = checkpointHistories[key] ?? CheckpointHistory()
        history.undoStack.append(checkpoint)
        if history.undoStack.count > Self.maximumUndoDepth {
            history.undoStack.removeFirst(history.undoStack.count - Self.maximumUndoDepth)
        }
        history.redoStack.removeAll()
        checkpointHistories[key] = history
        postCheckpointsChanged(repositoryURL: repositoryURL)
        return checkpoint
    }

    /// Restores the working tree to the most recent turn checkpoint using a
    /// disposable shadow index, preserving the user's real `.git/index` and
    /// pushing the pre-undo state onto the lossless redo stack.
    func undoTurnCheckpoint(at workspaceURL: URL) async throws -> MacTurnRestoreSummary {
        let repositoryURL = try await resolveRepositoryURL(at: workspaceURL)
        let key = repositoryKey(for: repositoryURL)
        guard var history = checkpointHistories[key], let target = history.undoStack.last else {
            throw MacWorkspaceGitError.commandFailed("No turn checkpoint available to undo.")
        }
        let currentTreeSHA = try await captureShadowTreeSHA(at: repositoryURL)
        let applied = try await applyShadowTree(from: currentTreeSHA, to: target.treeSHA, at: repositoryURL)
        history.undoStack.removeLast()
        history.redoStack.append(
            MacTurnCheckpoint(
                turnId: target.turnId,
                timestamp: Date(),
                label: target.label,
                treeSHA: currentTreeSHA,
                repositoryURL: repositoryURL
            )
        )
        checkpointHistories[key] = history
        postCheckpointsChanged(repositoryURL: repositoryURL)
        return MacTurnRestoreSummary(
            checkpoint: target,
            restoredFiles: applied.restored,
            removedFiles: applied.removed,
            remainingUndo: history.undoStack.count,
            remainingRedo: history.redoStack.count
        )
    }

    /// Reapplies the most recently undone turn state using a disposable shadow
    /// index, keeping the user's real `.git/index` untouched.
    func redoTurnCheckpoint(at workspaceURL: URL) async throws -> MacTurnRestoreSummary {
        let repositoryURL = try await resolveRepositoryURL(at: workspaceURL)
        let key = repositoryKey(for: repositoryURL)
        guard var history = checkpointHistories[key], let target = history.redoStack.last else {
            throw MacWorkspaceGitError.commandFailed("No turn checkpoint available to redo.")
        }
        let currentTreeSHA = try await captureShadowTreeSHA(at: repositoryURL)
        let applied = try await applyShadowTree(from: currentTreeSHA, to: target.treeSHA, at: repositoryURL)
        history.redoStack.removeLast()
        history.undoStack.append(
            MacTurnCheckpoint(
                turnId: target.turnId,
                timestamp: Date(),
                label: target.label,
                treeSHA: currentTreeSHA,
                repositoryURL: repositoryURL
            )
        )
        checkpointHistories[key] = history
        postCheckpointsChanged(repositoryURL: repositoryURL)
        return MacTurnRestoreSummary(
            checkpoint: target,
            restoredFiles: applied.restored,
            removedFiles: applied.removed,
            remainingUndo: history.undoStack.count,
            remainingRedo: history.redoStack.count
        )
    }

    func listWorktrees(at workspaceURL: URL) async throws -> [MacGitWorktree] {
        let repositoryURL = try await resolveRepositoryURL(at: workspaceURL)
        let output = try await runGit(["worktree", "list", "--porcelain"], at: repositoryURL)
        return Self.parseWorktrees(output, currentRepositoryURL: repositoryURL)
    }

    func createWorktree(at workspaceURL: URL, branchName: String) async throws -> MacGitWorktree {
        let repositoryURL = try await resolveRepositoryURL(at: workspaceURL)
        let validated = try Self.validatedBranchName(branchName)
        do {
            _ = try await runGit(["check-ref-format", "--branch", validated], at: repositoryURL)
        } catch {
            throw MacWorkspaceGitError.commandFailed("Invalid branch name: \(validated)")
        }
        let parentURL = repositoryURL.deletingLastPathComponent()
        let folderName = Self.worktreeDirectoryName(repositoryName: repositoryURL.lastPathComponent, branchName: validated)
        let targetURL = parentURL.appendingPathComponent(folderName, isDirectory: true).standardizedFileURL
        guard targetURL.deletingLastPathComponent().standardizedFileURL == parentURL.standardizedFileURL else {
            throw MacWorkspaceGitError.commandFailed("Invalid worktree target path.")
        }
        guard !FileManager.default.fileExists(atPath: targetURL.path) else {
            throw MacWorkspaceGitError.commandFailed("A directory already exists at \(folderName).")
        }
        _ = try await runGit(["worktree", "add", "-b", validated, targetURL.path], at: repositoryURL)
        let worktrees = try await listWorktrees(at: targetURL)
        let targetKey = repositoryKey(for: targetURL)
        if let created = worktrees.first(where: { repositoryKey(for: $0.path) == targetKey }) {
            return created
        }
        let head = (try? await runGit(["rev-parse", "HEAD"], at: targetURL))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return MacGitWorktree(
            path: targetURL,
            head: head,
            branch: validated,
            isCurrent: true,
            isMain: false
        )
    }

    nonisolated static func parseWorktrees(_ output: String, currentRepositoryURL: URL) -> [MacGitWorktree] {
        let normalizedCurrent = currentRepositoryURL.standardizedFileURL.resolvingSymlinksInPath().path
        var results: [MacGitWorktree] = []
        var currentPath: String?
        var currentHead = ""
        var currentBranch: String?

        func flush() {
            guard let path = currentPath, !path.isEmpty else { return }
            let url = URL(fileURLWithPath: path, isDirectory: true)
            let normalized = url.standardizedFileURL.resolvingSymlinksInPath().path
            results.append(
                MacGitWorktree(
                    path: url,
                    head: currentHead,
                    branch: currentBranch,
                    isCurrent: normalized == normalizedCurrent,
                    isMain: results.isEmpty
                )
            )
            currentPath = nil
            currentHead = ""
            currentBranch = nil
        }

        for rawLine in output.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline) {
            let line = String(rawLine)
            if line.isEmpty {
                flush()
                continue
            }
            if let path = line.strippingPrefix("worktree ") {
                flush()
                currentPath = path
            } else if let head = line.strippingPrefix("HEAD ") {
                currentHead = head
            } else if let ref = line.strippingPrefix("branch ") {
                currentBranch = ref.strippingPrefix("refs/heads/") ?? ref
            } else if line == "detached" {
                currentBranch = nil
            }
        }
        flush()
        return results
    }

    nonisolated static func parseDiffTreeNameStatus(_ data: Data) -> (restored: [String], removed: [String]) {
        let fields = data.split(separator: 0, omittingEmptySubsequences: true).map {
            String(decoding: Data($0), as: UTF8.self)
        }
        var restored: [String] = []
        var removed: [String] = []
        var index = 0
        while index + 1 < fields.count {
            let status = fields[index]
            let path = fields[index + 1]
            if status.hasPrefix("D") {
                removed.append(path)
            } else if !path.isEmpty {
                restored.append(path)
            }
            index += 2
        }
        return (restored.sorted(), removed.sorted())
    }

    nonisolated static func validatedBranchName(_ raw: String) throws -> String {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw MacWorkspaceGitError.commandFailed("Enter a branch name.")
        }
        guard !trimmed.hasPrefix("-"),
              !trimmed.hasPrefix("."),
              !trimmed.hasPrefix("/"),
              !trimmed.hasSuffix("."),
              !trimmed.hasSuffix("/"),
              !trimmed.hasSuffix(".lock"),
              !trimmed.contains(".."),
              !trimmed.contains("//"),
              !trimmed.contains("@{"),
              trimmed != "@" else {
            throw MacWorkspaceGitError.commandFailed("Invalid branch name: \(trimmed)")
        }
        let forbidden = CharacterSet.whitespacesAndNewlines
            .union(.controlCharacters)
            .union(CharacterSet(charactersIn: "~^:?*[\\"))
        guard trimmed.rangeOfCharacter(from: forbidden) == nil else {
            throw MacWorkspaceGitError.commandFailed("Invalid branch name: \(trimmed)")
        }
        return trimmed
    }

    nonisolated static func worktreeDirectoryName(repositoryName: String, branchName: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "._-"))
        var sanitized = ""
        var previousDash = false
        for scalar in branchName.unicodeScalars {
            if allowed.contains(scalar) {
                sanitized.unicodeScalars.append(scalar)
                previousDash = false
            } else if !previousDash {
                sanitized.append("-")
                previousDash = true
            }
        }
        let trimmed = sanitized.trimmingCharacters(in: CharacterSet(charactersIn: "-."))
        let suffix = trimmed.isEmpty ? "worktree" : trimmed
        return "\(repositoryName)-\(suffix)"
    }

    private func resolveRepositoryURL(at workspaceURL: URL) async throws -> URL {
        let rootOutput: String
        do {
            rootOutput = try await runGit(["rev-parse", "--show-toplevel"], at: workspaceURL)
        } catch {
            throw MacWorkspaceGitError.notRepository
        }
        let rootPath = rootOutput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !rootPath.isEmpty else { throw MacWorkspaceGitError.notRepository }
        return URL(fileURLWithPath: rootPath, isDirectory: true)
    }

    private func repositoryKey(for url: URL) -> String {
        url.standardizedFileURL.resolvingSymlinksInPath().path
    }

    private func captureShadowTreeSHA(at repositoryURL: URL) async throws -> String {
        let shadowDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-git-shadow-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: shadowDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: shadowDir) }
        // Non-existent index path so git initializes a brand-new shadow index
        // without touching the repository's real `.git/index`.
        let shadowIndexPath = shadowDir.appendingPathComponent("index").path
        let env = ["GIT_INDEX_FILE": shadowIndexPath]
        _ = try await runGit(["add", "-A"], at: repositoryURL, extraEnvironment: env)
        let sha = try await runGit(["write-tree"], at: repositoryURL, extraEnvironment: env)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sha.isEmpty else {
            throw MacWorkspaceGitError.commandFailed("Failed to write Git shadow tree.")
        }
        return sha
    }

    private func applyShadowTree(
        from fromTreeSHA: String,
        to toTreeSHA: String,
        at repositoryURL: URL
    ) async throws -> (restored: [String], removed: [String]) {
        guard fromTreeSHA != toTreeSHA else { return ([], []) }
        let diffData = try await runGitData(
            ["diff-tree", "-r", "--name-status", "--no-renames", "-z", fromTreeSHA, toTreeSHA],
            at: repositoryURL
        )
        let plan = Self.parseDiffTreeNameStatus(diffData)
        let fileManager = FileManager.default
        let canonicalRoot = repositoryURL.standardizedFileURL.resolvingSymlinksInPath()

        for relativePath in plan.removed {
            let fileURL = canonicalRoot.appendingPathComponent(relativePath).standardizedFileURL
            guard MacAppEnvironment.contains(fileURL, in: canonicalRoot),
                  fileURL.path != canonicalRoot.path else { continue }
            if fileManager.fileExists(atPath: fileURL.path) {
                try fileManager.removeItem(at: fileURL)
            }
            var parent = fileURL.deletingLastPathComponent().standardizedFileURL
            while parent.path != canonicalRoot.path && MacAppEnvironment.contains(parent, in: canonicalRoot) {
                let contents = (try? fileManager.contentsOfDirectory(atPath: parent.path)) ?? []
                if contents.isEmpty {
                    try? fileManager.removeItem(at: parent)
                    parent = parent.deletingLastPathComponent().standardizedFileURL
                } else {
                    break
                }
            }
        }

        if !plan.restored.isEmpty {
            let shadowDir = fileManager.temporaryDirectory
                .appendingPathComponent("nola-git-restore-\(UUID().uuidString)", isDirectory: true)
            try fileManager.createDirectory(at: shadowDir, withIntermediateDirectories: true)
            defer { try? fileManager.removeItem(at: shadowDir) }
            let shadowIndexPath = shadowDir.appendingPathComponent("index").path
            let env = ["GIT_INDEX_FILE": shadowIndexPath]
            let chunkSize = 100
            var offset = 0
            while offset < plan.restored.count {
                let end = min(offset + chunkSize, plan.restored.count)
                let batch = Array(plan.restored[offset..<end])
                _ = try await runGit(["checkout", toTreeSHA, "--"] + batch, at: repositoryURL, extraEnvironment: env)
                offset = end
            }
        }

        return plan
    }

    private func postCheckpointsChanged(repositoryURL: URL) {
        let path = repositoryKey(for: repositoryURL)
        Task { @MainActor in
            NotificationCenter.default.post(
                name: .nolaMacTurnCheckpointsDidChange,
                object: nil,
                userInfo: ["repositoryPath": path]
            )
        }
    }

    func snapshot(at workspaceURL: URL, allowEnclosingRepository: Bool = true) async throws -> MacGitWorkspaceSnapshot {
        let rootOutput: String
        do {
            rootOutput = try await runGit(["rev-parse", "--show-toplevel"], at: workspaceURL)
        } catch {
            throw MacWorkspaceGitError.notRepository
        }
        let rootPath = rootOutput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !rootPath.isEmpty else { throw MacWorkspaceGitError.notRepository }
        let repositoryURL = URL(fileURLWithPath: rootPath, isDirectory: true)
        guard allowEnclosingRepository || MacAppEnvironment.contains(repositoryURL, in: workspaceURL) else {
            throw MacWorkspaceGitError.notRepository
        }
        let branch = (try? await runGit(["branch", "--show-current"], at: repositoryURL))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let statusData = try await runGitData(
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            at: repositoryURL
        )
        let hasCommits = (try? await runGit(["rev-parse", "--verify", "HEAD"], at: repositoryURL)) != nil
        let remotes = (try? await runGit(["remote"], at: repositoryURL))?
            .split(whereSeparator: \ .isNewline)
            .map(String.init) ?? []
        let upstreamName = (try? await runGit(
            ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
            at: repositoryURL
        ))?.trimmingCharacters(in: .whitespacesAndNewlines)
        let divergence = await divergenceCounts(at: repositoryURL, hasUpstream: upstreamName?.isEmpty == false)
        return MacGitWorkspaceSnapshot(
            requestedURL: workspaceURL,
            repositoryURL: repositoryURL,
            branch: branch?.isEmpty == false ? branch! : "Detached HEAD",
            changes: parseStatus(statusData),
            hasCommits: hasCommits,
            hasOriginRemote: remotes.contains("origin"),
            upstreamName: upstreamName?.isEmpty == false ? upstreamName : nil,
            aheadCount: divergence.ahead,
            behindCount: divergence.behind
        )
    }

    func diff(for change: MacGitChange, in snapshot: MacGitWorkspaceSnapshot) async throws -> String {
        if change.isUntracked {
            return try previewUntrackedFile(snapshot.repositoryURL.appendingPathComponent(change.path))
        }
        let value = try await runGit(
            ["diff", "HEAD", "--no-ext-diff", "--no-color", "--", change.path],
            at: snapshot.repositoryURL,
            maximumBytes: 800_000
        )
        if value.isEmpty, change.indexStatus != " " {
            return try await runGit(
                ["diff", "--cached", "--no-ext-diff", "--no-color", "--", change.path],
                at: snapshot.repositoryURL,
                maximumBytes: 800_000
            )
        }
        return value.isEmpty ? "No textual diff is available for this file." : value
    }

    func suggestedCommitMessage(for snapshot: MacGitWorkspaceSnapshot) -> String {
        let changes = snapshot.changes
        guard !changes.isEmpty else { return "" }
        let paths = changes.map(\.path)
        let extensions = paths.map { URL(fileURLWithPath: $0).pathExtension.lowercased() }
        let testPaths = paths.filter(isTestPath)
        let implementationPaths = paths.filter { !isTestPath($0) }
        let subject = commitSubject(
            paths: implementationPaths.isEmpty ? paths : implementationPaths,
            totalChangeCount: changes.count
        )
        let summarySubject = !testPaths.isEmpty && !implementationPaths.isEmpty
            ? "\(subject) + \(String(localized: "test coverage"))"
            : subject
        if changes.allSatisfy(\.isUntracked) {
            return "feat: \(String.localizedStringWithFormat(String(localized: "add %@"), summarySubject))"
        }
        if changes.allSatisfy({ $0.indexStatus == "D" || $0.workTreeStatus == "D" }) {
            return "chore: \(String.localizedStringWithFormat(String(localized: "remove %@"), summarySubject))"
        }
        if testPaths.count == paths.count {
            return "test: \(String.localizedStringWithFormat(String(localized: "update %@"), subject))"
        }
        if extensions.allSatisfy({ ["md", "mdx", "txt", "rst"].contains($0) }) {
            return "docs: \(String.localizedStringWithFormat(String(localized: "update %@"), subject))"
        }
        if !testPaths.isEmpty {
            return "chore: \(String.localizedStringWithFormat(String(localized: "update %@"), summarySubject))"
        }
        return "chore: \(String.localizedStringWithFormat(String(localized: "update %@"), subject))"
    }

    private func isTestPath(_ path: String) -> Bool {
        let lowered = path.lowercased()
        return lowered.contains("/test")
            || lowered.contains("tests/")
            || lowered.hasSuffix("tests.swift")
            || lowered.hasSuffix("_test.py")
            || lowered.hasSuffix(".test.ts")
            || lowered.hasSuffix(".spec.ts")
    }

    private func commitSubject(paths: [String], totalChangeCount: Int) -> String {
        guard totalChangeCount > 1 else {
            return paths.first.map { URL(fileURLWithPath: $0).lastPathComponent }
                ?? String.localizedStringWithFormat(
                    String(localized: "%lld workspace files"),
                    Int64(totalChangeCount)
                )
        }
        let labels = Array(Set(paths.compactMap(topLevelLabel))).sorted()
        if labels.count == 1, let label = labels.first { return label }
        if labels.count == 2 { return labels.joined(separator: " and ") }
        return String.localizedStringWithFormat(
            String(localized: "%lld workspace files"),
            Int64(totalChangeCount)
        )
    }

    private func topLevelLabel(_ path: String) -> String? {
        let components = path.split(separator: "/")
        guard let first = components.first else { return nil }
        let component = first.lowercased() == "apps" && components.count > 1
            ? String(components[1]) : String(first)
        switch component.lowercased() {
        case "macos", "nolamac": return "Mac"
        case "ios", "nola": return "iOS"
        case "windows": return "Windows"
        case "android": return "Android"
        case "cli": return "CLI"
        case "web": return "Web"
        case "core": return "Core"
        case "backend": return "Backend"
        default: return component
        }
    }

    func commitAll(message: String, in snapshot: MacGitWorkspaceSnapshot) async throws -> MacGitOperationReceipt {
        let message = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty else {
            throw MacWorkspaceGitError.commandFailed(String(localized: "Enter a commit message first."))
        }
        _ = try await runGit(["add", "--all", "--", "."], at: snapshot.repositoryURL)
        let output = try await runGit(["commit", "-m", message], at: snapshot.repositoryURL)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return MacGitOperationReceipt(
            title: String(localized: "Changes committed"),
            detail: output
        )
    }

    func push(_ snapshot: MacGitWorkspaceSnapshot) async throws -> MacGitOperationReceipt {
        let branch = snapshot.branch.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !branch.isEmpty, branch != "Detached HEAD" else {
            throw MacWorkspaceGitError.commandFailed(String(localized: "Check out a branch before pushing."))
        }
        let arguments: [String]
        if snapshot.upstreamName != nil {
            arguments = ["push"]
        } else {
            guard snapshot.hasOriginRemote else {
                throw MacWorkspaceGitError.commandFailed(String(localized: "Add an origin remote before pushing."))
            }
            arguments = ["push", "--set-upstream", "origin", branch]
        }
        let output = try await runGit(arguments, at: snapshot.repositoryURL)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return MacGitOperationReceipt(
            title: String(localized: "Push completed"),
            detail: output.isEmpty ? String(localized: "The remote is up to date.") : output
        )
    }

    private func divergenceCounts(
        at repositoryURL: URL,
        hasUpstream: Bool
    ) async -> (ahead: Int, behind: Int) {
        guard hasUpstream,
              let output = try? await runGit(
                  ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
                  at: repositoryURL
              ) else {
            return (0, 0)
        }
        let values = output.split(whereSeparator: \ .isWhitespace).compactMap { Int($0) }
        guard values.count == 2 else { return (0, 0) }
        return (values[0], values[1])
    }

    private func parseStatus(_ data: Data) -> [MacGitChange] {
        let records = data.split(separator: 0, omittingEmptySubsequences: true).map { Data($0) }
        var output: [MacGitChange] = []
        var index = 0
        while index < records.count {
            let record = String(decoding: records[index], as: UTF8.self)
            guard record.count >= 3 else {
                index += 1
                continue
            }
            let characters = Array(record)
            let indexStatus = characters[0]
            let workTreeStatus = characters[1]
            let path = String(characters.dropFirst(3))
            var originalPath: String?
            if indexStatus == "R" || indexStatus == "C" || workTreeStatus == "R" || workTreeStatus == "C" {
                let nextIndex = index + 1
                if nextIndex < records.count {
                    originalPath = String(decoding: records[nextIndex], as: UTF8.self)
                    index = nextIndex
                }
            }
            output.append(MacGitChange(
                indexStatus: indexStatus,
                workTreeStatus: workTreeStatus,
                path: path,
                originalPath: originalPath
            ))
            index += 1
        }
        return output.sorted {
            if $0.group != $1.group { return $0.group.rawValue < $1.group.rawValue }
            return $0.path.localizedCaseInsensitiveCompare($1.path) == .orderedAscending
        }
    }

    private func previewUntrackedFile(_ url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let data = try handle.read(upToCount: 500_000) ?? Data()
        if data.contains(0) {
            return String.localizedStringWithFormat(
                String(localized: "Binary file · %@ previewed"),
                ByteCountFormatter.string(fromByteCount: Int64(data.count), countStyle: .file)
            )
        }
        let text = String(decoding: data, as: UTF8.self)
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        return lines.map { "+\($0)" }.joined(separator: "\n")
    }

    private func runGit(
        _ arguments: [String],
        at directory: URL,
        extraEnvironment: [String: String] = [:],
        maximumBytes: Int = 200_000
    ) async throws -> String {
        let data = try await runGitData(
            arguments,
            at: directory,
            extraEnvironment: extraEnvironment,
            maximumBytes: maximumBytes
        )
        return String(decoding: data, as: UTF8.self)
    }

    private func runGitData(
        _ arguments: [String],
        at directory: URL,
        extraEnvironment: [String: String] = [:],
        maximumBytes: Int = 2_000_000
    ) async throws -> Data {
        let process = Process()
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-c", "core.quotepath=false"] + arguments
        process.currentDirectoryURL = directory
        if !extraEnvironment.isEmpty {
            var environment = ProcessInfo.processInfo.environment
            for (key, value) in extraEnvironment {
                environment[key] = value
            }
            process.environment = environment
        }
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        process.standardInput = FileHandle.nullDevice
        try process.run()
        let outputTask = Task.detached { try outputPipe.fileHandleForReading.readToEnd() ?? Data() }
        let errorTask = Task.detached { try errorPipe.fileHandleForReading.readToEnd() ?? Data() }
        process.waitUntilExit()
        let output = try await outputTask.value
        let error = try await errorTask.value
        guard process.terminationStatus == 0 else {
            let message = String(decoding: error.prefix(4_000), as: UTF8.self)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw MacWorkspaceGitError.commandFailed(message.isEmpty ? "Git exited with status \(process.terminationStatus)." : message)
        }
        return output.count > maximumBytes ? output.prefix(maximumBytes) : output
    }
}

private extension String {
    nonisolated func strippingPrefix(_ prefix: String) -> String? {
        guard hasPrefix(prefix) else { return nil }
        return String(dropFirst(prefix.count))
    }
}
