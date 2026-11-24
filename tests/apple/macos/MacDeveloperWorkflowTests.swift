// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies macOS shadow-index turn checkpoints, Git worktrees, and Plan/Build read-only mode.

import Foundation
import XCTest
@testable import Nola

final class MacDeveloperWorkflowTests: XCTestCase {
    func testShadowIndexTurnCheckpointUndoRedoPreservesRealStagedIndex() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-checkpoint-test-\(UUID().uuidString)", isDirectory: true)
        let repo = root.appendingPathComponent("project", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: repo, withIntermediateDirectories: true)

        _ = try runGit(["init", "-b", "main"], at: repo)
        _ = try runGit(["config", "user.name", "Nola Tests"], at: repo)
        _ = try runGit(["config", "user.email", "nola-tests@example.invalid"], at: repo)

        let existingURL = repo.appendingPathComponent("existing.txt")
        let toDeleteURL = repo.appendingPathComponent("to_delete.txt")
        let stagedURL = repo.appendingPathComponent("staged.txt")
        try Data("v1\n".utf8).write(to: existingURL)
        try Data("keep me\n".utf8).write(to: toDeleteURL)
        try Data("base\n".utf8).write(to: stagedURL)
        _ = try runGit(["add", "-A"], at: repo)
        _ = try runGit(["commit", "-m", "initial"], at: repo)

        // Stage a real user edit in `.git/index` and make an additional unstaged edit.
        try Data("staged-v2\n".utf8).write(to: stagedURL)
        _ = try runGit(["add", "staged.txt"], at: repo)
        try Data("working-tree-v2\n".utf8).write(to: stagedURL)
        let cachedDiffBefore = try runGit(["diff", "--cached"], at: repo)
        let commitCountBefore = try runGit(["rev-list", "--count", "HEAD"], at: repo)
            .trimmingCharacters(in: .whitespacesAndNewlines)

        let service = MacWorkspaceGitService()
        let checkpoint = try await service.captureTurnCheckpoint(at: repo, label: "Refactor auth")
        XCTAssertEqual(checkpoint.treeSHA.count, 40)
        XCTAssertEqual(checkpoint.label, "Refactor auth")
        let countsAfterCapture = await service.checkpointCounts(at: repo)
        XCTAssertEqual(countsAfterCapture.undo, 1)
        XCTAssertEqual(countsAfterCapture.redo, 0)
        XCTAssertEqual(try runGit(["diff", "--cached"], at: repo), cachedDiffBefore)
        XCTAssertEqual(
            try runGit(["rev-list", "--count", "HEAD"], at: repo).trimmingCharacters(in: .whitespacesAndNewlines),
            commitCountBefore
        )

        // Simulate agent turn edits: modify, delete, create top-level and nested files.
        let createdURL = repo.appendingPathComponent("created.txt")
        let nestedDirURL = repo.appendingPathComponent("nested/deep", isDirectory: true)
        let nestedFileURL = nestedDirURL.appendingPathComponent("file.txt")
        try Data("v2-modified\n".utf8).write(to: existingURL)
        try FileManager.default.removeItem(at: toDeleteURL)
        try Data("brand new\n".utf8).write(to: createdURL)
        try FileManager.default.createDirectory(at: nestedDirURL, withIntermediateDirectories: true)
        try Data("deep new\n".utf8).write(to: nestedFileURL)

        // Undo turn checkpoint.
        let undoSummary = try await service.undoTurnCheckpoint(at: repo)
        XCTAssertEqual(undoSummary.restoredFiles, ["existing.txt", "to_delete.txt"])
        XCTAssertEqual(undoSummary.removedFiles, ["created.txt", "nested/deep/file.txt"])
        XCTAssertEqual(undoSummary.remainingUndo, 0)
        XCTAssertEqual(undoSummary.remainingRedo, 1)

