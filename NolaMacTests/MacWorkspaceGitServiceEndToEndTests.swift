// SPDX-License-Identifier: Apache-2.0

import Foundation
import AppKit
import SwiftUI
import XCTest
@testable import Nola

final class MacWorkspaceGitServiceEndToEndTests: XCTestCase {
    func testSuggestCommitAndFirstPushRoundTripThroughRealRepositories() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-git-e2e-\(UUID().uuidString)", isDirectory: true)
        let repository = root.appendingPathComponent("work", isDirectory: true)
        let remote = root.appendingPathComponent("remote.git", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
        _ = try runGit(["init", "--bare", remote.path], at: root)
        _ = try runGit(["init", "-b", "main"], at: repository)
        _ = try runGit(["config", "user.name", "Nola Tests"], at: repository)
        _ = try runGit(["config", "user.email", "nola-tests@example.invalid"], at: repository)
        _ = try runGit(["remote", "add", "origin", remote.path], at: repository)
        try Data("# Release notes\n".utf8).write(to: repository.appendingPathComponent("README.md"))

        let snapshot = try await MacWorkspaceGitService.shared.snapshot(at: repository)
        XCTAssertEqual(snapshot.changes.map(\.path), ["README.md"])
        XCTAssertFalse(snapshot.hasCommits)
        XCTAssertTrue(snapshot.hasOriginRemote)
        XCTAssertFalse(snapshot.canPush)
        let suggestion = await MacWorkspaceGitService.shared.suggestedCommitMessage(for: snapshot)
        XCTAssertTrue(suggestion.hasPrefix("feat: "))
        XCTAssertTrue(suggestion.contains("README.md"))

        let commit = try await MacWorkspaceGitService.shared.commitAll(message: suggestion, in: snapshot)
        XCTAssertEqual(commit.title, String(localized: "Changes committed"))
        let clean = try await MacWorkspaceGitService.shared.snapshot(at: repository)
        XCTAssertTrue(clean.changes.isEmpty)
        XCTAssertTrue(clean.hasCommits)
        XCTAssertNil(clean.upstreamName)
        XCTAssertTrue(clean.canPush)

        let push = try await MacWorkspaceGitService.shared.push(clean)
        XCTAssertEqual(push.title, String(localized: "Push completed"))
        let published = try await MacWorkspaceGitService.shared.snapshot(at: repository)
        XCTAssertEqual(published.upstreamName, "origin/main")
        XCTAssertEqual(published.aheadCount, 0)
        XCTAssertEqual(published.behindCount, 0)
        XCTAssertFalse(published.canPush)
        let remoteSubject = try runGit(["--git-dir", remote.path, "log", "-1", "--pretty=%s"], at: root)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        XCTAssertEqual(remoteSubject, suggestion)

        let sourceDirectory = repository.appendingPathComponent("NolaMac/UI", isDirectory: true)
        let testDirectory = repository.appendingPathComponent("NolaMacTests", isDirectory: true)
        try FileManager.default.createDirectory(at: sourceDirectory, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: testDirectory, withIntermediateDirectories: true)
        try Data("struct ConversationInspector {}\n".utf8).write(
            to: sourceDirectory.appendingPathComponent("ConversationInspector.swift")
        )
        try Data("final class ConversationInspectorTests {}\n".utf8).write(
            to: testDirectory.appendingPathComponent("ConversationInspectorTests.swift")
        )

        let sourceAndTests = try await MacWorkspaceGitService.shared.snapshot(at: repository)
        let sourceSuggestion = await MacWorkspaceGitService.shared.suggestedCommitMessage(for: sourceAndTests)
        XCTAssertTrue(sourceSuggestion.hasPrefix("feat: "))
        XCTAssertTrue(sourceSuggestion.contains("Mac"))
        XCTAssertTrue(sourceSuggestion.contains(String(localized: "test coverage")))
        _ = try await MacWorkspaceGitService.shared.commitAll(message: sourceSuggestion, in: sourceAndTests)
        let readyToPush = try await MacWorkspaceGitService.shared.snapshot(at: repository)
        XCTAssertTrue(readyToPush.canPush)
        _ = try await MacWorkspaceGitService.shared.push(readyToPush)
        let secondRemoteSubject = try runGit(
            ["--git-dir", remote.path, "log", "-1", "--pretty=%s"],
            at: root
        ).trimmingCharacters(in: .whitespacesAndNewlines)
        XCTAssertEqual(secondRemoteSubject, sourceSuggestion)
    }

