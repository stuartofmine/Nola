import Foundation
import XCTest
@testable import Nola

final class MacLocalAgentResumeAuthorizationTests: XCTestCase {
    func testExternalRecoveryUsesFrozenIdentityWithoutHTTPProfile() throws {
        let route: [String: Any] = ["kind": "external", "backend": "codex",
            "model": "gpt-6.1-sol", "provider": NSNull()]
        let identity = try XCTUnwrap(MacFrozenExternalExecutorIdentity.resolve(primary: route,
            modelProfileID: nil, fallbackCount: 0))
        XCTAssertEqual(identity.backend, "codex")
        XCTAssertEqual(identity.model, "gpt-6.1-sol")
        XCTAssertNil(identity.provider)
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: route,
            modelProfileID: "http-profile", fallbackCount: 0))
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: route,
            modelProfileID: nil, fallbackCount: 1))
        var invalid = route
        invalid["backend"] = "nola"
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: invalid,
            modelProfileID: nil, fallbackCount: 0))
        invalid = route
        invalid["endpoint"] = "https://unexpected.invalid"
        XCTAssertThrowsError(try MacFrozenExternalExecutorIdentity.resolve(primary: invalid,
            modelProfileID: nil, fallbackCount: 0))
        XCTAssertNil(try MacFrozenExternalExecutorIdentity.resolve(primary: ["kind": "http"],
            modelProfileID: "profile", fallbackCount: 0))
    }

    func testOriginalScopeRemainsRequiredAndNewGrantDoesNotReplaceIt() throws {
        let project = URL(fileURLWithPath: "/fixture/project")
        let original = URL(fileURLWithPath: "/fixture/original")
        let added = URL(fileURLWithPath: "/fixture/added")
        let frozen = MacLocalAgentFrozenAuthorization(projectURL: project,
            authorizedWorkspaceURLs: [original], permissionMode: .cautious)
        XCTAssertNoThrow(try frozen.validate(.init(projectURL: project,
            authorizedWorkspaceURLs: [original, added], permissionMode: .cautious)))
        XCTAssertThrowsError(try frozen.validate(.init(projectURL: project,
            authorizedWorkspaceURLs: [added], permissionMode: .cautious)))
        XCTAssertThrowsError(try frozen.validate(.init(projectURL: added,
            authorizedWorkspaceURLs: [project, original], permissionMode: .cautious)))
        XCTAssertNoThrow(try frozen.validate(.init(projectURL: project,
            authorizedWorkspaceURLs: [original], permissionMode: .cautious)))
        XCTAssertNoThrow(try frozen.validate(.init(projectURL: project,
            authorizedWorkspaceURLs: [original], permissionMode: .allowAll)))
    }

    func testModeForNextMessageDoesNotWidenFrozenRunOrIgnoreRevocation() throws {
        let project = URL(fileURLWithPath: "/fixture/project")
        func descriptor(_ mode: String) -> MacFrozenLaunchDescriptor {
            MacFrozenLaunchDescriptor(version: 1, input_origin: "interactive_user",
            project_path: project.path, authorized_workspace_paths: [], allowed_directory_arguments: [],
            skill_directory_arguments: [], active_skill_ids: [], model_profile_id: nil,
            model_profile_revision: nil, fallback_profile_ids: [], permission_mode: mode,
            memory_enabled: true, context_window_tokens: nil, max_iterations: 0,
            model_capabilities: [:], vision_input: false, permission_rules: [], host_attachments: [])
        }
        let cautious = descriptor("cautious")
        XCTAssertThrowsError(try descriptor("allow_all").validate(.init(projectURL: project, authorizedWorkspaceURLs: [], permissionMode: .cautious)))
        XCTAssertThrowsError(try descriptor("unsupported").validate(.init(projectURL: project, authorizedWorkspaceURLs: [], permissionMode: .allowAll)))
        try cautious.validate(.init(projectURL: project, authorizedWorkspaceURLs: [], permissionMode: .allowAll))
        XCTAssertEqual(cautious.permission_mode, "cautious", "Recovery must execute with the original mode")
        let full = MacLocalAgentFrozenAuthorization(projectURL: project,
            authorizedWorkspaceURLs: [], permissionMode: .allowAll)
        XCTAssertThrowsError(try full.validate(.init(projectURL: project,
            authorizedWorkspaceURLs: [], permissionMode: .cautious)))
        XCTAssertThrowsError(try cautious.validate(.init(projectURL: project.appendingPathComponent("other"),
            authorizedWorkspaceURLs: [project], permissionMode: .allowAll)))
    }

    func testRetargetedOriginalSymlinkCannotReuseFrozenLaunchArguments() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let original = root.appendingPathComponent("original")
        let replacement = root.appendingPathComponent("replacement")
        let link = root.appendingPathComponent("granted-link")
        for url in [original, replacement] {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        }
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: original)
        let frozen = MacLocalAgentFrozenAuthorization(projectURL: root,
            authorizedWorkspaceURLs: [link], permissionMode: .cautious)
        XCTAssertNoThrow(try frozen.validate(.init(projectURL: root,
            authorizedWorkspaceURLs: [link], permissionMode: .cautious)))
        try FileManager.default.removeItem(at: link)
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: replacement)
        XCTAssertThrowsError(try frozen.validate(.init(projectURL: root,
            authorizedWorkspaceURLs: [original, replacement, link], permissionMode: .cautious)),
            "Even granting both destinations must not retarget the original launch's --allow-dir")
    }
}