        XCTAssertEqual(try String(contentsOf: existingURL, encoding: .utf8), "v1\n")
        XCTAssertEqual(try String(contentsOf: toDeleteURL, encoding: .utf8), "keep me\n")
        XCTAssertEqual(try String(contentsOf: stagedURL, encoding: .utf8), "working-tree-v2\n")
        XCTAssertFalse(FileManager.default.fileExists(atPath: createdURL.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: nestedFileURL.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: repo.appendingPathComponent("nested").path))
        XCTAssertEqual(
            try runGit(["diff", "--cached"], at: repo),
            cachedDiffBefore,
            "Shadow-index undo must never modify the user's real staged index."
        )
        XCTAssertEqual(
            try runGit(["rev-list", "--count", "HEAD"], at: repo).trimmingCharacters(in: .whitespacesAndNewlines),
            commitCountBefore
        )

        // Redo turn checkpoint losslessly.
        let redoSummary = try await service.redoTurnCheckpoint(at: repo)
        XCTAssertEqual(redoSummary.restoredFiles, ["created.txt", "existing.txt", "nested/deep/file.txt"])
        XCTAssertEqual(redoSummary.removedFiles, ["to_delete.txt"])
        XCTAssertEqual(redoSummary.remainingUndo, 1)
        XCTAssertEqual(redoSummary.remainingRedo, 0)

        XCTAssertEqual(try String(contentsOf: existingURL, encoding: .utf8), "v2-modified\n")
        XCTAssertFalse(FileManager.default.fileExists(atPath: toDeleteURL.path))
        XCTAssertEqual(try String(contentsOf: createdURL, encoding: .utf8), "brand new\n")
        XCTAssertEqual(try String(contentsOf: nestedFileURL, encoding: .utf8), "deep new\n")
        XCTAssertEqual(
            try runGit(["diff", "--cached"], at: repo),
            cachedDiffBefore,
            "Shadow-index redo must never modify the user's real staged index."
        )

