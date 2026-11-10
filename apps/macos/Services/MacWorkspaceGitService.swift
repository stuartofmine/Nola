// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Reads, reviews, commits, and publishes a native Git workspace.

import Foundation

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

    func snapshot(at workspaceURL: URL) async throws -> MacGitWorkspaceSnapshot {
        let rootOutput: String
        do {
            rootOutput = try await runGit(["rev-parse", "--show-toplevel"], at: workspaceURL)
        } catch {
            throw MacWorkspaceGitError.notRepository
        }
        let rootPath = rootOutput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !rootPath.isEmpty else { throw MacWorkspaceGitError.notRepository }
        let repositoryURL = URL(fileURLWithPath: rootPath, isDirectory: true)
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
        maximumBytes: Int = 200_000
    ) async throws -> String {
        let data = try await runGitData(arguments, at: directory, maximumBytes: maximumBytes)
        return String(decoding: data, as: UTF8.self)
    }

    private func runGitData(
        _ arguments: [String],
        at directory: URL,
        maximumBytes: Int = 2_000_000
    ) async throws -> Data {
        let process = Process()
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-c", "core.quotepath=false"] + arguments
        process.currentDirectoryURL = directory
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