    @MainActor
    func testWorkspaceReviewRendersProductionUIAgainstRealRepository() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-git-ui-e2e-\(UUID().uuidString)", isDirectory: true)
        let repository = root.appendingPathComponent("work", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
        _ = try runGit(["init", "-b", "main"], at: repository)
        _ = try runGit(["config", "user.name", "Nola UI Tests"], at: repository)
        _ = try runGit(["config", "user.email", "nola-ui-tests@example.invalid"], at: repository)
        try Data("# Workspace review\n\nA real changed file.\n".utf8)
            .write(to: repository.appendingPathComponent("README.md"))

        let size = CGSize(width: 1_080, height: 720)
        let rootView = MacWorkspaceReviewView(route: MacWorkspaceReviewRoute(path: repository.path))
            .frame(width: size.width, height: size.height)
            .environment(\.colorScheme, .dark)
        let hostingView = NSHostingView(rootView: rootView)
        hostingView.appearance = NSAppearance(named: .darkAqua)
        hostingView.frame = CGRect(origin: .zero, size: size)
        let window = NSWindow(
            contentRect: CGRect(origin: .zero, size: size),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.appearance = hostingView.appearance
        window.isReleasedWhenClosed = false
        window.contentView = hostingView
        window.makeKeyAndOrderFront(nil)
        defer {
            window.orderOut(nil)
            window.close()
        }

        try await Task.sleep(for: .seconds(1))
        hostingView.layoutSubtreeIfNeeded()
        let representation = try XCTUnwrap(
            hostingView.bitmapImageRepForCachingDisplay(in: hostingView.bounds)
        )
        hostingView.cacheDisplay(in: hostingView.bounds, to: representation)
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        let screenshot = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        screenshot.name = "Mac Git workspace review with real repository"
        screenshot.lifetime = .keepAlways
        add(screenshot)

        XCTAssertGreaterThan(png.count, 35_000, "The production workspace review rendered as an empty surface.")
        XCTAssertLessThan(
            brightPixelRatio(in: representation, xRange: 0..<(representation.pixelsWide / 3)),
            0.45,
            "The dark-mode repository sidebar must not render as an empty white sheet."
        )
        XCTAssertLessThan(
            firstGreenContentRow(in: representation) ?? representation.pixelsHigh,
            representation.pixelsHigh / 3,
            "A short diff should start at the top instead of floating in the vertical center."
        )
        let snapshot = try await MacWorkspaceGitService.shared.snapshot(at: repository)
        XCTAssertEqual(snapshot.changes.map(\.path), ["README.md"])
        let suggestion = await MacWorkspaceGitService.shared.suggestedCommitMessage(for: snapshot)
        let localizedSubject = String.localizedStringWithFormat(
            String(localized: "add %@"),
            "README.md"
        )
        XCTAssertEqual(suggestion, "feat: \(localizedSubject)")
    }

    private func brightPixelRatio(in bitmap: NSBitmapImageRep, xRange: Range<Int>) -> Double {
        var samples = 0
        var bright = 0
        for x in stride(from: xRange.lowerBound, to: xRange.upperBound, by: 4) {
            for y in stride(from: 0, to: bitmap.pixelsHigh, by: 4) {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                samples += 1
                let luminance = 0.2126 * color.redComponent
                    + 0.7152 * color.greenComponent
                    + 0.0722 * color.blueComponent
                if luminance > 0.82 { bright += 1 }
            }
        }
        return samples == 0 ? 0 : Double(bright) / Double(samples)
    }

    private func firstGreenContentRow(in bitmap: NSBitmapImageRep) -> Int? {
        let startX = bitmap.pixelsWide / 3
        for y in 0..<bitmap.pixelsHigh {
            for x in stride(from: startX, to: bitmap.pixelsWide, by: 3) {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                if color.greenComponent > color.redComponent * 1.18,
                   color.greenComponent > color.blueComponent * 1.08,
                   color.greenComponent > 0.2 {
                    return y
                }
            }
        }
        return nil
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
                domain: "MacWorkspaceGitServiceEndToEndTests",
                code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: String(decoding: stderr, as: UTF8.self)]
            )
        }
        return String(decoding: stdout, as: UTF8.self)
    }
}