        // Second undo confirms lossless repeatability.
        let secondUndo = try await service.undoTurnCheckpoint(at: repo)
        XCTAssertEqual(secondUndo.remainingUndo, 0)
        XCTAssertEqual(secondUndo.remainingRedo, 1)
        XCTAssertEqual(try String(contentsOf: existingURL, encoding: .utf8), "v1\n")
        XCTAssertEqual(try String(contentsOf: toDeleteURL, encoding: .utf8), "keep me\n")
        XCTAssertFalse(FileManager.default.fileExists(atPath: createdURL.path))
    }

    func testParseWorktreesPorcelainHandlesMainLinkedAndDetached() {
        let porcelain = """
        worktree /Users/dev/project
        HEAD 1111111111111111111111111111111111111111
        branch refs/heads/main

        worktree /Users/dev/project-feat-auth
        HEAD 2222222222222222222222222222222222222222
        branch refs/heads/feat/auth

        worktree /Users/dev/project-detached
        HEAD 3333333333333333333333333333333333333333
        detached

        """
        let worktrees = MacWorkspaceGitService.parseWorktrees(
            porcelain,
            currentRepositoryURL: URL(fileURLWithPath: "/Users/dev/project-feat-auth", isDirectory: true)
        )
        XCTAssertEqual(worktrees.count, 3)

        XCTAssertEqual(worktrees[0].path.path, "/Users/dev/project")
        XCTAssertEqual(worktrees[0].branch, "main")
        XCTAssertTrue(worktrees[0].isMain)
        XCTAssertFalse(worktrees[0].isCurrent)

        XCTAssertEqual(worktrees[1].path.path, "/Users/dev/project-feat-auth")
        XCTAssertEqual(worktrees[1].branch, "feat/auth")
        XCTAssertFalse(worktrees[1].isMain)
        XCTAssertTrue(worktrees[1].isCurrent)

        XCTAssertEqual(worktrees[2].path.path, "/Users/dev/project-detached")
        XCTAssertNil(worktrees[2].branch)
        XCTAssertEqual(worktrees[2].displayBranch, "3333333")
        XCTAssertFalse(worktrees[2].isMain)
        XCTAssertFalse(worktrees[2].isCurrent)
    }

    func testBranchValidationAndWorktreeCreationEndToEnd() async throws {
        for invalid in ["", "   ", "-leading", ".dot", "trail.", "a..b", "a//b", "bad name", "ref.lock", "foo@{1}", "x~1"] {
            XCTAssertThrowsError(try MacWorkspaceGitService.validatedBranchName(invalid), "Should reject \(invalid)")
        }
        XCTAssertEqual(try MacWorkspaceGitService.validatedBranchName("  feat/worktree-ui  "), "feat/worktree-ui")

        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-worktree-test-\(UUID().uuidString)", isDirectory: true)
        let repo = root.appendingPathComponent("app", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: repo, withIntermediateDirectories: true)

        _ = try runGit(["init", "-b", "main"], at: repo)
        _ = try runGit(["config", "user.name", "Nola Tests"], at: repo)
        _ = try runGit(["config", "user.email", "nola-tests@example.invalid"], at: repo)
        try Data("hello\n".utf8).write(to: repo.appendingPathComponent("README.md"))
        _ = try runGit(["add", "-A"], at: repo)
        _ = try runGit(["commit", "-m", "initial"], at: repo)

        let service = MacWorkspaceGitService()
        let initialWorktrees = try await service.listWorktrees(at: repo)
        XCTAssertEqual(initialWorktrees.count, 1)
        XCTAssertEqual(initialWorktrees.first?.branch, "main")
        XCTAssertEqual(initialWorktrees.first?.isMain, true)
        XCTAssertEqual(initialWorktrees.first?.isCurrent, true)

        let created = try await service.createWorktree(at: repo, branchName: "feat/plan-mode")
        XCTAssertEqual(created.branch, "feat/plan-mode")
        XCTAssertFalse(created.isMain)
        XCTAssertTrue(created.isCurrent)
        XCTAssertEqual(created.path.lastPathComponent, "app-feat-plan-mode")
        XCTAssertTrue(FileManager.default.fileExists(atPath: created.path.appendingPathComponent("README.md").path))

        let listedFromNew = try await service.listWorktrees(at: created.path)
        XCTAssertEqual(listedFromNew.count, 2)
        XCTAssertEqual(listedFromNew.filter(\.isMain).count, 1)
        XCTAssertEqual(listedFromNew.filter(\.isCurrent).map(\.branch), ["feat/plan-mode"])
    }

    func testPlanModeReadOnlyRulesAndComposerContract() throws {
        let defaultState = MacComposerDisplayState(
            conversationID: "conv-1",
            draft: "inspect architecture",
            attachments: [],
            isRunning: false,
            isVisiblyRunning: false
        )
        XCTAssertFalse(defaultState.isPlanMode)

        let rules = MacPlanModePolicy.readOnlyPermissionRules
        XCTAssertTrue(rules.allSatisfy { $0.decision == "deny" && $0.source == "host" && $0.resourcePattern == "*" })
        let denied = Set(rules.map(\.capabilityPattern))
        for mutating in ["edit", "delete_file", "bash", "git_commit", "git_worktree_create", "workspace_checkpoint"] {
            XCTAssertTrue(denied.contains(mutating), "Plan mode must deny \(mutating)")
        }
        XCTAssertFalse(denied.contains("read"), "Plan mode must preserve read-only inspection")

        let request = try MacLocalAgentRunRequest(
            conversationID: "conv-1",
            prompt: "Design the refactor",
            projectURL: FileManager.default.temporaryDirectory,
            attachmentURLs: [],
            modelTestMode: true,
            permissionMode: .allowAll,
            isPlanMode: true
        )
        XCTAssertTrue(request.isPlanMode)
    }

    private func runGit(_ arguments: [String], at directory: URL) throws -> String {
        let process = Process()
        let output = Pipe()
        let error = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = arguments
        process.currentDirectoryURL = directory
        process.standardOutput = output
        process.standardError = error
        try process.run()
        process.waitUntilExit()
        let stdout = output.fileHandleForReading.readDataToEndOfFile()
        let stderr = error.fileHandleForReading.readDataToEndOfFile()
        guard process.terminationStatus == 0 else {
            throw NSError(
                domain: "MacDeveloperWorkflowTests",
                code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: String(decoding: stderr, as: UTF8.self)]
            )
        }
        return String(decoding: stdout, as: UTF8.self)
    }
}
